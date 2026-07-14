//! # ConnectAlso Core Types
//!
//! `ConnectAlso` 公共类型、配置与协议定义。
//! Common types, configuration, and protocol definitions.
//!
//! This crate provides shared types used across all other `ConnectAlso` crates,
//! including ACL rule evaluation and packet parsing utilities.
//!
//! 本 crate 提供所有 `ConnectAlso` crate 共享的类型，包括 ACL 规则评估和包解析工具。

use std::net::Ipv4Addr;

use serde::{Deserialize, Serialize};

/// DNS 记录，用于 Magic DNS 域名解析。
/// A DNS record for Magic DNS name resolution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsRecord {
    /// 主机名（不含域名后缀）。
    /// Hostname (without domain suffix).
    pub hostname: String,
    /// 虚拟 IPv4 地址。
    /// Virtual IPv4 address.
    pub ipv4: Ipv4Addr,
}

/// ACL 规则，用于包过滤。
/// An ACL rule for packet filtering.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AclRule {
    /// 规则优先级（值越小优先级越高）。
    /// Rule priority (lower = higher priority).
    pub priority: u32,
    /// 动作："allow" 或 "deny"。
    /// Action: "allow" or "deny".
    pub action: String,
    /// 源虚拟 IP（可选，空字符串表示任意）。
    /// Source virtual IP (optional, empty = any).
    #[serde(default)]
    pub src_ip: String,
    /// 目标虚拟 IP（可选，空字符串表示任意）。
    /// Destination virtual IP (optional, empty = any).
    #[serde(default)]
    pub dst_ip: String,
    /// 协议："tcp"、"udp"、"icmp"，或空字符串表示任意。
    /// Protocol: "tcp", "udp", "icmp", or "" for any.
    #[serde(default)]
    pub protocol: String,
    /// 源端口（0 表示任意）。
    /// Source port (0 = any).
    #[serde(default)]
    pub src_port: u16,
    /// 目标端口（0 表示任意）。
    /// Destination port (0 = any).
    #[serde(default)]
    pub dst_port: u16,
}

/// 解析后的 IP 包头信息，用于 ACL 匹配。
/// An IP packet header parsed for ACL matching.
#[derive(Debug)]
pub struct PacketInfo {
    /// 源 IPv4 地址。
    /// Source IPv4 address.
    pub src_ip: Ipv4Addr,
    /// 目标 IPv4 地址。
    /// Destination IPv4 address.
    pub dst_ip: Ipv4Addr,
    /// IP 协议号（6=TCP, 17=UDP, 1=ICMP）。
    /// IP protocol number (6=TCP, 17=UDP, 1=ICMP).
    pub protocol: u8,
    /// 源端口（仅 TCP/UDP）。
    /// Source port (TCP/UDP only).
    pub src_port: u16,
    /// 目标端口（仅 TCP/UDP）。
    /// Destination port (TCP/UDP only).
    pub dst_port: u16,
}

impl PacketInfo {
    /// 解析 IP 包，提取 ACL 相关字段。
    /// Parse an IP packet to extract ACL-relevant fields.
    ///
    /// # Returns
    ///
    /// 返回 `Some(PacketInfo)` 如果包是合法的 IPv4 包，否则返回 `None`。
    /// Returns `Some(PacketInfo)` if the packet is a valid IPv4 packet, otherwise `None`.
    #[must_use]
    pub fn parse(packet: &[u8]) -> Option<Self> {
        if packet.len() < 20 || packet[0] >> 4 != 4 {
            return None;
        }

        let src = Ipv4Addr::new(packet[12], packet[13], packet[14], packet[15]);
        let dst = Ipv4Addr::new(packet[16], packet[17], packet[18], packet[19]);
        let protocol = packet[9];
        let (src_port, dst_port) = if packet.len() >= 24 && (protocol == 6 || protocol == 17) {
            let ihl = ((packet[0] & 0x0F) * 4) as usize;
            if packet.len() >= ihl + 4 {
                (
                    u16::from_be_bytes([packet[ihl], packet[ihl + 1]]),
                    u16::from_be_bytes([packet[ihl + 2], packet[ihl + 3]]),
                )
            } else {
                (0, 0)
            }
        } else {
            (0, 0)
        };

        Some(Self { src_ip: src, dst_ip: dst, protocol, src_port, dst_port })
    }
}

