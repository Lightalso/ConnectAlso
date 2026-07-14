//! 加密UDP隧道、路径管理和中继客户端。
//!
//! 提供ChaCha20-Poly1305加密隧道、P2P直连与中继之间的路径切换，
//! 以及多区域中继池管理。
//!
//! Encrypted UDP tunnel, path management, and relay client.
//!
//! Provides ChaCha20-Poly1305 encrypted tunnels, path switching between
//! direct P2P and relay, and multi-region relay pool management.

use std::net::SocketAddr;

use connectalso_crypto::key_exchange::{CryptoError, SessionCipher};
use thiserror::Error;
use tokio::net::UdpSocket;

/// 响应者发送方向的Nonce起始偏移量，
/// 防止双方共享同一加密密钥时的Nonce重复。
/// Nonce starting offset for the responder's send direction,
/// preventing nonce reuse when both peers share the same encryption key.
const RESPONDER_TX_OFFSET: u64 = 1u64 << 32;

const MAX_PACKET: usize = 65536;

/// 路径管理，支持P2P直连+中继回退。
/// Path management with P2P direct + relay fallback.
pub mod path;
/// 中继客户端，用于通过中继服务器发送加密数据。
/// Relay client for sending encrypted data through relay servers.
pub mod relay;
/// 多区域中继池，支持自动故障转移。
/// Multi-region relay pool with automatic failover.
pub mod relay_pool;

/// 隧道操作错误。
/// Tunnel operation errors.
#[derive(Debug, Error)]
pub enum TunnelError {
    /// 底层UDP套接字上的I/O错误。
    /// I/O error on the underlying UDP socket.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// 加密操作失败。
    /// Cryptographic operation failed.
    #[error("crypto error: {0}")]
    Crypto(#[from] CryptoError),
}

/// 两个节点之间的加密UDP隧道。
///
/// 每个方向使用独立的Nonce空间以防Nonce重复。
/// 数据包使用ChaCha20-Poly1305 (AEAD)加密，
/// 密钥由X25519 Diffie-Hellman共享密钥派生。
///
/// # Fields
///
/// * `socket` — 底层UDP套接字。
/// * `tx_cipher` — 发送方向会话密码。
/// * `rx_cipher` — 接收方向会话密码。
///
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
    /// 绑定到本地UDP地址并初始化为**发起者**模式的隧道（发送Nonce从0开始）。
    ///
    /// # Errors
    ///
    /// * 如果绑定UDP套接字或创建会话密码失败，返回`TunnelError`。
    ///
    /// # Returns
    ///
    /// * 返回初始化好的`Tunnel`实例。
    ///
    /// Bind to a local UDP address and initialise the tunnel as the
    /// **initiator** (send nonces start at 0).
    pub async fn bind_initiator(local_addr: SocketAddr, shared_secret: &[u8; 32]) -> Result<Self, TunnelError> {
        Self::bind_with_nonce(local_addr, shared_secret, 0).await
    }

    /// 绑定到本地UDP地址并初始化为**响应者**模式的隧道（发送Nonce从`RESPONDER_TX_OFFSET`开始）。
    ///
    /// # Errors
    ///
    /// * 如果绑定UDP套接字或创建会话密码失败，返回`TunnelError`。
    ///
    /// # Returns
    ///
    /// * 返回初始化好的`Tunnel`实例。
    ///
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

    /// 返回本地套接字地址。
    ///
    /// # Returns
    ///
    /// * 本地`SocketAddr`，如果是I/O错误则返回错误。
    ///
    /// Return the local socket address.
    pub fn local_addr(&self) -> Result<SocketAddr, std::io::Error> {
        self.socket.local_addr()
    }

    /// 加密`plaintext`并发送给`peer`。
    ///
    /// # Errors
    ///
    /// * 加密失败或UDP发送失败时返回`TunnelError`。
    ///
    /// # Returns
    ///
    /// * 返回发送的明文字节数。
    ///
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

    /// 接收加密数据包，解密后返回明文及发送者地址。
    ///
    /// # Errors
    ///
    /// * 接收失败或解密失败时返回`TunnelError`。
    ///
    /// # Returns
    ///
    /// * 返回明文数据和发送者`SocketAddr`。
    ///
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

    /// 消耗隧道并返回底层UDP套接字。
    ///
    /// # Returns
    ///
    /// * 底层`UdpSocket`。
    ///
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
        let bob = Tunnel::bind_responder("127.0.0.1:0".parse().unwrap(), &shared).await.unwrap();

        let bob_addr = bob.local_addr().unwrap();

        for i in 0..10u8 {
            let msg = [i; 16];
            alice.send_to(&msg, bob_addr).await.unwrap();
            let (received, _) = bob.recv_from().await.unwrap();
            assert_eq!(&received[..], &msg[..]);
        }
    }
}
