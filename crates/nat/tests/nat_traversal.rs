//! Integration test for NAT traversal: STUN discovery, candidate exchange, hole punching.

use std::net::SocketAddr;

use connectalso_nat::candidate::Candidate;
use connectalso_nat::punch::Puncher;
use connectalso_nat::stun::StunClient;

async fn spawn_stun_server() -> SocketAddr {
    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.expect("bind stun");
    let addr = socket.local_addr().unwrap();

    tokio::spawn(async move {
        let mut buf = [0u8; 512];
        loop {
            match socket.recv_from(&mut buf).await {
                Ok((n, client)) => {
                    if let Some(rsp) = stun_response(&buf[..n], client) {
                        let _ = socket.send_to(&rsp, client).await;
                    }
                }
                Err(_) => break,
            }
        }
    });

    addr
}

fn stun_response(data: &[u8], client: SocketAddr) -> Option<Vec<u8>> {
    const MAGIC: u32 = 0x2112_A442;
    if data.len() < 20 {
        return None;
    }
    if u16::from_be_bytes([data[0], data[1]]) != 0x0001 {
        return None;
    }

    let tx_id = &data[8..20];
    let x_port = client.port() ^ (MAGIC >> 16) as u16;

    let (family, addr) = match client.ip() {
        std::net::IpAddr::V4(ip) => {
            let bits = u32::from_be_bytes(ip.octets());
            (0x01, (bits ^ MAGIC).to_be_bytes().to_vec())
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
async fn full_stun_candidate_punch_flow() {
    let _ = tracing_subscriber::fmt().try_init();

    let stun_addr = spawn_stun_server().await;

    // --- Peer A: STUN discover then punch ---
    let sock_a = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let client_a = StunClient::from_socket(sock_a);
    let _public_a = client_a.discover(stun_addr).await.unwrap();
    let sock_a = client_a.into_socket();
    let peer_a = Puncher::from_socket(sock_a);

    // --- Peer B: STUN discover then punch ---
    let sock_b = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let client_b = StunClient::from_socket(sock_b);
    let _public_b = client_b.discover(stun_addr).await.unwrap();
    let sock_b = client_b.into_socket();
    let peer_b = Puncher::from_socket(sock_b);

    let a_host = peer_a.local_addr().unwrap();
    let b_host = peer_b.local_addr().unwrap();

    // Candidates (host + server-reflexive for each peer)
    let a_candidates = vec![Candidate::host(b_host)];
    let b_candidates = vec![Candidate::host(a_host)];

    // Simultaneous hole punching
    let (a_res, b_res) =
        tokio::join!(peer_a.punch(&a_candidates, b"token-a"), peer_b.punch(&b_candidates, b"token-b"),);

    let a_rsp = a_res.unwrap();
    let b_rsp = b_res.unwrap();
    assert!(a_rsp.is_some(), "peer A should receive B's punch");
    assert!(b_rsp.is_some(), "peer B should receive A's punch");

    let (a_payload, a_from) = a_rsp.unwrap();
    assert_eq!(a_payload, b"token-b");
    assert_eq!(a_from, b_host);

    let (b_payload, b_from) = b_rsp.unwrap();
    assert_eq!(b_payload, b"token-a");
    assert_eq!(b_from, a_host);

    // Direct communication after punch
    peer_a.send_to(b"post-punch-a", b_host).await.unwrap();
    let mut buf = [0u8; 128];
    let (n, from) = peer_b.recv_from(&mut buf).await.unwrap();
    assert_eq!(&buf[..n], b"post-punch-a");
    assert_eq!(from, a_host);
}
