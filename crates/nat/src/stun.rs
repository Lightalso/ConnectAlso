use std::net::SocketAddr;

use rand::Rng;
use thiserror::Error;
use tokio::net::UdpSocket;

const MAGIC_COOKIE: u32 = 0x2112_A442;
const BINDING_REQUEST: u16 = 0x0001;
const BINDING_SUCCESS: u16 = 0x0101;
const ATTR_XOR_MAPPED_ADDRESS: u16 = 0x0020;
const HEADER_LEN: usize = 20;
const IPV4_FAMILY: u8 = 0x01;

/// STUN operation errors.
#[derive(Debug, Error)]
pub enum StunError {
    /// I/O error communicating with the STUN server.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// The STUN server returned an unexpected response type.
    #[error("unexpected STUN response type: {0:#06x}")]
    UnexpectedResponse(u16),
    /// The response transaction ID does not match the request.
    #[error("transaction ID mismatch")]
    TransactionMismatch,
    /// Failed to parse the XOR-MAPPED-ADDRESS attribute.
    #[error("no XOR-MAPPED-ADDRESS in response")]
    MissingMappedAddress,
    /// The server address could not be parsed.
    #[error("invalid server address")]
    InvalidServerAddress,
}

/// A minimal STUN client that discovers the public (server-reflexive)
/// address by sending a Binding Request to a STUN server.
pub struct StunClient {
    socket: UdpSocket,
}

impl StunClient {
    /// Create a new STUN client bound to an ephemeral local port.
    pub async fn bind() -> Result<Self, StunError> {
        let socket = UdpSocket::bind("0.0.0.0:0").await?;
        Ok(Self { socket })
    }

    /// Create a STUN client using an already-bound UDP socket.
    #[must_use]
    pub fn from_socket(socket: UdpSocket) -> Self {
        Self { socket }
    }

    /// Return the local address of the underlying socket.
    #[must_use]
    pub fn local_addr(&self) -> Result<SocketAddr, std::io::Error> {
        self.socket.local_addr()
    }

    /// Consume the client and return the underlying UDP socket.
    #[must_use]
    pub fn into_socket(self) -> UdpSocket {
        self.socket
    }

    /// Send a Binding Request to `server` and return the XOR-MAPPED-ADDRESS
    /// from the response — i.e. the public IP:port as seen by the server.
    pub async fn discover(&self, server: SocketAddr) -> Result<SocketAddr, StunError> {
        let tx_id: [u8; 12] = rand::thread_rng().gen();

        let request = build_binding_request(&tx_id);
        self.socket.send_to(&request, server).await?;

        let mut buf = [0u8; 512];
        let (n, _from) = self.socket.recv_from(&mut buf).await?;

        let (msg_type, resp_tx_id, attrs) = parse_response(&buf[..n])?;

        if msg_type != BINDING_SUCCESS {
            return Err(StunError::UnexpectedResponse(msg_type));
        }
        if resp_tx_id != tx_id {
            return Err(StunError::TransactionMismatch);
        }

        parse_xor_mapped_address(attrs).ok_or(StunError::MissingMappedAddress)
    }
}

fn build_binding_request(tx_id: &[u8; 12]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(HEADER_LEN);
    // Message Type (2 bytes)
    buf.extend_from_slice(&BINDING_REQUEST.to_be_bytes());
    // Message Length (2 bytes) — 0 attributes
    buf.extend_from_slice(&0u16.to_be_bytes());
    // Magic Cookie (4 bytes)
    buf.extend_from_slice(&MAGIC_COOKIE.to_be_bytes());
    // Transaction ID (12 bytes)
    buf.extend_from_slice(tx_id);
    buf
}

fn parse_response(data: &[u8]) -> Result<(u16, [u8; 12], &[u8]), StunError> {
    if data.len() < HEADER_LEN {
        return Err(StunError::MissingMappedAddress);
    }
    let msg_type = u16::from_be_bytes([data[0], data[1]]);
    let msg_len = u16::from_be_bytes([data[2], data[3]]) as usize;
    let magic = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);

    if magic != MAGIC_COOKIE {
        return Err(StunError::MissingMappedAddress);
    }

    let mut tx_id = [0u8; 12];
    tx_id.copy_from_slice(&data[8..20]);

    let attr_end = HEADER_LEN + msg_len.min(data.len() - HEADER_LEN);
    let attrs = &data[HEADER_LEN..attr_end.min(data.len())];

    Ok((msg_type, tx_id, attrs))
}

