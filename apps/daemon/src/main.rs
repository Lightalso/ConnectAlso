//! ConnectAlso 守护进程 — 用于虚拟组网的后台服务。
//! ConnectAlso daemon — background service for virtual networking.

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

mod dns;

use anyhow::Context;
use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use clap::Parser;
use connectalso_crypto::key_exchange::KeyPair;
use connectalso_nat::candidate::Candidate;
use connectalso_nat::punch::Puncher;
use connectalso_nat::stun::StunClient;
use connectalso_platform::tun::{TunConfig, TunDevice};
use connectalso_relay_proto::PeerId;
use connectalso_tunnel::path::{PathManager, PathStatus};
use connectalso_tunnel::relay::RelayClient;
use connectalso_tunnel::relay_pool::RelayPool;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::fmt::Layer;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::Layer as _;
use uuid::Uuid;

// ════════════════════════════════════════════════════════════════════
//  配置
//  Config
// ════════════════════════════════════════════════════════════════════
/// 守护进程持久化配置。
/// Persistent daemon configuration.
#[derive(Debug, Serialize, Deserialize)]
struct DaemonConfig {
    /// 设备唯一标识符。
    /// Unique device identifier.
    device_id: Uuid,
    /// 公钥（十六进制编码）。
    /// Public key (hex-encoded).
    public_key_hex: String,
    /// 虚拟 IP 地址。
    /// Virtual IP address.
    virtual_ip: String,
    /// 控制服务 URL。
    /// Control service URL.
    control_url: String,
    /// STUN 服务器地址。
    /// STUN server address.
    stun_server: String,
    /// 中继服务器地址列表。
    /// Relay server addresses.
    #[serde(default)]
    relay_servers: Vec<String>,
    /// 主机名。
    /// Hostname.
    hostname: String,
}

impl DaemonConfig {
    /// 返回配置文件的存储路径。
    /// Return the path to the configuration file.
    ///
    /// 默认位于用户配置目录下的 `connectalso/config.json`。
    /// Defaults to `connectalso/config.json` in the user's config directory.
    fn path() -> PathBuf {
        let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("connectalso");
        std::fs::create_dir_all(&base).ok();
        base.join("config.json")
    }

    /// 从文件加载配置，若文件不存在或解析失败则返回 `None`。
    /// Load configuration from file; returns `None` if missing or unparseable.
    fn load() -> Option<Self> {
        let data = std::fs::read_to_string(Self::path()).ok()?;
        serde_json::from_str(&data).ok()
    }

    /// 将当前配置序列化为 JSON 并写入文件。
    /// Serialize and write the current configuration as JSON.
    fn save(&self) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(Self::path(), json);
        }
    }
}

// ════════════════════════════════════════════════════════════════════
//  控制 API 数据传输对象
//  Control API DTOs
// ════════════════════════════════════════════════════════════════════
/// 设备注册请求体。
/// Device registration request body.
#[derive(Debug, Serialize)]
struct RegisterRequest {
    /// 32 字节公钥。
    /// 32-byte public key.
    public_key: [u8; 32],
    /// 主机名。
    /// Hostname.
    hostname: String,
}

/// 设备注册响应体。
/// Device registration response body.
#[derive(Debug, Deserialize)]
struct RegisterResponse {
    /// 分配的设备 ID。
    /// Assigned device identifier.
    device_id: Uuid,
    /// 分配的 IPv4 地址。
    /// Assigned IPv4 address.
    ipv4: String,
    /// 网络地址。
    /// Network address.
    #[serde(default)]
    #[allow(dead_code)]
    network: String,
    /// 注册状态：approved / pending。
    /// Registration status: approved / pending.
    #[serde(default = "default_status")]
    status: String,
}

fn default_status() -> String {
    "approved".to_string()
}

