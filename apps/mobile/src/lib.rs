//! ConnectAlso 移动端 — iOS / Android 平台集成库。
//! ConnectAlso Mobile — iOS / Android platform integration library.
//!
//! 提供跨平台的隧道引擎、iOS Packet Tunnel Provider 的 C ABI 接口、
//! 以及 Android VPN Service 的 JNI 绑定。
//! 由于 FFI/JNI 接口需要使用 `unsafe`、原始指针和 `no_mangle`，
//! 此平台专用 crate 已关闭所有编译警告。
//!
//! Provides a cross-platform tunnel engine, C ABI exports for iOS
//! Packet Tunnel Provider integration, and JNI bindings for Android
//! VPN Service. FFI/JNI code requires unsafe, raw pointers, and
//! no_mangle by design. All warnings are suppressed for this
//! platform-specific crate.

#![allow(warnings)]

/// Android JNI 绑定（仅供 Android 平台编译）。
/// Android JNI bindings for the mobile engine.
#[cfg(target_os = "android")]
pub mod android;
/// 跨平台隧道引擎（iOS 与 Android 共享）。
/// Cross-platform tunnel engine (shared between iOS and Android).
pub mod engine;
/// iOS C ABI 导出（供 Packet Tunnel Provider 集成）。
/// iOS C ABI exports for Packet Tunnel Provider integration.
pub mod ffi;
