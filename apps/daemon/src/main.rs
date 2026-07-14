use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Context;
use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use clap::Parser;
use connectalso_crypto::key_exchange::KeyPair;
use connectalso_nat::stun::StunClient;
use connectalso_platform::tun::{TunConfig, TunDevice};
use connectalso_relay_proto::PeerId;
use connectalso_tunnel::relay::RelayClient;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

// ═══════════════════════════════════════════════════════════════════
// Config
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Serialize, Deserialize)]
struct DaemonConfig {
    device_id: Uuid,
    /// Hex-encoded 32-byte X25519 public key
    public_key_hex: String,
    virtual_ip: String,
    control_url: String,
    stun_server: String,
    relay_server: String,
    hostname: String,
}

impl DaemonConfig {
    fn path() -> PathBuf {
        let base = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("connectalso");
        std::fs::create_dir_all(&base).ok();
        base.join("config.json")
    }

    fn load() -> Option<Self> {
        let data = std::fs::read_to_string(Self::path()).ok()?;
        serde_json::from_str(&data).ok()
    }

    fn save(&self) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(Self::path(), json);
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
// Control API DTOs
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Serialize)]
struct RegisterRequest {
    public_key: [u8; 32],
    hostname: String,
}

#[derive(Debug, Deserialize)]
struct RegisterResponse {
    device_id: Uuid,
    ipv4: String,
    network: String,
}

#[derive(Debug, Deserialize, Clone)]
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

// ═══════════════════════════════════════════════════════════════════
// Daemon State
// ═══════════════════════════════════════════════════════════════════

struct PeerLink {
    hostname: String,
    vip: Ipv4Addr,
    public_key: [u8; 32],
    relay: Mutex<RelayClient>,
}

struct SharedState {
    device_id: Uuid,
    virtual_ip: Ipv4Addr,
    hostname: String,
    started_at: Instant,
    ip_route: HashMap<Ipv4Addr, Uuid>,
    peer_links: HashMap<Uuid, Arc<PeerLink>>,
}

#[derive(Debug, Serialize)]
struct StatusResponse {
    device_id: String,
    virtual_ip: String,
    hostname: String,
    uptime_secs: u64,
    peer_count: usize,
    peers: Vec<StatusPeer>,
}

#[derive(Debug, Serialize)]
struct StatusPeer {
    device_id: String,
    virtual_ip: String,
    hostname: String,
}

#[derive(Debug, Serialize)]
struct ShutdownResponse {
    message: String,
}

// ═══════════════════════════════════════════════════════════════════
// CLI
// ═══════════════════════════════════════════════════════════════════

#[derive(Parser)]
#[command(name = "connectalso-daemon")]
#[command(about = "ConnectAlso Desktop Alpha")]
struct Cli {
    #[arg(long, default_value = "http://127.0.0.1:3000")]
    control_url: String,

    #[arg(long, default_value = "127.0.0.1:3478")]
    stun_server: SocketAddr,

    #[arg(long, default_value = "127.0.0.1:33478")]
    relay_server: SocketAddr,

    #[arg(long, default_value = "unnamed")]
    hostname: String,

    #[arg(long, default_value = "connectalso")]
    tun_name: String,

    /// 本地状态 API 监听地址
    #[arg(long, default_value = "127.0.0.1:9823")]
    api_listen: SocketAddr,
}