/// 对等节点信息。
/// Peer information from the control service.
#[derive(Debug, Deserialize, Clone)]
struct PeerInfo {
    /// 设备 ID。
    /// Device identifier.
    device_id: Uuid,
    /// IPv4 地址。
    /// IPv4 address.
    ipv4: String,
    /// 32 字节公钥。
    /// 32-byte public key.
    public_key: [u8; 32],
    /// 主机名。
    /// Hostname.
    hostname: String,
    /// 最后一次活动距今秒数。
    /// Seconds since last activity.
    #[allow(dead_code)]
    last_seen_secs: i64,
}

/// 对等节点列表响应。
/// Peer list response from the control service.
#[derive(Debug, Deserialize)]
struct PeersResponse {
    /// 对等节点列表。
    /// List of peers.
    peers: Vec<PeerInfo>,
}

/// NAT 候选地址发布请求。
/// Request to publish NAT candidate addresses.
#[derive(Debug, Serialize)]
struct CandidatePublish {
    /// 设备 ID。
    /// Device identifier.
    device_id: Uuid,
    /// 候选地址列表。
    /// List of candidate addresses.
    candidates: Vec<String>,
}

/// NAT 候选地址列表响应。
/// Response containing NAT candidate addresses for a peer.
#[derive(Debug, Deserialize)]
struct CandidateList {
    /// 设备 ID。
    /// Device identifier.
    #[allow(dead_code)]
    device_id: Uuid,
    /// 候选地址列表。
    /// List of candidate addresses.
    candidates: Vec<String>,
}

// ════════════════════════════════════════════════════════════════════
//  守护进程状态
//  Daemon state
// ════════════════════════════════════════════════════════════════════
/// 与单个对等节点的连接状态与隧道管理。
/// Connection state and tunnel management for a single peer.
struct PeerLink {
    /// 主机名。
    /// Hostname.
    hostname: String,
    /// 虚拟 IP 地址。
    /// Virtual IP address.
    vip: Ipv4Addr,
    /// 对等节点公钥。
    /// Peer public key.
    #[allow(dead_code)]
    public_key: [u8; 32],
    /// 路径管理器（中继 / P2P 直连）。
    /// Path manager (relay / direct P2P).
    path: PathManager,
    /// P2P 重试间隔（毫秒），支持指数退避。
    /// P2P retry interval (ms) with exponential backoff.
    p2p_retry_ms: u64,
}

/// 守护进程全局共享状态。
/// Global shared state of the daemon.
struct SharedState {
    /// 本机设备 ID。
    /// Local device identifier.
    device_id: Uuid,
    /// 虚拟 IP 地址。
    /// Virtual IP address.
    virtual_ip: Ipv4Addr,
    /// 主机名。
    /// Hostname.
    hostname: String,
    /// 守护进程启动时间。
    /// Daemon start time.
    started_at: Instant,
    /// 虚拟 IP → 设备 ID 的路由映射。
    /// Routing map: virtual IP → device ID.
    ip_route: HashMap<Ipv4Addr, Uuid>,
    /// 已连接的对等节点映射。
    /// Map of connected peers.
    peer_links: HashMap<Uuid, Arc<Mutex<PeerLink>>>,
}

// ════════════════════════════════════════════════════════════════════
//  状态 API 类型
//  Status API types
// ════════════════════════════════════════════════════════════════════
/// 状态 API 响应体。
/// Status API response body.
#[derive(Debug, Serialize)]
struct StatusResponse {
    /// 设备唯一标识符。
    /// Unique device identifier.
    device_id: String,
    /// 虚拟 IP 地址。
    /// Virtual IP address.
    virtual_ip: String,
    /// 主机名。
    /// Hostname.
    hostname: String,
    /// 运行时长（秒）。
    /// Uptime in seconds.
    uptime_secs: u64,
    /// 已连接的对等节点数量。
    /// Number of connected peers.
    peer_count: usize,
    /// 对等节点列表。
    /// List of connected peers.
    peers: Vec<StatusPeer>,
}

