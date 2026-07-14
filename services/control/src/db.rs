use std::net::Ipv4Addr;
use std::path::Path;
use std::str::FromStr;

use anyhow::Context;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use uuid::Uuid;

/// Device status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum DeviceStatus {
    Pending = 0,
    Approved = 1,
    Revoked = -1,
}

impl DeviceStatus {
    pub const fn from_i32(v: i32) -> Self {
        match v {
            1 => Self::Approved,
            -1 => Self::Revoked,
            _ => Self::Pending,
        }
    }
}

/// Database record for a registered device.
#[derive(Debug, Clone)]
pub struct DeviceRecord {
    pub device_id: Uuid,
    pub public_key: [u8; 32],
    pub hostname: String,
    pub ipv4: Ipv4Addr,
    pub status: DeviceStatus,
    pub created_at: i64,
    pub last_seen: i64,
}

/// Initialize the SQLite database and create tables.
pub async fn init_db(path: &str) -> anyhow::Result<SqlitePool> {
    let opts = SqliteConnectOptions::from_str(path)?.create_if_missing(true);

    let pool =
        SqlitePoolOptions::new().max_connections(5).connect_with(opts).await.context("failed to open database")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS devices (
            device_id  TEXT PRIMARY KEY,
            public_key BLOB NOT NULL UNIQUE,
            hostname   TEXT NOT NULL,
            ipv4       TEXT NOT NULL UNIQUE,
            status     INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL,
            last_seen  INTEGER NOT NULL
        )
        "#,
    )
    .execute(&pool)
    .await?;

    // Migration: add status column if missing (for existing databases)
    let _ = sqlx::query("ALTER TABLE devices ADD COLUMN status INTEGER NOT NULL DEFAULT 1").execute(&pool).await;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS ip_pool (
            ipv4       TEXT PRIMARY KEY,
            allocated  INTEGER NOT NULL DEFAULT 0
        )
        "#,
    )
    .execute(&pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS candidates (
            device_id  TEXT NOT NULL,
            address    TEXT NOT NULL,
            updated_at INTEGER NOT NULL,
            PRIMARY KEY (device_id, address)
        )
        "#,
    )
    .execute(&pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS acl_rules (
            id         INTEGER PRIMARY KEY AUTOINCREMENT,
            priority   INTEGER NOT NULL DEFAULT 0,
            action     TEXT NOT NULL DEFAULT 'allow',
            src_ip     TEXT NOT NULL DEFAULT '',
            dst_ip     TEXT NOT NULL DEFAULT '',
            protocol   TEXT NOT NULL DEFAULT '',
            src_port   INTEGER NOT NULL DEFAULT 0,
            dst_port   INTEGER NOT NULL DEFAULT 0
        )
        "#,
    )
    .execute(&pool)
    .await?;

    tracing::info!("database initialized: {path}");
    Ok(pool)
}

/// Find a device by its public key, or return `None`.
pub async fn find_by_public_key(pool: &SqlitePool, pk: &[u8; 32]) -> Option<DeviceRecord> {
    sqlx::query_as::<_, DeviceRow>(
        "SELECT device_id, public_key, hostname, ipv4, status, created_at, last_seen FROM devices WHERE public_key = ?",
    )
    .bind(pk.as_slice())
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .map(|r| r.into())
}

/// Insert a new device.
pub async fn insert_device(pool: &SqlitePool, dev: &DeviceRecord) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        INSERT INTO devices (device_id, public_key, hostname, ipv4, status, created_at, last_seen)
        VALUES (?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(dev.device_id.to_string())
    .bind(dev.public_key.as_slice())
    .bind(&dev.hostname)
    .bind(dev.ipv4.to_string())
    .bind(dev.status as i32)
    .bind(dev.created_at)
    .bind(dev.last_seen)
    .execute(pool)
    .await?;
    Ok(())
}

/// Update only the last_seen timestamp.
pub async fn heartbeat(pool: &SqlitePool, device_id: Uuid) -> anyhow::Result<bool> {
    let now = unix_now();
    let rows = sqlx::query("UPDATE devices SET last_seen = ? WHERE device_id = ?")
        .bind(now)
        .bind(device_id.to_string())
        .execute(pool)
        .await?;
    Ok(rows.rows_affected() > 0)
}

/// List all approved devices (for peer discovery).
pub async fn list_approved(pool: &SqlitePool) -> anyhow::Result<Vec<DeviceRecord>> {
    let rows = sqlx::query_as::<_, DeviceRow>(
        "SELECT device_id, public_key, hostname, ipv4, status, created_at, last_seen
         FROM devices WHERE status = 1 ORDER BY created_at",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|r| r.into()).collect())
}

/// List all devices (for admin).
pub async fn list_all(pool: &SqlitePool) -> anyhow::Result<Vec<DeviceRecord>> {
    let rows = sqlx::query_as::<_, DeviceRow>(
        "SELECT device_id, public_key, hostname, ipv4, status, created_at, last_seen FROM devices ORDER BY created_at",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|r| r.into()).collect())
}

