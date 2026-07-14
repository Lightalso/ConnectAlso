//! ConnectAlso CLI — command-line interface for managing the daemon.

use std::net::SocketAddr;

use clap::{Parser, Subcommand};
use serde::Deserialize;

#[derive(Parser)]
#[command(name = "connectalso")]
#[command(about = "ConnectAlso — 简单、安全的跨平台异地组网工具")]
struct Cli {
    #[arg(long, default_value = "http://127.0.0.1:9823", global = true)]
    daemon_url: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Status {
        #[arg(short, long)]
        verbose: bool,
    },
    Diag,
    Start {
        #[arg(long, default_value = "http://127.0.0.1:3000")]
        control_url: String,
        #[arg(long, default_value = "127.0.0.1:3478")]
        stun_server: SocketAddr,
        #[arg(long, default_value = "127.0.0.1:33478")]
        relay_server: SocketAddr,
        #[arg(short, long, default_value = "unnamed")]
        hostname: String,
    },
    Stop,
    /// 管理员命令
    Admin {
        #[command(subcommand)]
        action: AdminCmd,
    },
    /// 备份控制服务数据库
    Backup {
        #[arg(long, default_value = "http://127.0.0.1:3000")]
        control_url: String,
    },
    /// 从备份恢复控制服务数据库
    Restore {
        #[arg(long, default_value = "http://127.0.0.1:3000")]
        control_url: String,
    },
}

#[derive(Subcommand)]
enum AdminCmd {
    /// 列出待审批设备
    Pending {
        #[arg(long, default_value = "http://127.0.0.1:3000")]
        control_url: String,
    },
    /// 审批设备
    Approve {
        #[arg(long, default_value = "http://127.0.0.1:3000")]
        control_url: String,
        device_id: String,
    },
    /// 撤销设备
    Revoke {
        #[arg(long, default_value = "http://127.0.0.1:3000")]
        control_url: String,
        device_id: String,
    },
    /// 列出所有设备（含状态）
    Peers {
        #[arg(long, default_value = "http://127.0.0.1:3000")]
        control_url: String,
    },
}

#[derive(Debug, Deserialize)]
struct StatusResponse {
    device_id: String,
    virtual_ip: String,
    hostname: String,
    uptime_secs: u64,
    peer_count: usize,
    peers: Vec<StatusPeer>,
}

#[derive(Debug, Deserialize)]
struct StatusPeer {
    device_id: String,
    virtual_ip: String,
    hostname: String,
    #[serde(default)]
    path: String,
}

#[derive(Debug, Deserialize)]
struct DiagnosticsResponse {
    daemon: CheckResult,
    control: CheckResult,
    stun: CheckResult,
    relay: CheckResult,
    tun: CheckResult,
    peers: Vec<PeerDiag>,
}

#[derive(Debug, Deserialize)]
struct CheckResult {
    status: String,
    detail: String,
    latency_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct PeerDiag {
    hostname: String,
    virtual_ip: String,
    path: String,
    reachable: bool,
}

fn fmt_duration(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{h}h {m}m {s}s")
    } else if m > 0 {
        format!("{m}m {s}s")
    } else {
        format!("{s}s")
    }
}

fn status_icon(status: &str) -> &str {
    match status {
        "ok" => "✓",
        "warn" => "⚠",
        "error" => "✗",
        _ => "?",
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Status { verbose } => cmd_status(&cli.daemon_url, verbose).await?,
        Commands::Diag => cmd_diag(&cli.daemon_url).await?,
        Commands::Start { control_url, stun_server, relay_server, hostname } => {
            cmd_start(&control_url, stun_server, relay_server, &hostname).await?
        }
        Commands::Stop => cmd_stop(&cli.daemon_url).await?,
        Commands::Admin { action } => match action {
            AdminCmd::Pending { control_url } => cmd_admin_pending(&control_url).await?,
            AdminCmd::Approve { control_url, device_id } => cmd_admin_approve(&control_url, &device_id).await?,
            AdminCmd::Revoke { control_url, device_id } => cmd_admin_revoke(&control_url, &device_id).await?,
            AdminCmd::Peers { control_url } => cmd_admin_peers(&control_url).await?,
        },
        Commands::Backup { control_url } => cmd_backup(&control_url).await?,
        Commands::Restore { control_url } => cmd_restore(&control_url).await?,
    }
    Ok(())
}

