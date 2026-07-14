use std::net::SocketAddr;

/// Type of a connection candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateType {
    /// A local interface address (behind NAT).
    Host,
    /// Public address discovered via STUN.
    ServerReflexive,
}

/// A connection candidate for a peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Candidate {
    /// The socket address.
    pub addr: SocketAddr,
    /// The type of candidate.
    pub ty: CandidateType,
}

impl Candidate {
    /// Create a host candidate from a local address.
    #[must_use]
    pub fn host(addr: SocketAddr) -> Self {
        Self { addr, ty: CandidateType::Host }
    }

    /// Create a server-reflexive candidate from a STUN response.
    #[must_use]
    pub fn server_reflexive(addr: SocketAddr) -> Self {
        Self { addr, ty: CandidateType::ServerReflexive }
    }
}
