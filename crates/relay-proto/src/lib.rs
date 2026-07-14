//! # ConnectAlso Relay Protocol
//!
//! 中继协议定义，用于加密流量转发。
//! Relay protocol definitions for encrypted traffic forwarding.
//!
//! This crate defines the frame format used between peers and relay servers,
//! including peer identification, message types, and wire encoding.
//!
//! 本 crate 定义了节点与中继服务器之间使用的帧格式，
//! 包括节点标识、消息类型和线缆编码。

use thiserror::Error;
use uuid::Uuid;

/// 节点标识符。在生产环境中，此标识符应派生自设备公钥指纹。
/// A peer identifier. In production this would be derived from the
/// device's public key fingerprint.
pub type PeerId = Uuid;

/// 当前协议版本号。
/// Current protocol version byte.
pub const PROTO_VERSION: u8 = 0x01;

/// 单个中继 DATA 帧的最大负载大小。
/// Maximum payload size for a single relayed DATA frame.
pub const MAX_PAYLOAD: usize = 2048;

/// 总头大小：version(1) + type(1) + sender(16) + target(16) + len(2)。
/// Total header size: version(1) + type(1) + sender(16) + target(16) + len(2).
pub const HEADER_LEN: usize = 1 + 1 + 16 + 16 + 2;

/// 中继协议错误类型。
/// Relay protocol errors.
#[derive(Debug, Error)]
pub enum ProtoError {
    /// 帧数据太短，不包含有效头。
    /// The frame data is too short to contain a valid header.
    #[error("frame too short: {0} bytes")]
    FrameTooShort(usize),
    /// 未知协议版本。
    /// Unknown protocol version.
    #[error("unknown protocol version: {0}")]
    UnknownVersion(u8),
    /// 未知消息类型。
    /// Unknown message type.
    #[error("unknown message type: {0}")]
    UnknownType(u8),
    /// 负载超过最大允许大小。
    /// Payload exceeds maximum allowed size.
    #[error("payload too large: {0} > {MAX_PAYLOAD}")]
    PayloadTooLarge(usize),
}

/// 中继消息类型。
/// Relay message types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MsgType {
    /// 向中继注册此节点，或刷新注册。
    /// Register this peer with the relay, or refresh the registration.
    Hello = 0x01,
    /// 将加密数据从发送节点转发到目标节点。
    /// Forward encrypted data from sender to target peer.
    Data = 0x02,
    /// 心跳 / 保活。
    /// Keepalive / heartbeat.
    Keepalive = 0x03,
}

impl MsgType {
    /// 从原始字节解析消息类型。
    /// Parse a message type from a raw byte.
    #[must_use]
    pub const fn from_byte(b: u8) -> Option<Self> {
        match b {
            0x01 => Some(Self::Hello),
            0x02 => Some(Self::Data),
            0x03 => Some(Self::Keepalive),
            _ => None,
        }
    }
}

/// 中继协议帧。
/// A relay protocol frame.
///
/// 线缆格式：
/// Wire format:
/// `[version:1][type:1][sender_id:16][target_id:16][payload_len:2 BE][payload:N]`
///
/// - 对于 `Hello` 和 `Keepalive`，`target_id` 设为 nil（全零）。
/// - 对于 `Data`，`sender_id` 是发送方，`target_id` 是目标接收方。
///   中继将负载转发给 `target_id`。
///
/// - For `Hello` and `Keepalive`, `target_id` is set to nil (all zeros).
/// - For `Data`, `sender_id` is the origin and `target_id` is the
///   intended recipient. The relay forwards the payload to `target_id`.
#[derive(Debug, Clone)]
pub struct RelayFrame {
    /// 发起此帧的节点。
    /// The peer that originated this frame.
    pub sender_id: PeerId,
    /// 目标接收方（Hello / Keepalive 忽略）。
    /// The intended recipient (ignored for Hello / Keepalive).
    pub target_id: PeerId,
    /// 消息类型。
    /// Message type.
    pub msg_type: MsgType,
    /// 负载 — Data 帧中为加密的隧道包。
    /// Payload — for Data frames this is the encrypted tunnel packet.
    pub payload: Vec<u8>,
}

impl RelayFrame {
    /// 创建 HELLO 帧。
    /// Create a HELLO frame from a peer.
    #[must_use]
    pub fn hello(sender_id: PeerId) -> Self {
        Self { sender_id, target_id: PeerId::nil(), msg_type: MsgType::Hello, payload: Vec::new() }
    }

    /// 创建 DATA 帧，携带从 `sender` 到 `target` 的加密负载。
    /// Create a DATA frame carrying encrypted payload from `sender` to `target`.
    #[must_use]
    pub fn data(sender_id: PeerId, target_id: PeerId, encrypted_payload: Vec<u8>) -> Self {
        Self { sender_id, target_id, msg_type: MsgType::Data, payload: encrypted_payload }
    }

