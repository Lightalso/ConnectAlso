package com.connectalso.mobile

/**
 * JNI bridge to the Rust native library (libconnectalso_mobile.so).
 *
 * The native methods are implemented in apps/mobile/src/android.rs
 * and compiled into the shared library.
 *
 * ## Loading the library
 *
 * The library must be loaded before using any methods:
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
     *
     * @param controlUrl  Control service URL
     * @param relayServer Relay server address (host:port)
     * @param hostname    Device name
     * @return true on success
     */
    external fun init(controlUrl: String, relayServer: String, hostname: String): Boolean

    /**
     * Send an outgoing IP packet through the tunnel.
     *
     * @param packet Raw IP packet bytes from TUN
     * @return Result bytes, or null on error
     */
    external fun sendPacket(packet: ByteArray): ByteArray?

    /**
     * Poll for an incoming IP packet from the tunnel.
     *
     * @return Decrypted IP packet bytes, or null if no data
     */
    external fun recvPacket(): ByteArray?

    /**
     * Get the virtual IP address assigned by the control service.
     *
     * @return IPv4 address string, or null
     */
    external fun getVirtualIP(): String?

    /**
     * Protect a UDP socket from VPN routing.
     *
     * Call VpnService.protect() on the socket, then register the fd.
     *
     * @param fd Socket file descriptor
     */
    external fun protectSocket(fd: Int)

    /**
     * Shut down the engine and release resources.
     */
    external fun shutdown()
}
