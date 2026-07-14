//! ConnectAlso Mobile — iOS / Android 平台集成库。
//!
//! # iOS Packet Tunnel Provider
//!
//! This crate provides a C-compatible FFI layer for integrating
//! ConnectAlso into iOS NetworkExtension Packet Tunnel Providers.
//!
//! ## Architecture
//!
//! ```text
//! iOS NetworkExtension Process
//! ┌──────────────────────────────────────┐
//! │  Swift PacketTunnelProvider          │
//! │  ┌────────────────────────────────┐  │
//! │  │  NEPacketTunnelNetworkSettings  │  │
//! │  │  packetFlow.readPackets()       │  │
//! │  │  packetFlow.writePackets()      │  │
//! │  └──────────┬─────────────────────┘  │
//! │             │                         │
//! │  ┌──────────▼─────────────────────┐  │
//! │  │  connectalso_mobile static lib  │  │
//! │  │  (Rust FFI via C ABI)          │  │
//! │  │  ┌──────────────────────────┐  │  │
//! │  │  │ TunnelEngine              │  │  │
//! │  │  │  • Key exchange           │  │  │
//! │  │  │  • Encrypted relay        │  │  │
//! │  │  │  • Peer management        │  │  │
//! │  │  └──────────────────────────┘  │  │
//! │  └───────────────────────────────┘  │
//! └──────────────────────────────────────┘
//!          │ UDP
//!          ▼
//!    Relay Server / P2P
//! ```

pub mod engine;
pub mod ffi;
