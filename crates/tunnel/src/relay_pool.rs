use std::net::SocketAddr;
use std::time::{Duration, Instant};

/// A relay server with latency tracking.
#[derive(Debug, Clone)]
pub struct RelayServer {
    /// Server address.
    pub addr: SocketAddr,
    /// Measured latency (None = not yet measured).
    pub latency: Option<Duration>,
    /// When the latency was last measured.
    pub last_probe: Option<Instant>,
    /// Whether this relay is currently considered healthy.
    pub healthy: bool,
    /// Consecutive failures.
    pub failures: u32,
}

impl RelayServer {
    /// Create a new relay server entry.
    #[must_use]
    pub fn new(addr: SocketAddr) -> Self {
        Self { addr, latency: None, last_probe: None, healthy: true, failures: 0 }
    }
}

/// A pool of relay servers with automatic selection and failover.
pub struct RelayPool {
    servers: Vec<RelayServer>,
    /// Index of the currently active relay.
    active: usize,
}

impl RelayPool {
    /// Create a new relay pool with the given server addresses.
    ///
    /// # Panics
    /// Panics if `addrs` is empty.
    pub fn new(addrs: &[SocketAddr]) -> Self {
        assert!(!addrs.is_empty(), "at least one relay server required");
        Self { servers: addrs.iter().map(|a| RelayServer::new(*a)).collect(), active: 0 }
    }

    /// Return the currently active relay address.
    #[must_use]
    pub fn active_addr(&self) -> SocketAddr {
        self.servers[self.active].addr
    }

    /// Return the number of relay servers in the pool.
    #[must_use]
    pub fn len(&self) -> usize {
        self.servers.len()
    }

    /// Probe all relay servers and update latency measurements.
    ///
    /// Sends a small UDP packet to each server and measures RTT.
    /// Updates `latency`, `last_probe`, and `healthy` fields.
    pub async fn probe_all(&mut self) {
        for server in &mut self.servers {
            let start = Instant::now();

            match tokio::net::UdpSocket::bind("0.0.0.0:0").await {
                Ok(sock) => {
                    // Send a small probe (1 byte) and wait for echo
                    let _ = sock.send_to(b"P", server.addr).await;
                    let mut buf = [0u8; 1];

                    match tokio::time::timeout(Duration::from_secs(2), sock.recv_from(&mut buf)).await {
                        Ok(Ok(_)) => {
                            server.latency = Some(start.elapsed());
                            server.healthy = true;
                            server.failures = 0;
                        }
                        _ => {
                            server.failures += 1;
                            if server.failures >= 3 {
                                server.healthy = false;
                            }
                        }
                    }
                }
                Err(_) => {
                    server.failures += 1;
                    server.healthy = false;
                }
            }
            server.last_probe = Some(start);
        }

        // Select best relay
        self.select_best();
    }

    /// Mark the active relay as failed and switch to the next healthy one.
    ///
    /// Returns `true` if a failover occurred.
    pub fn failover(&mut self) -> bool {
        let old = self.active;
        self.servers[old].failures += 1;
        if self.servers[old].failures >= 3 {
            self.servers[old].healthy = false;
        }
        self.select_best();
        self.active != old
    }

    /// Select the relay with the lowest latency among healthy servers.
    fn select_best(&mut self) {
        let mut best = None;
        let mut best_latency = Duration::MAX;

        for (i, server) in self.servers.iter().enumerate() {
            if server.healthy {
                if let Some(lat) = server.latency {
                    if lat < best_latency {
                        best_latency = lat;
                        best = Some(i);
                    }
                } else if best.is_none() {
                    // Prefer a healthy unmeasured relay over nothing
                    best = Some(i);
                }
            }
        }

        if let Some(i) = best {
            self.active = i;
        }
        // If no healthy relay, keep current active (will retry)
    }

    /// Return a summary of all relays for diagnostics.
    #[must_use]
    pub fn summary(&self) -> Vec<RelaySummary> {
        self.servers
            .iter()
            .enumerate()
            .map(|(i, s)| RelaySummary {
                addr: s.addr,
                latency_ms: s.latency.map(|d| d.as_millis() as u64),
                healthy: s.healthy,
                active: i == self.active,
            })
            .collect()
    }
}

/// Summary of a relay server for diagnostics display.
#[derive(Debug)]
pub struct RelaySummary {
    /// Server address.
    pub addr: SocketAddr,
    /// Measured latency in milliseconds, if known.
    pub latency_ms: Option<u64>,
    /// Whether the relay is currently considered healthy.
    pub healthy: bool,
    /// Whether this relay is the active (selected) one.
    pub active: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn relay_pool_selection() {
        let _ = tracing_subscriber::fmt().try_init();

        let servers = vec!["127.0.0.1:33478".parse().unwrap(), "127.0.0.1:33479".parse().unwrap()];
        let mut pool = RelayPool::new(&servers);

        assert_eq!(pool.len(), 2);
        assert_eq!(pool.active_addr(), servers[0]);

        // Probe — should mark unreachable servers as unhealthy
        pool.probe_all().await;
        // Both should be unhealthy (no relay listening on localhost ports)
        assert!(!pool.servers[0].healthy);
        assert!(!pool.servers[1].healthy);

        // Failover should cycle
        pool.failover();
        // Still at 0 if no healthy relay exists
        assert_eq!(pool.active, 0);
    }

    #[test]
    fn relay_summary() {
        let servers = vec!["127.0.0.1:33478".parse().unwrap()];
        let pool = RelayPool::new(&servers);
        let summary = pool.summary();
        assert_eq!(summary.len(), 1);
        assert_eq!(summary[0].addr, servers[0]);
        assert!(summary[0].healthy);
        assert!(summary[0].active);
    }
}
