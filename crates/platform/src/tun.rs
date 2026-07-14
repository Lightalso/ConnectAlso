use std::net::Ipv4Addr;

use tun2::{create_as_async, AsyncDevice, Configuration, Layer};

const DEFAULT_MTU: u16 = 1500;

/// TUN device operation errors.
#[derive(Debug, Error)]
pub enum TunError {
    /// Failed to create the TUN device.
    #[error("failed to create TUN device: {0}")]
    Create(String),

    /// Failed to send a packet through the TUN device.
    #[error("failed to send packet: {0}")]
    Send(#[source] std::io::Error),

    /// Failed to receive a packet from the TUN device.
    #[error("failed to receive packet: {0}")]
    Recv(#[source] std::io::Error),

    /// The receive buffer is too small for the configured MTU.
    #[error("buffer too small: need {need}, got {got}")]
    BufferTooSmall { need: usize, got: usize },
}

/// Configuration for creating a TUN device.
#[derive(Debug, Clone)]
pub struct TunConfig {
    /// Optional name for the TUN interface.
    pub name: Option<String>,
    /// IPv4 address to assign to the TUN interface.
    pub address: Ipv4Addr,
    /// Subnet mask for the TUN interface.
    pub netmask: Ipv4Addr,
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
    /// Create a new TUN configuration with the given address and netmask.
    #[must_use]
    pub fn new(address: Ipv4Addr, netmask: Ipv4Addr) -> Self {
        Self { address, netmask, ..Default::default() }
    }

    /// Set the TUN interface name.
    #[must_use]
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

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

/// A virtual TUN network device providing L3 packet send/receive.
///
/// Wraps [`tun2::AsyncDevice`] with a higher-level API and
/// configuration tracking.
pub struct TunDevice {
    inner: AsyncDevice,
    config: TunConfig,
}

impl TunDevice {
    /// Create a new TUN device with the given configuration.
    ///
    /// This instantiates the platform-specific TUN interface and
    /// assigns the configured IP address and netmask.
    pub async fn create(config: TunConfig) -> Result<Self, TunError> {
        let tun2_config = config.to_tun2_config();
        let device = create_as_async(&tun2_config).map_err(|e| TunError::Create(e.to_string()))?;
        Ok(Self { inner: device, config })
    }

    /// Return the configured interface name, or `"tun"` if none was set.
    #[must_use]
    pub fn name(&self) -> &str {
        self.config.name.as_deref().unwrap_or("tun")
    }

    /// Return the configured MTU.
    #[must_use]
    pub fn mtu(&self) -> u16 {
        self.config.mtu
    }

    /// Return the assigned IPv4 address.
    #[must_use]
    pub fn address(&self) -> Ipv4Addr {
        self.config.address
    }

    /// Return the configured subnet mask.
    #[must_use]
    pub fn netmask(&self) -> Ipv4Addr {
        self.config.netmask
    }

    /// Receive a raw IP packet from the TUN device.
    ///
    /// The buffer must be at least as large as the configured MTU.
    pub async fn recv(&self, buf: &mut [u8]) -> Result<usize, TunError> {
        if buf.len() < self.config.mtu as usize {
            return Err(TunError::BufferTooSmall { need: self.config.mtu as usize, got: buf.len() });
        }
        self.inner.recv(buf).await.map_err(TunError::Recv)
    }

    /// Send a raw IP packet through the TUN device.
    pub async fn send(&self, packet: &[u8]) -> Result<usize, TunError> {
        self.inner.send(packet).await.map_err(TunError::Send)
    }

    /// Consume this wrapper and return the underlying [`AsyncDevice`].
    #[must_use]
    pub fn into_inner(self) -> AsyncDevice {
        self.inner
    }
}
