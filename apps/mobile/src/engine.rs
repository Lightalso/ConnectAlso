use std::sync::Mutex as StdMutex;

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

use anyhow::Context;
use connectalso_crypto::key_exchange::KeyPair;
use connectalso_relay_proto::PeerId;
use connectalso_tunnel::relay::RelayClient;
use serde::Deserialize;
use tokio::runtime::Runtime;
use tokio::sync::Mutex;
use uuid::Uuid;

pub(crate) static RUNTIME: std::sync::OnceLock<Runtime> = std::sync::OnceLock::new();

/// Persistent state for the mobile tunnel engine.
struct TunnelEngine {
    keypair: KeyPair,
    our_id: Uuid,
    our_ip: Ipv4Addr,
    relay_server: SocketAddr,
    peers: HashMap<Uuid, PeerState>,
    http: reqwest::Client,
    control_url: String,
}

struct PeerState {
    vip: Ipv4Addr,
    relay: Mutex<RelayClient>,
}

#[derive(Debug, Deserialize)]
struct RegisterResponse {
    device_id: Uuid,
    ipv4: String,
    #[serde(default)]
    status: String,
}

#[derive(Debug, Deserialize)]
struct PeerInfo {
    device_id: Uuid,
    ipv4: String,
    public_key: [u8; 32],
    hostname: String,
}

#[derive(Debug, Deserialize)]
struct PeersResponse {
    peers: Vec<PeerInfo>,
}

impl TunnelEngine {
    async fn new(control_url: &str, relay_server: SocketAddr, hostname: &str) -> anyhow::Result<Self> {
        let keypair = KeyPair::generate();
        let pubkey = keypair.public_key_bytes();
        let http = reqwest::Client::new();

        let reg: RegisterResponse = http
            .post(format!("{control_url}/api/v1/register"))
            .json(&serde_json::json!({
                "public_key": pubkey,
                "hostname": hostname,
            }))
            .send()
            .await?
            .json()
            .await?;

        let our_ip: Ipv4Addr = reg.ipv4.parse()?;
        tracing::info!(id = %reg.device_id, ip = %our_ip, "mobile registered");

        if reg.status == "pending" {
            tracing::warn!("Device pending approval — contact admin");
        }

        Ok(Self {
            keypair,
            our_id: reg.device_id,
            our_ip,
            relay_server,
            peers: HashMap::new(),
            http,
            control_url: control_url.to_string(),
        })
    }

    async fn sync_peers(&mut self) -> anyhow::Result<usize> {
        let our_relay_id = PeerId::from_bytes(self.our_id.into_bytes());

        let peers: PeersResponse = self
            .http
            .get(format!("{}/api/v1/peers", self.control_url))
            .send()
            .await?
            .json()
            .await?;

        let mut count = 0;
        for p in peers.peers.into_iter().filter(|p| p.device_id != self.our_id) {
            if self.peers.contains_key(&p.device_id) {
                continue;
            }
            let vip: Ipv4Addr = p.ipv4.parse()?;
            let peer_relay_id = PeerId::from_bytes(p.device_id.into_bytes());

            match RelayClient::register(
                "0.0.0.0:0".parse()?,
                self.relay_server,
                our_relay_id,
                peer_relay_id,
            )
            .await
            {
                Ok(relay) => {
                    self.peers.insert(p.device_id, PeerState {
                        vip,
                        relay: Mutex::new(relay),
                    });
                    count += 1;
                }
                Err(e) => tracing::warn!(peer = %p.hostname, %e, "relay failed"),
            }
        }
        Ok(count)
    }

    async fn send_to_peer(&self, packet: &[u8], dst_ip: Ipv4Addr) -> anyhow::Result<()> {
        for peer in self.peers.values() {
            if peer.vip == dst_ip {
                let relay = peer.relay.lock().await;
                relay.send(packet).await?;
                return Ok(());
            }
        }
        anyhow::bail!("no route to {dst_ip}")
    }

    async fn recv_from_any(&self) -> anyhow::Result<Vec<u8>> {
        // Poll all peers for incoming data
        for peer in self.peers.values() {
            let relay = peer.relay.lock().await;
            match tokio::time::timeout(
                std::time::Duration::from_millis(10),
                relay.recv(),
            )
            .await
            {
                Ok(Ok((data, _sender))) => return Ok(data),
                _ => continue,
            }
        }
        anyhow::bail!("no data")
    }
}

/// Global engine instance, initialized once per tunnel session.
pub(crate) static ENGINE: StdMutex<Option<Arc<Mutex<TunnelEngine>>>> = StdMutex::new(None);

/// Initialize the mobile tunnel engine.
pub(crate) async fn engine_init(
    control_url: &str,
    relay_server: SocketAddr,
    hostname: &str,
) -> anyhow::Result<()> {
    let mut engine = TunnelEngine::new(control_url, relay_server, hostname).await?;
    engine.sync_peers().await?;
    let arc = Arc::new(Mutex::new(engine));
    *ENGINE.lock().unwrap() = Some(arc);
    Ok(())
}

/// Send an IP packet from the TUN interface to the tunnel network.
pub(crate) async fn engine_send_packet(packet: &[u8]) -> anyhow::Result<Vec<u8>> {
    let guard = ENGINE.lock().unwrap();
    let engine = guard.as_ref().context("engine not initialized")?;
    let e = engine.lock().await;

    let dst = parse_dst_ip(packet).context("invalid packet")?;
    if dst == e.our_ip {
        return Ok(packet.to_vec());
    }

    e.send_to_peer(packet, dst).await?;
    Ok(Vec::new())
}

/// Receive a packet from the tunnel network destined for the TUN interface.
pub(crate) async fn engine_recv_packet() -> anyhow::Result<Vec<u8>> {
    let guard = ENGINE.lock().unwrap();
    let engine = guard.as_ref().context("engine not initialized")?;
    let e = engine.lock().await;
    e.recv_from_any().await
}

fn parse_dst_ip(packet: &[u8]) -> Option<Ipv4Addr> {
    if packet.len() < 20 || packet[0] >> 4 != 4 {
        return None;
    }
    Some(Ipv4Addr::new(packet[16], packet[17], packet[18], packet[19]))
}
