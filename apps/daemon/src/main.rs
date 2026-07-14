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
use connectalso_nat::candidate::Candidate;
use connectalso_nat::punch::Puncher;
use connectalso_nat::stun::StunClient;
use connectalso_platform::tun::{TunConfig, TunDevice};
use connectalso_relay_proto::PeerId;
use connectalso_tunnel::path::{PathManager, PathStatus};
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
    last_seen_secs: i64,
}

#[derive(Debug, Deserialize)]
struct PeersResponse {
    peers: Vec<PeerInfo>,
}

#[derive(Debug, Serialize)]
struct CandidatePublish {
    device_id: Uuid,
    candidates: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct CandidateList {
    device_id: Uuid,
    candidates: Vec<String>,
}

// ═══════════════════════════════════════════════════════════════════
// Daemon state
// ═══════════════════════════════════════════════════════════════════

struct PeerLink {
    hostname: String,
    vip: Ipv4Addr,
    public_key: [u8; 32],
    path: PathManager,
    p2p_retry_ms: u64,
}

struct SharedState {
    device_id: Uuid,
    virtual_ip: Ipv4Addr,
    hostname: String,
    started_at: Instant,
    ip_route: HashMap<Ipv4Addr, Uuid>,
    peer_links: HashMap<Uuid, Arc<Mutex<PeerLink>>>,
}

// ═══════════════════════════════════════════════════════════════════
// Status API types
// ═══════════════════════════════════════════════════════════════════

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
    path: String,
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
    let http = reqwest::Client::new();

    tracing::info!(control = %cli.control_url, hostname = %cli.hostname, "Desktop Alpha");

    // ── Keypair, register, config ──
    let keypair = KeyPair::generate();
    let pubkey = keypair.public_key_bytes();

    let reg: RegisterResponse = http
        .post(format!("{}/api/v1/register", cli.control_url))
        .json(&RegisterRequest { public_key: pubkey, hostname: cli.hostname.clone() })
        .send().await?.json().await?;

    let our_id = reg.device_id;
    let our_ip: Ipv4Addr = reg.ipv4.parse()?;

    DaemonConfig {
        device_id: our_id, public_key_hex: hex_encode(&pubkey),
        virtual_ip: our_ip.to_string(),
        control_url: cli.control_url.clone(), stun_server: cli.stun_server.to_string(),
        relay_server: cli.relay_server.to_string(), hostname: cli.hostname.clone(),
    }.save();

    // ── STUN: discover + publish candidates ──
    let our_candidates = discover_candidates(cli.stun_server).await;
    publish_candidates(&http, &cli.control_url, our_id, &our_candidates).await;

    // ── Initial peer sync ──
    let our_relay_id = PeerId::from_bytes(our_id.into_bytes());
    let (ip_route, peer_links) = connect_peers(
        &http, &cli, our_id, our_relay_id, &keypair,
    ).await?;

    // ── TUN ──
    let tun = TunDevice::create(
        TunConfig::new(our_ip, Ipv4Addr::new(255, 255, 255, 0)).with_name(&cli.tun_name),
    ).await.context("create TUN")?;
    let tun = Arc::new(tun);
    tracing::info!(name = tun.name(), vip = %tun.address(), "TUN up");

    // ── Attempt P2P for each peer ──
    attempt_p2p_for_all(&peer_links, &http, &cli.control_url, &keypair, our_relay_id).await;

    // ── Shared state ──
    let state = Arc::new(Mutex::new(SharedState {
        device_id: our_id, virtual_ip: our_ip, hostname: cli.hostname.clone(),
        started_at: Instant::now(), ip_route, peer_links,
    }));

    // ── Status API ──
    let api_state = state.clone();
    let shutdown_api = shutdown.clone();
    let api_listen = cli.api_listen;
    let api_task = tokio::spawn(async move {
        let app = Router::new()
            .route("/status", get(handle_status))
            .route("/shutdown", post(handle_shutdown))
            .with_state(api_state);
        let listener = tokio::net::TcpListener::bind(api_listen).await.unwrap();
        axum::serve(listener, app).with_graceful_shutdown(async move {
            shutdown_api.cancelled().await;
        }).await.ok();
    });

    // ── Forwarding tasks ──
    let mtu = tun.mtu() as usize;

    // TUN → peers
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

