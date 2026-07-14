//! ConnectAlso STUN、NAT 探测与 UDP 打洞。

/// Connection candidate types for NAT traversal.
pub mod candidate;
/// NAT type detection utilities.
pub mod detector;
/// NAT behavior classification enums.
pub mod nat_type;
/// UDP hole punching for NAT traversal.
pub mod punch;
/// STUN client implementation (RFC 5389).
pub mod stun;
