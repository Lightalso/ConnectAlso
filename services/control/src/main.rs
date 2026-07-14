//! ConnectAlso 控制服务 — 设备注册、节点发现、ACL 管理。
//! ConnectAlso control service — device registration, peer discovery, ACL management.

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

/// 设备注册请求体。
/// Device registration request body.
#[derive(Debug, Deserialize)]
struct RegisterRequest {
    public_key: [u8; 32],
    hostname: String,
}

/// 设备注册响应体。
/// Device registration response body.
#[derive(Debug, Serialize)]
struct RegisterResponse {
    device_id: Uuid,
    ipv4: String,
    network: String,
    status: String,
}

/// 对等节点信息（对客户端公开）。
/// Peer information exposed to clients.
#[derive(Debug, Serialize)]
struct PeerInfo {
    device_id: Uuid,
    ipv4: String,
    public_key: [u8; 32],
    hostname: String,
    last_seen_secs: i64,
}

/// 管理端对等节点信息（含设备状态）。
/// Admin peer information with device status.
#[derive(Debug, Serialize)]
struct AdminPeerInfo {
    device_id: Uuid,
    ipv4: String,
    hostname: String,
    status: String,
    last_seen_secs: i64,
}

/// 对等节点列表响应体。
/// Peer list response body.
#[derive(Debug, Serialize)]
struct PeersResponse {
    peers: Vec<PeerInfo>,
}

/// 健康检查响应体。
/// Health check response body.
#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
}

/// 心跳响应体。
/// Heartbeat response body.
#[derive(Debug, Serialize)]
struct HeartbeatResponse {
    ok: bool,
}

/// 设备删除响应体。
/// Device deletion response body.
#[derive(Debug, Serialize)]
struct DeleteResponse {
    deleted: bool,
}

/// IP 地址池分配状态响应体。
/// IP pool allocation status response body.
#[derive(Debug, Serialize)]
struct AllocationResponse {
    total: i64,
    used: i64,
    free: i64,
    network: String,
}

/// 心跳请求体。
/// Heartbeat request body.
#[derive(Debug, Deserialize)]
struct HeartbeatRequest {
    device_id: Uuid,
}

/// ICE/STUN 候选地址发布请求体。
/// ICE/STUN candidate publish request body.
#[derive(Debug, Deserialize)]
struct CandidatePublishRequest {
    device_id: Uuid,
    candidates: Vec<String>,
}

/// 候选地址列表响应体。
/// Candidate list response body.
#[derive(Debug, Serialize)]
struct CandidateListResponse {
    device_id: Uuid,
    candidates: Vec<String>,
}

/// 审批响应体。
/// Approval response body.
#[derive(Debug, Serialize)]
struct ApprovalResponse {
    approved: bool,
    device_id: Uuid,
}

/// 撤销审批响应体。
/// Revoke response body.
#[derive(Debug, Serialize)]
struct RevokeResponse {
    revoked: bool,
    device_id: Uuid,
}

/// 备份响应体。
/// Backup response body.
#[derive(Debug, Serialize)]
struct BackupResponse {
    path: String,
    success: bool,
}

/// 还原响应体。
/// Restore response body.
#[derive(Debug, Serialize)]
struct RestoreResponse {
    success: bool,
}

// ═══════════════════════════════════════════════════════════════════
// App state
// ═══════════════════════════════════════════════════════════════════

/// 应用全局状态。
/// Application global state.
struct AppState {
    /// 数据库连接池。
    /// Database connection pool.
    db: SqlitePool,
    /// 数据库文件路径。
    /// Database file path.
    db_path: String,
    /// IP 地址池起始地址（网络前缀）。
    /// IP pool base address (network prefix).
    ip_base: u32,
    /// IP 地址池最大偏移量。
    /// Maximum offset from base for IP allocation.
    ip_max_offset: u32,
    /// 虚拟网络 CIDR 表示法。
    /// Virtual network in CIDR notation.
    network_cidr: String,
    /// 设备过期超时时间（秒）。
    /// Stale device timeout in seconds.
    stale_timeout: i64,
}

// ═══════════════════════════════════════════════════════════════════
// CLI
// ═══════════════════════════════════════════════════════════════════

