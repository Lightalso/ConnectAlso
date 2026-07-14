use std::net::Ipv4Addr;
use std::str::FromStr;

use anyhow::Context;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use uuid::Uuid;

/// Database record for a registered device.
#[derive(Debug, Clone)]
pub struct DeviceRecord {
    pub device_id: Uuid,
    pub public_key: [u8; 32],
    pub hostname: String,
    pub ipv4: Ipv4Addr,
    pub created_at: i64,
    pub last_seen: i64,
}

/// Initialize the SQLite database and create tables.
pub async fn init_db(path: &str) -> anyhow::Result<SqlitePool> {
    let opts = SqliteConnectOptions::from_str(path)?
        .create_if_missing(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(opts)
        .await
        .context("failed to open database")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS devices (
            device_id  TEXT PRIMARY KEY,
            public_key BLOB NOT NULL UNIQUE,
            hostname   TEXT NOT NULL,
            ipv4       TEXT NOT NULL UNIQUE,
            created_at INTEGER NOT NULL,
            last_seen  INTEGER NOT NULL
        )
        "#,
    )
    .execute(&pool)
    .await?;

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

    tracing::info!("database initialized: {path}");
    Ok(pool)
}

/// Find a device by its public key, or return `None`.
pub async fn find_by_public_key(pool: &SqlitePool, pk: &[u8; 32]) -> Option<DeviceRecord> {
    sqlx::query_as::<_, DeviceRow>(
        "SELECT device_id, public_key, hostname, ipv4, created_at, last_seen FROM devices WHERE public_key = ?",
    )
    .bind(pk.as_slice())
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .map(|r| r.into())
}

/// Insert or update (heartbeat) a device.
pub async fn upsert_device(pool: &SqlitePool, dev: &DeviceRecord) -> anyhow::Result<()> {
    let now = unix_now();
    sqlx::query(
        r#"
        INSERT INTO devices (device_id, public_key, hostname, ipv4, created_at, last_seen)
        VALUES (?, ?, ?, ?, ?, ?)
        ON CONFLICT(device_id) DO UPDATE SET
            hostname = excluded.hostname,
            last_seen = excluded.last_seen
        "#,
    )
    .bind(dev.device_id.to_string())
    .bind(dev.public_key.as_slice())
    .bind(&dev.hostname)
    .bind(dev.ipv4.to_string())
    .bind(dev.created_at)
    .bind(now)
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

/// List all devices.
pub async fn list_all(pool: &SqlitePool) -> anyhow::Result<Vec<DeviceRecord>> {
    let rows = sqlx::query_as::<_, DeviceRow>(
        "SELECT device_id, public_key, hostname, ipv4, created_at, last_seen FROM devices ORDER BY created_at",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|r| r.into()).collect())
}

/// Delete a device and free its IP.
pub async fn delete_device(pool: &SqlitePool, device_id: Uuid) -> anyhow::Result<bool> {
    // Free the IP first
    sqlx::query("DELETE FROM ip_pool WHERE ipv4 = (SELECT ipv4 FROM devices WHERE device_id = ?)")
        .bind(device_id.to_string())
        .execute(pool)
        .await?;

    let rows = sqlx::query("DELETE FROM devices WHERE device_id = ?")
        .bind(device_id.to_string())
        .execute(pool)
        .await?;
    Ok(rows.rows_affected() > 0)
}

/// Remove devices that haven't sent a heartbeat in `timeout_secs`.
pub async fn purge_stale(pool: &SqlitePool, timeout_secs: i64) -> anyhow::Result<usize> {
    let cutoff = unix_now() - timeout_secs;
    // Free IPs for stale devices
    sqlx::query("DELETE FROM ip_pool WHERE ipv4 IN (SELECT ipv4 FROM devices WHERE last_seen < ?)")
        .bind(cutoff)
        .execute(pool)
        .await?;

    let rows = sqlx::query("DELETE FROM devices WHERE last_seen < ?")
        .bind(cutoff)
        .execute(pool)
        .await?;
    let count = rows.rows_affected() as usize;
    if count > 0 {
        tracing::info!(count, "purged stale devices");
    }
    Ok(count)
}

// ── IP Pool ──

/// Allocate the next available IPv4 from the configured CIDR range.
/// Returns `None` if the pool is exhausted.
pub async fn allocate_ip(pool: &SqlitePool, base: u32, max_offset: u32) -> Option<Ipv4Addr> {
    // Find first unallocated offset
    let row = sqlx::query_as::<_, IpPoolRow>(
        "SELECT ipv4, allocated FROM ip_pool WHERE allocated = 0 ORDER BY ipv4 LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();

    if let Some(r) = row {
        // Mark as allocated
        sqlx::query("UPDATE ip_pool SET allocated = 1 WHERE ipv4 = ?")
            .bind(&r.ipv4)
            .execute(pool)
            .await
            .ok();
        return Some(r.ipv4.parse().ok()?);
    }

    // No free slot — allocate new one if within range
    let used: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ip_pool")
        .fetch_one(pool)
        .await
        .unwrap_or(0);

    if used as u32 > max_offset {
        return None;
    }

    let ip_u32 = base + used as u32;
    let ip = Ipv4Addr::from(ip_u32.to_be_bytes());
    let ip_str = ip.to_string();

    sqlx::query("INSERT INTO ip_pool (ipv4, allocated) VALUES (?, 1)")
        .bind(&ip_str)
        .execute(pool)
        .await
        .ok();

    Some(ip)
}

/// Return the number of allocated IPs.
pub async fn allocated_count(pool: &SqlitePool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM ip_pool WHERE allocated = 1")
        .fetch_one(pool)
        .await
        .unwrap_or(0)
}

/// Return the total pool size (allocated + free).
pub async fn pool_size(pool: &SqlitePool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM ip_pool")
        .fetch_one(pool)
        .await
        .unwrap_or(0)
}

// ── Row types ──

#[derive(sqlx::FromRow)]
struct DeviceRow {
    device_id: String,
    public_key: Vec<u8>,
    hostname: String,
    ipv4: String,
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

// ── Candidate exchange ──

/// Publish or refresh a candidate address for a device.
pub async fn upsert_candidate(
    pool: &SqlitePool,
    device_id: Uuid,
    address: &str,
) -> anyhow::Result<()> {
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

/// List candidate addresses for a device.
pub async fn get_candidates(pool: &SqlitePool, device_id: Uuid) -> anyhow::Result<Vec<String>> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT address FROM candidates WHERE device_id = ? ORDER BY updated_at DESC",
    )
    .bind(device_id.to_string())
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|r| r.0).collect())
}

/// Purge candidate entries older than `timeout_secs`.
pub async fn purge_stale_candidates(pool: &SqlitePool, timeout_secs: i64) -> anyhow::Result<usize> {
    let cutoff = unix_now() - timeout_secs;
    let rows = sqlx::query("DELETE FROM candidates WHERE updated_at < ?")
        .bind(cutoff)
        .execute(pool)
        .await?;
    Ok(rows.rows_affected() as usize)
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