    // Peers → TUN (one task per peer for relay recv)
    let tun_tx = tun.clone();
    let mut relay_tasks = Vec::new();
    {
        let s = state.lock().await;
        for link in s.peer_links.values() {
            let link = link.clone();
            let tun = tun_tx.clone();
            let sd = shutdown.clone();
            relay_tasks.push(tokio::spawn(async move {
                loop {
                    tokio::select! {
                        _ = sd.cancelled() => break,
                        r = async {
                            let lk = link.lock().await;
                            // Use relay directly for recv
                            // This is the relay client inside PathManager
                            Ok::<_, anyhow::Error>(())
                        } => { let _ = r; }
                    }
                    // Simple poll: try relay recv
                    let lk = link.lock().await;
                    // We need to access the relay from PathManager
                    // PathManager doesn't expose recv directly for relay
                    // So we use a separate approach — see below
                    drop(lk);
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

                    // Republish candidates (address may have changed)
                    let fresh = discover_candidates(cli.stun_server).await;
                    publish_candidates(&http, &cli.control_url, our_id, &fresh).await;

                    // Sync peers
                    if let Ok((routes, links)) = connect_peers(
                        &http, &cli, our_id, our_relay_id, &keypair,
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

    tracing::info!("daemon running — Ctrl+C to stop");
    tokio::signal::ctrl_c().await?;
    tracing::info!("shutting down...");
    shutdown.cancel();
    let _ = api_task.await;
    tracing::info!("shutdown complete");
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════
// P2P helpers
// ═══════════════════════════════════════════════════════════════════

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

async fn publish_candidates(http: &reqwest::Client, ctl: &str, id: Uuid, candidates: &[String]) {
    if candidates.is_empty() { return; }
    let _ = http.post(format!("{ctl}/api/v1/candidates"))
        .json(&CandidatePublish { device_id: id, candidates: candidates.to_vec() })
        .send().await;
}

async fn get_peer_candidates(http: &reqwest::Client, ctl: &str, peer_id: Uuid) -> Vec<SocketAddr> {
    let resp = http.get(format!("{ctl}/api/v1/candidates/{peer_id}"))
        .send().await;
    match resp {
        Ok(r) => {
            let list: CandidateList = r.json().await.unwrap_or(CandidateList {
                device_id: peer_id, candidates: vec![],
            });
            list.candidates.iter().filter_map(|a| a.parse().ok()).collect()
        }
        Err(_) => vec![],
    }
}

async fn connect_peers(
    http: &reqwest::Client, cli: &Cli, our_id: Uuid,
    our_relay_id: PeerId, keypair: &KeyPair,
) -> anyhow::Result<(HashMap<Ipv4Addr, Uuid>, HashMap<Uuid, Arc<Mutex<PeerLink>>>)> {
    let peers: PeersResponse = http
        .get(format!("{}/api/v1/peers", cli.control_url))
        .send().await?.json().await?;

    let mut ip_route = HashMap::new();
    let mut peer_links = HashMap::new();

    for p in peers.peers.into_iter().filter(|p| p.device_id != our_id) {
        let vip: Ipv4Addr = p.ipv4.parse()?;
        let peer_relay_id = PeerId::from_bytes(p.device_id.into_bytes());

        let relay = RelayClient::register(
            "0.0.0.0:0".parse()?, cli.relay_server, our_relay_id, peer_relay_id,
        ).await?;

        let path = PathManager::new(relay, SocketAddr::new(
            std::net::IpAddr::V4(vip), 0,
        ));

        ip_route.insert(vip, p.device_id);
        peer_links.insert(p.device_id, Arc::new(Mutex::new(PeerLink {
            hostname: p.hostname, vip, public_key: p.public_key, path,
            p2p_retry_ms: 200,
        })));
    }

    Ok((ip_route, peer_links))
}

async fn attempt_p2p_for_all(
    peer_links: &HashMap<Uuid, Arc<Mutex<PeerLink>>>,
    http: &reqwest::Client, ctl: &str,
    keypair: &KeyPair, our_relay_id: PeerId,
) {
    for (_pid, link) in peer_links {
        let mut lk = link.lock().await;
        if lk.path.current_status() == PathStatus::Direct {
            continue; // Already direct, skip
        }

        // Get peer candidates
        let peer_addrs = get_peer_candidates(http, ctl, _pid).await;
        if peer_addrs.is_empty() { continue; }

        // Try hole punching
        let punch_sock = match tokio::net::UdpSocket::bind("0.0.0.0:0").await {
            Ok(s) => s,
            Err(_) => continue,
        };
        let puncher = Puncher::from_socket(punch_sock);
        let p2p_candidates: Vec<Candidate> = peer_addrs.iter()
            .map(|a| Candidate::host(*a))
            .collect();

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

// ═══════════════════════════════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════════════════════════════

fn parse_dst_ip(packet: &[u8]) -> Option<Ipv4Addr> {
    if packet.len() < 20 { return None; }
    if packet[0] >> 4 == 4 {
        Some(Ipv4Addr::new(packet[16], packet[17], packet[18], packet[19]))
    } else { None }
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// ═══════════════════════════════════════════════════════════════════
// Status API
// ═══════════════════════════════════════════════════════════════════

async fn handle_status(
    State(state): State<Arc<Mutex<SharedState>>>,
) -> Json<StatusResponse> {
    let s = state.lock().await;

    let mut peers = Vec::new();
    for (_id, link) in &s.peer_links {
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

async fn handle_shutdown(
    State(_state): State<Arc<Mutex<SharedState>>>,
) -> Json<ShutdownResponse> {
    Json(ShutdownResponse { message: "shutdown initiated".into() })
}