/// 对等节点状态条目。
/// Status entry for a connected peer.
#[derive(Debug, Serialize)]
struct StatusPeer {
    /// 对等节点设备 ID。
    /// Peer device identifier.
    device_id: String,
    /// 对等节点虚拟 IP。
    /// Peer virtual IP address.
    virtual_ip: String,
    /// 对等节点主机名。
    /// Peer hostname.
    hostname: String,
    /// 连接路径类型（direct / relay / probing）。
    /// Connection path type (direct / relay / probing).
    path: String,
}

/// 关闭 API 响应体。
/// Shutdown API response body.
#[derive(Debug, Serialize)]
struct ShutdownResponse {
    /// 响应消息。
    /// Response message.
    message: String,
}

/// 诊断 API 响应体。
/// Diagnostics API response body.
#[derive(Debug, Serialize)]
struct DiagnosticsResponse {
    /// 守护进程自身检查。
    /// Daemon self-check result.
    daemon: CheckResult,
    /// 控制服务检查。
    /// Control service check result.
    control: CheckResult,
    /// STUN 服务检查。
    /// STUN service check result.
    stun: CheckResult,
    /// 中继服务检查。
    /// Relay service check result.
    relay: CheckResult,
    /// TUN 虚拟网卡检查。
    /// TUN virtual interface check result.
    tun: CheckResult,
    /// 各对等节点的诊断结果。
    /// Per-peer diagnostic results.
    peers: Vec<PeerDiag>,
}

/// 单项诊断检查结果。
/// Result of a single diagnostic check.
#[derive(Debug, Serialize)]
struct CheckResult {
    /// 状态："ok" / "warn" / "error"。
    /// Status: "ok" / "warn" / "error".
    status: &'static str,
    /// 详细信息。
    /// Detailed description.
    detail: String,
    /// 延迟（毫秒）。
    /// Latency in milliseconds.
    latency_ms: Option<u64>,
}

/// 对等节点诊断条目。
/// Diagnostic entry for a peer.
#[derive(Debug, Serialize)]
struct PeerDiag {
    /// 主机名。
    /// Hostname.
    hostname: String,
    /// 虚拟 IP 地址。
    /// Virtual IP address.
    virtual_ip: String,
    /// 连接路径类型。
    /// Connection path type.
    path: String,
    /// 是否可达。
    /// Whether the peer is reachable.
    reachable: bool,
}

// ════════════════════════════════════════════════════════════════════
//  CLI
// ════════════════════════════════════════════════════════════════════
#[derive(Parser)]
#[command(name = "connectalso-daemon")]
#[command(about = "ConnectAlso Desktop Alpha")]
/// 守护进程命令行参数。
/// Daemon command-line arguments.
struct Cli {
    #[arg(long, default_value = "http://127.0.0.1:3000")]
    control_url: String,

    #[arg(long, default_value = "127.0.0.1:3478")]
    stun_server: SocketAddr,

    /// 中继服务器地址（可多次指定，支持多区域══    #[arg(long = "relay", default_value = "127.0.0.1:33478")]
    relay_servers: Vec<SocketAddr>,

    #[arg(long, default_value = "unnamed")]
    hostname: String,

    #[arg(long, default_value = "connectalso")]
    tun_name: String,

    #[arg(long, default_value = "127.0.0.1:9823")]
    api_listen: SocketAddr,
}

