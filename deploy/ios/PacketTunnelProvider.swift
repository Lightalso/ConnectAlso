import NetworkExtension
import Network
import os.log

/// ConnectAlso Packet Tunnel Provider for iOS.
/// iOS 平台的 ConnectAlso 数据包隧道提供者。
///
/// This class implements the NEPacketTunnelProvider protocol to provide
/// a virtual network interface that tunnels all traffic through ConnectAlso.
/// 此类实现 NEPacketTunnelProvider 协议，提供虚拟网络接口，
/// 将所有流量通过 ConnectAlso 隧道传输。
///
/// ## Setup in Xcode / Xcode 设置
///
/// 1. Add `libconnectalso_mobile.a` to your NetworkExtension target
///    将 `libconnectalso_mobile.a` 添加到 NetworkExtension target
/// 2. Add the bridging header with C function declarations
///    添加包含 C 函数声明的桥接头文件
/// 3. Set "Packet Tunnel" capability in your app target
///    在应用 target 中启用 "Packet Tunnel" 能力
/// 4. Configure the NetworkExtension in your app's Info.plist
///    在应用的 Info.plist 中配置 NetworkExtension
///
/// ## Bridging Header / 桥接头文件 (ConnectAlso-Bridging-Header.h):
/// ```c
/// int32_t connectalso_init(const char *control_url, const char *stun_server,
///                          const char *relay_server, const char *hostname);
/// int32_t connectalso_send_packet(const uint8_t *packet, uint32_t len,
///                                  uint8_t *out, uint32_t out_len);
/// int32_t connectalso_recv_packet(uint8_t *out, uint32_t out_len);
/// int32_t connectalso_get_virtual_ip(char *out, uint32_t out_len);
/// void    connectalso_shutdown(void);
/// ```

class PacketTunnelProvider: NEPacketTunnelProvider {

    private let log = OSLog(subsystem: "com.connectalso.ios", category: "tunnel")
    private var readLoopActive = false
    private var pathMonitor: NWPathMonitor?
    private let monitorQueue = DispatchQueue(label: "com.connectalso.network-monitor")

    // ── Configuration (set by the container app via protocolConfiguration) ──

    private var controlURL: String = "http://127.0.0.1:3000"
    private var relayServer: String = "127.0.0.1:33478"
    private var hostname: String = "ios-device"

    // ── Tunnel lifecycle / 隧道生命周期 ──

    /// Start the packet tunnel.
    /// 启动数据包隧道。
    ///
    /// Initializes the Rust engine, configures TUN network settings, routes all
    /// traffic through the tunnel, and starts network monitoring and packet loops.
    /// 初始化 Rust 引擎，配置 TUN 网络设置，将所有流量路由到隧道，
    /// 并启动网络监控和数据包循环。
    override func startTunnel(options: [String: NSObject]?,
                              completionHandler: @escaping (Error?) -> Void) {
        os_log("ConnectAlso tunnel starting...", log: log, type: .info)

        // Extract configuration from protocolConfiguration
        if let config = protocolConfiguration as? NETunnelProviderProtocol {
            controlURL = config.providerConfiguration?["control_url"] as? String ?? controlURL
            relayServer = config.providerConfiguration?["relay_server"] as? String ?? relayServer
            hostname = config.providerConfiguration?["hostname"] as? String ?? hostname
        }

        // Initialize the Rust engine
        let result = connectalso_init(
            (controlURL as NSString).utf8String,
            "", // stun_server — not used on iOS, relay always available
            (relayServer as NSString).utf8String,
            (hostname as NSString).utf8String
        )

        guard result == 0 else {
            os_log("Engine init failed: %d", log: log, type: .error, result)
            completionHandler(NSError(domain: "ConnectAlso", code: -1))
            return
        }

        // Configure TUN network settings
        let vip = getVirtualIP()
        let networkSettings = NEPacketTunnelNetworkSettings(tunnelRemoteAddress: vip)
        networkSettings.ipv4Settings = NEIPv4Settings(
            addresses: [vip],
            subnetMasks: ["255.255.255.0"]
        )
        networkSettings.mtu = 1500

        // Route all traffic through the tunnel
        networkSettings.ipv4Settings?.includedRoutes = [NEIPv4Route.default()]

        setTunnelNetworkSettings(networkSettings) { [weak self] error in
            if let error = error {
                os_log("Network settings failed: %{public}@", log: self?.log ?? .default, type: .error, error.localizedDescription)
                completionHandler(error)
                return
            }

            os_log("Tunnel configured (VIP: %{public}@)", log: self?.log ?? .default, type: .info, vip)
            self?.startNetworkMonitor()
            self?.startPacketLoop()
            completionHandler(nil)
        }
    }

    /// Stop the packet tunnel.
    /// 停止数据包隧道。
    ///
    /// Stops the packet read loop and shuts down the Rust engine.
    /// 停止数据包读取循环并关闭 Rust 引擎。
    override func stopTunnel(with reason: NEProviderStopReason,
                              completionHandler: @escaping () -> Void) {
        os_log("Tunnel stopping...", log: log, type: .info)
        readLoopActive = false
        connectalso_shutdown()
        completionHandler()
    }

