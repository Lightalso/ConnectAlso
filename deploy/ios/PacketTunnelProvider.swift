import NetworkExtension
import Network
import os.log

/// ConnectAlso Packet Tunnel Provider for iOS
///
/// This class implements the NEPacketTunnelProvider protocol to provide
/// a virtual network interface that tunnels all traffic through ConnectAlso.
///
/// ## Setup in Xcode
///
/// 1. Add `libconnectalso_mobile.a` to your NetworkExtension target
/// 2. Add the bridging header with C function declarations
/// 3. Set "Packet Tunnel" capability in your app target
/// 4. Configure the NetworkExtension in your app's Info.plist
///
/// ## Bridging Header (ConnectAlso-Bridging-Header.h):
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

    // ── Tunnel lifecycle ──

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

    override func stopTunnel(with reason: NEProviderStopReason,
                              completionHandler: @escaping () -> Void) {
        os_log("Tunnel stopping...", log: log, type: .info)
        readLoopActive = false
        connectalso_shutdown()
        completionHandler()
    }

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

    // ── Packet forwarding loop ──

    private func startPacketLoop() {
        readLoopActive = true

        // Outbound: TUN → Rust engine → network
        readOutboundPackets()

        // Inbound: Rust engine → TUN
        readInboundPackets()
    }

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

    private func readInboundPackets() {
        DispatchQueue.global(qos: .default).async { [weak self] in
            guard let self = self else { return }

            var outBuf = [UInt8](repeating: 0, count: 65536)

            while self.readLoopActive {
                let received = connectalso_recv_packet(&outBuf, UInt32(outBuf.count))

                if received > 0 {
                    let data = Data(bytes: outBuf, count: Int(received))
                    self.packetFlow.writePackets([data], withProtocols: [NSNumber(value: AF_INET)])
                } else {
                    // No data — brief sleep to avoid busy-waiting
                    Thread.sleep(forTimeInterval: 0.01)
                }
            }
        }
    }

    // ── Helpers ──

    private func getVirtualIP() -> String {
        var buf = [CChar](repeating: 0, count: 64)
        let len = connectalso_get_virtual_ip(&buf, UInt32(buf.count))
        if len > 0 {
            return String(cString: buf)
        }
        return "100.64.0.1"
    }

    // ── Network path monitoring (Wi-Fi ↔ Cellular) ──

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
