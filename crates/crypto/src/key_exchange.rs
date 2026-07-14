//! # Key Exchange
//!
//! X25519 密钥交换与会话密钥派生。
//! X25519 key exchange and session key derivation.
//!
//! Supports ephemeral key pair generation, Diffie-Hellman shared secret
//! derivation, and ChaCha20-Poly1305 AEAD session encryption with
//! monotonic nonce counters.
//!
//! 支持临时密钥对生成、Diffie-Hellman 共享密钥派生，
//! 以及使用单调 nonce 计数器的 ChaCha20-Poly1305 AEAD 会话加密。

use chacha20poly1305::aead::Aead;
use chacha20poly1305::{ChaCha20Poly1305, Key, KeyInit, Nonce};
use thiserror::Error;
use tracing::debug;
use x25519_dalek::{EphemeralSecret, PublicKey};

const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;

/// 加密操作错误类型。
/// Cryptographic operation errors.
#[derive(Debug, Error)]
pub enum CryptoError {
    /// 加密失败。
    /// Encryption failed.
    #[error("encryption failed")]
    Encrypt,
    /// 解密失败（密钥错误、数据被篡改或包损坏）。
    /// Decryption failed (wrong key, tampered data, or corrupted packet).
    #[error("decryption failed")]
    Decrypt,
}

/// X25519 密钥对，用于单方通信。
/// An X25519 key pair for a single party.
pub struct KeyPair {
    secret: EphemeralSecret,
    public: PublicKey,
}

impl KeyPair {
    /// 生成一个新的随机 X25519 密钥对。
    /// Generate a new random X25519 key pair.
    pub fn generate() -> Self {
        let secret = EphemeralSecret::random();
        let public = PublicKey::from(&secret);
        debug!("generated new X25519 key pair");
        Self { secret, public }
    }

    /// 返回公钥字节。
    /// Return the public key bytes.
    #[must_use]
    pub fn public_key_bytes(&self) -> [u8; KEY_LEN] {
        *self.public.as_bytes()
    }

    /// 执行 Diffie-Hellman 密钥交换，与对端派生共享密钥。
    /// Perform Diffie-Hellman to derive a shared secret with a peer.
    ///
    /// 消耗密钥对（临时密钥为单次使用）。
    /// Consumes the key pair (ephemeral secret is single-use).
    #[must_use]
    pub fn diffie_hellman(self, peer_public: &[u8; KEY_LEN]) -> [u8; KEY_LEN] {
        let peer_key = PublicKey::from(*peer_public);
        let shared = self.secret.diffie_hellman(&peer_key);
        *shared.as_bytes()
    }
}

/// 会话加密状态，用于单个通信方向。
/// Session encryption state for a single communication direction.
pub struct SessionCipher {
    cipher: ChaCha20Poly1305,
    counter: u64,
}

impl SessionCipher {
    /// 使用原始 32 字节共享密钥创建新的会话加密器。
    /// Create a new session cipher from a raw 32-byte shared secret.
    #[must_use]
    pub fn new(shared_secret: &[u8; KEY_LEN], init_counter: u64) -> Self {
        let key = Key::from_slice(shared_secret);
        let cipher = ChaCha20Poly1305::new(key);
        Self { cipher, counter: init_counter }
    }

    /// 加密明文包，返回带 nonce 前缀的 AEAD 认证密文。
    /// Encrypt a plaintext packet, returning the AEAD-authenticated ciphertext
    /// with the nonce prepended.
    ///
    /// 输出格式：`[nonce: 12 bytes][ciphertext: plaintext.len() + 16 bytes]`
    /// Output format: `[nonce: 12 bytes][ciphertext: plaintext.len() + 16 bytes]`
    ///
    /// # Errors
    ///
    /// 如果加密操作失败，返回 `CryptoError::Encrypt`。
    /// Returns `CryptoError::Encrypt` if the encryption operation fails.
    pub fn encrypt(&mut self, plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
        let nonce = self.next_nonce();
        let ciphertext = self.cipher.encrypt(&nonce, plaintext).map_err(|_| CryptoError::Encrypt)?;

        let mut packet = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        packet.extend_from_slice(nonce.as_slice());
        packet.extend_from_slice(&ciphertext);
        Ok(packet)
    }

