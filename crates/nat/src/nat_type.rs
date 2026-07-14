use std::fmt;

/// NAT behavior classification (RFC 3489 / 5780).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NatType {
    /// Full Cone: any external host can send packets to the mapped
    /// address once an internal host has created a mapping.
    FullCone,
    /// Restricted Cone: only external hosts that have previously
    /// received a packet from the internal host can send back.
    RestrictedCone,
    /// Port Restricted Cone: like Restricted Cone, but the external
    /// host must also use the same port.
    PortRestrictedCone,
    /// Symmetric: each request to a different destination creates a
    /// new mapping with a potentially different external port.
    Symmetric,
    /// NAT type could not be determined (e.g. no STUN server available).
    Unknown,
    /// No NAT detected (public IP equals local IP).
    Open,
}

impl fmt::Display for NatType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FullCone => write!(f, "Full Cone"),
            Self::RestrictedCone => write!(f, "Restricted Cone"),
            Self::PortRestrictedCone => write!(f, "Port Restricted Cone"),
            Self::Symmetric => write!(f, "Symmetric"),
            Self::Unknown => write!(f, "Unknown"),
            Self::Open => write!(f, "Open (no NAT)"),
        }
    }
}

impl NatType {
    /// Whether P2P hole punching is likely to succeed for this NAT type.
    #[must_use]
    pub const fn supports_p2p(&self) -> bool {
        matches!(self, Self::FullCone | Self::RestrictedCone | Self::PortRestrictedCone | Self::Open)
    }

    /// Whether a relay is recommended for this NAT type.
    #[must_use]
    pub const fn needs_relay(&self) -> bool {
        matches!(self, Self::Symmetric | Self::Unknown)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nat_type_display() {
        assert_eq!(format!("{}", NatType::FullCone), "Full Cone");
        assert_eq!(format!("{}", NatType::Symmetric), "Symmetric");
        assert_eq!(format!("{}", NatType::Open), "Open (no NAT)");
    }

    #[test]
    fn p2p_support() {
        assert!(NatType::FullCone.supports_p2p());
        assert!(NatType::RestrictedCone.supports_p2p());
        assert!(NatType::PortRestrictedCone.supports_p2p());
        assert!(!NatType::Symmetric.supports_p2p());
        assert!(NatType::Open.supports_p2p());
        assert!(!NatType::Unknown.supports_p2p());
    }

    #[test]
    fn relay_need() {
        assert!(NatType::Symmetric.needs_relay());
        assert!(NatType::Unknown.needs_relay());
        assert!(!NatType::PortRestrictedCone.needs_relay());
        assert!(!NatType::Open.needs_relay());
    }
}