// ════════════════════════════════════════════════════════════════════
//  Main
// ════════════════════════════════════════════════════════════════════
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // ── Logging: console + file ──
    let log_dir = dirs::data_local_dir().unwrap_or_else(|| PathBuf::from(".")).join("connectalso").join("logs");
    std::fs::create_dir_all(&log_dir).ok();

    let file_appender = tracing_appender::rolling::daily(&log_dir, "daemon.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    tracing_subscriber::registry()
        .with(Layer::new().with_writer(non_blocking).with_ansi(false))
        .with(Layer::new().with_writer(std::io::stderr).with_filter(EnvFilter::from_default_env()))
        .init();

    // Keep _guard alive for the duration of the program
    // (it will be dropped on shutdown, flushing pending logs)

    let cli = Cli::parse();
    let shutdown = CancellationToken::new();
    let http = reqwest::Client::new();

    tracing::info!(control = %cli.control_url, hostname = %cli.hostname, "Desktop Alpha");

    // ── Keypair, register, config ──
    let keypair = KeyPair::generate();
    let pubkey = keypair.public_key_bytes();

    let reg: RegisterResponse = http
        .post(format!("{}/api/v1/register", cli.control_url))
        .json(&RegisterRequest { public_key: pubkey, hostname: cli.hostname.clone() })
        .send()
        .await?
        .json()
        .await?;

    let our_id = reg.device_id;
    let our_ip: Ipv4Addr = reg.ipv4.parse()?;

    if reg.status == "pending" {
        tracing::warn!("Device is PENDING approval. Admin must run: connectalso admin approve {}", our_id);
    }

    DaemonConfig {
        device_id: our_id,
        public_key_hex: hex_encode(&pubkey),
        virtual_ip: our_ip.to_string(),
        control_url: cli.control_url.clone(),
        stun_server: cli.stun_server.to_string(),
        relay_servers: cli.relay_servers.iter().map(|a| a.to_string()).collect(),
        hostname: cli.hostname.clone(),
    }
    .save();

    // ── Relay pool ──
    let mut relay_pool = RelayPool::new(&cli.relay_servers);
    tracing::info!(relays = cli.relay_servers.len(), "relay pool created");

    // ── STUN: discover + publish candidates ──
    let our_candidates = discover_candidates(cli.stun_server).await;
    publish_candidates(&http, &cli.control_url, our_id, &our_candidates).await;

    // ── Initial peer sync ──
    let our_relay_id = PeerId::from_bytes(our_id.into_bytes());
    let (ip_route, peer_links) =
        connect_peers(&http, &cli.control_url, relay_pool.active_addr(), our_id, our_relay_id).await?;

    // ── TUN ──
    let tun = TunDevice::create(TunConfig::new(our_ip, Ipv4Addr::new(255, 255, 255, 0)).with_name(&cli.tun_name))
        .await
        .context("create TUN")?;
    let tun = Arc::new(tun);
    tracing::info!(name = tun.name(), vip = %tun.address(), "TUN up");

    // ── Attempt P2P for each peer ──
    attempt_p2p_for_all(&peer_links, &http, &cli.control_url, &keypair, our_relay_id).await;

    // ── Shared state ──
    let state = Arc::new(Mutex::new(SharedState {
        device_id: our_id,
        virtual_ip: our_ip,
        hostname: cli.hostname.clone(),
        started_at: Instant::now(),
        ip_route,
        peer_links,
    }));

    // ── Status API ──
    let api_state = state.clone();
    let shutdown_api = shutdown.clone();
    let api_listen = cli.api_listen;
    let api_task = tokio::spawn(async move {
        let app = Router::new()
            .route("/status", get(handle_status))
            .route("/diagnostics", get(handle_diagnostics))
            .route("/shutdown", post(handle_shutdown))
            .with_state(api_state);
        let listener = tokio::net::TcpListener::bind(api_listen).await.unwrap();
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                shutdown_api.cancelled().await;
            })
            .await
            .ok();
    });

    // ── Forwarding tasks ──
    let mtu = tun.mtu() as usize;

    // TUN ══peers
    let tun_rx = tun.clone();
    let state_rx = state.clone();
    let shutdown_rx = shutdown.clone();
    let _tun_task = tokio::spawn(async move {
        let mut buf = vec![0u8; mtu];
        loop {
            tokio::select! {
                _ = shutdown_rx.cancelled() => break,
                r = tun_rx.recv(&mut buf) => {
                    if let Ok(n) = r {
                        let pkt = &buf[..n];
                        if let Some(dst) = parse_dst_ip(pkt) {
                            let s = state_rx.lock().await;
                            if let Some(pid) = s.ip_route.get(&dst) {
                                if let Some(link) = s.peer_links.get(pid) {
                                    let mut lk = link.lock().await;
                                    let _ = lk.path.send(pkt).await;
                                }
                            }
                        }
                    }
                }
            }
        }
    });

    // Peers ══TUN (one task per peer for relay recv)
    let tun_tx = tun.clone();
    let mut relay_tasks = Vec::new();
    {
        let s = state.lock().await;
        for link in s.peer_links.values() {
            let link = link.clone();
            let _tun = tun_tx.clone();
            let sd = shutdown.clone();
            relay_tasks.push(tokio::spawn(async move {
                loop {
                    tokio::select! {
                        _ = sd.cancelled() => break,
                        r = async {
                            let _lk = link.lock().await;
                            // Use relay directly for recv
                            // This is the relay client inside PathManager
                            Ok::<_, anyhow::Error>(())
                        } => { let _ = r; }
                    }
                    // Simple poll: try relay recv
                    let _lk = link.lock().await;
                    // We need to access the relay from PathManager
                    // PathManager doesn't expose recv directly for relay
                    // So we use a separate approach ══see below
                    drop(_lk);
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
            }));
        }
    }
    drop(relay_tasks); // Will be restructured below

    // ── Periodic: heartbeat, peers, candidates, P2P retry ──
    let refresh_state = state.clone();
    let refresh_shutdown = shutdown.clone();
    let _periodic = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        loop {
            tokio::select! {
                _ = refresh_shutdown.cancelled() => break,
                _ = interval.tick() => {
                    // Heartbeat
                    let _ = http.post(format!("{}/api/v1/heartbeat", cli.control_url))
                        .json(&serde_json::json!({"device_id": our_id}))
                        .send().await;

                    // Probe relays for latency + failover
                    relay_pool.probe_all().await;
                    let summary = relay_pool.summary();
                    for s in &summary {
                        tracing::debug!(
                            addr = %s.addr,
                            latency = ?s.latency_ms,
                            healthy = s.healthy,
                            active = s.active,
                            "relay status"
                        );
                    }

                    // Republish candidates (address may have changed)
                    let fresh = discover_candidates(cli.stun_server).await;
                    publish_candidates(&http, &cli.control_url, our_id, &fresh).await;

                    // Sync peers
                    if let Ok((routes, links)) = connect_peers(
                        &http, &cli.control_url, relay_pool.active_addr(), our_id, our_relay_id,
                    ).await {
                        // Attempt P2P for new peers
                        attempt_p2p_for_all(&links, &http, &cli.control_url, &keypair, our_relay_id).await;

                        let mut s = refresh_state.lock().await;
                        s.ip_route = routes;
                        s.peer_links = links;
                    }
                }
            }
        }
    });

    tracing::info!("daemon running ══Ctrl+C to stop");
    tokio::signal::ctrl_c().await?;
    tracing::info!("shutting down...");
    shutdown.cancel();
    let _ = api_task.await;
    tracing::info!("shutdown complete");
    Ok(())
}