    /// 解密从对端接收的包。
    /// Decrypt a packet received from the peer.
    ///
    /// 输入格式：`[nonce: 12 bytes][ciphertext + tag]`
    /// Expects input format: `[nonce: 12 bytes][ciphertext + tag]`
    ///
    /// # Errors
    ///
    /// 如果认证失败，返回 `CryptoError::Decrypt`。
    /// Returns `CryptoError::Decrypt` if authentication fails.
    pub fn decrypt(&self, packet: &[u8]) -> Result<Vec<u8>, CryptoError> {
        if packet.len() < NONCE_LEN + TAG_LEN {
            return Err(CryptoError::Decrypt);
        }
        let (nonce_bytes, ciphertext) = packet.split_at(NONCE_LEN);
        let nonce = Nonce::from_slice(nonce_bytes);
        self.cipher.decrypt(nonce, ciphertext).map_err(|_| CryptoError::Decrypt)
    }

    fn next_nonce(&mut self) -> Nonce {
        let c = self.counter;
        self.counter = self.counter.wrapping_add(1);
        let mut nonce_bytes = [0u8; NONCE_LEN];
        nonce_bytes[4..12].copy_from_slice(&c.to_le_bytes());
        *Nonce::from_slice(&nonce_bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_exchange_and_encrypt_decrypt() {
        let alice = KeyPair::generate();
        let bob = KeyPair::generate();

        let alice_pub = alice.public_key_bytes();
        let bob_pub = bob.public_key_bytes();

        let alice_shared = alice.diffie_hellman(&bob_pub);
        let bob_shared = bob.diffie_hellman(&alice_pub);
        assert_eq!(alice_shared, bob_shared);

        // Alice sends, Bob receives (different nonce starting points)
        let mut alice_tx = SessionCipher::new(&alice_shared, 0);
        let bob_rx = SessionCipher::new(&bob_shared, 0);

        let msg = b"hello encrypted world";
        let encrypted = alice_tx.encrypt(msg).unwrap();
        let decrypted = bob_rx.decrypt(&encrypted).unwrap();
        assert_eq!(&decrypted, msg);
    }

    #[test]
    fn wrong_key_fails_decryption() {
        let alice = KeyPair::generate();
        let bob = KeyPair::generate();
        let eve = KeyPair::generate();

        let bob_pub = bob.public_key_bytes();

        let alice_shared = alice.diffie_hellman(&bob_pub);
        let eve_shared = eve.diffie_hellman(&bob_pub);

        let mut alice_tx = SessionCipher::new(&alice_shared, 0);
        let eve_rx = SessionCipher::new(&eve_shared, 0);

        let msg = b"secret message";
        let encrypted = alice_tx.encrypt(msg).unwrap();
        let result = eve_rx.decrypt(&encrypted);
        assert!(result.is_err());
    }

    #[test]
    fn nonce_increments() {
        let key = [0u8; KEY_LEN];
        let mut cipher = SessionCipher::new(&key, 0);

        let c1 = cipher.encrypt(b"msg1").unwrap();
        let c2 = cipher.encrypt(b"msg2").unwrap();

        // Nonces should differ (first 12 bytes of each ciphertext)
        assert_ne!(&c1[..NONCE_LEN], &c2[..NONCE_LEN]);

        let rx = SessionCipher::new(&key, 0);
        assert_eq!(rx.decrypt(&c1).unwrap(), b"msg1");
        assert_eq!(rx.decrypt(&c2).unwrap(), b"msg2");
    }

    #[test]
    fn truncated_packet_fails() {
        let key = [0u8; KEY_LEN];
        let mut cipher = SessionCipher::new(&key, 0);
        let rx = SessionCipher::new(&key, 0);

        let encrypted = cipher.encrypt(b"test").unwrap();
        let result = rx.decrypt(&encrypted[..10]);
        assert!(result.is_err());
    }
}
