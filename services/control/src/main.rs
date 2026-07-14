use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use clap::Parser;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

/// Control service shared state.
#[derive(Default)]
struct AppState {
    peers: RwLock<HashMap<Uuid, PeerRecord>>,
    next_ip: RwLock<u32>,
}

struct PeerRecord {
    device_id: Uuid,
    public_key: [u8; 32],
    hostname: String,
    ipv4: Ipv4Addr,
}

// ── API types ──

#[derive(Debug, Deserialize)]
struct RegisterRequest {
    public_key: [u8; 32],
    hostname: String,
}

#[derive(Debug, Serialize)]
struct RegisterResponse {
    device_id: Uuid,
    ipv4: String,
    network: String,
}

#[derive(Debug, Serialize)]
struct PeerInfo {
    device_id: Uuid,
    ipv4: String,
    public_key: [u8; 32],
    hostname: String,
}

#[derive(Debug, Serialize)]
struct PeersResponse {
    peers: Vec<PeerInfo>,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
}

// ── CLI ──

#[derive(Parser)]
#[command(name = "connectalso-control")]
#[command(about = "ConnectAlso 控制服务")]
struct Cli {
    #[arg(long, default_value = "0.0.0.0:3000")]
    listen: SocketAddr,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    let state = Arc::new(AppState::default());

    let app = Router::new()
        .route("/api/v1/register", post(handle_register))
        .route("/api/v1/peers", get(handle_peers))
        .route("/api/v1/health", get(handle_health))
        .with_state(state);

    tracing::info!("Control service listening on {}", cli.listen);
    let listener = tokio::net::TcpListener::bind(cli.listen).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

// ── Handlers ──

async fn handle_register(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RegisterRequest>,
) -> Result<Json<RegisterResponse>, StatusCode> {
    let mut peers = state.peers.write().await;

    // Check if this public key is already registered
    for existing in peers.values() {
        if existing.public_key == req.public_key {
            let resp = RegisterResponse {
                device_id: existing.device_id,
                ipv4: existing.ipv4.to_string(),
                network: "100.64.0.0/10".to_string(),
            };
            return Ok(Json(resp));
        }
    }

    let device_id = Uuid::new_v4();
    let ip = allocate_ip(&state).await;

    let record = PeerRecord {
        device_id,
        public_key: req.public_key,
        hostname: req.hostname,
        ipv4: ip,
    };

    tracing::info!(%device_id, %ip, "device registered");
    peers.insert(device_id, record);

    let resp = RegisterResponse {
        device_id,
        ipv4: ip.to_string(),
        network: "100.64.0.0/10".to_string(),
    };
    Ok(Json(resp))
}

async fn handle_peers(
    State(state): State<Arc<AppState>>,
) -> Json<PeersResponse> {
    let peers = state.peers.read().await;
    let list: Vec<PeerInfo> = peers
        .values()
        .map(|p| PeerInfo {
            device_id: p.device_id,
            ipv4: p.ipv4.to_string(),
            public_key: p.public_key,
            hostname: p.hostname.clone(),
        })
        .collect();
    Json(PeersResponse { peers: list })
}

async fn handle_health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

// ── IP allocation ──

const IP_BASE: u32 = 0x6440_0000; // 100.64.0.0

async fn allocate_ip(state: &AppState) -> Ipv4Addr {
    let mut next = state.next_ip.write().await;
    let offset = *next;
    *next += 1;
    let ip_u32 = IP_BASE + offset;
    Ipv4Addr::from(ip_u32.to_be_bytes())
}
