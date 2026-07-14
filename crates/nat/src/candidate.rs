use std::net::SocketAddr;

/// 连接候选的类型。
/// Type of a connection candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateType {
    /// 本地接口地址（位于 NAT 之后）。
    /// A local interface address (behind NAT).
    Host,
    /// 通过 STUN 发现的公网地址。
    /// Public address discovered via STUN.
    ServerReflexive,
}

/// 对等方的连接候选信息。
/// A connection candidate for a peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Candidate {
    /// 套接字地址。
    /// The socket address.
    pub addr: SocketAddr,
    /// 候选类型。
    /// The type of candidate.
    pub ty: CandidateType,
}

impl Candidate {
    /// 从本地地址创建一个主机候选。
    /// Create a host candidate from a local address.
    #[must_use]
    pub fn host(addr: SocketAddr) -> Self {
        Self { addr, ty: CandidateType::Host }
    }

    /// 从 STUN 响应创建一个服务器反射候选。
    /// Create a server-reflexive candidate from a STUN response.
    #[must_use]
    pub fn server_reflexive(addr: SocketAddr) -> Self {
        Self { addr, ty: CandidateType::ServerReflexive }
    }
}
