//! Encrypted UDP tunnel, path management, and relay client.
//!
//! Provides ChaCha20-Poly1305 encrypted tunnels, path switching between
//! direct P2P and relay, and multi-region relay pool management.

use std::net::SocketAddr;

use connectalso_crypto::key_exchange::{CryptoError, SessionCipher};
use thiserror::Error;
use tokio::net::UdpSocket;

/// Nonce starting offset for the responder's send direction,
/// preventing nonce reuse when both peers share the same encryption key.
const RESPONDER_TX_OFFSET: u64 = 1u64 << 32;

const MAX_PACKET: usize = 65536;

/// Path management with P2P direct + relay fallback.
pub mod path;
/// Relay client for sending encrypted data through relay servers.
pub mod relay;
/// Multi-region relay pool with automatic failover.
pub mod relay_pool;

/// Tunnel operation errors.
#[derive(Debug, Error)]
pub enum TunnelError {
    /// I/O error on the underlying UDP socket.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// Cryptographic operation failed.
    #[error("crypto error: {0}")]
    Crypto(#[from] CryptoError),
}

/// An encrypted UDP tunnel between two nodes.
///
/// Each direction uses its own nonce space to prevent nonce reuse.
/// Packets are encrypted with ChaCha20-Poly1305 (AEAD) using a key
/// derived from an X25519 Diffie-Hellman shared secret.
pub struct Tunnel {
    socket: UdpSocket,
    tx_cipher: SessionCipher,
    rx_cipher: SessionCipher,
}

impl Tunnel {
    /// Bind to a local UDP address and initialise the tunnel as the
    /// **initiator** (send nonces start at 0).
    pub async fn bind_initiator(local_addr: SocketAddr, shared_secret: &[u8; 32]) -> Result<Self, TunnelError> {
        Self::bind_with_nonce(local_addr, shared_secret, 0).await
    }

    /// Bind to a local UDP address and initialise the tunnel as the
    /// **responder** (send nonces start at `RESPONDER_TX_OFFSET`).
    pub async fn bind_responder(local_addr: SocketAddr, shared_secret: &[u8; 32]) -> Result<Self, TunnelError> {
        Self::bind_with_nonce(local_addr, shared_secret, RESPONDER_TX_OFFSET).await
    }

    async fn bind_with_nonce(
        local_addr: SocketAddr,
        shared_secret: &[u8; 32],
        tx_nonce_start: u64,
    ) -> Result<Self, TunnelError> {
        let socket = UdpSocket::bind(local_addr).await?;
        tracing::info!(%local_addr, "tunnel socket bound");

        let tx_cipher = SessionCipher::new(shared_secret, tx_nonce_start);
        let rx_cipher = SessionCipher::new(shared_secret, 0);

        Ok(Self { socket, tx_cipher, rx_cipher })
    }

    /// Return the local socket address.
    #[must_use]
    pub fn local_addr(&self) -> Result<SocketAddr, std::io::Error> {
        self.socket.local_addr()
    }

    /// Encrypt `plaintext` and send it to `peer`.
    ///
    /// Returns the number of plaintext bytes sent.
    pub async fn send_to(&mut self, plaintext: &[u8], peer: SocketAddr) -> Result<usize, TunnelError> {
        let packet = self.tx_cipher.encrypt(plaintext)?;
        let sent = self.socket.send_to(&packet, peer).await?;
        tracing::debug!(
            %peer,
            plaintext_len = plaintext.len(),
            packet_len = sent,
            "encrypted packet sent"
        );
        Ok(plaintext.len())
    }

    /// Receive an encrypted packet, decrypt it, and return the plaintext
    /// together with the sender's address.
    pub async fn recv_from(&self) -> Result<(Vec<u8>, SocketAddr), TunnelError> {
        let mut buf = vec![0u8; MAX_PACKET];
        let (n, peer) = self.socket.recv_from(&mut buf).await?;
        buf.truncate(n);

        let plaintext = self.rx_cipher.decrypt(&buf)?;
        tracing::debug!(
            %peer,
            plaintext_len = plaintext.len(),
            packet_len = n,
            "packet decrypted"
        );
        Ok((plaintext, peer))
    }

    /// Consume the tunnel and return the underlying socket.
    #[must_use]
    pub fn into_socket(self) -> UdpSocket {
        self.socket
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use connectalso_crypto::key_exchange::KeyPair;

    #[tokio::test]
    async fn two_node_encrypted_exchange() {
        let _ = tracing_subscriber::fmt().try_init();

        let alice_keys = KeyPair::generate();
        let bob_keys = KeyPair::generate();
        let shared = alice_keys.diffie_hellman(&bob_keys.public_key_bytes());

        let mut alice = Tunnel::bind_initiator("127.0.0.1:0".parse().unwrap(), &shared).await.unwrap();
        let mut bob = Tunnel::bind_responder("127.0.0.1:0".parse().unwrap(), &shared).await.unwrap();

        let bob_addr = bob.local_addr().unwrap();
        let alice_addr = alice.local_addr().unwrap();

        let msg = b"hello from alice";
        alice.send_to(msg, bob_addr).await.unwrap();
        let (received, from) = bob.recv_from().await.unwrap();
        assert_eq!(&received[..], msg);
        assert_eq!(from, alice_addr);

        let msg = b"echo from bob";
        bob.send_to(msg, alice_addr).await.unwrap();
        let (received, from) = alice.recv_from().await.unwrap();
        assert_eq!(&received[..], msg);
        assert_eq!(from, bob_addr);
    }

    #[tokio::test]
    async fn wrong_key_rejected() {
        let _ = tracing_subscriber::fmt().try_init();

        let alice_keys = KeyPair::generate();
        let bob_keys = KeyPair::generate();
        let eve_keys = KeyPair::generate();
        let shared = alice_keys.diffie_hellman(&bob_keys.public_key_bytes());
        let alice_for_eve = KeyPair::generate();
        let eve_shared = alice_for_eve.diffie_hellman(&eve_keys.public_key_bytes());

        let mut alice = Tunnel::bind_initiator("127.0.0.1:0".parse().unwrap(), &shared).await.unwrap();
        let eve = Tunnel::bind_responder("127.0.0.1:0".parse().unwrap(), &eve_shared).await.unwrap();

        let eve_addr = eve.local_addr().unwrap();

        alice.send_to(b"secret", eve_addr).await.unwrap();

        let result = eve.recv_from().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn multiple_packets_in_order() {
        let _ = tracing_subscriber::fmt().try_init();

        let alice_keys = KeyPair::generate();
        let bob_keys = KeyPair::generate();
        let shared = alice_keys.diffie_hellman(&bob_keys.public_key_bytes());

        let mut alice = Tunnel::bind_initiator("127.0.0.1:0".parse().unwrap(), &shared).await.unwrap();
        let mut bob = Tunnel::bind_responder("127.0.0.1:0".parse().unwrap(), &shared).await.unwrap();

        let bob_addr = bob.local_addr().unwrap();

        for i in 0..10u8 {
            let msg = [i; 16];
            alice.send_to(&msg, bob_addr).await.unwrap();
            let (received, _) = bob.recv_from().await.unwrap();
            assert_eq!(&received[..], &msg[..]);
        }
    }
}
