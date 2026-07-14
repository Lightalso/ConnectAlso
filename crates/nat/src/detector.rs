use std::net::SocketAddr;

use tracing::info;

use crate::nat_type::NatType;
use crate::stun::{StunClient, StunError};

/// A minimal NAT type detector.
///
/// Full classification (RFC 5780) requires a STUN server with
/// CHANGE-REQUEST support and multiple IP addresses. This detector
/// provides a basic check: whether the local address differs from
/// the STUN-discovered public address.
pub struct NatDetector;

impl NatDetector {
    /// Detect whether the host is behind NAT by comparing the local
    /// socket address with the STUN-discovered public address.
    ///
    /// Uses the provided STUN server. Returns `NatType::Open` if the
    /// addresses match, `NatType::Unknown` if NAT is detected but
    /// the specific type cannot be classified without CHANGE-REQUEST.
    pub async fn detect(stun_server: SocketAddr) -> Result<NatType, StunError> {
        let client = StunClient::bind().await?;
        let local = client.local_addr()?;
        let public = client.discover(stun_server).await?;

        info!(%local, %public, "NAT detection: local vs public");

        if local.ip() == public.ip() && local.port() == public.port() {
            Ok(NatType::Open)
        } else {
            // NAT is present; full classification requires RFC 5780
            // CHANGE-REQUEST support on the STUN server
            Ok(NatType::Unknown)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spawns a mock STUN server that echoes back the client address.
    async fn spawn_echo_stun() -> SocketAddr {
        let socket = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .unwrap();
        let addr = socket.local_addr().unwrap();

        tokio::spawn(async move {
            let mut buf = [0u8; 512];
            loop {
                let (n, client) = match socket.recv_from(&mut buf).await {
                    Ok(r) => r,
                    Err(_) => break,
                };
                // Build a minimal binding success response
                if let Some(rsp) = build_response(&buf[..n], client) {
                    let _ = socket.send_to(&rsp, client).await;
                }
            }
        });

        addr
    }

    fn build_response(data: &[u8], client: SocketAddr) -> Option<Vec<u8>> {
        const MAGIC: u32 = 0x2112_A442;
        if data.len() < 20 {
            return None;
        }
        let tx_id = &data[8..20];

        let x_port = client.port() ^ (MAGIC >> 16) as u16;
        let (family, addr) = match client.ip() {
            std::net::IpAddr::V4(ip) => {
                let bits = u32::from_be_bytes(ip.octets());
                (0x01u8, (bits ^ MAGIC).to_be_bytes().to_vec())
            }
            _ => return None,
        };

        let mut attr = Vec::new();
        attr.extend_from_slice(&0x0020u16.to_be_bytes());
        attr.extend_from_slice(&8u16.to_be_bytes());
        attr.push(0x00);
        attr.push(family);
        attr.extend_from_slice(&x_port.to_be_bytes());
        attr.extend_from_slice(&addr);

        let mut rsp = Vec::new();
        rsp.extend_from_slice(&0x0101u16.to_be_bytes());
        rsp.extend_from_slice(&(attr.len() as u16).to_be_bytes());
        rsp.extend_from_slice(&MAGIC.to_be_bytes());
        rsp.extend_from_slice(tx_id);
        rsp.extend_from_slice(&attr);
        Some(rsp)
    }

    #[tokio::test]
    async fn detect_open_internet() {
        let stun = spawn_echo_stun().await;
        let result = NatDetector::detect(stun).await.unwrap();
        // On localhost, the STUN server sees the same address as local
        assert_eq!(result, NatType::Open);
    }
}
