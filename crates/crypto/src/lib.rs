//! # ConnectAlso Crypto
//!
//! ConnectAlso 身份、会话和密钥轮换。
//! Identity, session, and key rotation for ConnectAlso.
//!
//! This crate provides X25519 Elliptic Curve Diffie-Hellman key exchange
//! and ChaCha20-Poly1305 AEAD session encryption.
//!
//! 本 crate 提供 X25519 椭圆曲线 Diffie-Hellman 密钥交换
//! 和 ChaCha20-Poly1305 AEAD 会话加密。

/// X25519 密钥交换和 ChaCha20-Poly1305 会话加密。
/// X25519 key exchange and ChaCha20-Poly1305 session encryption.
pub mod key_exchange;
