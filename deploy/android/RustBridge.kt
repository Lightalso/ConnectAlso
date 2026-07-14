package com.connectalso.mobile

/**
 * JNI bridge to the Rust native library (libconnectalso_mobile.so).
 * 连接 Rust 原生库（libconnectalso_mobile.so）的 JNI 桥接类。
 *
 * The native methods are implemented in apps/mobile/src/android.rs
 * and compiled into the shared library.
 * 原生方法实现在 apps/mobile/src/android.rs，并编译为共享库。
 *
 * ## Loading the library / 加载库
 *
 * The library must be loaded before using any methods:
 * 使用任何方法前必须先加载库：
 * ```kotlin
 * class MainActivity : AppCompatActivity() {
 *     init {
 *         System.loadLibrary("connectalso_mobile")
 *     }
 * }
 * ```
 */
object RustBridge {

    init {
        System.loadLibrary("connectalso_mobile")
    }

    /**
     * Initialize the ConnectAlso engine.
     * 初始化 ConnectAlso 引擎。
     *
     * @param controlUrl  Control service URL / 控制服务地址
     * @param relayServer Relay server address (host:port) / 中继服务器地址
     * @param hostname    Device name / 设备名称
     * @return true on success / 成功返回 true
     */
    external fun init(controlUrl: String, relayServer: String, hostname: String): Boolean

    /**
     * Send an outgoing IP packet through the tunnel.
     * 通过隧道发送出站 IP 数据包。
     *
     * @param packet Raw IP packet bytes from TUN / 来自 TUN 的原始 IP 包字节
     * @return Result bytes, or null on error / 返回结果字节，出错返回 null
     */
    external fun sendPacket(packet: ByteArray): ByteArray?

    /**
     * Poll for an incoming IP packet from the tunnel.
     * 从隧道轮询入站 IP 数据包。
     *
     * @return Decrypted IP packet bytes, or null if no data
     *         解密后的 IP 包字节，无数据时返回 null
     */
    external fun recvPacket(): ByteArray?

    /**
     * Get the virtual IP address assigned by the control service.
     * 获取控制服务分配的虚拟 IP 地址。
     *
     * @return IPv4 address string, or null / IPv4 地址字符串，失败返回 null
     */
    external fun getVirtualIP(): String?

    /**
     * Protect a UDP socket from VPN routing.
     * 保护 UDP 套接字免于 VPN 路由。
     *
     * Call VpnService.protect() on the socket, then register the fd.
     * 先在套接字上调用 VpnService.protect()，然后注册文件描述符。
     *
     * @param fd Socket file descriptor / 套接字文件描述符
     */
    external fun protectSocket(fd: Int)

    /**
     * Shut down the engine and release resources.
     * 关闭引擎并释放资源。
     */
    external fun shutdown()

    /**
     * Trigger reconnection after a network change (Wi-Fi ↔ Cellular).
     * 网络变化后触发重连（Wi-Fi ↔ 蜂窝网络切换）。
     *
     * @return true on success / 成功返回 true
     */
    external fun reconnect(): Boolean
}