async fn cmd_status(daemon_url: &str, verbose: bool) -> anyhow::Result<()> {
    let http = reqwest::Client::new();
    let resp = http.get(format!("{daemon_url}/status")).send().await;

    match resp {
        Ok(r) if r.status().is_success() => {
            let s: StatusResponse = r.json().await?;
            println!("ConnectAlso Daemon");
            println!("  Device ID : {}", s.device_id);
            println!("  Virtual IP: {}", s.virtual_ip);
            println!("  Hostname  : {}", s.hostname);
            println!("  Uptime    : {}", fmt_duration(s.uptime_secs));
            println!("  Peers     : {}", s.peer_count);

            if !s.peers.is_empty() {
                println!();
                if verbose {
                    println!("  {:<16}  {:<16}  {:<10}  HOSTNAME", "PEER", "VIRTUAL IP", "PATH");
                    println!("  {}", "-".repeat(64));
                    for p in &s.peers {
                        println!(
                            "  {:<16}  {:<16}  {:<10}  {}",
                            &p.device_id[..p.device_id.len().min(16)],
                            p.virtual_ip,
                            p.path,
                            p.hostname
                        );
                    }
                } else {
                    println!("  {:<20}  {:<16}  HOSTNAME", "PEER", "VIRTUAL IP");
                    println!("  {}", "-".repeat(58));
                    for p in &s.peers {
                        println!(
                            "  {:<20}  {:<16}  {}",
                            &p.device_id[..p.device_id.len().min(20)],
                            p.virtual_ip,
                            p.hostname
                        );
                    }
                }
            }
        }
        Ok(r) if r.status().is_server_error() => println!("Daemon error."),
        _ => println!("Daemon not running. Use: connectalso start"),
    }
    Ok(())
}

async fn cmd_diag(daemon_url: &str) -> anyhow::Result<()> {
    let http = reqwest::Client::new();
    let resp = http.get(format!("{daemon_url}/diagnostics")).send().await;

    match resp {
        Ok(r) if r.status().is_success() => {
            let d: DiagnosticsResponse = r.json().await?;
            println!("ConnectAlso Diagnostics");
            println!("{}", "─".repeat(52));

            let checks = [
                ("Daemon ", &d.daemon),
                ("Control", &d.control),
                ("STUN   ", &d.stun),
                ("Relay  ", &d.relay),
                ("TUN    ", &d.tun),
            ];

            for (name, check) in &checks {
                let icon = status_icon(&check.status);
                let lat = check.latency_ms.map_or("".into(), |ms| format!(" ({ms}ms)"));
                println!("  {icon} {name}: {}{lat}", check.detail);
            }

            if !d.peers.is_empty() {
                println!();
                println!("  Peers:");
                for p in &d.peers {
                    let icon = if p.reachable { "✓" } else { "✗" };
                    println!("    {icon} {:<12}  {:<16}  path={}", p.hostname, p.virtual_ip, p.path);
                }
            }

            let all_ok = checks.iter().all(|(_, c)| c.status == "ok");
            if all_ok {
                println!("\nAll checks passed.");
            }
        }
        _ => println!("Daemon not running. Use: connectalso start"),
    }
    Ok(())
}

async fn cmd_start(
    control_url: &str,
    stun_server: SocketAddr,
    relay_server: SocketAddr,
    hostname: &str,
) -> anyhow::Result<()> {
    println!("ConnectAlso Desktop Alpha");
    println!("  Control : {control_url}");
    println!("  STUN    : {stun_server}");
    println!("  Relay   : {relay_server}");
    println!("  Host    : {hostname}");

    let http = reqwest::Client::new();
    if http.get("http://127.0.0.1:9823/status").send().await.is_ok() {
        println!("\nDaemon already running.");
        return Ok(());
    }

    let child = std::process::Command::new("connectalso-daemon")
        .arg("--control-url")
        .arg(control_url)
        .arg("--stun-server")
        .arg(stun_server.to_string())
        .arg("--relay-server")
        .arg(relay_server.to_string())
        .arg("--hostname")
        .arg(hostname)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();

    match child {
        Ok(c) => println!("Daemon started (PID: {}).", c.id()),
        Err(_) => {
            eprintln!("'connectalso-daemon' not found. Run:");
            eprintln!("  cargo run -p connectalso-daemon -- --control-url {control_url} --hostname {hostname}");
        }
    }
    Ok(())
}