/// 命令行参数。
/// Command-line arguments.
#[derive(Parser)]
#[command(name = "connectalso-control")]
#[command(about = "ConnectAlso 控制服务")]
struct Cli {
    /// 监听地址。
    /// Listening address.
    #[arg(long, default_value = "0.0.0.0:3000")]
    listen: SocketAddr,

    /// SQLite 数据库路径
    /// Path to the SQLite database file.
    #[arg(long, default_value = "connectalso.db")]
    db_path: String,

    /// 虚拟网络 CIDR (IPv4 地址池)
    /// Virtual network CIDR (IPv4 address pool).
    #[arg(long, default_value = "100.64.0.0/16")]
    network: String,

    /// 设备心跳超时（秒），超时后自动注销
    /// Device heartbeat timeout in seconds; expired devices are purged.
    #[arg(long, default_value_t = 300)]
    stale_timeout: i64,
}

/// 解析 CIDR 格式的 IP 前缀，返回（网络基地址, 最大偏移量）。
///
/// # Returns
/// 成功时返回 `(ip_base, ip_max_offset)`。
///
/// # Errors
/// 如果格式无效或前缀长度超过 32 则返回错误。
///
/// Parse a CIDR IP prefix, returning (network base address, max offset).
///
/// # Returns
/// On success, returns `(ip_base, ip_max_offset)`.
///
/// # Errors
/// Returns an error if the format is invalid or the prefix exceeds 32.
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

/// 处理设备注册请求：分配 IP、写入数据库、如果是首台设备则自动审批。
///
/// # Errors
/// 如果 IP 池已耗尽则返回 409，如果数据库写入失败则返回 500。
///
/// Handle device registration: allocate IP, write to DB, auto-approve first device.
///
/// # Errors
/// Returns 409 if IP pool exhausted, 500 on database write failure.
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

/// 判断当前是否为第一台注册的设备（数据库中无任何记录）。
/// Determine if this is the first registered device (no rows in DB).
async fn is_first_device(pool: &SqlitePool) -> bool {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM devices").fetch_one(pool).await.unwrap_or(0);
    count == 0
}

/// 获取已审批设备列表供节点发现使用。
///
/// # Errors
/// 数据库查询失败时返回 500。
///
/// List approved devices for peer discovery.
///
/// # Errors
/// Returns 500 on database query failure.
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

/// 获取所有设备列表（管理端），含设备状态。
///
/// # Errors
/// 数据库查询失败时返回 500。
///
/// List all devices for admin view, including status.
///
/// # Errors
/// Returns 500 on database query failure.
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

/// 处理设备心跳：更新 `last_seen` 时间戳。
///
/// # Returns
/// 返回 `{ ok: true/false }` 表示设备是否存在。
///
/// Handle heartbeat: update `last_seen` timestamp.
///
/// # Returns
/// Returns `{ ok: true/false }` indicating whether the device exists.
async fn handle_heartbeat(
    State(state): State<Arc<AppState>>,
    Json(req): Json<HeartbeatRequest>,
) -> Result<Json<HeartbeatResponse>, StatusCode> {
    let ok = db::heartbeat(&state.db, req.device_id).await.unwrap_or(false);
    Ok(Json(HeartbeatResponse { ok }))
}

/// 注销设备：完全删除设备和 IP 分配记录。
///
/// # Returns
/// 返回 `{ deleted: true }` 如果成功。
///
/// Unregister a device: delete device and IP allocation records entirely.
///
/// # Returns
/// Returns `{ deleted: true }` on success.
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

/// 健康检查端点。
/// Health check endpoint.
async fn handle_health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

/// 查询 IP 地址池分配统计。
///
/// # Errors
/// 数据库读写失败时返回 500。
///
/// Query IP pool allocation statistics.
///
/// # Errors
/// Returns 500 on database read failure.
async fn handle_allocations(State(state): State<Arc<AppState>>) -> Result<Json<AllocationResponse>, StatusCode> {
    let total = db::pool_size(&state.db).await;
    let used = db::allocated_count(&state.db).await;
    Ok(Json(AllocationResponse { total, used, free: total.saturating_sub(used), network: state.network_cidr.clone() }))
}

/// 发布 ICE/STUN 候选地址。
///
/// # Errors
/// 数据库写入失败时返回 500。
///
/// Publish ICE/STUN candidate addresses.
///
/// # Errors
/// Returns 500 on database write failure.
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

