//! ConnectAlso Mobile — iOS / Android 平台集成库。

/// Android JNI bindings for the mobile engine.
pub mod android;
/// Cross-platform tunnel engine (shared between iOS and Android).
pub mod engine;
/// iOS C ABI exports for Packet Tunnel Provider integration.
pub mod ffi;