impl AclRule {
    /// 检查此规则是否匹配给定的包。
    /// Check if this rule matches a packet.
    ///
    /// # Returns
    ///
    /// 如果规则匹配包则返回 `true`。
    /// Returns `true` if the rule matches the packet.
    #[must_use]
    pub fn matches(&self, pkt: &PacketInfo) -> bool {
        if !self.src_ip.is_empty() {
            if let Ok(ip) = self.src_ip.parse::<Ipv4Addr>() {
                if ip != pkt.src_ip {
                    return false;
                }
            }
        }
        if !self.dst_ip.is_empty() {
            if let Ok(ip) = self.dst_ip.parse::<Ipv4Addr>() {
                if ip != pkt.dst_ip {
                    return false;
                }
            }
        }
        if !self.protocol.is_empty() {
            let proto_num = match self.protocol.as_str() {
                "tcp" => 6,
                "udp" => 17,
                "icmp" => 1,
                _ => return false,
            };
            if pkt.protocol != proto_num {
                return false;
            }
        }
        if self.src_port != 0 && pkt.src_port != self.src_port {
            return false;
        }
        if self.dst_port != 0 && pkt.dst_port != self.dst_port {
            return false;
        }
        true
    }
}

/// 评估一组 ACL 规则是否适用于一个包。
/// Evaluate a list of ACL rules against a packet.
///
/// 按优先级顺序评估规则，返回第一个匹配规则的动作；
/// 如果没有规则匹配则默认返回 "allow"。
/// Rules are evaluated in priority order. Returns the action of the
/// first matching rule, or "allow" if no rules match.
///
/// # Arguments
///
/// * `rules` - 按优先级排序的 ACL 规则切片。
/// * `packet` - 原始 IP 包字节。
///
/// * `rules` - Slice of ACL rules sorted by priority.
/// * `packet` - Raw IP packet bytes.
///
/// # Returns
///
/// 返回 `"allow"` 或 `"deny"`。
/// Returns `"allow"` or `"deny"`.
#[must_use]
pub fn evaluate_acls(rules: &[AclRule], packet: &[u8]) -> &'static str {
    let Some(pkt_info) = PacketInfo::parse(packet) else {
        return "allow"; // Can't parse — allow by default
    };

    let mut sorted: Vec<&AclRule> = rules.iter().collect();
    sorted.sort_by_key(|r| r.priority);

    for rule in &sorted {
        if rule.matches(&pkt_info) {
            return if rule.action == "deny" { "deny" } else { "allow" };
        }
    }
    "allow" // Default: allow all
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_parse_ipv4_tcp() {
        // Minimal TCP SYN packet: IPv4 header (20B) + TCP header (20B)
        let mut pkt = vec![0u8; 40];
        pkt[0] = 0x45; // Version=4, IHL=5
        pkt[9] = 6; // TCP
        pkt[12..16].copy_from_slice(&[10, 0, 0, 1]); // src
        pkt[16..20].copy_from_slice(&[10, 0, 0, 2]); // dst
        pkt[20..22].copy_from_slice(&12345u16.to_be_bytes()); // src port
        pkt[22..24].copy_from_slice(&80u16.to_be_bytes()); // dst port

        let info = PacketInfo::parse(&pkt).unwrap();
        assert_eq!(info.src_ip, Ipv4Addr::new(10, 0, 0, 1));
        assert_eq!(info.src_port, 12345);
        assert_eq!(info.dst_port, 80);
    }

    #[test]
    fn acl_deny_ssh() {
        let rules = vec![AclRule {
            priority: 10,
            action: "deny".into(),
            src_ip: String::new(),
            dst_ip: String::new(),
            protocol: "tcp".into(),
            src_port: 0,
            dst_port: 22,
        }];

        let mut pkt = vec![0u8; 40];
        pkt[0] = 0x45;
        pkt[9] = 6;
        pkt[12..16].copy_from_slice(&[10, 0, 0, 1]);
        pkt[16..20].copy_from_slice(&[10, 0, 0, 2]);
        pkt[22..24].copy_from_slice(&22u16.to_be_bytes());

        assert_eq!(evaluate_acls(&rules, &pkt), "deny");
    }

    #[test]
    fn acl_allow_default() {
        let rules: Vec<AclRule> = vec![];
        let pkt = vec![0u8; 40];
        assert_eq!(evaluate_acls(&rules, &pkt), "allow");
    }
}
