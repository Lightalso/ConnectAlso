//! ConnectAlso desktop tray application.

use std::time::Duration;

use anyhow::Context;
use muda::{Menu, MenuItem, PredefinedMenuItem};
use serde::Deserialize;
use tray_icon::{Icon, TrayIconBuilder};

const DAEMON_URL: &str = "http://127.0.0.1:9823";

#[derive(Debug, Deserialize, Default)]
struct StatusResponse {
    device_id: String,
    #[allow(dead_code)]
    virtual_ip: String,
    #[allow(dead_code)]
    hostname: String,
    #[allow(dead_code)]
    uptime_secs: u64,
    peer_count: usize,
    #[serde(default)]
    peers: Vec<StatusPeer>,
}

#[derive(Debug, Deserialize, Default)]
struct StatusPeer {
    #[serde(default)]
    #[allow(dead_code)]
    hostname: String,
    #[serde(default)]
    #[allow(dead_code)]
    virtual_ip: String,
    #[serde(default)]
    path: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ConnStatus {
    Connected,
    RelayOnly,
    Disconnected,
}

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

fn status_color(status: ConnStatus) -> [u8; 4] {
    match status {
        ConnStatus::Connected => [0x4C, 0xAF, 0x50, 0xFF],
        ConnStatus::RelayOnly => [0xFF, 0x98, 0x00, 0xFF],
        ConnStatus::Disconnected => [0x9E, 0x9E, 0x9E, 0xFF],
    }
}

fn fmt_status(status: ConnStatus) -> &'static str {
    match status {
        ConnStatus::Connected => "Connected (P2P)",
        ConnStatus::RelayOnly => "Connected (Relay)",
        ConnStatus::Disconnected => "Disconnected",
    }
}

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
