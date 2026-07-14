//! ConnectAlso 中继服务 — 在对等节点之间转发加密数据包。
//! ConnectAlso relay service — forwards encrypted packets between peers.

use std::collections::HashMap;
use std::net::SocketAddr;

use clap::Parser;
use connectalso_relay_proto::{MsgType, PeerId, RelayFrame};
use tracing_subscriber::EnvFilter;

/// 命令行参数。
/// Command-line arguments.
#[derive(Parser)]
#[command(name = "connectalso-relay")]
#[command(about = "ConnectAlso 流量中继服务")]
struct Cli {
    /// 监听地址。
    /// Listening address.
    #[arg(long, default_value = "0.0.0.0:33478")]
    listen: SocketAddr,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter(EnvFilter::from_default_env()).init();

    let cli = Cli::parse();
    let socket = tokio::net::UdpSocket::bind(cli.listen).await?;
    tracing::info!("Relay server listening on {}", cli.listen);

    let mut peers: HashMap<PeerId, SocketAddr> = HashMap::new();
    let mut buf = [0u8; 4096];

    loop {
        let (n, src_addr) = socket.recv_from(&mut buf).await?;
        let frame = match RelayFrame::decode(&buf[..n]) {
            Ok(f) => f,
            Err(e) => {
                tracing::debug!(%src_addr, error = %e, "bad relay frame");
                continue;
            }
        };

        match frame.msg_type {
            MsgType::Hello => {
                let old = peers.insert(frame.sender_id, src_addr);
                if old.is_none() {
                    tracing::info!(peer = %frame.sender_id, %src_addr, "peer registered");
                } else {
                    tracing::debug!(peer = %frame.sender_id, %src_addr, "peer refreshed");
                }
            }

            MsgType::Data => {
                if let Some(&target_addr) = peers.get(&frame.target_id) {
                    let fwd = RelayFrame::data(frame.sender_id, frame.target_id, frame.payload);
                    let encoded = fwd.encode()?;
                    socket.send_to(&encoded, target_addr).await?;
                    tracing::debug!(
                        from = %frame.sender_id,
                        to = %frame.target_id,
                        len = encoded.len(),
                        "packet forwarded"
                    );
                } else {
                    tracing::warn!(
                        target = %frame.target_id,
                        "target peer not registered — dropping DATA"
                    );
                }
            }

            MsgType::Keepalive => {
                peers.insert(frame.sender_id, src_addr);
                tracing::trace!(peer = %frame.sender_id, "keepalive");
            }
        }
    }
}
