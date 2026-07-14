use std::net::{IpAddr, SocketAddr};

use clap::Parser;
use tracing_subscriber::EnvFilter;

const MAGIC_COOKIE: u32 = 0x2112_A442;
const BINDING_REQUEST: u16 = 0x0001;
const BINDING_SUCCESS: u16 = 0x0101;
const ATTR_XOR_MAPPED_ADDRESS: u16 = 0x0020;

#[derive(Parser)]
#[command(name = "connectalso-stun")]
#[command(about = "ConnectAlso STUN 服务 (仅用于开发测试)")]
struct Cli {
    #[arg(long, default_value = "0.0.0.0:3478")]
    listen: SocketAddr,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter(EnvFilter::from_default_env()).init();

    let cli = Cli::parse();
    let socket = tokio::net::UdpSocket::bind(cli.listen).await?;
    tracing::info!("STUN server listening on {}", cli.listen);

    let mut buf = [0u8; 512];
    loop {
        let (n, client_addr) = socket.recv_from(&mut buf).await?;
        if let Some(response) = handle_binding_request(&buf[..n], client_addr) {
            socket.send_to(&response, client_addr).await?;
            tracing::debug!(%client_addr, "binding response sent");
        }
    }
}

fn handle_binding_request(data: &[u8], client_addr: SocketAddr) -> Option<Vec<u8>> {
    if data.len() < 20 {
        return None;
    }
    let msg_type = u16::from_be_bytes([data[0], data[1]]);
    if msg_type != BINDING_REQUEST {
        return None;
    }
    let magic = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
    if magic != MAGIC_COOKIE {
        return None;
    }

    let tx_id = &data[8..20];

    let mapped_attr = match client_addr.ip() {
        IpAddr::V4(ip) => build_xor_mapped_v4(ip, client_addr.port()),
        IpAddr::V6(ip) => build_xor_mapped_v6(ip, client_addr.port()),
    };

    let mut response = Vec::with_capacity(20 + mapped_attr.len());
    response.extend_from_slice(&BINDING_SUCCESS.to_be_bytes());
    response.extend_from_slice(&(mapped_attr.len() as u16).to_be_bytes());
    response.extend_from_slice(&MAGIC_COOKIE.to_be_bytes());
    response.extend_from_slice(tx_id);
    response.extend_from_slice(&mapped_attr);
    Some(response)
}

fn build_xor_mapped_v4(ip: std::net::Ipv4Addr, port: u16) -> Vec<u8> {
    let x_port = port ^ (MAGIC_COOKIE >> 16) as u16;
    let addr_bits = u32::from_be_bytes(ip.octets());
    let x_addr = addr_bits ^ MAGIC_COOKIE;

    let mut attr = Vec::with_capacity(12);
    attr.extend_from_slice(&ATTR_XOR_MAPPED_ADDRESS.to_be_bytes());
    attr.extend_from_slice(&8u16.to_be_bytes()); // length = 8
    attr.push(0x00); // reserved
    attr.push(0x01); // family = IPv4
    attr.extend_from_slice(&x_port.to_be_bytes());
    attr.extend_from_slice(&x_addr.to_be_bytes());
    attr
}

fn build_xor_mapped_v6(ip: std::net::Ipv6Addr, port: u16) -> Vec<u8> {
    let x_port = port ^ (MAGIC_COOKIE >> 16) as u16;
    let octets = ip.octets();
    let mc = MAGIC_COOKIE.to_be_bytes();
    let mut x_addr = octets;
    x_addr[0] ^= mc[0];
    x_addr[1] ^= mc[1];
    x_addr[2] ^= mc[2];
    x_addr[3] ^= mc[3];

    let mut attr = Vec::with_capacity(24);
    attr.extend_from_slice(&ATTR_XOR_MAPPED_ADDRESS.to_be_bytes());
    attr.extend_from_slice(&20u16.to_be_bytes()); // length = 20
    attr.push(0x00); // reserved
    attr.push(0x02); // family = IPv6
    attr.extend_from_slice(&x_port.to_be_bytes());
    attr.extend_from_slice(&x_addr);
    attr
}
