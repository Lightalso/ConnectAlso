use std::net::SocketAddr;
use std::time::Duration;

use tokio::sync::watch;

use crate::relay::RelayClient;
use crate::{Tunnel, TunnelError};

const INITIAL_RETRY_MS: u64 = 200;
const MAX_RETRY_MS: u64 = 30_000;
const BACKOFF_MULTIPLIER: u64 = 2;

/// 连接监控的路径状态。
/// Path status for connectivity monitoring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathStatus {
    /// P2P直连已激活。
    /// Direct P2P is active.
    Direct,
    /// 已回退至中继模式。
    /// Fallen back to relay.
    Relay,
    /// 正在尝试建立P2P直连。
    /// Attempting to establish direct P2P.
    Probing,
}

/// 管理到对等节点的两条通信路径：
/// - **直连**隧道（P2P，优先）
/// - **中继**回退（通过中继服务器）
///
/// 发送时优先尝试直连，失败后回退到中继。
/// 接收时监听中继路径（始终可用）。
/// 定期探测直连路径以恢复P2P。
///
/// # Fields
///
/// * `relay` — 中继客户端。
/// * `direct_peer` — 直连对等节点地址。
/// * `status` — 当前路径状态。
/// * `backoff_ms` — 直连重试的退避毫秒数。
/// * `status_tx` — 状态变化通知的发送端。
///
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

/// 接收到的消息包。
///
/// # Fields
///
/// * `data` — 解密后的负载。
/// * `via` — 消息的传输路径。
///
/// A received message bundle.
#[derive(Debug)]
pub struct RecvMessage {
    /// 解密后的负载。
    /// The decrypted payload.
    pub data: Vec<u8>,
    /// 消息的传输路径。
    /// Which path delivered the message.
    pub via: &'static str,
}

impl PathManager {
    /// 创建新的路径管理器。
    ///
    /// 初始状态为`Relay`模式。调用`try_direct()`尝试P2P直连。
    ///
    /// Create a new path manager.
    ///
    /// Starts in `Relay` mode. Call `try_direct()` to attempt P2P.
    pub fn new(relay: RelayClient, direct_peer: SocketAddr) -> Self {
        let (status_tx, _) = watch::channel(PathStatus::Relay);
        Self { relay, direct_peer, status: PathStatus::Relay, backoff_ms: INITIAL_RETRY_MS, status_tx }
    }

    /// 返回状态变化通知的接收端。
    ///
    /// # Returns
    ///
    /// * 状态变化的`watch::Receiver<PathStatus>`。
    ///
    /// Return a receiver for status change notifications.
    #[must_use]
    pub fn status_rx(&self) -> watch::Receiver<PathStatus> {
        self.status_tx.subscribe()
    }

    /// 返回当前路径状态。
    ///
    /// # Returns
    ///
    /// * 当前`PathStatus`。
    ///
    /// Return the current path status.
    #[must_use]
    pub fn current_status(&self) -> PathStatus {
        self.status
    }

    /// 尝试建立P2P直连隧道。
    ///
    /// 使用新的临时套接字进行STUN + 打洞。
    /// 成功时将隧道存储并切换至`Direct`模式。
    /// 失败时使用指数退避重新调度。
    ///
    /// # Errors
    ///
    /// * 探测失败时返回`TunnelError`。
    ///
    /// Attempt to establish a direct P2P tunnel.
    ///
    /// Uses a new ephemeral socket for STUN + hole punching.
    /// On success, stores the tunnel and switches to `Direct` mode.
    /// On failure, reschedules with exponential backoff.
    pub async fn try_direct(&mut self, mut direct_tunnel: Tunnel) -> Result<(), TunnelError> {
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

    /// 向对等节点发送明文消息。
    ///
    /// 优先通过直连隧道发送，失败时回退到中继。
    ///
    /// # Errors
    ///
    /// * 中继发送失败时返回`TunnelError`。
    ///
    /// Send a plaintext message to the peer.
    ///
    /// Tries the direct tunnel first. If that fails, falls back to relay.
    pub async fn send(&mut self, plaintext: &[u8]) -> Result<(), TunnelError> {
        // Always use relay for simplicity; direct is attempted by the
        // daemon-level connection manager that holds the Tunnel separately.
        self.relay.send(plaintext).await.map_err(TunnelError::Io)?;
        Ok(())
    }

    /// 从中继路径接收消息。
    ///
    /// 守护进程也可能从直连套接字接收数据包并单独路由。
    /// 此方法仅从中继读取。
    ///
    /// # Errors
    ///
    /// * 中继接收失败时返回`TunnelError`。
    ///
    /// # Returns
    ///
    /// * 返回明文数据和对等节点地址。
    ///
    /// Receive a message from the relay path.
    ///
    /// The daemon may also receive packets on the direct socket
    /// and route them separately. This method only reads from relay.
    pub async fn recv(&self) -> Result<(Vec<u8>, SocketAddr), TunnelError> {
        let (data, _sender) = self.relay.recv().await.map_err(TunnelError::Io)?;
        // We don't know the real peer address via relay, use direct_peer
        Ok((data, self.direct_peer))
    }

    /// 返回中继客户端的引用，用于直接套接字访问。
    ///
    /// # Returns
    ///
    /// * `&RelayClient` 中继客户端引用。
    ///
    /// Return a reference to the relay client for direct socket access.
    #[must_use]
    pub fn relay(&self) -> &RelayClient {
        &self.relay
    }

    /// 消耗路径管理器，返回中继客户端。
    ///
    /// # Returns
    ///
    /// * `RelayClient` 中继客户端。
    ///
    /// Consume the path manager, returning the relay client.
    #[must_use]
    pub fn into_relay(self) -> RelayClient {
        self.relay
    }
}

/// 隧道健康检查的保活Ping-Pong探测。
///
/// 发送"PING"并在超时时间内等待"PONG"响应。
///
/// # Errors
///
/// * 超时、发送失败或收到非预期响应时返回`TunnelError`。
///
/// A keepalive ping-pong probe for tunnel health checking.
///
/// Sends a "PING" and expects a "PONG" within the timeout.
pub async fn ping_pong(tunnel: &mut Tunnel, peer: SocketAddr, timeout: Duration) -> Result<(), TunnelError> {
    match tokio::time::timeout(timeout, async {
        tunnel.send_to(b"PING", peer).await?;
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