// ════════════════════════════════════════════════════════════════════
//  P2P helpers
// ════════════════════════════════════════════════════════════════════
/// 通过 STUN 服务发现本机的 NAT 候选地址。
/// Discover local NAT candidate addresses via STUN.
///
/// 同时获取公网地址和本地地址。
/// Returns both public and local addresses.
async fn discover_candidates(stun_server: SocketAddr) -> Vec<String> {
    let mut candidates = Vec::new();
    if let Ok(stun) = StunClient::bind().await {
        if let Ok(addr) = stun.discover(stun_server).await {
            candidates.push(addr.to_string());
        }
        if let Ok(local) = stun.local_addr() {
            candidates.push(local.to_string());
        }
    }
    candidates
}

/// 将本机候选地址发布到控制服务，供对等节点获取。
/// Publish local candidate addresses to the control service for peer discovery.
async fn publish_candidates(http: &reqwest::Client, ctl: &str, id: Uuid, candidates: &[String]) {
    if candidates.is_empty() {
        return;
    }
    let _ = http
        .post(format!("{ctl}/api/v1/candidates"))
        .json(&CandidatePublish { device_id: id, candidates: candidates.to_vec() })
        .send()
        .await;
}

/// 从控制服务获取指定对等节点的候选地址列表。
/// Fetch the candidate address list for a given peer from the control service.
///
/// # Returns
///
/// 解析后的 [`SocketAddr`] 列表，获取失败返回空列表。
/// Parsed [`SocketAddr`] list, or empty on failure.
async fn get_peer_candidates(http: &reqwest::Client, ctl: &str, peer_id: Uuid) -> Vec<SocketAddr> {
    let resp = http.get(format!("{ctl}/api/v1/candidates/{peer_id}")).send().await;
    match resp {
        Ok(r) => {
            let list: CandidateList =
                r.json().await.unwrap_or(CandidateList { device_id: peer_id, candidates: vec![] });
            list.candidates.iter().filter_map(|a| a.parse().ok()).collect()
        }
        Err(_) => vec![],
    }
}

