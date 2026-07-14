//! Relay protocol definitions for encrypted traffic forwarding.
//!
//! This crate defines the frame format used between peers and relay servers,
//! including peer identification, message types, and wire encoding.

use thiserror::Error;
use uuid::Uuid;

/// A peer identifier. In production this would be derived from the
/// device's public key fingerprint.
pub type PeerId = Uuid;

/// Current protocol version byte.
pub const PROTO_VERSION: u8 = 0x01;

/// Maximum payload size for a single relayed DATA frame.
pub const MAX_PAYLOAD: usize = 2048;

/// Total header size: version(1) + type(1) + sender(16) + target(16) + len(2).
pub const HEADER_LEN: usize = 1 + 1 + 16 + 16 + 2;

/// Relay protocol errors.
#[derive(Debug, Error)]
pub enum ProtoError {
    /// The frame data is too short to contain a valid header.
    #[error("frame too short: {0} bytes")]
    FrameTooShort(usize),
    /// Unknown protocol version.
    #[error("unknown protocol version: {0}")]
    UnknownVersion(u8),
    /// Unknown message type.
    #[error("unknown message type: {0}")]
    UnknownType(u8),
    /// Payload exceeds maximum allowed size.
    #[error("payload too large: {0} > {MAX_PAYLOAD}")]
    PayloadTooLarge(usize),
}

/// Relay message types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MsgType {
    /// Register this peer with the relay, or refresh the registration.
    Hello = 0x01,
    /// Forward encrypted data from sender to target peer.
    Data = 0x02,
    /// Keepalive / heartbeat.
    Keepalive = 0x03,
}

impl MsgType {
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

/// A relay protocol frame.
///
/// Wire format:
/// `[version:1][type:1][sender_id:16][target_id:16][payload_len:2 BE][payload:N]`
///
/// - For `Hello` and `Keepalive`, `target_id` is set to nil (all zeros).
/// - For `Data`, `sender_id` is the origin and `target_id` is the
///   intended recipient. The relay forwards the payload to `target_id`.
#[derive(Debug, Clone)]
pub struct RelayFrame {
    /// The peer that originated this frame.
    pub sender_id: PeerId,
    /// The intended recipient (ignored for Hello / Keepalive).
    pub target_id: PeerId,
    /// Message type.
    pub msg_type: MsgType,
    /// Payload — for Data frames this is the encrypted tunnel packet.
    pub payload: Vec<u8>,
}

impl RelayFrame {
    /// Create a HELLO frame from a peer.
    #[must_use]
    pub fn hello(sender_id: PeerId) -> Self {
        Self { sender_id, target_id: PeerId::nil(), msg_type: MsgType::Hello, payload: Vec::new() }
    }

    /// Create a DATA frame carrying encrypted payload from `sender` to `target`.
    #[must_use]
    pub fn data(sender_id: PeerId, target_id: PeerId, encrypted_payload: Vec<u8>) -> Self {
        Self { sender_id, target_id, msg_type: MsgType::Data, payload: encrypted_payload }
    }

    /// Create a KEEPALIVE frame.
    #[must_use]
    pub fn keepalive(sender_id: PeerId) -> Self {
        Self { sender_id, target_id: PeerId::nil(), msg_type: MsgType::Keepalive, payload: Vec::new() }
    }

    /// Serialize this frame to bytes suitable for sending over UDP.
    ///
    /// # Errors
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

    /// Deserialize a frame from raw bytes received over UDP.
    ///
    /// # Errors
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