/// List pending devices (status = 0).
pub async fn list_pending(pool: &SqlitePool) -> anyhow::Result<Vec<DeviceRecord>> {
    let rows = sqlx::query_as::<_, DeviceRow>(
        "SELECT device_id, public_key, hostname, ipv4, status, created_at, last_seen
         FROM devices WHERE status = 0 ORDER BY created_at",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|r| r.into()).collect())
}

/// Find a device by ID.
pub async fn find_by_id(pool: &SqlitePool, device_id: Uuid) -> Option<DeviceRecord> {
    sqlx::query_as::<_, DeviceRow>(
        "SELECT device_id, public_key, hostname, ipv4, status, created_at, last_seen
         FROM devices WHERE device_id = ?",
    )
    .bind(device_id.to_string())
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .map(|r| r.into())
}

/// Approve a pending device.
pub async fn approve_device(pool: &SqlitePool, device_id: Uuid) -> anyhow::Result<bool> {
    let rows = sqlx::query("UPDATE devices SET status = 1 WHERE device_id = ? AND status = 0")
        .bind(device_id.to_string())
        .execute(pool)
        .await?;
    Ok(rows.rows_affected() > 0)
}

/// Revoke an approved device (keeps record, frees IP).
pub async fn revoke_device(pool: &SqlitePool, device_id: Uuid) -> anyhow::Result<bool> {
    // Free IP
    sqlx::query("DELETE FROM ip_pool WHERE ipv4 = (SELECT ipv4 FROM devices WHERE device_id = ?)")
        .bind(device_id.to_string())
        .execute(pool)
        .await?;

    let rows = sqlx::query("UPDATE devices SET status = -1, ipv4 = '' WHERE device_id = ? AND status = 1")
        .bind(device_id.to_string())
        .execute(pool)
        .await?;
    Ok(rows.rows_affected() > 0)
}

/// Delete a device completely.
pub async fn delete_device(pool: &SqlitePool, device_id: Uuid) -> anyhow::Result<bool> {
    sqlx::query("DELETE FROM ip_pool WHERE ipv4 = (SELECT ipv4 FROM devices WHERE device_id = ?)")
        .bind(device_id.to_string())
        .execute(pool)
        .await?;

    let rows = sqlx::query("DELETE FROM devices WHERE device_id = ?").bind(device_id.to_string()).execute(pool).await?;
    Ok(rows.rows_affected() > 0)
}

/// Remove devices that haven't sent a heartbeat in `timeout_secs`.
pub async fn purge_stale(pool: &SqlitePool, timeout_secs: i64) -> anyhow::Result<usize> {
    let cutoff = unix_now() - timeout_secs;
    sqlx::query("DELETE FROM ip_pool WHERE ipv4 IN (SELECT ipv4 FROM devices WHERE last_seen < ?)")
        .bind(cutoff)
        .execute(pool)
        .await?;

    let rows =
        sqlx::query("DELETE FROM devices WHERE last_seen < ? AND status != 1").bind(cutoff).execute(pool).await?;
    let count = rows.rows_affected() as usize;
    if count > 0 {
        tracing::info!(count, "purged stale devices");
    }
    Ok(count)
}

// ── IP Pool ──

pub async fn allocate_ip(pool: &SqlitePool, base: u32, max_offset: u32) -> Option<Ipv4Addr> {
    let row =
        sqlx::query_as::<_, IpPoolRow>("SELECT ipv4, allocated FROM ip_pool WHERE allocated = 0 ORDER BY ipv4 LIMIT 1")
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();

    if let Some(r) = row {
        sqlx::query("UPDATE ip_pool SET allocated = 1 WHERE ipv4 = ?").bind(&r.ipv4).execute(pool).await.ok();
        return Some(r.ipv4.parse().ok()?);
    }

    let used: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ip_pool").fetch_one(pool).await.unwrap_or(0);

    if used as u32 > max_offset {
        return None;
    }

    let ip_u32 = base + used as u32;
    let ip = Ipv4Addr::from(ip_u32.to_be_bytes());
    let ip_str = ip.to_string();

    sqlx::query("INSERT INTO ip_pool (ipv4, allocated) VALUES (?, 1)").bind(&ip_str).execute(pool).await.ok();

    Some(ip)
}

pub async fn allocated_count(pool: &SqlitePool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM ip_pool WHERE allocated = 1").fetch_one(pool).await.unwrap_or(0)
}

pub async fn pool_size(pool: &SqlitePool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM ip_pool").fetch_one(pool).await.unwrap_or(0)
}

// ── Backup & Restore ──

/// Path to the backup file.
pub fn backup_path(db_path: &str) -> String {
    format!("{db_path}.backup")
}

/// Create a backup by copying the SQLite database file.
pub async fn create_backup(db_path: &str) -> anyhow::Result<String> {
    let src = Path::new(db_path);
    let dst = Path::new(&backup_path(db_path));

    anyhow::ensure!(src.exists(), "database file not found");
    tokio::fs::copy(src, dst).await?;

    let size = tokio::fs::metadata(dst).await?.len();
    tracing::info!(path = %dst.display(), size, "backup created");
    Ok(dst.display().to_string())
}

/// Restore from a backup file.
pub async fn restore_backup(db_path: &str) -> anyhow::Result<()> {
    let src = Path::new(&backup_path(db_path));
    anyhow::ensure!(src.exists(), "backup file not found");

    let dst = Path::new(db_path);
    tokio::fs::copy(src, dst).await?;

    tracing::info!(path = %dst.display(), "database restored from backup");
    Ok(())
}

// ── Candidate exchange ──

pub async fn upsert_candidate(pool: &SqlitePool, device_id: Uuid, address: &str) -> anyhow::Result<()> {
    let now = unix_now();
    sqlx::query(
        "INSERT INTO candidates (device_id, address, updated_at) VALUES (?, ?, ?)
         ON CONFLICT(device_id, address) DO UPDATE SET updated_at = ?",
    )
    .bind(device_id.to_string())
    .bind(address)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_candidates(pool: &SqlitePool, device_id: Uuid) -> anyhow::Result<Vec<String>> {
    let rows: Vec<(String,)> =
        sqlx::query_as("SELECT address FROM candidates WHERE device_id = ? ORDER BY updated_at DESC")
            .bind(device_id.to_string())
            .fetch_all(pool)
            .await?;
    Ok(rows.into_iter().map(|r| r.0).collect())
}

pub async fn purge_stale_candidates(pool: &SqlitePool, timeout_secs: i64) -> anyhow::Result<usize> {
    let cutoff = unix_now() - timeout_secs;
    let rows = sqlx::query("DELETE FROM candidates WHERE updated_at < ?").bind(cutoff).execute(pool).await?;
    Ok(rows.rows_affected() as usize)
}

// ── Row types ──

#[derive(sqlx::FromRow)]
struct DeviceRow {
    device_id: String,
    public_key: Vec<u8>,
    hostname: String,
    ipv4: String,
    status: i32,
    created_at: i64,
    last_seen: i64,
}

impl From<DeviceRow> for DeviceRecord {
    fn from(r: DeviceRow) -> Self {
        let mut pk = [0u8; 32];
        pk.copy_from_slice(&r.public_key[..32.min(r.public_key.len())]);
        Self {
            device_id: Uuid::parse_str(&r.device_id).unwrap_or_else(|_| Uuid::nil()),
            public_key: pk,
            hostname: r.hostname,
            ipv4: r.ipv4.parse().unwrap_or(Ipv4Addr::UNSPECIFIED),
            status: DeviceStatus::from_i32(r.status),
            created_at: r.created_at,
            last_seen: r.last_seen,
        }
    }
}

#[derive(sqlx::FromRow)]
struct IpPoolRow {
    ipv4: String,
    allocated: i64,
}

// ── ACL ──

/// A database row representing an ACL rule.
#[derive(sqlx::FromRow)]
pub struct AclRuleRow {
    pub id: i64,
    pub priority: i32,
    pub action: String,
    pub src_ip: String,
    pub dst_ip: String,
    pub protocol: String,
    pub src_port: i32,
    pub dst_port: i32,
}

pub async fn list_acl_rules(pool: &SqlitePool) -> anyhow::Result<Vec<AclRuleRow>> {
    sqlx::query_as::<_, AclRuleRow>(
        "SELECT id, priority, action, src_ip, dst_ip, protocol, src_port, dst_port FROM acl_rules ORDER BY priority",
    )
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

pub async fn upsert_acl_rule(pool: &SqlitePool, rule: &AclRuleRow) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO acl_rules (id, priority, action, src_ip, dst_ip, protocol, src_port, dst_port)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET
            priority=excluded.priority, action=excluded.action,
            src_ip=excluded.src_ip, dst_ip=excluded.dst_ip,
            protocol=excluded.protocol, src_port=excluded.src_port, dst_port=excluded.dst_port",
    )
    .bind(rule.id)
    .bind(rule.priority)
    .bind(&rule.action)
    .bind(&rule.src_ip)
    .bind(&rule.dst_ip)
    .bind(&rule.protocol)
    .bind(rule.src_port)
    .bind(rule.dst_port)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete_acl_rule(pool: &SqlitePool, id: i64) -> anyhow::Result<bool> {
    let rows = sqlx::query("DELETE FROM acl_rules WHERE id = ?").bind(id).execute(pool).await?;
    Ok(rows.rows_affected() > 0)
}

// ── DNS ──

pub async fn list_dns_records(pool: &SqlitePool) -> anyhow::Result<Vec<(String, String)>> {
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT hostname, ipv4 FROM devices WHERE status = 1 AND hostname != '' AND ipv4 != ''")
            .fetch_all(pool)
            .await?;
    Ok(rows)
}

fn unix_now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64
}