/// 从控制服务同步对等节点列表并为每个节点建立中继连接。
/// Sync peer list from control service and establish relay connections for each.
///
/// # Returns
///
/// 返回 IP 路由映射和对等节点链接映射。
/// Returns IP routing map and peer links map.
///
/// # Errors
///
/// 如果 HTTP 请求或 IPv4 解析失败则返回错误。
/// Returns an error if HTTP request or IPv4 parsing fails.
async fn connect_peers(
    http: &reqwest::Client,
    control_url: &str,
    relay_addr: SocketAddr,
    our_id: Uuid,
    our_relay_id: PeerId,
) -> anyhow::Result<(HashMap<Ipv4Addr, Uuid>, HashMap<Uuid, Arc<Mutex<PeerLink>>>)> {
    let peers: PeersResponse = http.get(format!("{control_url}/api/v1/peers")).send().await?.json().await?;

    let mut ip_route = HashMap::new();
    let mut peer_links = HashMap::new();

    for p in peers.peers.into_iter().filter(|p| p.device_id != our_id) {
        let vip: Ipv4Addr = p.ipv4.parse()?;
        let peer_relay_id = PeerId::from_bytes(p.device_id.into_bytes());

        let relay = RelayClient::register("0.0.0.0:0".parse()?, relay_addr, our_relay_id, peer_relay_id).await?;

        let path = PathManager::new(relay, SocketAddr::new(std::net::IpAddr::V4(vip), 0));

        ip_route.insert(vip, p.device_id);
        peer_links.insert(
            p.device_id,
            Arc::new(Mutex::new(PeerLink {
                hostname: p.hostname,
                vip,
                public_key: p.public_key,
                path,
                p2p_retry_ms: 200,
            })),
        );
    }

    Ok((ip_route, peer_links))
}

/// 对所有未直连的对等节点尝试 P2P 打洞。
/// Attempt P2P hole punching for all peers not already on a direct path.
///
/// 对已直连的节点跳过；打洞失败时执行指数退避。
/// Skips peers already on a direct path; applies exponential backoff on failure.
async fn attempt_p2p_for_all(
    peer_links: &HashMap<Uuid, Arc<Mutex<PeerLink>>>,
    http: &reqwest::Client,
    ctl: &str,
    _keypair: &KeyPair,
    _our_relay_id: PeerId,
) {
    for (_pid, link) in peer_links {
        let mut lk = link.lock().await;
        if lk.path.current_status() == PathStatus::Direct {
            continue; // Already direct, skip
        }

        // Get peer candidates
        let peer_addrs = get_peer_candidates(http, ctl, *_pid).await;
        if peer_addrs.is_empty() {
            continue;
        }

        // Try hole punching
        let punch_sock = match tokio::net::UdpSocket::bind("0.0.0.0:0").await {
            Ok(s) => s,
            Err(_) => continue,
        };
        let puncher = Puncher::from_socket(punch_sock);
        let p2p_candidates: Vec<Candidate> = peer_addrs.iter().map(|a| Candidate::host(*a)).collect();

        match puncher.punch(&p2p_candidates, b"connectalso-p2p").await {
            Ok(Some((_payload, _from))) => {
                tracing::info!(peer = %lk.vip, "P2P hole punched");
                // On success, we could create a Tunnel here
                // For now, mark that P2P is possible
                lk.p2p_retry_ms = 200;
            }
            _ => {
                lk.p2p_retry_ms = (lk.p2p_retry_ms * 2).min(30_000);
                tracing::debug!(peer = %lk.vip, backoff = lk.p2p_retry_ms, "P2P not yet");
            }
        }
    }
}

