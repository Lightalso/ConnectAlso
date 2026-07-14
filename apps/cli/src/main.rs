use std::net::SocketAddr;

use clap::{Parser, Subcommand};
use serde::Deserialize;

#[derive(Parser)]
#[command(name = "connectalso")]
#[command(about = "ConnectAlso — 简单、安全的跨平台异地组网工具")]
struct Cli {
    /// 守护进程本地 API 地址
    #[arg(long, default_value = "http://127.0.0.1:9823", global = true)]
    daemon_url: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// 查看守护进程状态和已连接对等端
    Status,
    /// 启动守护进程（后台运行）
    Start {
        /// 控制服务地址
        #[arg(long, default_value = "http://127.0.0.1:3000")]
        control_url: String,

        #[arg(long, default_value = "127.0.0.1:3478")]
        stun_server: SocketAddr,

        #[arg(long, default_value = "127.0.0.1:33478")]
        relay_server: SocketAddr,

        #[arg(short, long, default_value = "unnamed")]
        hostname: String,
    },
    /// 停止守护进程
    Stop,
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
}

#[derive(Debug, Deserialize)]
struct ShutdownResponse {
    message: String,
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Status => cmd_status(&cli.daemon_url).await?,
        Commands::Start {
            control_url,
            stun_server,
            relay_server,
            hostname,
        } => cmd_start(&control_url, stun_server, relay_server, &hostname).await?,
        Commands::Stop => cmd_stop(&cli.daemon_url).await?,
    }

    Ok(())
}

async fn cmd_status(daemon_url: &str) -> anyhow::Result<()> {
    let http = reqwest::Client::new();
    let resp = http
        .get(format!("{daemon_url}/status"))
        .send()
        .await;

    match resp {
        Ok(r) if r.status().is_success() => {
            let status: StatusResponse = r.json().await?;
            println!("ConnectAlso Daemon");
            println!("  Device ID : {}", status.device_id);
            println!("  Virtual IP: {}", status.virtual_ip);
            println!("  Hostname  : {}", status.hostname);
            println!("  Uptime    : {}", fmt_duration(status.uptime_secs));
            println!("  Peers     : {}", status.peer_count);

            if !status.peers.is_empty() {
                println!();
                println!("  {:<20}  {:<16}  {}", "PEER", "VIRTUAL IP", "HOSTNAME");
                println!("  {}", "-".repeat(58));
                for p in &status.peers {
                    println!(
                        "  {:<20}  {:<16}  {}",
                        &p.device_id[..p.device_id.len().min(20)],
                        p.virtual_ip,
                        p.hostname
                    );
                }
            }
        }
        Ok(r) if r.status().is_server_error() => {
            println!("Daemon returned an error.");
        }
        _ => {
            println!("Daemon not running.");
            println!("Start with: connectalso start");
        }
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

    // Check if daemon already running
    let http = reqwest::Client::new();
    if http
        .get("http://127.0.0.1:9823/status")
        .send()
        .await
        .is_ok()
    {
        println!("\nDaemon is already running. Use 'connectalso status'.");
        return Ok(());
    }

    println!("\nStarting daemon...");

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
        Ok(c) => {
            println!("Daemon started (PID: {}).", c.id());
            println!("Run 'connectalso status' to view state.");
        }
        Err(_) => {
            eprintln!("Binary 'connectalso-daemon' not found.");
            eprintln!("Build and run manually:");
            eprintln!("  cargo run -p connectalso-daemon -- \\");
            eprintln!("    --control-url {control_url} \\");
            eprintln!("    --stun-server {stun_server} \\");
            eprintln!("    --relay-server {relay_server} \\");
            eprintln!("    --hostname {hostname}");
        }
    }

    Ok(())
}

async fn cmd_stop(daemon_url: &str) -> anyhow::Result<()> {
    let http = reqwest::Client::new();
    let resp = http
        .post(format!("{daemon_url}/shutdown"))
        .send()
        .await;

    match resp {
        Ok(r) if r.status().is_success() => {
            println!("Shutdown requested.");
        }
        _ => {
            println!("Daemon not running or shutdown failed.");
        }
    }

    Ok(())
}