async fn cmd_stop(daemon_url: &str) -> anyhow::Result<()> {
    let http = reqwest::Client::new();
    match http.post(format!("{daemon_url}/shutdown")).send().await {
        Ok(r) if r.status().is_success() => println!("Shutdown requested."),
        _ => println!("Daemon not running."),
    }
    Ok(())
}

// ── Admin commands ──

async fn cmd_admin_pending(control_url: &str) -> anyhow::Result<()> {
    let http = reqwest::Client::new();
    let resp = http.get(format!("{control_url}/api/v1/register/pending")).send().await?;
    let body: serde_json::Value = resp.json().await?;
    let empty_arr = vec![];
    let pending = body["pending"].as_array().unwrap_or(&empty_arr);
    if pending.is_empty() {
        println!("No pending devices.");
    } else {
        println!("Pending devices ({})", pending.len());
        for d in pending {
            println!(
                "  {}  {}  {}",
                d["device_id"].as_str().unwrap_or("-"),
                d["hostname"].as_str().unwrap_or("-"),
                d["ipv4"].as_str().unwrap_or("-"),
            );
        }
        println!("\nApprove: connectalso admin approve <device_id>");
    }
    Ok(())
}

async fn cmd_admin_approve(control_url: &str, device_id: &str) -> anyhow::Result<()> {
    let http = reqwest::Client::new();
    let resp: serde_json::Value =
        http.put(format!("{control_url}/api/v1/register/{device_id}/approve")).send().await?.json().await?;
    if resp["approved"].as_bool().unwrap_or(false) {
        println!("Device {device_id} approved.");
    } else {
        println!("Approval failed — device not found or already approved.");
    }
    Ok(())
}

async fn cmd_admin_revoke(control_url: &str, device_id: &str) -> anyhow::Result<()> {
    let http = reqwest::Client::new();
    let resp: serde_json::Value =
        http.put(format!("{control_url}/api/v1/register/{device_id}/revoke")).send().await?.json().await?;
    if resp["revoked"].as_bool().unwrap_or(false) {
        println!("Device {device_id} revoked.");
    } else {
        println!("Revocation failed.");
    }
    Ok(())
}

async fn cmd_admin_peers(control_url: &str) -> anyhow::Result<()> {
    let http = reqwest::Client::new();
    let resp: serde_json::Value = http.get(format!("{control_url}/api/v1/admin/peers")).send().await?.json().await?;
    let empty_arr = vec![];
    let peers = resp["peers"].as_array().unwrap_or(&empty_arr);
    println!("All devices ({})", peers.len());
    println!("  {:<38}  {:<16}  {:<12}  HOSTNAME", "DEVICE ID", "IP", "STATUS");
    for d in peers {
        println!(
            "  {:<38}  {:<16}  {:<12}  {}",
            d["device_id"].as_str().unwrap_or("-"),
            d["ipv4"].as_str().unwrap_or("-"),
            d["status"].as_str().unwrap_or("-"),
            d["hostname"].as_str().unwrap_or("-"),
        );
    }
    Ok(())
}

async fn cmd_backup(control_url: &str) -> anyhow::Result<()> {
    let http = reqwest::Client::new();
    let resp: serde_json::Value = http.post(format!("{control_url}/api/v1/backup")).send().await?.json().await?;
    if resp["success"].as_bool().unwrap_or(false) {
        println!("Backup created: {}", resp["path"].as_str().unwrap_or("-"));
    } else {
        println!("Backup failed.");
    }
    Ok(())
}

async fn cmd_restore(control_url: &str) -> anyhow::Result<()> {
    println!("Warning: this will overwrite the current database.");
    println!("Continue? [y/N]");
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    if input.trim().to_lowercase() != "y" {
        return Ok(());
    }

    let http = reqwest::Client::new();
    let resp: serde_json::Value = http.post(format!("{control_url}/api/v1/restore")).send().await?.json().await?;
    if resp["success"].as_bool().unwrap_or(false) {
        println!("Database restored from backup.");
    } else {
        println!("Restore failed.");
    }
    Ok(())
}
