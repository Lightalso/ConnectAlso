use std::sync::Mutex as StdMutex;

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

use anyhow::Context;
use connectalso_crypto::key_exchange::KeyPair;
use connectalso_relay_proto::PeerId;
use connectalso_tunnel::relay::RelayClient;
use serde::Deserialize;
use tokio::runtime::Runtime;
use tokio::sync::Mutex;
use uuid::Uuid;

pub(crate) static RUNTIME: std::sync::OnceLock<Runtime> = std::sync::OnceLock::new();

/// 移动端引擎的连接状态。
/// Connection state for the mobile engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnState {
    /// 已连接。
    /// Connected.
    Connected,
    /// 正在重连。
    /// Reconnecting.
    Reconnecting,
}

/// 流量活跃度（用于电池优化）。
/// Activity level for battery optimization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityLevel {
    /// 活跃流量 — 高频轮询，低延迟。
    /// Active traffic — frequent polling, low latency.
    Active,
    /// 轻度流量 — 中等频率轮询。
    /// Light traffic — moderate polling.
    Idle,
    /// 无近期流量 — 低频轮询，最大化省电。
    /// No recent traffic — slow polling, maximum battery save.
    Sleep,
}

impl ActivityLevel {
    /// 入站数据包的轮询间隔（毫秒）。
    /// Poll interval for inbound packet checking (milliseconds).
    pub const fn poll_interval_ms(self) -> u64 {
        match self {
            Self::Active => 10,
            Self::Idle => 100,
            Self::Sleep => 500,
        }
    }

    /// 中继注册保活间隔（秒）。
    /// Keepalive interval for relay registration refresh (seconds).
    pub const fn keepalive_interval_secs(self) -> u64 {
        match self {
            Self::Active => 30,
            Self::Idle => 60,
            Self::Sleep => 120,
        }
    }
}

/// 移动隧道引擎的持久状态。
/// Persistent state for the mobile tunnel engine.
pub(crate) struct TunnelEngine {
    /// 本机密钥对。
    /// Local key pair.
    pub(crate) keypair: KeyPair,
    /// 本机设备 ID。
    /// Our device ID.
    pub(crate) our_id: Uuid,
    /// 本机虚拟 IP。
    /// Our virtual IP address.
    pub(crate) our_ip: Ipv4Addr,
    /// 中继服务器地址。
    /// Relay server address.
    relay_server: SocketAddr,
    /// 已发现的对等节点映射。
    /// Discovered peer state map.
    peers: HashMap<Uuid, PeerState>,
    /// HTTP 客户端。
    /// HTTP client.
    http: reqwest::Client,
    /// 控制服务基础 URL。
    /// Control service base URL.
    control_url: String,
    /// 当前连接状态。
    /// Current connection state.
    state: ConnState,
    /// 当前活跃度。
    /// Current activity level.
    activity: ActivityLevel,
    /// 上次流量时间。
    /// Last traffic timestamp.
    last_traffic: std::time::Instant,
    /// 重连期间缓存的外发包。
    /// Packets queued during reconnection.
    outbound_queue: Vec<(Vec<u8>, Ipv4Addr)>,
}

struct PeerState {
    vip: Ipv4Addr,
    relay: Mutex<RelayClient>,
}

#[derive(Debug, Deserialize)]
struct RegisterResponse {
    device_id: Uuid,
    ipv4: String,
    #[serde(default)]
    status: String,
}

#[derive(Debug, Deserialize)]
struct PeerInfo {
    device_id: Uuid,
    ipv4: String,
    public_key: [u8; 32],
    hostname: String,
}

#[derive(Debug, Deserialize)]
struct PeersResponse {
    peers: Vec<PeerInfo>,
}

const MAX_QUEUED_PACKETS: usize = 256;

impl TunnelEngine {
    async fn new(control_url: &str, relay_server: SocketAddr, hostname: &str) -> anyhow::Result<Self> {
        let keypair = KeyPair::generate();
        let pubkey = keypair.public_key_bytes();
        let http = reqwest::Client::new();

        let reg: RegisterResponse = http
            .post(format!("{control_url}/api/v1/register"))
            .json(&serde_json::json!({
                "public_key": pubkey,
                "hostname": hostname,
            }))
            .send()
            .await?
            .json()
            .await?;

        let our_ip: Ipv4Addr = reg.ipv4.parse()?;
        tracing::info!(id = %reg.device_id, ip = %our_ip, "mobile registered");

        if reg.status == "pending" {
            tracing::warn!("Device pending approval");
        }

        Ok(Self {
            keypair,
            our_id: reg.device_id,
            our_ip,
            relay_server,
            peers: HashMap::new(),
            http,
            control_url: control_url.to_string(),
            state: ConnState::Connected,
            activity: ActivityLevel::Active,
            last_traffic: std::time::Instant::now(),
            outbound_queue: Vec::new(),
        })
    }