    /// Handle IPC messages from the container app.
    /// 处理来自容器应用的进程间通信消息。
    ///
    /// Supports "status" query to return the current virtual IP address.
    /// 支持 "status" 查询以返回当前虚拟 IP 地址。
    override func handleAppMessage(_ messageData: Data,
                                    completionHandler: ((Data?) -> Void)?) {
        // Handle messages from the container app
        if let msg = String(data: messageData, encoding: .utf8) {
            os_log("App message: %{public}@", log: log, type: .debug, msg)

            if msg == "status" {
                let vip = getVirtualIP()
                let response = "{\"virtual_ip\":\"\(vip)\"}"
                completionHandler?(response.data(using: .utf8))
                return
            }
        }
        completionHandler?(nil)
    }

    // ── Packet forwarding loop / 数据包转发循环 ──

    /// Start the bidirectional packet forwarding loops.
    /// 启动双向数据包转发循环。
    private func startPacketLoop() {
        readLoopActive = true
        readOutboundPackets()
        readInboundPacketsAdaptive()
    }

    /// Read outbound packets from TUN and forward them to the Rust engine.
    /// 从 TUN 读取出站数据包并转发到 Rust 引擎。
    ///
    /// Continuously reads packets using packetFlow.readPackets, sends each
    /// through the Rust engine, and writes locally-delivered packets back to TUN.
    /// 持续使用 packetFlow.readPackets 读取数据包，通过 Rust 引擎发送，
    /// 并将本地投递的数据包写回 TUN。
    private func readOutboundPackets() {
        packetFlow.readPackets { [weak self] packets, protocols in
            guard let self = self, self.readLoopActive else { return }

            for (index, packet) in packets.enumerated() {
                var outBuf = [UInt8](repeating: 0, count: 65536)

                let sent = packet.withUnsafeBytes { ptr in
                    let base = ptr.bindMemory(to: UInt8.self).baseAddress!
                    return connectalso_send_packet(base, UInt32(packet.count),
                                                    &outBuf, UInt32(outBuf.count))
                }

                if sent > 0 {
                    // Local delivery — write back to TUN
                    let localData = Data(bytes: outBuf, count: Int(sent))
                    self.packetFlow.writePackets([localData], withProtocols: [protocols[index]])
                }
            }

            // Loop
            self.readOutboundPackets()
        }
    }

    /// Poll for inbound packets from the Rust engine with adaptive timing.
    /// 以自适应时间间隔从 Rust 引擎轮询入站数据包。
    ///
    /// Uses shorter poll intervals when traffic is active and longer intervals
    /// when idle, to balance latency and battery consumption.
    /// 流量活跃时使用较短的轮询间隔，空闲时使用较长的间隔，
    /// 以平衡延迟和电池消耗。
    private func readInboundPacketsAdaptive() {
        DispatchQueue.global(qos: .default).async { [weak self] in
            guard let self = self else { return }
            var outBuf = [UInt8](repeating: 0, count: 65536)
            var idleCount = 0

            while self.readLoopActive {
                // Adaptive timeout: 10ms active, 100ms idle, 500ms sleep
                let timeout = idleCount < 50 ? 10 :
                              idleCount < 300 ? 100 : 500

                let received = connectalso_recv_packet_timeout(&outBuf, UInt32(outBuf.count), UInt32(timeout))

                if received > 0 {
                    let data = Data(bytes: outBuf, count: Int(received))
                    self.packetFlow.writePackets([data], withProtocols: [NSNumber(value: AF_INET)])
                    idleCount = 0 // Reset on traffic
                } else {
                    idleCount += 1
                }
            }
        }
    }

    // Deprecated: replaced by readInboundPacketsAdaptive
    private func readInboundPackets() {
        readInboundPacketsAdaptive()
    }

    // ── Helpers / 辅助方法 ──

    private func getVirtualIP() -> String {
        var buf = [CChar](repeating: 0, count: 64)
        let len = connectalso_get_virtual_ip(&buf, UInt32(buf.count))
        if len > 0 {
            return String(cString: buf)
        }
        return "100.64.0.1"
    }

    // ── Network path monitoring (Wi-Fi ↔ Cellular) / 网络路径监控（Wi-Fi ↔ 蜂窝网络）──

    /// Start monitoring network path changes for Wi-Fi/Cellular switching.
    /// 启动网络路径变化监控，检测 Wi-Fi 与蜂窝网络切换。
    ///
    /// Triggers a Rust engine reconnect when the active network interface changes,
    /// ensuring the P2P connections survive network transitions.
    /// 当活跃网络接口变化时触发 Rust 引擎重连，
    /// 确保 P2P 连接在网络切换后继续工作。
    private func startNetworkMonitor() {
        pathMonitor = NWPathMonitor()
        pathMonitor?.pathUpdateHandler = { [weak self] path in
            let iface = path.availableInterfaces.first?.name ?? "unknown"
            let isExpensive = path.isExpensive // true = cellular
            os_log("Network changed: %{public}@ (expensive=%{public}@)",
                   log: self?.log ?? .default, type: .info, iface, isExpensive.description)

            // Trigger Rust engine reconnect
            let result = connectalso_reconnect()
            if result == 0 {
                os_log("Reconnect successful", log: self?.log ?? .default, type: .info)
            } else {
                os_log("Reconnect failed: %d", log: self?.log ?? .default, type: .error, result)
            }
        }
        pathMonitor?.start(queue: monitorQueue)
        os_log("Network monitor started", log: log, type: .info)
    }

    private func stopNetworkMonitor() {
        pathMonitor?.cancel()
        pathMonitor = nil
    }
}
