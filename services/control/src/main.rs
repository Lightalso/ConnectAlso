mod db;

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post, put};
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
    status: String,
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
struct AdminPeerInfo {
    device_id: Uuid,
    ipv4: String,
    hostname: String,
    status: String,
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

#[derive(Debug, Serialize)]
struct ApprovalResponse {
    approved: bool,
    device_id: Uuid,
}

#[derive(Debug, Serialize)]
struct RevokeResponse {
    revoked: bool,
    device_id: Uuid,
}

#[derive(Debug, Serialize)]
struct BackupResponse {
    path: String,
    success: bool,
}

#[derive(Debug, Serialize)]
struct RestoreResponse {
    success: bool,
}

// ═══════════════════════════════════════════════════════════════════
// App state
// ═══════════════════════════════════════════════════════════════════

struct AppState {
    db: SqlitePool,
    db_path: String,
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
    tracing_subscriber::fmt().with_env_filter(EnvFilter::from_default_env()).init();

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
        db_path: cli.db_path.clone(),
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
        .route("/api/v1/register/{device_id}/approve", put(handle_approve))
        .route("/api/v1/register/{device_id}/revoke", put(handle_revoke))
        .route("/api/v1/register/pending", get(handle_pending))
        .route("/api/v1/peers", get(handle_peers))
        .route("/api/v1/admin/peers", get(handle_admin_peers))
        .route("/api/v1/heartbeat", post(handle_heartbeat))
        .route("/api/v1/health", get(handle_health))
        .route("/api/v1/allocations", get(handle_allocations))
        .route("/api/v1/candidates", post(handle_publish_candidates))
        .route("/api/v1/candidates/{device_id}", get(handle_get_candidates))
        .route("/api/v1/backup", post(handle_backup))
        .route("/api/v1/restore", post(handle_restore))
        .route("/api/v1/dns", get(handle_dns_records))
        .route("/api/v1/acl", get(handle_list_acl))
        .route("/api/v1/acl", post(handle_upsert_acl))
        .route("/api/v1/acl/{id}", delete(handle_delete_acl))
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
        let status_str = match existing.status {
            db::DeviceStatus::Approved => "approved",
            db::DeviceStatus::Pending => "pending",
            db::DeviceStatus::Revoked => "revoked",
        };
        return Ok(Json(RegisterResponse {
            device_id: existing.device_id,
            ipv4: existing.ipv4.to_string(),
            network: state.network_cidr.clone(),
            status: status_str.to_string(),
        }));
    }

    let device_id = Uuid::new_v4();
    let now = unix_now();

    let ip = db::allocate_ip(&state.db, state.ip_base, state.ip_max_offset).await.ok_or(StatusCode::CONFLICT)?;

    let status = if is_first_device(&state.db).await { db::DeviceStatus::Approved } else { db::DeviceStatus::Pending };

    let record = db::DeviceRecord {
        device_id,
        public_key: req.public_key,
        hostname: req.hostname,
        ipv4: ip,
        status,
        created_at: now,
        last_seen: now,
    };

    db::insert_device(&state.db, &record).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let status_str = if status == db::DeviceStatus::Approved { "approved" } else { "pending" };
    tracing::info!(%device_id, %ip, %status_str, "device registered");

    Ok(Json(RegisterResponse {
        device_id,
        ipv4: ip.to_string(),
        network: state.network_cidr.clone(),
        status: status_str.to_string(),
    }))
}

async fn is_first_device(pool: &SqlitePool) -> bool {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM devices").fetch_one(pool).await.unwrap_or(0);
    count == 0
}