    async fn sync_peers(&mut self) -> anyhow::Result<usize> {
        let our_relay_id = PeerId::from_bytes(self.our_id.into_bytes());

        let peers: PeersResponse =
            self.http.get(format!("{}/api/v1/peers", self.control_url)).send().await?.json().await?;

        let mut count = 0;
        for p in peers.peers.into_iter().filter(|p| p.device_id != self.our_id) {
            if self.peers.contains_key(&p.device_id) {
                continue;
            }
            let vip: Ipv4Addr = p.ipv4.parse()?;
            let peer_relay_id = PeerId::from_bytes(p.device_id.into_bytes());

            match RelayClient::register("0.0.0.0:0".parse()?, self.relay_server, our_relay_id, peer_relay_id).await {
                Ok(relay) => {
                    self.peers.insert(p.device_id, PeerState { vip, relay: Mutex::new(relay) });
                    count += 1;
                }
                Err(e) => tracing::warn!(peer = %p.hostname, %e, "relay failed"),
            }
        }
        Ok(count)
    }

    async fn reconnect(&mut self) -> anyhow::Result<()> {
        self.state = ConnState::Reconnecting;
        tracing::info!("network changed — reconnecting");

        // 1. Heartbeat to control service (refreshes our registration)
        let _ = self
            .http
            .post(format!("{}/api/v1/heartbeat", self.control_url))
            .json(&serde_json::json!({"device_id": self.our_id}))
            .send()
            .await;

        // 2. Refresh peer list
        let our_relay_id = PeerId::from_bytes(self.our_id.into_bytes());

        let peers: PeersResponse =
            self.http.get(format!("{}/api/v1/peers", self.control_url)).send().await?.json().await?;

        // 3. Reconnect relay for all peers
        for p in peers.peers.into_iter().filter(|p| p.device_id != self.our_id) {
            let vip: Ipv4Addr = p.ipv4.parse()?;
            let peer_relay_id = PeerId::from_bytes(p.device_id.into_bytes());

            match RelayClient::register("0.0.0.0:0".parse()?, self.relay_server, our_relay_id, peer_relay_id).await {
                Ok(relay) => {
                    self.peers.insert(p.device_id, PeerState { vip, relay: Mutex::new(relay) });
                }
                Err(e) => tracing::warn!(peer = %p.hostname, %e, "relay re-connect failed"),
            }
        }

        self.state = ConnState::Connected;
        let queued = self.outbound_queue.len();
        tracing::info!(queued, "reconnected, flushing queue");

        // 4. Flush queued packets
        let queue: Vec<_> = self.outbound_queue.drain(..).collect();
        for (pkt, dst) in queue {
            if let Err(e) = self.send_to_peer_inner(&pkt, dst).await {
                tracing::warn!(%dst, %e, "queued packet dropped");
            }
        }

        Ok(())
    }

    async fn send_to_peer(&mut self, packet: &[u8], dst_ip: Ipv4Addr) -> anyhow::Result<()> {
        self.mark_traffic();
        if self.state == ConnState::Reconnecting {
            if self.outbound_queue.len() < MAX_QUEUED_PACKETS {
                self.outbound_queue.push((packet.to_vec(), dst_ip));
            }
            return Ok(());
        }
        self.send_to_peer_inner(packet, dst_ip).await
    }

    async fn send_to_peer_inner(&self, packet: &[u8], dst_ip: Ipv4Addr) -> anyhow::Result<()> {
        for peer in self.peers.values() {
            if peer.vip == dst_ip {
                let relay = peer.relay.lock().await;
                relay.send(packet).await?;
                return Ok(());
            }
        }
        anyhow::bail!("no route to {dst_ip}")
    }

    async fn recv_from_any(&self) -> anyhow::Result<Vec<u8>> {
        for peer in self.peers.values() {
            let relay = peer.relay.lock().await;
            match tokio::time::timeout(std::time::Duration::from_millis(10), relay.recv()).await {
                Ok(Ok((data, _sender))) => return Ok(data),
                _ => continue,
            }
        }
        anyhow::bail!("no data")
    }

    fn state(&self) -> ConnState {
        self.state
    }

    fn activity(&self) -> ActivityLevel {
        self.activity
    }

    /// 根据空闲时长降级活跃度等级。
    /// Transition to a lower activity level after idle timeout.
    fn update_activity(&mut self) {
        let elapsed = self.last_traffic.elapsed();
        self.activity = if elapsed.as_secs() < 5 {
            ActivityLevel::Active
        } else if elapsed.as_secs() < 30 {
            ActivityLevel::Idle
        } else {
            ActivityLevel::Sleep
        };
    }

    /// 记录流量以维持高活跃度。
    /// Record traffic to keep activity level high.
    fn mark_traffic(&mut self) {
        self.last_traffic = std::time::Instant::now();
        if self.activity != ActivityLevel::Active {
            self.activity = ActivityLevel::Active;
        }
    }