fn parse_xor_mapped_address(attrs: &[u8]) -> Option<SocketAddr> {
    let mut pos = 0;
    while pos + 4 <= attrs.len() {
        let attr_type = u16::from_be_bytes([attrs[pos], attrs[pos + 1]]);
        let attr_len = u16::from_be_bytes([attrs[pos + 2], attrs[pos + 3]]) as usize;
        pos += 4;

        if attr_type == ATTR_XOR_MAPPED_ADDRESS && attr_len >= 8 && pos + attr_len <= attrs.len() {
            let _reserved = attrs[pos]; // 0x00
            let family = attrs[pos + 1];
            if family != IPV4_FAMILY {
                return None;
            }

            // X-Port = port XOR (magic cookie >> 16)
            let x_port = u16::from_be_bytes([attrs[pos + 2], attrs[pos + 3]]);
            let port = x_port ^ (MAGIC_COOKIE >> 16) as u16;

            // X-Address = address XOR magic cookie
            let x_addr = u32::from_be_bytes([
                attrs[pos + 4],
                attrs[pos + 5],
                attrs[pos + 6],
                attrs[pos + 7],
            ]);
            let addr = x_addr ^ MAGIC_COOKIE;
            let ip = std::net::Ipv4Addr::from(addr.to_be_bytes());

            return Some(SocketAddr::new(std::net::IpAddr::V4(ip), port));
        }
        pos += attr_len;
        // Attributes are padded to 4-byte boundaries
        let pad = (4 - (attr_len % 4)) % 4;
        pos += pad;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, SocketAddr};

    fn build_success_response(tx_id: &[u8; 12], mapped: SocketAddr) -> Vec<u8> {
        // XOR-MAPPED-ADDRESS attribute
        let mut attr = Vec::new();
        attr.extend_from_slice(&ATTR_XOR_MAPPED_ADDRESS.to_be_bytes());
        attr.extend_from_slice(&8u16.to_be_bytes()); // attr len
        attr.push(0x00); // reserved
        attr.push(IPV4_FAMILY); // IPv4

        let port = mapped.port() ^ (MAGIC_COOKIE >> 16) as u16;
        attr.extend_from_slice(&port.to_be_bytes());

        if let std::net::IpAddr::V4(ip) = mapped.ip() {
            let addr_bits = u32::from_be_bytes(ip.octets());
            let x_addr = addr_bits ^ MAGIC_COOKIE;
            attr.extend_from_slice(&x_addr.to_be_bytes());
        }

        let mut response = Vec::new();
        response.extend_from_slice(&BINDING_SUCCESS.to_be_bytes());
        response.extend_from_slice(&(attr.len() as u16).to_be_bytes());
        response.extend_from_slice(&MAGIC_COOKIE.to_be_bytes());
        response.extend_from_slice(tx_id);
        response.extend_from_slice(&attr);
        response
    }

    #[tokio::test]
    async fn decode_success_response() {
        let client = StunClient::bind().await.unwrap();
        let client_addr = client.socket.local_addr().unwrap();
        let tx_id: [u8; 12] = rand::thread_rng().gen();

        let mapped = "1.2.3.4:5678".parse::<SocketAddr>().unwrap();
        let response = build_success_response(&tx_id, mapped);

        let (msg_type, resp_tx_id, attrs) = parse_response(&response).unwrap();
        assert_eq!(msg_type, BINDING_SUCCESS);
        assert_eq!(resp_tx_id, tx_id);

        let addr = parse_xor_mapped_address(attrs).unwrap();
        assert_eq!(addr, mapped);
    }

    #[tokio::test]
    async fn parse_xor_mapped_address_roundtrip() {
        let cases: Vec<(SocketAddr,)> = vec![
            ("1.2.3.4:5678".parse().unwrap(),),
            ("192.168.1.1:12345".parse().unwrap(),),
            ("10.0.0.1:9".parse().unwrap(),),
        ];

        for (addr,) in cases {
            let tx_id = [0u8; 12];
            let response = build_success_response(&tx_id, addr);
            let (_mt, _tid, attrs) = parse_response(&response).unwrap();
            let parsed = parse_xor_mapped_address(attrs).unwrap();
            assert_eq!(parsed, addr, "roundtrip failed for {addr}");
        }
    }
}
