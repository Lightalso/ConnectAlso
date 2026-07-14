use std::net::SocketAddr;

use connectalso_relay_proto::{PeerId, RelayFrame};
use tokio::net::UdpSocket;

/// 通过中继服务器进行通信的最小化客户端。
///
/// 中继客户端将加密数据封装在中继DATA帧中发送，
/// 并接收由中继服务器转发的其他对等节点的DATA帧。
///
/// # Fields
///
/// * `socket` — 与中继服务器通信的UDP套接字。
/// * `relay_addr` — 中继服务器地址。
/// * `our_id` — 本节点ID。
/// * `peer_id` — 目标对等节点ID。
///
/// A minimal client for communicating through a relay server.
///
/// The relay client sends encrypted data wrapped in relay DATA frames
/// and receives DATA frames from other peers forwarded by the relay.
pub struct RelayClient {
    socket: UdpSocket,
    relay_addr: SocketAddr,
    our_id: PeerId,
    peer_id: PeerId,
}

impl RelayClient {
    /// 绑定UDP套接字并通过HELLO帧向中继服务器注册。
    ///
    /// # Errors
    ///
    /// * UDP绑定或发送HELLO帧失败时返回I/O错误。
    ///
    /// # Returns
    ///
    /// * 返回已注册的`RelayClient`实例。
    ///
    /// Bind a UDP socket and register with the relay server via a HELLO.
    pub async fn register(
        local_addr: SocketAddr,
        relay_addr: SocketAddr,
        our_id: PeerId,
        peer_id: PeerId,
    ) -> Result<Self, std::io::Error> {
        let socket = UdpSocket::bind(local_addr).await?;

        let hello = RelayFrame::hello(our_id);
        let encoded = hello.encode().expect("hello fits");
        socket.send_to(&encoded, relay_addr).await?;
        tracing::info!(%our_id, %relay_addr, "registered with relay");

        Ok(Self { socket, relay_addr, our_id, peer_id })
    }

    /// 通过中继向目标对等节点发送加密数据。
    ///
    /// # Errors
    ///
    /// * 编码或发送失败时返回I/O错误。
    ///
    /// # Returns
    ///
    /// * 返回发送的字节数。
    ///
    /// Send encrypted data to the target peer via the relay.
    pub async fn send(&self, encrypted_data: &[u8]) -> Result<usize, std::io::Error> {
        let frame = RelayFrame::data(self.our_id, self.peer_id, encrypted_data.to_vec());
        let encoded = frame.encode().expect("payload fits");
        self.socket.send_to(&encoded, self.relay_addr).await
    }

    /// 接收来自远程对等节点的中继DATA帧。
    ///
    /// # Errors
    ///
    /// * 接收或解码失败时返回I/O错误。
    ///
    /// # Returns
    ///
    /// * 返回负载数据（加密的隧道数据包）和发送者ID。
    ///
    /// Receive a relayed DATA frame from the remote peer.
    /// Returns the payload (encrypted tunnel packet) and the sender's ID.
    pub async fn recv(&self) -> Result<(Vec<u8>, PeerId), std::io::Error> {
        loop {
            let mut buf = [0u8; 4096];
            let (n, _from) = self.socket.recv_from(&mut buf).await?;

            if let Ok(frame) = RelayFrame::decode(&buf[..n]) {
                if frame.msg_type == connectalso_relay_proto::MsgType::Data && frame.sender_id == self.peer_id {
                    return Ok((frame.payload, frame.sender_id));
                }
            }
        }
    }

    /// 向中继服务器发送保活包以维持注册状态。
    ///
    /// # Errors
    ///
    /// * 编码或发送保活帧失败时返回I/O错误。
    ///
    /// Send a keepalive to the relay to maintain the registration.
    pub async fn keepalive(&self) -> Result<(), std::io::Error> {
        let frame = RelayFrame::keepalive(self.our_id);
        let encoded = frame.encode().expect("keepalive fits");
        self.socket.send_to(&encoded, self.relay_addr).await?;
        Ok(())
    }
}
