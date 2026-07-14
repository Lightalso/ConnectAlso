use std::net::SocketAddr;

use connectalso_relay_proto::PeerId;

use crate::relay::RelayClient;
use crate::{Tunnel, TunnelError};

/// Path status for connectivity monitoring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathStatus {
    /// Path is up and usable.
    Up,
    /// Path is down (failed to send or no recent response).
    Down,
}

/// Manages two communication paths to a peer:
/// - **Direct** tunnel (P2P, preferred)
/// - **Relay** fallback (via relay server)
///
/// On send, tries direct first; falls back to relay.
/// On recv, listens on both paths via `tokio::select!`.
pub struct PathManager {
    direct: Option<Tunnel>,
    relay: RelayClient,
    direct_peer: SocketAddr,
    direct_status: PathStatus,
}

/// A received message bundle.
#[derive(Debug)]
pub struct RecvMessage {
    /// The decrypted payload.
    pub data: Vec<u8>,
    /// Which path delivered the message.
    pub via: &'static str,
}

impl PathManager {
    /// Create a new path manager with both direct and relay paths.
    pub fn new(
        direct: Option<Tunnel>,
        relay: RelayClient,
        direct_peer: SocketAddr,
    ) -> Self {
        let direct_status = if direct.is_some() {
            PathStatus::Up
        } else {
            PathStatus::Down
        };

        Self {
            direct,
            relay,
            direct_peer,
            direct_status,
        }
    }

    /// Return the current direct-path status.
    #[must_use]
    pub const fn direct_status(&self) -> PathStatus {
        self.direct_status
    }

    /// Send a plaintext message to the peer.
    ///
    /// Tries the direct tunnel first. If that fails, marks direct as
    /// down and falls back to the relay.
    pub async fn send(&mut self, plaintext: &[u8]) -> Result<(), TunnelError> {
        if let Some(ref mut tunnel) = self.direct {
            if self.direct_status == PathStatus::Up {
                match tunnel.send_to(plaintext, self.direct_peer).await {
                    Ok(_) => {
                        tracing::debug!("sent via direct");
                        return Ok(());
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "direct send failed, switching to relay");
                        self.direct_status = PathStatus::Down;
                    }
                }
            }
        }

