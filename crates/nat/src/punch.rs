use std::net::SocketAddr;

use tokio::net::UdpSocket;
use tracing::instrument;

use crate::candidate::Candidate;

const PUNCH_RETRIES: usize = 10;
const PUNCH_INTERVAL_MS: u64 = 50;

/// UDP hole-punching state for a single peer.
pub struct Puncher {
    socket: UdpSocket,
}

impl Puncher {
    /// Create a new puncher bound to a local UDP port.
    pub async fn bind(addr: SocketAddr) -> Result<Self, std::io::Error> {
        let socket = UdpSocket::bind(addr).await?;
        tracing::info!(local = %socket.local_addr()?, "puncher bound");
        Ok(Self { socket })
    }

    /// Create a puncher from an already-bound UDP socket.
    #[must_use]
    pub fn from_socket(socket: UdpSocket) -> Self {
        Self { socket }
    }

    /// Return the local address of the bound socket.
    pub fn local_addr(&self) -> Result<SocketAddr, std::io::Error> {
        self.socket.local_addr()
    }

    /// Perform UDP hole punching towards a list of peer candidates.
    ///
    /// This sends repeated "punch" packets to each candidate while
    /// listening for a response. Returns `true` if bidirectional
    /// communication was established.
    ///
    /// `our_token` is sent in the punch payload so the peer can
    /// identify us; the function returns the first received payload.
    #[instrument(skip(self), fields(local = %self.socket.local_addr().unwrap()))]
    pub async fn punch(
        &self,
        peer_candidates: &[Candidate],
        our_token: &[u8],
    ) -> Result<Option<(Vec<u8>, SocketAddr)>, std::io::Error> {
        let punch_payload = our_token;

        for _round in 0..PUNCH_RETRIES {
            // Send punch packets to all peer candidates
            for candidate in peer_candidates {
                let sent = self.socket.send_to(punch_payload, candidate.addr).await;
                if let Err(ref e) = sent {
                    tracing::debug!(peer = %candidate.addr, error = %e, "punch send failed");
                }
            }

            // Check for an incoming packet (short timeout)
            let mut buf = [0u8; 1024];
            match tokio::time::timeout(
                std::time::Duration::from_millis(PUNCH_INTERVAL_MS),
                self.socket.recv_from(&mut buf),
            )
            .await
            {
                Ok(Ok((n, from))) => {
                    let payload = buf[..n].to_vec();
                    tracing::info!(%from, len = n, "received punch response");
                    return Ok(Some((payload, from)));
                }
                Ok(Err(e)) => {
                    tracing::error!(error = %e, "recv error during punch");
                }
                Err(_timeout) => {
                    // Continue punching
                }
            }
        }

        tracing::warn!("hole punching failed after {PUNCH_RETRIES} rounds");
        Ok(None)
    }

    /// Send a message to a peer address. Used after punching succeeds.
    pub async fn send_to(&self, data: &[u8], peer: SocketAddr) -> Result<usize, std::io::Error> {
        self.socket.send_to(data, peer).await
    }

    /// Receive a message from any peer.
    pub async fn recv_from(&self, buf: &mut [u8]) -> Result<(usize, SocketAddr), std::io::Error> {
        self.socket.recv_from(buf).await
    }

    /// Wait for a punch packet from a peer. Used by the responder side.
    pub async fn wait_for_punch(&self) -> Result<(Vec<u8>, SocketAddr), std::io::Error> {
        let mut buf = [0u8; 1024];
        let (n, from) = self.socket.recv_from(&mut buf).await?;
        tracing::info!(%from, len = n, "received initial punch");
        Ok((buf[..n].to_vec(), from))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candidate::{Candidate, CandidateType};

    #[tokio::test]
    async fn hole_punch_simulation() {
        let _ = tracing_subscriber::fmt().try_init();

        // Peer A and Peer B on different localhost ports (simulating NAT endpoints)
        let peer_a = Puncher::bind("127.0.0.1:0".parse().unwrap()).await.unwrap();
        let peer_b = Puncher::bind("127.0.0.1:0".parse().unwrap()).await.unwrap();

        let a_addr = peer_a.local_addr().unwrap();
        let b_addr = peer_b.local_addr().unwrap();

        // Exchange candidates (simulating signaling server)
        let a_candidates = vec![Candidate::host(b_addr)];
        let b_candidates = vec![Candidate::host(a_addr)];

        // Both peers punch simultaneously
        let a_token = b"alice-token-42";
        let b_token = b"bob-token-07";

        let (a_result, b_result) =
            tokio::join!(peer_a.punch(&a_candidates, a_token), peer_b.punch(&b_candidates, b_token),);

        let a_response = a_result.unwrap();
        let b_response = b_result.unwrap();

        assert!(a_response.is_some(), "Alice should receive Bob's punch");
        assert!(b_response.is_some(), "Bob should receive Alice's punch");

        let (a_payload, a_from) = a_response.unwrap();
        let (b_payload, b_from) = b_response.unwrap();

        assert_eq!(a_payload, b_token, "Alice should see Bob's token");
        assert_eq!(a_from, b_addr);

        assert_eq!(b_payload, a_token, "Bob should see Alice's token");
        assert_eq!(b_from, a_addr);
    }

    #[tokio::test]
    async fn punch_then_communicate() {
        let _ = tracing_subscriber::fmt().try_init();

        let peer_a = Puncher::bind("127.0.0.1:0".parse().unwrap()).await.unwrap();
        let peer_b = Puncher::bind("127.0.0.1:0".parse().unwrap()).await.unwrap();

        let a_addr = peer_a.local_addr().unwrap();
        let b_addr = peer_b.local_addr().unwrap();

        // Punch holes
        let (a_res, b_res) = tokio::join!(
            peer_a.punch(&[Candidate::host(b_addr)], b"ping"),
            peer_b.punch(&[Candidate::host(a_addr)], b"pong"),
        );
        assert!(a_res.unwrap().is_some());
        assert!(b_res.unwrap().is_some());

        // Now send application data
        peer_a.send_to(b"hello bob", b_addr).await.unwrap();
        let mut buf = [0u8; 1024];
        let (n, from) = peer_b.recv_from(&mut buf).await.unwrap();
        assert_eq!(&buf[..n], b"hello bob");
        assert_eq!(from, a_addr);

        peer_b.send_to(b"hi alice", a_addr).await.unwrap();
        let (n, from) = peer_a.recv_from(&mut buf).await.unwrap();
        assert_eq!(&buf[..n], b"hi alice");
        assert_eq!(from, b_addr);
    }
}
