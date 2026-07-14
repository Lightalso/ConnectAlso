//! ConnectAlso STUN、NAT 探测与 UDP 打洞。
//! ConnectAlso STUN, NAT detection and UDP hole punching.

/// NAT 穿透的连接候选类型。
/// Connection candidate types for NAT traversal.
pub mod candidate;
/// NAT 类型探测工具。
/// NAT type detection utilities.
pub mod detector;
/// NAT 行为分类枚举。
/// NAT behavior classification enums.
pub mod nat_type;
/// NAT 穿透的 UDP 打洞实现。
/// UDP hole punching for NAT traversal.
pub mod punch;
/// STUN 客户端实现（RFC 5389）。
/// STUN client implementation (RFC 5389).
pub mod stun;