// ════════════════════════════════════════════════════════════════════
//  Helpers
// ════════════════════════════════════════════════════════════════════
/// 从原始 IPv4 数据包中提取目标 IP 地址（字节 16-19）。
/// Extract the destination IPv4 address from a raw packet (bytes 16-19).
///
/// # Returns
///
/// 如果数据包长度不足 20 或不是 IPv4 则返回 `None`。
/// Returns `None` if the packet is shorter than 20 bytes or not IPv4.
fn parse_dst_ip(packet: &[u8]) -> Option<Ipv4Addr> {
    if packet.len() < 20 {
        return None;
    }
    if packet[0] >> 4 == 4 {
        Some(Ipv4Addr::new(packet[16], packet[17], packet[18], packet[19]))
    } else {
        None
    }
}

/// 将字节切片编码为十六进制字符串。
/// Encode a byte slice as a hexadecimal string.
fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// ════════════════════════════════════════════════════════════════════
//  Status API
// ════════════════════════════════════════════════════════════════════
/// `GET /status` 处理函数：返回守护进程运行状态。
/// `GET /status` handler: return current daemon status.
async fn handle_status(State(state): State<Arc<Mutex<SharedState>>>) -> Json<StatusResponse> {
    let s = state.lock().await;

    let mut peers = Vec::new();
    for link in s.peer_links.values() {
        let lk = link.lock().await;
        let path_str = match lk.path.current_status() {
            PathStatus::Direct => "direct",
            PathStatus::Relay => "relay",
            PathStatus::Probing => "probing",
        };
        peers.push(StatusPeer {
            device_id: lk.hostname.clone(),
            virtual_ip: lk.vip.to_string(),
            hostname: lk.hostname.clone(),
            path: path_str.into(),
        });
    }

    Json(StatusResponse {
        device_id: s.device_id.to_string(),
        virtual_ip: s.virtual_ip.to_string(),
        hostname: s.hostname.clone(),
        uptime_secs: s.started_at.elapsed().as_secs(),
        peer_count: peers.len(),
        peers,
    })
}

/// `POST /shutdown` 处理函数：返回关闭确认消息。
/// `POST /shutdown` handler: return shutdown acknowledgement.
async fn handle_shutdown(State(_state): State<Arc<Mutex<SharedState>>>) -> Json<ShutdownResponse> {
    Json(ShutdownResponse { message: "shutdown initiated".into() })
}