        // Fall through to relay
        // Note: the relay sends encrypted data, so we need to encrypt here.
        // For the prototype, the caller manages encryption; we pass through.
        self.relay
            .send(plaintext)
            .await
            .map_err(TunnelError::Io)?;
        tracing::debug!(via = "relay", "sent via relay");
        Ok(())
    }

    /// Receive a message from either the direct path or relay.
    ///
    /// Listens on both sockets concurrently.
    pub async fn recv(&self) -> Result<RecvMessage, TunnelError> {
        if let Some(ref tunnel) = self.direct {
            tokio::select! {
                result = tunnel.recv_from() => {
                    let (data, _from) = result?;
                    return Ok(RecvMessage { data, via: "direct" });
                }
                result = self.relay.recv() => {
                    let (data, _sender) = result.map_err(TunnelError::Io)?;
                    return Ok(RecvMessage { data, via: "relay" });
                }
            }
        }

        // No direct tunnel — only relay
        let (data, _sender) = self.relay.recv().await.map_err(TunnelError::Io)?;
        Ok(RecvMessage {
            data,
            via: "relay",
        })
    }

    /// Probe the direct path to see if it has recovered.
    ///
    /// Sends a small probe packet. Returns `true` if the direct path
    /// is now reachable.
    ///
    /// Call this periodically to attempt P2P restoration.
    pub async fn probe_direct(&mut self) -> bool {
        if self.direct_status == PathStatus::Up {
            return true;
        }

        if let Some(ref mut tunnel) = self.direct {
            let probe = b"PROBE";
            if tunnel.send_to(probe, self.direct_peer).await.is_ok() {
                tracing::info!("direct path restored");
                self.direct_status = PathStatus::Up;
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use connectalso_crypto::key_exchange::KeyPair;
    use connectalso_relay_proto::{PeerId, RelayFrame};
    use crate::{Tunnel, TunnelError};
    use tokio::net::UdpSocket;

    /// Spawn a minimal relay server on a random port and return its address.
    async fn spawn_relay() -> SocketAddr {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let addr = socket.local_addr().unwrap();
        tokio::spawn(async move {
            let mut peers: std::collections::HashMap<PeerId, SocketAddr> =
                std::collections::HashMap::new();
            let mut buf = [0u8; 4096];
            loop {
                let (n, src) = match socket.recv_from(&mut buf).await {
                    Ok(r) => r,
                    Err(_) => break,
                };
                let frame = match RelayFrame::decode(&buf[..n]) {
                    Ok(f) => f,
                    Err(_) => continue,
                };
                use connectalso_relay_proto::MsgType;
                match frame.msg_type {
                    MsgType::Hello | MsgType::Keepalive => {
                        peers.insert(frame.sender_id, src);
                    }
                    MsgType::Data => {
                        if let Some(&target) = peers.get(&frame.target_id) {
                            let fwd = RelayFrame::data(
                                frame.sender_id,
                                frame.target_id,
                                frame.payload,
                            );
                            let enc = fwd.encode().unwrap();
                            let _ = socket.send_to(&enc, target).await;
                        }
                    }
                }
            }
        });
        addr
    }

    #[tokio::test]
    async fn relay_fallback_and_restore() {
        let _ = tracing_subscriber::fmt().try_init();

        let relay_addr = spawn_relay().await;

        let alice_keys = KeyPair::generate();
        let bob_keys = KeyPair::generate();
        let shared = alice_keys.diffie_hellman(&bob_keys.public_key_bytes());

        let alice_id = PeerId::new_v4();
        let bob_id = PeerId::new_v4();

        // Direct tunnel for Alice (active) and Bob (active)
        let alice_direct = Tunnel::bind_initiator("127.0.0.1:0".parse().unwrap(), &shared)
            .await
            .unwrap();
        let bob_direct = Tunnel::bind_responder("127.0.0.1:0".parse().unwrap(), &shared)
            .await
            .unwrap();
        let alice_addr = alice_direct.local_addr().unwrap();
        let bob_addr = bob_direct.local_addr().unwrap();

        // Relay clients for both sides
        let alice_relay = RelayClient::register(
            "127.0.0.1:0".parse().unwrap(),
            relay_addr,
            alice_id,
            bob_id,
        )
        .await
        .unwrap();

        let bob_relay = RelayClient::register(
            "127.0.0.1:0".parse().unwrap(),
            relay_addr,
            bob_id,
            alice_id,
        )
        .await
        .unwrap();

        let mut alice = PathManager::new(Some(alice_direct), alice_relay, bob_addr);
        let mut bob = PathManager::new(Some(bob_direct), bob_relay, alice_addr);

        // 1. Send directly (P2P)
        alice.send(b"direct hello").await.unwrap();
        let msg = bob.recv().await.unwrap();
        assert_eq!(&msg.data[..], b"direct hello");
        assert_eq!(msg.via, "direct");

        // 2. Simulate direct failure: drop Alice's direct tunnel
        alice.direct = None;
        alice.direct_status = PathStatus::Down;

        // Now Alice sends via relay
        alice.send(b"relayed hello").await.unwrap();
        let msg = bob.recv().await.unwrap();
        assert_eq!(&msg.data[..], b"relayed hello");
        assert_eq!(msg.via, "relay");

        // Bob can still reply via relay
        bob.send(b"relayed reply").await.unwrap();
        let msg = alice.recv().await.unwrap();
        assert_eq!(&msg.data[..], b"relayed reply");
        assert_eq!(msg.via, "relay");

        // 3. Restore direct path
        let new_direct = Tunnel::bind_initiator("127.0.0.1:0".parse().unwrap(), &shared)
            .await
            .unwrap();
        alice.direct = Some(new_direct);
        // Update direct peer address to Bob's current address
        alice.direct_peer = bob_addr;

        // Probe to restore
        let restored = alice.probe_direct().await;
        assert!(restored);
        assert_eq!(alice.direct_status, PathStatus::Up);

        // Now Alice sends via restored direct
        alice.send(b"restored p2p").await.unwrap();
        let msg = bob.recv().await.unwrap();
        assert_eq!(&msg.data[..], b"restored p2p");
        assert_eq!(msg.via, "direct");
    }
}
