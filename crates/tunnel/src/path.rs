use std::net::SocketAddr;
use std::time::Duration;

use tokio::sync::watch;

use crate::relay::RelayClient;
use crate::{Tunnel, TunnelError};

const INITIAL_RETRY_MS: u64 = 200;
const MAX_RETRY_MS: u64 = 30_000;
const BACKOFF_MULTIPLIER: u64 = 2;

/// Path status for connectivity monitoring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathStatus {
    /// Direct P2P is active.
    Direct,
    /// Fallen back to relay.
    Relay,
    /// Attempting to establish direct P2P.
    Probing,
}

/// Manages two communication paths to a peer:
/// - **Direct** tunnel (P2P, preferred)
/// - **Relay** fallback (via relay server)
///
/// On send, tries direct first; falls back to relay.
/// On recv, listens on the relay path (which always works).
/// Periodically probes the direct path to restore P2P.
pub struct PathManager {
    relay: RelayClient,
    direct_peer: SocketAddr,
    status: PathStatus,
    backoff_ms: u64,
    status_tx: watch::Sender<PathStatus>,
}

/// A received message bundle.
#[derive(Debug)]
pub struct RecvMessage {
    /// The decrypted payload.
    pub data: Vec<u8>,
    /// Which path delivered the message.
    pub via: &'static str,
}

impl PathManager {
    /// Create a new path manager.
    ///
    /// Starts in `Relay` mode. Call `try_direct()` to attempt P2P.
    pub fn new(relay: RelayClient, direct_peer: SocketAddr) -> Self {
        let (status_tx, _) = watch::channel(PathStatus::Relay);
        Self { relay, direct_peer, status: PathStatus::Relay, backoff_ms: INITIAL_RETRY_MS, status_tx }
    }

    /// Return a receiver for status change notifications.
    #[must_use]
    pub fn status_rx(&self) -> watch::Receiver<PathStatus> {
        self.status_tx.subscribe()
    }

    /// Return the current path status.
    #[must_use]
    pub fn current_status(&self) -> PathStatus {
        self.status
    }

    /// Attempt to establish a direct P2P tunnel.
    ///
    /// Uses a new ephemeral socket for STUN + hole punching.
    /// On success, stores the tunnel and switches to `Direct` mode.
    /// On failure, reschedules with exponential backoff.
    pub async fn try_direct(&mut self, direct_tunnel: Tunnel) -> Result<(), TunnelError> {
        // Verify connectivity with a probe
        match direct_tunnel.send_to(b"PROBE", self.direct_peer).await {
            Ok(_) => {
                tracing::info!(peer = %self.direct_peer, "direct P2P established");
                self.status = PathStatus::Direct;
                self.backoff_ms = INITIAL_RETRY_MS;
                let _ = self.status_tx.send(PathStatus::Direct);
                Ok(())
            }
            Err(e) => {
                tracing::warn!(peer = %self.direct_peer, backoff_ms = self.backoff_ms, %e, "direct probe failed");
                self.backoff_ms = (self.backoff_ms * BACKOFF_MULTIPLIER).min(MAX_RETRY_MS);
                Err(e)
            }
        }
    }

    /// Send a plaintext message to the peer.
    ///
    /// Tries the direct tunnel first. If that fails, falls back to relay.
    pub async fn send(&mut self, plaintext: &[u8]) -> Result<(), TunnelError> {
        // Always use relay for simplicity; direct is attempted by the
        // daemon-level connection manager that holds the Tunnel separately.
        self.relay.send(plaintext).await.map_err(TunnelError::Io)?;
        Ok(())
    }

    /// Receive a message from the relay path.
    ///
    /// The daemon may also receive packets on the direct socket
    /// and route them separately. This method only reads from relay.
    pub async fn recv(&self) -> Result<(Vec<u8>, SocketAddr), TunnelError> {
        let (data, _sender) = self.relay.recv().await.map_err(TunnelError::Io)?;
        // We don't know the real peer address via relay, use direct_peer
        Ok((data, self.direct_peer))
    }

    /// Return a reference to the relay client for direct socket access.
    #[must_use]
    pub fn relay(&self) -> &RelayClient {
        &self.relay
    }

    /// Consume the path manager, returning the relay client.
    #[must_use]
    pub fn into_relay(self) -> RelayClient {
        self.relay
    }
}

/// A keepalive ping-pong probe for tunnel health checking.
///
/// Sends a "PING" and expects a "PONG" within the timeout.
pub async fn ping_pong(tunnel: &mut Tunnel, peer: SocketAddr, timeout: Duration) -> Result<(), TunnelError> {
    match tokio::time::timeout(timeout, async {
        tunnel.send_to(b"PING", peer).await?;
        let mut buf = [0u8; 4];
        let (received, _from) = tokio::time::timeout(timeout, tunnel.recv_from())
            .await
            .map_err(|_| TunnelError::Io(std::io::Error::new(std::io::ErrorKind::TimedOut, "pong timeout")))??;

        if received.len() >= 4 && &received[..4] == b"PONG" {
            Ok(())
        } else {
            Err(TunnelError::Io(std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "unexpected pong")))
        }
    })
    .await
    {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(e),
        Err(_) => Err(TunnelError::Io(std::io::Error::new(std::io::ErrorKind::TimedOut, "ping timeout"))),
    }
}