    /// 创建 KEEPALIVE 帧。
    /// Create a KEEPALIVE frame.
    #[must_use]
    pub fn keepalive(sender_id: PeerId) -> Self {
        Self { sender_id, target_id: PeerId::nil(), msg_type: MsgType::Keepalive, payload: Vec::new() }
    }

    /// 将帧序列化为适合通过 UDP 发送的字节。
    /// Serialize this frame to bytes suitable for sending over UDP.
    ///
    /// # Errors
    ///
    /// 如果负载超过 `MAX_PAYLOAD`，返回 `ProtoError::PayloadTooLarge`。
    /// Returns `ProtoError::PayloadTooLarge` if the payload exceeds `MAX_PAYLOAD`.
    pub fn encode(&self) -> Result<Vec<u8>, ProtoError> {
        if self.payload.len() > MAX_PAYLOAD {
            return Err(ProtoError::PayloadTooLarge(self.payload.len()));
        }
        let payload_len = self.payload.len() as u16;
        let mut buf = Vec::with_capacity(HEADER_LEN + self.payload.len());
        buf.push(PROTO_VERSION);
        buf.push(self.msg_type as u8);
        buf.extend_from_slice(self.sender_id.as_bytes());
        buf.extend_from_slice(self.target_id.as_bytes());
        buf.extend_from_slice(&payload_len.to_be_bytes());
        buf.extend_from_slice(&self.payload);
        Ok(buf)
    }

    /// 从 UDP 接收的原始字节反序列化帧。
    /// Deserialize a frame from raw bytes received over UDP.
    ///
    /// # Errors
    ///
    /// 对于格式错误的输入，返回 `ProtoError` 相应变体。
    /// Returns `ProtoError` variants for malformed input.
    pub fn decode(data: &[u8]) -> Result<Self, ProtoError> {
        if data.len() < HEADER_LEN {
            return Err(ProtoError::FrameTooShort(data.len()));
        }

        let version = data[0];
        if version != PROTO_VERSION {
            return Err(ProtoError::UnknownVersion(version));
        }

        let msg_type = MsgType::from_byte(data[1]).ok_or(ProtoError::UnknownType(data[1]))?;

        let mut sender_bytes = [0u8; 16];
        sender_bytes.copy_from_slice(&data[2..18]);
        let sender_id = PeerId::from_bytes(sender_bytes);

        let mut target_bytes = [0u8; 16];
        target_bytes.copy_from_slice(&data[18..34]);
        let target_id = PeerId::from_bytes(target_bytes);

        let payload_len = u16::from_be_bytes([data[34], data[35]]) as usize;
        let payload_end = HEADER_LEN + payload_len;

        if data.len() < payload_end {
            return Err(ProtoError::FrameTooShort(data.len()));
        }
        let payload = data[HEADER_LEN..payload_end].to_vec();

        Ok(Self { sender_id, target_id, msg_type, payload })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_roundtrip() {
        let id = PeerId::new_v4();
        let frame = RelayFrame::hello(id);
        let encoded = frame.encode().unwrap();
        let decoded = RelayFrame::decode(&encoded).unwrap();
        assert_eq!(decoded.sender_id, id);
        assert_eq!(decoded.target_id, PeerId::nil());
        assert_eq!(decoded.msg_type, MsgType::Hello);
        assert!(decoded.payload.is_empty());
    }

    #[test]
    fn data_roundtrip() {
        let alice = PeerId::new_v4();
        let bob = PeerId::new_v4();
        let payload = vec![0xAA; 256];
        let frame = RelayFrame::data(alice, bob, payload.clone());
        let encoded = frame.encode().unwrap();
        let decoded = RelayFrame::decode(&encoded).unwrap();
        assert_eq!(decoded.sender_id, alice);
        assert_eq!(decoded.target_id, bob);
        assert_eq!(decoded.msg_type, MsgType::Data);
        assert_eq!(decoded.payload, payload);
    }

    #[test]
    fn payload_too_large() {
        let id = PeerId::new_v4();
        let frame = RelayFrame::data(id, id, vec![0; MAX_PAYLOAD + 1]);
        assert!(frame.encode().is_err());
    }

    #[test]
    fn decode_truncated() {
        assert!(RelayFrame::decode(&[0x01]).is_err());
        assert!(RelayFrame::decode(&[0u8; 35]).is_err());
    }

    #[test]
    fn decode_bad_version() {
        let mut data = vec![0u8; HEADER_LEN];
        data[0] = 0xFF;
        assert!(RelayFrame::decode(&data).is_err());
    }

    #[test]
    fn decode_bad_type() {
        let mut data = vec![0u8; HEADER_LEN];
        data[0] = PROTO_VERSION;
        data[1] = 0xFF;
        assert!(RelayFrame::decode(&data).is_err());
    }
}