/// 获取指定设备的候选地址列表。
///
/// # Errors
/// 设备未找到时返回 404。
///
/// Retrieve candidate addresses for a given device.
///
/// # Errors
/// Returns 404 if device not found.
async fn handle_get_candidates(
    State(state): State<Arc<AppState>>,
    Path(device_id): Path<Uuid>,
) -> Result<Json<CandidateListResponse>, StatusCode> {
    let candidates = db::get_candidates(&state.db, device_id).await.map_err(|_| StatusCode::NOT_FOUND)?;
    Ok(Json(CandidateListResponse { device_id, candidates }))
}

/// 审批待定设备，使其可被其他节点发现。
///
/// # Errors
/// 数据库操作失败时返回 500。
///
/// Approve a pending device, making it discoverable by other peers.
///
/// # Errors
/// Returns 500 on database operation failure.
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

/// 撤销已审批设备的权限（保留记录，释放 IP）。
///
/// # Errors
/// 数据库操作失败时返回 500。
///
/// Revoke an approved device (keep record, free IP).
///
/// # Errors
/// Returns 500 on database operation failure.
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

/// 列出所有待审批的设备。
///
/// # Errors
/// 数据库查询失败时返回 500。
///
/// List all pending approval devices.
///
/// # Errors
/// Returns 500 on database query failure.
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

/// 创建数据库备份。
///
/// # Errors
/// 文件操作失败时在 payload 中返回 `success: false`。
///
/// Create a database backup.
///
/// # Errors
/// Returns `success: false` in payload on file operation failure.
async fn handle_backup(State(state): State<Arc<AppState>>) -> Result<Json<BackupResponse>, StatusCode> {
    match db::create_backup(&state.db_path).await {
        Ok(path) => Ok(Json(BackupResponse { path, success: true })),
        Err(e) => {
            tracing::error!(%e, "backup failed");
            Ok(Json(BackupResponse { path: String::new(), success: false }))
        }
    }
}

/// 从备份文件还原数据库。
///
/// # Errors
/// 备份文件不存在或复制失败时在 payload 中返回 `success: false`。
///
/// Restore database from backup file.
///
/// # Errors
/// Returns `success: false` on missing backup or copy failure.
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

/// 查询 DNS 记录（已审批设备的主机名→IP 映射）。
///
/// # Errors
/// 数据库查询失败时返回 500。
///
/// Query DNS records (hostname→IP mapping for approved devices).
///
/// # Errors
/// Returns 500 on database query failure.
async fn handle_dns_records(State(state): State<Arc<AppState>>) -> Result<Json<serde_json::Value>, StatusCode> {
    let records = db::list_dns_records(&state.db).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let list: Vec<serde_json::Value> =
        records.into_iter().map(|(hostname, ipv4)| serde_json::json!({"hostname": hostname, "ipv4": ipv4})).collect();

    Ok(Json(serde_json::json!({"records": list})))
}

// ── ACL ──

/// 列出所有 ACL 规则。
///
/// # Errors
/// 数据库查询失败时返回 500。
///
/// List all ACL rules.
///
/// # Errors
/// Returns 500 on database query failure.
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

/// 插入或更新一条 ACL 规则。
///
/// # Errors
/// 数据库写入失败时返回 500。
///
/// Insert or update an ACL rule.
///
/// # Errors
/// Returns 500 on database write failure.
async fn handle_upsert_acl(
    State(state): State<Arc<AppState>>,
    Json(req): Json<db::AclRuleRow>,
) -> Result<StatusCode, StatusCode> {
    db::upsert_acl_rule(&state.db, &req).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(StatusCode::OK)
}

/// 删除一条 ACL 规则。
///
/// # Errors
/// 数据库操作失败时返回 500。
///
/// Delete an ACL rule.
///
/// # Errors
/// Returns 500 on database operation failure.
async fn handle_delete_acl(State(state): State<Arc<AppState>>, Path(id): Path<i64>) -> Result<StatusCode, StatusCode> {
    db::delete_acl_rule(&state.db, id).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(StatusCode::OK)
}

/// 获取当前 UNIX 时间戳（秒）。
///
/// # Panics
/// 当系统时钟早于 UNIX 纪元时，会静默返回 0。
///
/// Get the current UNIX timestamp in seconds.
///
/// # Panics
/// Silently returns 0 if system clock is earlier than the UNIX epoch.
fn unix_now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64
}
