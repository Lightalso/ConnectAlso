use std::net::Ipv4Addr;

use anyhow::Context;
use clap::Parser;
use connectalso_platform::tun::{TunConfig, TunDevice};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "connectalso-daemon")]
#[command(about = "ConnectAlso 客户端后台服务")]
struct Cli {
    /// TUN 接口名称
    #[arg(long, default_value = "connectalso")]
    tun_name: String,

    /// TUN 接口 IPv4 地址
    #[arg(long, default_value = "100.64.0.1")]
    tun_address: Ipv4Addr,

    /// TUN 接口子网掩码
    #[arg(long, default_value = "255.255.255.0")]
    tun_netmask: Ipv4Addr,

    /// TUN 接口 MTU
    #[arg(long, default_value_t = 1500)]
    tun_mtu: u16,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    tracing::info!("ConnectAlso Daemon starting...");

    let config = TunConfig::new(cli.tun_address, cli.tun_netmask)
        .with_name(&cli.tun_name)
        .with_mtu(cli.tun_mtu);

    let tun = TunDevice::create(config).await.context("failed to create TUN device")?;
    tracing::info!(
        name = tun.name(),
        address = %tun.address(),
        netmask = %tun.netmask(),
        mtu = tun.mtu(),
        "TUN device created"
    );

    let mut buf = vec![0u8; tun.mtu() as usize];
    loop {
        match tun.recv(&mut buf).await {
            Ok(n) => {
                let packet = &buf[..n];
                log_packet_info(packet);

                let sent = tun.send(packet).await.context("failed to echo packet back to TUN")?;
                tracing::debug!(sent, "echoed packet back to TUN");
            }
            Err(e) => {
                tracing::error!(error = %e, "TUN recv error");
            }
        }
    }
}

fn log_packet_info(packet: &[u8]) {
    if packet.is_empty() {
        return;
    }
    let version_ihl = packet[0];
    let version = version_ihl >> 4;

    if packet.len() >= 20 && version == 4 {
        let protocol = packet[9];
        let src = Ipv4Addr::new(packet[12], packet[13], packet[14], packet[15]);
        let dst = Ipv4Addr::new(packet[16], packet[17], packet[18], packet[19]);
        tracing::info!(size = packet.len(), %src, %dst, protocol, "recv IPv4 packet");
    } else if version == 6 {
        tracing::info!(size = packet.len(), "recv IPv6 packet");
    } else {
        tracing::info!(size = packet.len(), ip_version = version, "recv packet");
    }
}
