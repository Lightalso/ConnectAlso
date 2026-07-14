use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use muda::{Menu, MenuItem, PredefinedMenuItem};
use serde::Deserialize;
use tokio::sync::Mutex;
use tray_icon::{Icon, TrayIconBuilder};

const DAEMON_URL: &str = "http://127.0.0.1:9823";

#[derive(Debug, Deserialize, Default)]
struct StatusResponse {
    device_id: String,
    virtual_ip: String,
    hostname: String,
    uptime_secs: u64,
    peer_count: usize,
    #[serde(default)]
    peers: Vec<StatusPeer>,
}

#[derive(Debug, Deserialize, Default)]
struct StatusPeer {
    #[serde(default)]
    hostname: String,
    #[serde(default)]
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
        ConnStatus::Connected => [0x4C, 0xAF, 0x50, 0xFF],    // green
        ConnStatus::RelayOnly => [0xFF, 0x98, 0x00, 0xFF],    // orange
        ConnStatus::Disconnected => [0x9E, 0x9E, 0x9E, 0xFF], // gray
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
    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("ConnectAlso")
        .with_icon(icon)
        .build()
        .context("failed to create tray icon")?;

    let current_status = Arc::new(Mutex::new(ConnStatus::Disconnected));

    let tray_handle = tray.clone();
    let status_arc = current_status.clone();
    let url = daemon_url.clone();
    let sl = status_label.clone();

    tokio::spawn(async move {
        loop {
            let new_status = fetch_status(&url).await;
            let mut prev = status_arc.lock().await;
            if *prev != new_status {
                *prev = new_status;
                let _ = tray_handle.set_icon(Some(make_icon(status_color(new_status))));
                let tooltip = format!("ConnectAlso — {}", fmt_status(new_status));
                let _ = tray_handle.set_tooltip(Some(&tooltip));
                let _ = sl.set_text(format!("Status: {}", fmt_status(new_status)));
                tracing::info!("{}", tooltip);
            }
            drop(prev);
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });

    // Keep the process alive. The tray runs on the main thread.
    // Menu items display info but don't have click handlers without an event loop.
    tracing::info!("Tray icon active. Right-click for menu.");
    tracing::info!("Use 'connectalso status' for full status.");

    loop {
        tokio::time::sleep(Duration::from_secs(3600)).await;
    }
}