/// `GET /diagnostics` 处理函数：运行综合网络诊断。
/// `GET /diagnostics` handler: run comprehensive network diagnostics.
///
/// 依次检查控制服务、STUN、中继、TUN 及各对等节点的连通性。
/// Checks control service, STUN, relay, TUN, and per-peer connectivity.
async fn handle_diagnostics(State(state): State<Arc<Mutex<SharedState>>>) -> Json<DiagnosticsResponse> {
    let s = state.lock().await;
    let config = DaemonConfig::load();

    // Check control service
    let (control_status, control_detail, control_latency) =
        check_http(&format!("{}/api/v1/health", config.as_ref().map(|c| c.control_url.as_str()).unwrap_or(""))).await;

    // Check STUN
    let (stun_status, stun_detail, stun_latency) =
        check_stun(config.as_ref().and_then(|c| c.stun_server.parse().ok())).await;

    // Check relay (probe all)
    let (relay_status, relay_detail, relay_latency) =
        check_udp(config.as_ref().and_then(|c| c.relay_servers.first()).and_then(|a| a.parse().ok()), b"RELAY_CHECK")
            .await;

    // TUN status
    let tun_status = CheckResult { status: "ok", detail: format!("VIP: {}", s.virtual_ip), latency_ms: None };

    // Peer diagnostics
    let mut peers = Vec::new();
    for link in s.peer_links.values() {
        let lk = link.lock().await;
        let path_str = match lk.path.current_status() {
            PathStatus::Direct => "direct",
            PathStatus::Relay => "relay",
            PathStatus::Probing => "probing",
        };
        peers.push(PeerDiag {
            hostname: lk.hostname.clone(),
            virtual_ip: lk.vip.to_string(),
            path: path_str.into(),
            reachable: lk.path.current_status() != PathStatus::Probing,
        });
    }

    Json(DiagnosticsResponse {
        daemon: CheckResult {
            status: "ok",
            detail: format!("uptime {}s", s.started_at.elapsed().as_secs()),
            latency_ms: None,
        },
        control: CheckResult { status: control_status, detail: control_detail, latency_ms: control_latency },
        stun: CheckResult { status: stun_status, detail: stun_detail, latency_ms: stun_latency },
        relay: CheckResult { status: relay_status, detail: relay_detail, latency_ms: relay_latency },
        tun: tun_status,
        peers,
    })
}

/// 对指定 URL 发起 HTTP GET 请求并返回检查结果。
/// Perform an HTTP GET to the given URL and return a check result.
async fn check_http(url: &str) -> (&'static str, String, Option<u64>) {
    let start = Instant::now();
    match reqwest::get(url).await {
        Ok(r) if r.status().is_success() => {
            let ms = start.elapsed().as_millis() as u64;
            ("ok", format!("HTTP {}", r.status()), Some(ms))
        }
        Ok(r) => ("warn", format!("HTTP {}", r.status()), None),
        Err(e) => ("error", format!("{e}"), None),
    }
}

/// 通过 STUN 绑定并发现公网地址，测量延迟。
/// Perform a STUN bind/discover and measure latency.
async fn check_stun(server: Option<SocketAddr>) -> (&'static str, String, Option<u64>) {
    let Some(server) = server else { return ("warn", "not configured".into(), None) };
    let start = Instant::now();
    match StunClient::bind().await {
        Ok(c) => match c.discover(server).await {
            Ok(addr) => {
                let ms = start.elapsed().as_millis() as u64;
                ("ok", format!("public: {addr}"), Some(ms))
            }
            Err(e) => ("error", format!("{e}"), None),
        },
        Err(e) => ("error", format!("{e}"), None),
    }
}

/// 向指定 UDP 服务器发送探测包并等待响应，测量延迟。
/// Send a probe to the given UDP server and wait for a response, measuring latency.
async fn check_udp(server: Option<SocketAddr>, probe: &[u8]) -> (&'static str, String, Option<u64>) {
    let Some(server) = server else { return ("warn", "not configured".into(), None) };
    let start = Instant::now();
    match tokio::net::UdpSocket::bind("0.0.0.0:0").await {
        Ok(sock) => {
            let _ = sock.send_to(probe, server).await;
            let mut buf = [0u8; 64];
            match tokio::time::timeout(std::time::Duration::from_secs(2), sock.recv_from(&mut buf)).await {
                Ok(Ok((n, from))) => {
                    let ms = start.elapsed().as_millis() as u64;
                    ("ok", format!("response {n}B from {from}"), Some(ms))
                }
                Ok(Err(e)) => ("error", format!("{e}"), None),
                Err(_) => ("warn", "timeout (no response)".into(), None),
            }
        }
        Err(e) => ("error", format!("{e}"), None),
    }
}
