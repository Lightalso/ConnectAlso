use std::fmt;

/// NAT 行为分类（RFC 3489 / 5780）。
/// NAT behavior classification (RFC 3489 / 5780).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NatType {
    /// 完全锥形 NAT：内部主机创建映射后，任意外部主机均可向该映射地址发送数据包。
    /// Full Cone: any external host can send packets to the mapped
    /// address once an internal host has created a mapping.
    FullCone,
    /// 受限锥形 NAT：仅先前收到过内部主机数据包的外部主机可回传。
    /// Restricted Cone: only external hosts that have previously
    /// received a packet from the internal host can send back.
    RestrictedCone,
    /// 端口受限锥形 NAT：类似受限锥形，但外部主机还必须使用相同端口。
    /// Port Restricted Cone: like Restricted Cone, but the external
    /// host must also use the same port.
    PortRestrictedCone,
    /// 对称 NAT：每个发往不同目标的请求都会创建新的映射，且外部端口可能不同。
    /// Symmetric: each request to a different destination creates a
    /// new mapping with a potentially different external port.
    Symmetric,
    /// NAT 类型无法确定（例如无可用的 STUN 服务器）。
    /// NAT type could not be determined (e.g. no STUN server available).
    Unknown,
    /// 未检测到 NAT（公网 IP 与本地 IP 一致）。
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
    /// 判断此 NAT 类型是否可能成功进行 P2P 打洞。
    /// Whether P2P hole punching is likely to succeed for this NAT type.
    ///
    /// # Returns
    ///
    /// 对于完全锥形、受限锥形、端口受限锥形以及无 NAT（Open）的情况返回 `true`。
    /// Returns `true` for Full Cone, Restricted Cone, Port Restricted Cone, and Open.
    #[must_use]
    pub const fn supports_p2p(&self) -> bool {
        matches!(self, Self::FullCone | Self::RestrictedCone | Self::PortRestrictedCone | Self::Open)
    }

    /// 判断此 NAT 类型是否建议使用中继服务器。
    /// Whether a relay is recommended for this NAT type.
    ///
    /// # Returns
    ///
    /// 对于对称 NAT 和未知类型返回 `true`。
    /// Returns `true` for Symmetric and Unknown.
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