async fn handle_peers(State(state): State<Arc<AppState>>) -> Result<Json<PeersResponse>, StatusCode> {
    let devices = db::list_approved(&state.db).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let now = unix_now();

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

async fn handle_admin_peers(State(state): State<Arc<AppState>>) -> Result<Json<serde_json::Value>, StatusCode> {
    let devices = db::list_all(&state.db).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let now = unix_now();
    let peers: Vec<AdminPeerInfo> = devices
        .into_iter()
        .map(|d| AdminPeerInfo {
            device_id: d.device_id,
            ipv4: d.ipv4.to_string(),
            hostname: d.hostname,
            status: match d.status {
                db::DeviceStatus::Approved => "approved",
                db::DeviceStatus::Pending => "pending",
                db::DeviceStatus::Revoked => "revoked",
            }
            .to_string(),
            last_seen_secs: now.saturating_sub(d.last_seen),
        })
        .collect();

    Ok(Json(serde_json::json!({ "peers": peers })))
}

async fn handle_heartbeat(
    State(state): State<Arc<AppState>>,
    Json(req): Json<HeartbeatRequest>,
) -> Result<Json<HeartbeatResponse>, StatusCode> {
    let ok = db::heartbeat(&state.db, req.device_id).await.unwrap_or(false);
    Ok(Json(HeartbeatResponse { ok }))
}

async fn handle_unregister(
    State(state): State<Arc<AppState>>,
    Path(device_id): Path<Uuid>,
) -> Result<Json<DeleteResponse>, StatusCode> {
    let deleted = db::delete_device(&state.db, device_id).await.unwrap_or(false);

    if deleted {
        tracing::info!(%device_id, "device unregistered");
    }

    Ok(Json(DeleteResponse { deleted }))
}

async fn handle_health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

async fn handle_allocations(State(state): State<Arc<AppState>>) -> Result<Json<AllocationResponse>, StatusCode> {
    let total = db::pool_size(&state.db).await;
    let used = db::allocated_count(&state.db).await;
    Ok(Json(AllocationResponse { total, used, free: total.saturating_sub(used), network: state.network_cidr.clone() }))
}

async fn handle_publish_candidates(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CandidatePublishRequest>,
) -> Result<StatusCode, StatusCode> {
    for addr in &req.candidates {
        db::upsert_candidate(&state.db, req.device_id, addr).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }
    tracing::debug!(device = %req.device_id, count = req.candidates.len(), "candidates published");
    Ok(StatusCode::OK)
}

async fn handle_get_candidates(
    State(state): State<Arc<AppState>>,
    Path(device_id): Path<Uuid>,
) -> Result<Json<CandidateListResponse>, StatusCode> {
    let candidates = db::get_candidates(&state.db, device_id).await.map_err(|_| StatusCode::NOT_FOUND)?;
    Ok(Json(CandidateListResponse { device_id, candidates }))
}

async fn handle_approve(
    State(state): State<Arc<AppState>>,
    Path(device_id): Path<Uuid>,
) -> Result<Json<ApprovalResponse>, StatusCode> {
    let approved = db::approve_device(&state.db, device_id).await.unwrap_or(false);
    if approved {
        tracing::info!(%device_id, "device approved");
    }
    Ok(Json(ApprovalResponse { approved, device_id }))
}

async fn handle_revoke(
    State(state): State<Arc<AppState>>,
    Path(device_id): Path<Uuid>,
) -> Result<Json<RevokeResponse>, StatusCode> {
    let revoked = db::revoke_device(&state.db, device_id).await.unwrap_or(false);
    if revoked {
        tracing::info!(%device_id, "device revoked");
    }
    Ok(Json(RevokeResponse { revoked, device_id }))
}

async fn handle_pending(State(state): State<Arc<AppState>>) -> Result<Json<serde_json::Value>, StatusCode> {
    let devices = db::list_pending(&state.db).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let pending: Vec<serde_json::Value> = devices
        .into_iter()
        .map(|d| {
            serde_json::json!({
                "device_id": d.device_id.to_string(),
                "hostname": d.hostname,
                "ipv4": d.ipv4.to_string(),
                "created_at": d.created_at,
            })
        })
        .collect();

    Ok(Json(serde_json::json!({ "pending": pending })))
}

async fn handle_backup(State(state): State<Arc<AppState>>) -> Result<Json<BackupResponse>, StatusCode> {
    match db::create_backup(&state.db_path).await {
        Ok(path) => Ok(Json(BackupResponse { path, success: true })),
        Err(e) => {
            tracing::error!(%e, "backup failed");
            Ok(Json(BackupResponse { path: String::new(), success: false }))
        }
    }
}

async fn handle_restore(State(state): State<Arc<AppState>>) -> Result<Json<RestoreResponse>, StatusCode> {
    match db::restore_backup(&state.db_path).await {
        Ok(()) => Ok(Json(RestoreResponse { success: true })),
        Err(e) => {
            tracing::error!(%e, "restore failed");
            Ok(Json(RestoreResponse { success: false }))
        }
    }
}

// ── DNS ──

async fn handle_dns_records(State(state): State<Arc<AppState>>) -> Result<Json<serde_json::Value>, StatusCode> {
    let records = db::list_dns_records(&state.db).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let list: Vec<serde_json::Value> =
        records.into_iter().map(|(hostname, ipv4)| serde_json::json!({"hostname": hostname, "ipv4": ipv4})).collect();

    Ok(Json(serde_json::json!({"records": list})))
}

// ── ACL ──

async fn handle_list_acl(State(state): State<Arc<AppState>>) -> Result<Json<serde_json::Value>, StatusCode> {
    let rules = db::list_acl_rules(&state.db).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let list: Vec<serde_json::Value> = rules
        .into_iter()
        .map(|r| {
            serde_json::json!({
                "id": r.id,
                "priority": r.priority,
                "action": r.action,
                "src_ip": r.src_ip,
                "dst_ip": r.dst_ip,
                "protocol": r.protocol,
                "src_port": r.src_port,
                "dst_port": r.dst_port,
            })
        })
        .collect();

    Ok(Json(serde_json::json!({"rules": list})))
}

async fn handle_upsert_acl(
    State(state): State<Arc<AppState>>,
    Json(req): Json<db::AclRuleRow>,
) -> Result<StatusCode, StatusCode> {
    db::upsert_acl_rule(&state.db, &req).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(StatusCode::OK)
}

async fn handle_delete_acl(State(state): State<Arc<AppState>>, Path(id): Path<i64>) -> Result<StatusCode, StatusCode> {
    db::delete_acl_rule(&state.db, id).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(StatusCode::OK)
}

fn unix_now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64
}
