//! ConnectAlso 桌面托盘应用。
//! ConnectAlso desktop tray application.

use std::time::Duration;

use anyhow::Context;
use muda::{Menu, MenuItem, PredefinedMenuItem};
use serde::Deserialize;
use tray_icon::{Icon, TrayIconBuilder};

const DAEMON_URL: &str = "http://127.0.0.1:9823";

/// 守护进程状态响应数据。
/// Daemon status response data.
#[derive(Debug, Deserialize, Default)]
struct StatusResponse {
    /// 设备唯一标识符。
    /// Unique device identifier.
    device_id: String,
    /// 虚拟 IP 地址。
    /// Virtual IP address.
    #[allow(dead_code)]
    virtual_ip: String,
    /// 主机名。
    /// Hostname.
    #[allow(dead_code)]
    hostname: String,
    /// 运行时长（秒）。
    /// Uptime in seconds.
    #[allow(dead_code)]
    uptime_secs: u64,
    /// 已连接的对等节点数量。
    /// Number of connected peers.
    peer_count: usize,
    /// 对等节点列表。
    /// List of connected peers.
    #[serde(default)]
    peers: Vec<StatusPeer>,
}

/// 对等节点状态信息。
/// Status information for a connected peer.
#[derive(Debug, Deserialize, Default)]
struct StatusPeer {
    /// 对等节点主机名。
    /// Peer hostname.
    #[serde(default)]
    #[allow(dead_code)]
    hostname: String,
    /// 对等节点虚拟 IP。
    /// Peer virtual IP address.
    #[serde(default)]
    #[allow(dead_code)]
    virtual_ip: String,
    /// 连接路径类型（direct / relay / probing）。
    /// Connection path type (direct / relay / probing).
    #[serde(default)]
    path: String,
}

/// 系统托盘图标对应的连接状态。
/// Connection status represented by the tray icon.
#[derive(Debug, Clone, Copy, PartialEq)]
enum ConnStatus {
    /// P2P 直连。
    /// Direct P2P connection.
    Connected,
    /// 仅通过中继连接。
    /// Connected via relay only.
    RelayOnly,
    /// 未连接。
    /// Disconnected.
    Disconnected,
}

/// 根据 RGBA 颜色生成一个 32x32 的圆形托盘图标。
/// Generate a 32x32 circular tray icon from an RGBA color.
///
/// # Panics
///
/// 如果 RGBA 数据无效则会 panic。
/// Panics if the RGBA data is invalid.
fn make_icon(color: [u8; 4]) -> Icon {
    let (r, g, b, a) = (color[0], color[1], color[2], color[3]);
    let size: u32 = 32;
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let dx = (x as i32 - 16).abs();
            let dy = (y as i32 - 16).abs();
            let dist = ((dx * dx + dy * dy) as f32).sqrt();
            if dist <= 13.0 {
                let alpha = if dist > 11.5 { ((a as f32) * (13.0 - dist) / 1.5) as u8 } else { a };
                rgba.extend_from_slice(&[r, g, b, alpha]);
            } else {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
    }
    Icon::from_rgba(rgba, size, size).expect("valid icon")
}

/// 根据连接状态返回对应的 RGBA 颜色值。
/// Return the RGBA color corresponding to the connection status.
///
/// | Status       | Color  |
/// |--------------|--------|
/// | Connected    | 绿色   |
/// | RelayOnly    | 橙色   |
/// | Disconnected | 灰色   |
///
/// | Status       | Color  |
/// |--------------|--------|
/// | Connected    | Green  |
/// | RelayOnly    | Orange |
/// | Disconnected | Gray   |
fn status_color(status: ConnStatus) -> [u8; 4] {
    match status {
        ConnStatus::Connected => [0x4C, 0xAF, 0x50, 0xFF],
        ConnStatus::RelayOnly => [0xFF, 0x98, 0x00, 0xFF],
        ConnStatus::Disconnected => [0x9E, 0x9E, 0x9E, 0xFF],
    }
}

/// 将连接状态转换为可读的文本标签。
/// Convert the connection status to a human-readable label.
fn fmt_status(status: ConnStatus) -> &'static str {
    match status {
        ConnStatus::Connected => "Connected (P2P)",
        ConnStatus::RelayOnly => "Connected (Relay)",
        ConnStatus::Disconnected => "Disconnected",
    }
}

/// 通过 HTTP 请求获取守护进程的连接状态。
/// Fetch the daemon's connection status via HTTP.
///
/// # Returns
///
/// 根据对等节点的路径类型返回对应的 [`ConnStatus`]。
/// Returns [`ConnStatus`] based on the peers' path types.
async fn fetch_status(url: &str) -> ConnStatus {
    match reqwest::get(format!("{url}/status")).await {
        Ok(r) if r.status().is_success() => match r.json::<StatusResponse>().await {
            Ok(s) if !s.device_id.is_empty() => {
                if s.peers.iter().any(|p| p.path == "direct") {
                    ConnStatus::Connected
                } else if s.peer_count > 0 {
                    ConnStatus::RelayOnly
                } else {
                    ConnStatus::Disconnected
                }
            }
            _ => ConnStatus::Disconnected,
        },
        _ => ConnStatus::Disconnected,
    }
}

/// 桌面托盘应用入口：创建系统托盘图标并周期性轮询状态。
/// Desktop tray application entry: create tray icon and poll status periodically.
///
/// # Errors
///
/// 如果无法创建系统托盘图标则返回错误。
/// Returns an error if the system tray icon cannot be created.
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).init();

    let daemon_url = std::env::var("CONNECTALSO_DAEMON").unwrap_or_else(|_| DAEMON_URL.to_string());
    tracing::info!("ConnectAlso Desktop Tray (daemon: {daemon_url})");

    let menu = Menu::new();
    let status_label = MenuItem::new("Status: checking...", true, None);
    let sep1 = PredefinedMenuItem::separator();
    let diag_label = MenuItem::new("Run 'connectalso diag' in terminal", true, None);
    let sep2 = PredefinedMenuItem::separator();
    let about_label = MenuItem::new("ConnectAlso Desktop Alpha v0.1.0", true, None);
    let quit_item = MenuItem::new("Quit", true, None);

    menu.append_items(&[&status_label, &sep1, &diag_label, &sep2, &about_label, &quit_item])?;

    let icon = make_icon(status_color(ConnStatus::Disconnected));
    let _tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("ConnectAlso")
        .with_icon(icon)
        .build()
        .context("failed to create tray icon")?;

    // Note: tray-icon/muda objects are not Send.
    // Full interactive menu requires a platform event loop (winit/tao).
    // For now, the tray shows a static icon as a visual indicator.
    // Use `connectalso status` for interactive CLI status.

    tracing::info!("Tray icon active — use 'connectalso status' for details.");
    tracing::info!("Press Ctrl+C to exit.");

    // Poll status in the main loop and update icon
    let mut current = ConnStatus::Disconnected;
    loop {
        let new_status = fetch_status(&daemon_url).await;
        if new_status != current {
            current = new_status;
            tracing::info!("Status: {}", fmt_status(current));
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}
