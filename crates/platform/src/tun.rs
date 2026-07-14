use std::net::Ipv4Addr;

use thiserror::Error;
use tun2::{Configuration, Device, Layer};

const DEFAULT_MTU: u16 = 1500;

/// TUN 设备操作错误类型。
/// TUN device operation errors.
#[derive(Debug, Error)]
pub enum TunError {
    /// 创建 TUN 设备失败。
    /// Failed to create the TUN device.
    #[error("failed to create TUN device: {0}")]
    Create(String),

    /// 通过 TUN 设备发送包失败。
    /// Failed to send a packet through the TUN device.
    #[error("failed to send packet: {0}")]
    Send(#[source] std::io::Error),

    /// 从 TUN 设备接收包失败。
    /// Failed to receive a packet from the TUN device.
    #[error("failed to receive packet: {0}")]
    Recv(#[source] std::io::Error),

    /// 接收缓冲区小于配置的 MTU。
    /// The receive buffer is too small for the configured MTU.
    #[error("buffer too small: need {need}, got {got}")]
    BufferTooSmall {
        /// 所需缓冲区大小。
        /// Required buffer size.
        need: usize,
        /// 实际缓冲区大小。
        /// Actual buffer size.
        got: usize,
    },
}

/// TUN 设备创建配置。
/// Configuration for creating a TUN device.
#[derive(Debug, Clone)]
pub struct TunConfig {
    /// TUN 接口名称（可选）。
    /// Optional name for the TUN interface.
    pub name: Option<String>,
    /// 分配给 TUN 接口的 IPv4 地址。
    /// IPv4 address to assign to the TUN interface.
    pub address: Ipv4Addr,
    /// TUN 接口的子网掩码。
    /// Subnet mask for the TUN interface.
    pub netmask: Ipv4Addr,
    /// 接口的最大传输单元 (MTU)。
    /// Maximum Transmission Unit (MTU) for the interface.
    pub mtu: u16,
}

impl Default for TunConfig {
    fn default() -> Self {
        Self {
            name: None,
            address: Ipv4Addr::new(100, 64, 0, 1),
            netmask: Ipv4Addr::new(255, 255, 255, 0),
            mtu: DEFAULT_MTU,
        }
    }
}

impl TunConfig {
    /// 使用给定的地址和子网掩码创建新的 TUN 配置。
    /// Create a new TUN configuration with the given address and netmask.
    #[must_use]
    pub fn new(address: Ipv4Addr, netmask: Ipv4Addr) -> Self {
        Self { address, netmask, ..Default::default() }
    }

    /// 设置 TUN 接口名称。
    /// Set the TUN interface name.
    #[must_use]
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// 设置 TUN 接口的 MTU。
    /// Set the MTU for the TUN interface.
    #[must_use]
    pub fn with_mtu(mut self, mtu: u16) -> Self {
        self.mtu = mtu;
        self
    }

    fn to_tun2_config(&self) -> Configuration {
        let mut config = Configuration::default();
        config.address(self.address).netmask(self.netmask).mtu(self.mtu).layer(Layer::L3).up();
        if let Some(ref name) = self.name {
            config.tun_name(name.as_str());
        }
        config
    }
}

/// 虚拟 TUN 网络设备，提供 L3 包收发功能。
/// A virtual TUN network device providing L3 packet send/receive.
pub struct TunDevice {
    inner: Device,
    config: TunConfig,
}

impl TunDevice {
    /// 使用给定配置创建新的 TUN 设备。
    /// Create a new TUN device with the given configuration.
    pub async fn create(config: TunConfig) -> Result<Self, TunError> {
        let tun2_config = config.to_tun2_config();
        let device = tokio::task::spawn_blocking(move || tun2::create(&tun2_config))
            .await
            .map_err(|e| TunError::Create(e.to_string()))?
            .map_err(|e| TunError::Create(e.to_string()))?;
        Ok(Self { inner: device, config })
    }

    /// 返回配置的接口名称。
    /// Return the configured interface name.
    #[must_use]
    pub fn name(&self) -> &str {
        self.config.name.as_deref().unwrap_or("tun")
    }

    /// 返回配置的 MTU。
    /// Return the configured MTU.
    #[must_use]
    pub fn mtu(&self) -> u16 {
        self.config.mtu
    }

    /// 返回分配的 IPv4 地址。
    /// Return the assigned IPv4 address.
    #[must_use]
    pub fn address(&self) -> Ipv4Addr {
        self.config.address
    }

    /// 返回配置的子网掩码。
    /// Return the configured subnet mask.
    #[must_use]
    pub fn netmask(&self) -> Ipv4Addr {
        self.config.netmask
    }

    /// 从 TUN 设备接收原始 IP 包。
    /// Receive a raw IP packet from the TUN device.
    pub async fn recv(&self, buf: &mut [u8]) -> Result<usize, TunError> {
        if buf.len() < self.config.mtu as usize {
            return Err(TunError::BufferTooSmall { need: self.config.mtu as usize, got: buf.len() });
        }
        let n = self.inner.recv(buf).map_err(TunError::Recv)?;
        Ok(n)
    }

    /// 通过 TUN 设备发送原始 IP 包。
    /// Send a raw IP packet through the TUN device.
    pub async fn send(&self, packet: &[u8]) -> Result<usize, TunError> {
        self.inner.send(packet).map_err(TunError::Send)
    }

    /// 消费此包装器并返回底层设备。
    /// Consume this wrapper and return the underlying device.
    #[must_use]
    pub fn into_inner(self) -> Device {
        self.inner
    }
}
