use std::collections::HashMap;
use std::net::Ipv4Addr;

use tokio::net::UdpSocket;

/// 一个微型 DNS 服务器，将 ConnectAlso 主机名解析为虚拟 IP 地址。
/// A minimal DNS server that resolves ConnectAlso hostnames to virtual IPs.
///
/// 监听本地 UDP 端口，响应 `<name>.connectalso` 或裸 `<name>` 的 A 记录查询。
/// Listens on a local UDP port and responds to A-record queries for
/// hostnames matching `<name>.connectalso` or bare `<name>`.
#[allow(dead_code)]
pub struct DnsServer {
    socket: UdpSocket,
    records: HashMap<String, Ipv4Addr>,
    upstream: String,
}

#[allow(dead_code)]
impl DnsServer {
    /// 创建并绑定一个新的 DNS 服务器。
    /// Create a new DNS server bound to `listen_addr`.
    ///
    /// # Errors
    ///
    /// 如果无法绑定到指定地址则返回 I/O 错误。
    /// Returns an I/O error if the address cannot be bound.
    pub async fn bind(listen_addr: &str, upstream: &str) -> Result<Self, std::io::Error> {
        let socket = UdpSocket::bind(listen_addr).await?;
        tracing::info!(%listen_addr, %upstream, "DNS server started");
        Ok(Self { socket, records: HashMap::new(), upstream: upstream.to_string() })
    }

    /// 根据当前对等节点列表更新 DNS 记录。
    /// Update the DNS records from the current peer list.
    pub fn update_records(&mut self, hosts: &[(String, Ipv4Addr)]) {
        self.records.clear();
        for (name, ip) in hosts {
            self.records.insert(name.to_lowercase(), *ip);
        }
    }

    /// 运行 DNS 服务器循环，永不返回。
    /// Run the DNS server loop. Never returns.
    pub async fn serve(self) {
        let mut buf = [0u8; 512];
        loop {
            let (n, src) = match self.socket.recv_from(&mut buf).await {
                Ok(r) => r,
                Err(_) => continue,
            };

            let query = &buf[..n];
            if let Some(response) = self.handle_query(query) {
                let _ = self.socket.send_to(&response, src).await;
            }
        }
    }

    /// 解析 DNS 查询并构建 A 记录响应。
    /// Parse a DNS query and build an A-record response.
    ///
    /// # Returns
    ///
    /// 如果查询名称匹配已知主机则返回 DNS 响应包，否则返回 `None`。
    /// Returns the DNS response packet if the queried name matches a known host, otherwise `None`.
    fn handle_query(&self, query: &[u8]) -> Option<Vec<u8>> {
        if query.len() < 12 {
            return None;
        }

        // Extract transaction ID (2 bytes) and question count
        let tx_id = [query[0], query[1]];
        let qdcount = u16::from_be_bytes([query[4], query[5]]);
        if qdcount == 0 {
            return None;
        }

        // Parse the question name (simple label parsing)
        let mut pos = 12;
        let mut name_parts = Vec::new();
        while pos < query.len() {
            let len = query[pos] as usize;
            if len == 0 {
                pos += 1;
                break;
            }
            if len & 0xC0 != 0 {
                pos += 2;
                break;
            } // Compressed name — skip
            pos += 1;
            if pos + len > query.len() {
                break;
            }
            name_parts.push(std::str::from_utf8(&query[pos..pos + len]).ok()?.to_lowercase());
            pos += len;
        }
        let name = name_parts.join(".");

        // Skip QTYPE (2) + QCLASS (2)
        if pos + 4 > query.len() {
            return None;
        }

        // Look up the name
        // Strip ".connectalso" suffix if present
        let lookup = name.strip_suffix(".connectalso").unwrap_or(&name);
        let ip = self.records.get(lookup)?;

        // Build DNS response
        let mut resp = Vec::with_capacity(query.len() + 16);
        resp.extend_from_slice(&tx_id);
        resp.extend_from_slice(&[
            0x81, 0x80, // Flags: standard response, no error
            0x00, 0x01, // Questions: 1
            0x00, 0x01, // Answers: 1
            0x00, 0x00, // Authority: 0
            0x00, 0x00, // Additional: 0
        ]);
        // Echo the question section
        resp.extend_from_slice(&query[12..pos + 4]);
        // Answer section: pointer to name + A record
        resp.extend_from_slice(&[
            0xC0, 0x0C, // Name pointer to offset 12
            0x00, 0x01, // Type: A
            0x00, 0x01, // Class: IN
            0x00, 0x00, 0x00, 60, // TTL: 60 seconds
            0x00, 0x04, // RDLENGTH: 4
        ]);
        resp.extend_from_slice(&ip.octets());

        Some(resp)
    }
}