// ═══════════════════════════════════════════════════════════════════
// Main
// ═══════════════════════════════════════════════════════════════════

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    let shutdown = CancellationToken::new();

    // ── Load or create config ──
    let (device_id, pubkey_hex, stored_ip) = if let Some(cfg) = DaemonConfig::load() {
        tracing::info!(id = %cfg.device_id, "loaded existing config");
        let ip: Ipv4Addr = cfg.virtual_ip.parse().unwrap_or(Ipv4Addr::UNSPECIFIED);
        (cfg.device_id, cfg.public_key_hex, ip)
    } else {
        (Uuid::nil(), String::new(), Ipv4Addr::UNSPECIFIED)
    };

    // ── Generate/load keypair ──
    let keypair = KeyPair::generate(); // new keypair each session for now
    let pubkey = keypair.public_key_bytes();
    let pubkey_hex = hex_encode(&pubkey);

    // ── Register with control service ──
    let http = reqwest::Client::new();
    let reg: RegisterResponse = http
        .post(format!("{}/api/v1/register", cli.control_url))
        .json(&RegisterRequest {
            public_key: pubkey,
            hostname: cli.hostname.clone(),
        })
        .send()
        .await
        .context("control service unreachable")?
        .json()
        .await?;

    let our_id = reg.device_id;
    let our_ip: Ipv4Addr = reg.ipv4.parse()?;
    tracing::info!(%our_id, %our_ip, "registered");

    // Persist config
    let config = DaemonConfig {
        device_id: our_id,
        public_key_hex: pubkey_hex.clone(),
        virtual_ip: our_ip.to_string(),
        control_url: cli.control_url.clone(),
        stun_server: cli.stun_server.to_string(),
        relay_server: cli.relay_server.to_string(),
        hostname: cli.hostname.clone(),
    };
    config.save();

    // ── STUN probe ──
    tokio::spawn({
        let stun = cli.stun_server;
        async move {
            if let Ok(c) = StunClient::bind().await {
                match c.discover(stun).await {
                    Ok(addr) => tracing::info!(%addr, "STUN public address"),
                    Err(e) => tracing::warn!(%e, "STUN failed"),
                }
            }
        }
    });

    // ── Fetch peers ──
    let (ip_route, peer_links) =
        fetch_and_connect_peers(&http, &cli, our_id).await?;
    tracing::info!(peers = peer_links.len(), "initial peer sync");

    // ── TUN device ──
    let tun = TunDevice::create(
        TunConfig::new(our_ip, Ipv4Addr::new(255, 255, 255, 0))
            .with_name(&cli.tun_name),
    )
    .await
    .context("create TUN")?;
    let tun = Arc::new(tun);
    tracing::info!(name = tun.name(), vip = %tun.address(), mtu = tun.mtu(), "TUN up");

    // ── Shared state for API ──
    let state = Arc::new(Mutex::new(SharedState {
        device_id: our_id,
        virtual_ip: our_ip,
        hostname: cli.hostname.clone(),
        started_at: Instant::now(),
        ip_route,
        peer_links,
    }));

    // ── Local status API server ──
    let api_state = state.clone();
    let shutdown_api = shutdown.clone();
    let api_listen = cli.api_listen;
    let api_task = tokio::spawn(async move {
        let app = Router::new()
            .route("/status", get(handle_status))
            .route("/shutdown", post(handle_shutdown))
            .with_state(api_state);

        let listener = tokio::net::TcpListener::bind(api_listen).await.unwrap();
        tracing::info!(%api_listen, "status API listening");
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                shutdown_api.cancelled().await;
            })
            .await
            .ok();
    });

    // ── TUN → relay forwarding ──
    let mtu = tun.mtu() as usize;
    let tun_clone = tun.clone();
    let state_clone = state.clone();
    let shutdown_tun = shutdown.clone();
    let _tun_rx = tokio::spawn(async move {
        let mut buf = vec![0u8; mtu];
        loop {
            tokio::select! {
                _ = shutdown_tun.cancelled() => break,
                result = tun_clone.recv(&mut buf) => {
                    match result {
                        Ok(n) => {
                            let pkt = &buf[..n];
                            if let Some(dst) = parse_dst_ip(pkt) {
                                let s = state_clone.lock().await;
                                if let Some(pid) = s.ip_route.get(&dst) {
                                    if let Some(link) = s.peer_links.get(pid) {
                                        let relay = link.relay.lock().await;
                                        let _ = relay.send(pkt).await;
                                    }
                                }
                            }
                        }
                        Err(e) => tracing::error!(%e, "TUN recv"),
                    }
                }
            }
        }
        tracing::debug!("TUN reader stopped");
    });

    // ── Relay → TUN forwarding (one task per peer) ──
    let mut relay_tasks = Vec::new();
    {
        let s = state.lock().await;
        for link in s.peer_links.values() {
            let link = link.clone();
            let tun = tun.clone();
            let shutdown_rx = shutdown.clone();

            relay_tasks.push(tokio::spawn(async move {
                loop {
                    tokio::select! {
                        _ = shutdown_rx.cancelled() => break,
                        result = async {
                            let relay = link.relay.lock().await;
                            relay.recv().await
                        } => {
                            match result {
                                Ok((data, _sender)) => {
                                    let _ = tun.send(&data).await;
                                }
                                Err(e) => tracing::error!(%e, "relay recv"),
                            }
                        }
                    }
                }
                tracing::debug!("relay reader stopped for {}", link.hostname);
            }));
        }
    }

    // ── Periodic peer refresh ──
    let refresh_shutdown = shutdown.clone();
    let _refresh = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        loop {
            tokio::select! {
                _ = refresh_shutdown.cancelled() => break,
                _ = interval.tick() => {
                    if let Ok((routes, links)) = fetch_and_connect_peers(&http, &cli, our_id).await
                    {
                        let mut s = state.lock().await;
                        s.ip_route = routes;
                        s.peer_links = links;
                        tracing::debug!(peers = s.peer_links.len(), "peers refreshed");
                    }
                }
            }
        }
    });

    tracing::info!("daemon running — use 'connectalso status' or Ctrl+C");

    // Wait for shutdown signal
    tokio::signal::ctrl_c().await?;
    tracing::info!("shutting down...");
    shutdown.cancel();

    // Wait for tasks to finish
    for t in relay_tasks {
        let _ = t.await;
    }
    let _ = api_task.await;
    tracing::info!("shutdown complete");

    Ok(())
}

