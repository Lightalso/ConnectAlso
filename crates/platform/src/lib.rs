//! # ConnectAlso Platform
//!
//! ConnectAlso 平台抽象层：TUN、路由、DNS 和安全存储。
//! Platform abstraction layer: TUN, routing, DNS, and secure storage.
//!
//! Provides cross-platform TUN device management for creating
//! virtual L3 network interfaces used by the daemon.
//!
//! 提供跨平台 TUN 设备管理，用于创建守护进程使用的虚拟 L3 网络接口。

/// 跨平台 TUN 设备抽象。
/// Cross-platform TUN device abstraction.
pub mod tun;
