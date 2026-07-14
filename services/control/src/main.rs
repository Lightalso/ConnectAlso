mod db;

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use clap::Parser;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

// ═══════════════════════════════════════════════════════════════════
// API DTOs
// ═══════════════════════════════════════════════════════════════════

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
    last_seen_secs: i64,
}

#[derive(Debug, Serialize)]
struct PeersResponse {
    peers: Vec<PeerInfo>,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
}

#[derive(Debug, Serialize)]
struct HeartbeatResponse {
    ok: bool,
}

#[derive(Debug, Serialize)]
struct DeleteResponse {
    deleted: bool,
}

#[derive(Debug, Serialize)]
struct AllocationResponse {
    total: i64,
    used: i64,
    free: i64,
    network: String,
}

#[derive(Debug, Deserialize)]
struct HeartbeatRequest {
    device_id: Uuid,
}

#[derive(Debug, Deserialize)]
struct CandidatePublishRequest {
    device_id: Uuid,
    candidates: Vec<String>,
}

#[derive(Debug, Serialize)]
struct CandidateListResponse {
    device_id: Uuid,
    candidates: Vec<String>,
}

// ═══════════════════════════════════════════════════════════════════
// App state
// ═══════════════════════════════════════════════════════════════════

struct AppState {
    db: SqlitePool,
    ip_base: u32,
    ip_max_offset: u32,
    network_cidr: String,
    stale_timeout: i64,
}

// ═══════════════════════════════════════════════════════════════════
// CLI
// ═══════════════════════════════════════════════════════════════════

#[derive(Parser)]
#[command(name = "connectalso-control")]
#[command(about = "ConnectAlso 控制服务")]
struct Cli {
    #[arg(long, default_value = "0.0.0.0:3000")]
    listen: SocketAddr,

    /// SQLite 数据库路径
    #[arg(long, default_value = "connectalso.db")]
    db_path: String,

    /// 虚拟网络 CIDR (IPv4 地址池)
    #[arg(long, default_value = "100.64.0.0/16")]
    network: String,

    /// 设备心跳超时（秒），超时后自动注销
    #[arg(long, default_value_t = 300)]
    stale_timeout: i64,
}

fn parse_cidr(cidr: &str) -> anyhow::Result<(u32, u32)> {
    let parts: Vec<&str> = cidr.split('/').collect();
    anyhow::ensure!(parts.len() == 2, "invalid CIDR format");
    let ip: Ipv4Addr = parts[0].parse()?;
    let prefix: u8 = parts[1].parse()?;
    anyhow::ensure!(prefix <= 32, "prefix must be <= 32");

    let base = u32::from_be_bytes(ip.octets());
    let max_offset = (1u32 << (32 - prefix)) - 1;
    Ok((base, max_offset))
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
    let (ip_base, ip_max_offset) = parse_cidr(&cli.network)?;

    tracing::info!(
        network = %cli.network,
        max_hosts = ip_max_offset + 1,
        "IP pool configured"
    );

    let db = db::init_db(&cli.db_path).await?;

    let state = Arc::new(AppState {
        db,
        ip_base,
        ip_max_offset,
        network_cidr: cli.network.clone(),
        stale_timeout: cli.stale_timeout,
    });

    // Spawn stale device purger
    let purge_state = state.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            if let Err(e) = db::purge_stale(&purge_state.db, purge_state.stale_timeout).await {
                tracing::error!(%e, "stale purge failed");
            }
        }
    });

    let app = Router::new()
        .route("/api/v1/register", post(handle_register))
        .route("/api/v1/register/{device_id}", delete(handle_unregister))
        .route("/api/v1/peers", get(handle_peers))
        .route("/api/v1/heartbeat", post(handle_heartbeat))
        .route("/api/v1/health", get(handle_health))
        .route("/api/v1/allocations", get(handle_allocations))
        .route("/api/v1/candidates", post(handle_publish_candidates))
        .route("/api/v1/candidates/{device_id}", get(handle_get_candidates))
        .with_state(state);

    tracing::info!("Control service listening on {}", cli.listen);
    let listener = tokio::net::TcpListener::bind(cli.listen).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════
// Handlers
// ═══════════════════════════════════════════════════════════════════

async fn handle_register(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RegisterRequest>,
) -> Result<Json<RegisterResponse>, StatusCode> {
    // Check if public key already registered
    if let Some(existing) = db::find_by_public_key(&state.db, &req.public_key).await {
        return Ok(Json(RegisterResponse {
            device_id: existing.device_id,
            ipv4: existing.ipv4.to_string(),
            network: state.network_cidr.clone(),
        }));
    }

    let device_id = Uuid::new_v4();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let ip = db::allocate_ip(&state.db, state.ip_base, state.ip_max_offset)
        .await
        .ok_or(StatusCode::CONFLICT)?;

    let record = db::DeviceRecord {
        device_id,
        public_key: req.public_key,
        hostname: req.hostname,
        ipv4: ip,
        created_at: now,
        last_seen: now,
    };

    db::upsert_device(&state.db, &record)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    tracing::info!(%device_id, %ip, "device registered");

    Ok(Json(RegisterResponse {
        device_id,
        ipv4: ip.to_string(),
        network: state.network_cidr.clone(),
    }))
}

async fn handle_peers(
    State(state): State<Arc<AppState>>,
) -> Result<Json<PeersResponse>, StatusCode> {
    let devices = db::list_all(&state.db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let peers: Vec<PeerInfo> = devices
        .into_iter()
        .map(|d| PeerInfo {
            device_id: d.device_id,
            ipv4: d.ipv4.to_string(),
            public_key: d.public_key,
            hostname: d.hostname,
            last_seen_secs: now.saturating_sub(d.last_seen),
        })
        .collect();

    Ok(Json(PeersResponse { peers }))
}

async fn handle_heartbeat(
    State(state): State<Arc<AppState>>,
    Json(req): Json<HeartbeatRequest>,
) -> Result<Json<HeartbeatResponse>, StatusCode> {
    let ok = db::heartbeat(&state.db, req.device_id)
        .await
        .unwrap_or(false);
    Ok(Json(HeartbeatResponse { ok }))
}

async fn handle_unregister(
    State(state): State<Arc<AppState>>,
    Path(device_id): Path<Uuid>,
) -> Result<Json<DeleteResponse>, StatusCode> {
    let deleted = db::delete_device(&state.db, device_id)
        .await
        .unwrap_or(false);

    if deleted {
        tracing::info!(%device_id, "device unregistered");
    }

    Ok(Json(DeleteResponse { deleted }))
}

async fn handle_health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

async fn handle_allocations(
    State(state): State<Arc<AppState>>,
) -> Result<Json<AllocationResponse>, StatusCode> {
    let total = db::pool_size(&state.db).await;
    let used = db::allocated_count(&state.db).await;
    Ok(Json(AllocationResponse {
        total,
        used,
        free: total.saturating_sub(used),
        network: state.network_cidr.clone(),
    }))
}

async fn handle_publish_candidates(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CandidatePublishRequest>,
) -> Result<StatusCode, StatusCode> {
    for addr in &req.candidates {
        db::upsert_candidate(&state.db, req.device_id, addr)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }
    tracing::debug!(device = %req.device_id, count = req.candidates.len(), "candidates published");
    Ok(StatusCode::OK)
}

async fn handle_get_candidates(
    State(state): State<Arc<AppState>>,
    Path(device_id): Path<Uuid>,
) -> Result<Json<CandidateListResponse>, StatusCode> {
    let candidates = db::get_candidates(&state.db, device_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    Ok(Json(CandidateListResponse {
        device_id,
        candidates,
    }))
}