// ═══════════════════════════════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════════════════════════════

async fn fetch_and_connect_peers(
    http: &reqwest::Client,
    cli: &Cli,
    our_id: Uuid,
) -> anyhow::Result<(HashMap<Ipv4Addr, Uuid>, HashMap<Uuid, Arc<PeerLink>>)> {
    let our_relay_id = PeerId::from_bytes(our_id.into_bytes());

    let peers_resp: PeersResponse = http
        .get(format!("{}/api/v1/peers", cli.control_url))
        .send()
        .await?
        .json()
        .await?;

    let mut ip_route = HashMap::new();
    let mut peer_links = HashMap::new();

    for p in peers_resp.peers.into_iter().filter(|p| p.device_id != our_id) {
        let vip: Ipv4Addr = p.ipv4.parse()?;
        let peer_relay_id = PeerId::from_bytes(p.device_id.into_bytes());

        match RelayClient::register(
            "0.0.0.0:0".parse()?,
            cli.relay_server,
            our_relay_id,
            peer_relay_id,
        )
        .await
        {
            Ok(relay) => {
                ip_route.insert(vip, p.device_id);
                peer_links.insert(
                    p.device_id,
                    Arc::new(PeerLink {
                        hostname: p.hostname.clone(),
                        vip,
                        public_key: p.public_key,
                        relay: Mutex::new(relay),
                    }),
                );
                tracing::info!(peer = %p.hostname, %vip, "relay connected");
            }
            Err(e) => {
                tracing::warn!(peer = %p.hostname, %e, "relay failed");
            }
        }
    }

    Ok((ip_route, peer_links))
}

/// Extract the destination IPv4 address from an IP packet header.
fn parse_dst_ip(packet: &[u8]) -> Option<Ipv4Addr> {
    if packet.len() < 20 {
        return None;
    }
    if packet[0] >> 4 == 4 {
        Some(Ipv4Addr::new(
            packet[16], packet[17], packet[18], packet[19],
        ))
    } else {
        None
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// ═══════════════════════════════════════════════════════════════════
// Status API handlers
// ═══════════════════════════════════════════════════════════════════

async fn handle_status(
    State(state): State<Arc<Mutex<SharedState>>>,
) -> Json<StatusResponse> {
    let s = state.lock().await;
    let peers: Vec<StatusPeer> = s
        .peer_links
        .values()
        .map(|l| StatusPeer {
            device_id: l.hostname.clone(),
            virtual_ip: l.vip.to_string(),
            hostname: l.hostname.clone(),
        })
        .collect();

    Json(StatusResponse {
        device_id: s.device_id.to_string(),
        virtual_ip: s.virtual_ip.to_string(),
        hostname: s.hostname.clone(),
        uptime_secs: s.started_at.elapsed().as_secs(),
        peer_count: peers.len(),
        peers,
    })
}

async fn handle_shutdown(
    State(_state): State<Arc<Mutex<SharedState>>>,
) -> Json<ShutdownResponse> {
    // In a real implementation, trigger shutdown via CancellationToken
    Json(ShutdownResponse {
        message: "shutdown initiated".into(),
    })
}