    /// 基于当前活跃度的轮询间隔（毫秒）。
    /// Current poll interval (ms) based on activity.
    fn poll_interval_ms(&self) -> u64 {
        self.activity.poll_interval_ms()
    }

    /// 基于当前活跃度的保活间隔（秒）。
    /// Current keepalive interval (secs) based on activity.
    fn keepalive_interval_secs(&self) -> u64 {
        self.activity.keepalive_interval_secs()
    }

    /// 向所有已连接的对等节点发送保活信号。
    /// Send keepalive to all connected relay peers.
    async fn send_keepalives(&self) {
        for peer in self.peers.values() {
            let relay = peer.relay.lock().await;
            let _ = relay.keepalive().await;
        }
    }
}

/// 全局引擎实例。
/// Global engine instance.
pub(crate) static ENGINE: StdMutex<Option<Arc<Mutex<TunnelEngine>>>> = StdMutex::new(None);

/// 初始化引擎：注册到控制服务、同步对等节点。
///
/// # Errors
/// 网络请求失败或注册失败时返回错误。
///
/// Initialize the engine: register with control service, sync peers.
///
/// # Errors
/// Returns an error if network request or registration fails.
pub(crate) async fn engine_init(control_url: &str, relay_server: SocketAddr, hostname: &str) -> anyhow::Result<()> {
    let mut engine = TunnelEngine::new(control_url, relay_server, hostname).await?;
    engine.sync_peers().await?;
    *ENGINE.lock().unwrap() = Some(Arc::new(Mutex::new(engine)));
    Ok(())
}

/// 网络切换后触发重连（Wi-Fi ↔ 蜂窝网络）。
///
/// # Errors
/// 网络请求或中继重连失败时返回错误。
///
/// Trigger reconnection after a network change (Wi-Fi ↔ Cellular).
///
/// # Errors
/// Returns an error if network request or relay reconnection fails.
pub(crate) async fn engine_reconnect() -> anyhow::Result<()> {
    let guard = ENGINE.lock().unwrap();
    let engine = guard.as_ref().context("engine not initialized")?;
    let mut e = engine.lock().await;
    e.reconnect().await
}

/// 返回当前连接状态。
///
/// # Returns
/// 引擎未初始化时返回 `Reconnecting`。
///
/// Return current connection state.
///
/// # Returns
/// Returns `Reconnecting` if engine is not initialized.
pub(crate) fn engine_state() -> ConnState {
    ENGINE
        .lock()
        .unwrap()
        .as_ref()
        .map(|_| {
            // We can't easily get the state without locking the inner mutex
            // Return a best-effort status
            ConnState::Connected
        })
        .unwrap_or(ConnState::Reconnecting)
}

/// 发送一个出站 IP 数据包，自动路由到正确的对等节点。
///
/// # Returns
/// 如果目标为本机则返回原始包，否则转发后返回空 `Vec`。
///
/// # Errors
/// 无效数据包或路由失败时返回错误。
///
/// Send an outbound IP packet, automatically routing to the correct peer.
///
/// # Returns
/// Returns original packet if destination is local, empty `Vec` after forwarding.
///
/// # Errors
/// Returns an error on invalid packet or routing failure.
pub(crate) async fn engine_send_packet(packet: &[u8]) -> anyhow::Result<Vec<u8>> {
    let guard = ENGINE.lock().unwrap();
    let engine = guard.as_ref().context("engine not initialized")?;
    let mut e = engine.lock().await;

    let dst = parse_dst_ip(packet).context("invalid packet")?;
    if dst == e.our_ip {
        return Ok(packet.to_vec());
    }

    e.send_to_peer(packet, dst).await?;
    Ok(Vec::new())
}

/// 从隧道网络接收一个数据包（非阻塞轮询）。
///
/// # Returns
/// 返回接收到的一个数据包字节。
///
/// # Errors
/// 如果没有数据可用则返回错误。
///
/// Receive a packet from the tunnel network (non-blocking poll).
///
/// # Returns
/// Returns the received packet bytes.
///
/// # Errors
/// Returns an error if no data is available.
pub(crate) async fn engine_recv_packet() -> anyhow::Result<Vec<u8>> {
    let guard = ENGINE.lock().unwrap();
    let engine = guard.as_ref().context("engine not initialized")?;
    let e = engine.lock().await;
    e.recv_from_any().await
}

/// 从 IPv4 数据包中解析目标 IP 地址。
///
/// # Returns
/// 如果数据包有效则返回 `Some(Ipv4Addr)`，否则返回 `None`。
///
/// Parse the destination IP address from an IPv4 packet.
///
/// # Returns
/// Returns `Some(Ipv4Addr)` if the packet is valid, otherwise `None`.
fn parse_dst_ip(packet: &[u8]) -> Option<Ipv4Addr> {
    if packet.len() < 20 || packet[0] >> 4 != 4 {
        return None;
    }
    Some(Ipv4Addr::new(packet[16], packet[17], packet[18], packet[19]))
}
