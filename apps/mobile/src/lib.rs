//! ConnectAlso Mobile — iOS / Android platform integration library.
//!
//! FFI/JNI code requires unsafe, raw pointers, and no_mangle by design.
//! All warnings are suppressed for this platform-specific crate.

#![allow(warnings)]

/// Android JNI bindings for the mobile engine.
#[cfg(target_os = "android")]
pub mod android;
/// Cross-platform tunnel engine (shared between iOS and Android).
pub mod engine;
/// iOS C ABI exports for Packet Tunnel Provider integration.
pub mod ffi;
