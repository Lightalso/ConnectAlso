package com.connectalso.mobile

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Intent
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.net.VpnService
import android.os.ParcelFileDescriptor
import android.os.PowerManager
import android.util.Log
import java.io.FileInputStream
import java.io.FileOutputStream
import java.net.DatagramSocket
import kotlin.concurrent.thread

/**
 * ConnectAlso VpnService for Android.
 * Android 平台的 ConnectAlso VPN 服务。
 *
 * Creates a TUN interface via VpnService.Builder and forwards packets
 * through the Rust engine (libconnectalso_mobile.so via JNI).
 * 通过 VpnService.Builder 创建 TUN 虚拟网卡，并通过 Rust 引擎
 * （libconnectalso_mobile.so，JNI 调用）转发数据包。
 *
 * ## Setup in AndroidManifest.xml / AndroidManifest.xml 配置
 *
 * ```xml
 * <service
 *     android:name=".ConnectAlsoVpnService"
 *     android:permission="android.permission.BIND_VPN_SERVICE"
 *     android:exported="false">
 *     <intent-filter>
 *         <action android:name="android.net.VpnService" />
 *     </intent-filter>
 * </service>
 * ```
 *
 * ## Required permissions / 所需权限
 *
 * ```xml
 * <uses-permission android:name="android.permission.INTERNET" />
 * <uses-permission android:name="android.permission.FOREGROUND_SERVICE" />
 * <uses-permission android:name="android.permission.FOREGROUND_SERVICE_SPECIAL_USE" />
 * <uses-permission android:name="android.permission.POST_NOTIFICATIONS" />
 * ```
 */
class ConnectAlsoVpnService : VpnService() {

    companion object {
        private const val TAG = "ConnectAlsoVPN"
        private const val CHANNEL_ID = "connectalso_vpn"
        private const val NOTIFICATION_ID = 1
        private const val VPN_MTU = 1500
        private const val VPN_ADDRESS = "100.64.0.1"
        private const val VPN_ROUTE = "0.0.0.0/0"

        // Action for the main activity to start/stop the VPN
        const val ACTION_START = "com.connectalso.mobile.START"
        const val ACTION_STOP = "com.connectalso.mobile.STOP"
        const val EXTRA_CONTROL_URL = "control_url"
        const val EXTRA_RELAY_SERVER = "relay_server"
        const val EXTRA_HOSTNAME = "hostname"
    }

    private var tunFd: ParcelFileDescriptor? = null
    private var running = false
    private var wakelock: PowerManager.WakeLock? = null
    private var connectivityManager: ConnectivityManager? = null
    private var networkCallback: ConnectivityManager.NetworkCallback? = null

    override fun onCreate() {
        super.onCreate()
        createNotificationChannel()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_START -> {
                val controlUrl = intent.getStringExtra(EXTRA_CONTROL_URL) ?: "http://10.0.2.2:3000"
                val relayServer = intent.getStringExtra(EXTRA_RELAY_SERVER) ?: "10.0.2.2:33478"
                val hostname = intent.getStringExtra(EXTRA_HOSTNAME) ?: android.os.Build.MODEL

                startVpn(controlUrl, relayServer, hostname)
            }
            ACTION_STOP -> stopVpn()
        }
        return START_STICKY
    }

    // ═══════════════════════════════════════════════════════════════
    // VPN Control / VPN 控制
    // ═══════════════════════════════════════════════════════════════

    /**
     * Start the VPN tunnel.
     * 启动 VPN 隧道。
     *
     * Initializes the Rust engine, creates a TUN interface, acquires a
     * wakelock, and starts bidirectional packet forwarding threads.
     * 初始化 Rust 引擎，创建 TUN 虚拟网卡，获取唤醒锁，并启动双向数据包转发线程。
     *
     * @param controlUrl  Control service URL / 控制服务地址
     * @param relayServer Relay server address / 中继服务器地址
     * @param hostname    Device hostname / 设备主机名
     */
    private fun startVpn(controlUrl: String, relayServer: String, hostname: String) {
        if (running) {
            Log.w(TAG, "VPN already running")
            return
        }

        // Show foreground notification (required for VpnService)
        startForeground(NOTIFICATION_ID, buildNotification("Connecting..."))

        // Initialize Rust engine via JNI
        val initOk = RustBridge.init(controlUrl, relayServer, hostname)
        if (!initOk) {
            Log.e(TAG, "Rust engine init failed")
            stopVpn()
            return
        }

        val virtualIp = RustBridge.getVirtualIP() ?: VPN_ADDRESS
        Log.i(TAG, "Engine initialized, virtual IP: $virtualIp")

        // Build TUN interface
        val builder = Builder()
            .setSession("ConnectAlso")
            .addAddress(virtualIp, 24)
            .addRoute(VPN_ROUTE, 0)
            .setMtu(VPN_MTU)
            .setBlocking(true)
            .addDnsServer("8.8.8.8")
            .addDnsServer("1.1.1.1")

        tunFd = builder.establish() ?: run {
            Log.e(TAG, "TUN establish failed")
            stopVpn()
            return
        }

        running = true
        updateNotification("Connected — $virtualIp")

        // Acquire partial wakelock to keep CPU awake for packet forwarding
        acquireWakelock()

        // Register network change listener
        registerNetworkCallback()

        // Start packet forwarding threads
        startPacketForwarding()

        Log.i(TAG, "VPN started ($virtualIp)")
    }

    /**
     * Stop the VPN tunnel.
     * 停止 VPN 隧道。
     *
     * Unregisters network callbacks, releases the wakelock, shuts down
     * the Rust engine, and closes the TUN file descriptor.
     * 注销网络回调，释放唤醒锁，关闭 Rust 引擎，并关闭 TUN 文件描述符。
     */
    private fun stopVpn() {
        running = false

        unregisterNetworkCallback()
        releaseWakelock()

        RustBridge.shutdown()

        try { tunFd?.close() } catch (_: Exception) {}
        tunFd = null

        try { relaySocket?.close() } catch (_: Exception) {}
        relaySocket = null

        stopForeground(STOP_FOREGROUND_REMOVE)
        stopSelf()

        Log.i(TAG, "VPN stopped")
    }

    override fun onDestroy() {
        stopVpn()
        super.onDestroy()
    }

    // ═══════════════════════════════════════════════════════════════
    // Packet Forwarding / 数据包转发
    // ═══════════════════════════════════════════════════════════════

    /**
     * Start bidirectional packet forwarding between TUN and Rust engine.
     * 启动 TUN 与 Rust 引擎之间的双向数据包转发。
     *
     * Spawns two threads: one reading from TUN and forwarding to the Rust
     * engine (outbound), and one polling the Rust engine and writing to TUN
     * (inbound).
     * 启动两个线程：一个从 TUN 读取并转发到 Rust 引擎（出站），
     * 另一个从 Rust 引擎轮询并写入 TUN（入站）。
     */
    private fun startPacketForwarding() {
        val fd = tunFd ?: return
        val input = FileInputStream(fd.fileDescriptor)
        val output = FileOutputStream(fd.fileDescriptor)

        // Thread 1: TUN → Rust engine → Network
        thread(name = "ConnectAlso-outbound") {
            val buffer = ByteArray(VPN_MTU)
            while (running) {
                try {
                    val len = input.read(buffer)
                    if (len > 0) {
                        val packet = buffer.copyOf(len)
                        val result = RustBridge.sendPacket(packet)
                        // Result is null on error; packets are forwarded
                        // via relay internally by the Rust engine
                    }
                } catch (e: Exception) {
                    if (running) Log.e(TAG, "outbound error: ${e.message}")
                }
            }
        }

        // Thread 2: Rust engine → TUN
        thread(name = "ConnectAlso-inbound") {
            while (running) {
                try {
                    val packet = RustBridge.recvPacket()
                    if (packet != null && packet.isNotEmpty()) {
                        output.write(packet)
                    } else {
                        Thread.sleep(10) // No data — brief sleep
                    }
                } catch (e: Exception) {
                    if (running) Log.e(TAG, "inbound error: ${e.message}")
                }
            }
        }
    }

    // ═══════════════════════════════════════════════════════════════
    // Notification / 通知
    // ═══════════════════════════════════════════════════════════════

    private fun createNotificationChannel() {
        if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.O) {
            val channel = NotificationChannel(
                CHANNEL_ID,
                "ConnectAlso VPN",
                NotificationManager.IMPORTANCE_LOW
            ).apply {
                description = "ConnectAlso VPN connection status"
            }
            val manager = getSystemService(NotificationManager::class.java)
            manager.createNotificationChannel(channel)
        }
    }

    private fun buildNotification(text: String): Notification {
        val pendingIntent = PendingIntent.getActivity(
            this, 0,
            Intent(this, javaClass).setAction(ACTION_STOP),
            PendingIntent.FLAG_IMMUTABLE
        )

        return Notification.Builder(this, CHANNEL_ID)
            .setContentTitle("ConnectAlso")
            .setContentText(text)
            .setSmallIcon(android.R.drawable.ic_menu_share)
            .setContentIntent(pendingIntent)
            .setOngoing(true)
            .build()
    }

    private fun updateNotification(text: String) {
        val manager = getSystemService(NotificationManager::class.java)
        manager.notify(NOTIFICATION_ID, buildNotification(text))
    }

    // ═══════════════════════════════════════════════════════════════
    // Network change detection (Wi-Fi ↔ Cellular) / 网络变化检测（Wi-Fi ↔ 蜂窝网络）
    // ═══════════════════════════════════════════════════════════════

    /**
     * Register a network callback to detect connectivity changes.
     * 注册网络回调以检测网络连接变化。
     *
     * Monitors network availability, loss, and capability changes
     * (e.g., switching between Wi-Fi and cellular) to trigger VPN
     * reconnection.
     * 监控网络可用性、丢失和能力变化（如 Wi-Fi 与蜂窝网络切换），
     * 以触发 VPN 重连。
     */
    private fun registerNetworkCallback() {
        connectivityManager = getSystemService(ConnectivityManager::class.java)
        networkCallback = object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network: Network) {
                Log.i(TAG, "network available")
                triggerReconnect()
            }

            override fun onLost(network: Network) {
                Log.w(TAG, "network lost")
            }

            override fun onCapabilitiesChanged(
                network: Network,
                capabilities: NetworkCapabilities
            ) {
                // Detect Wi-Fi ↔ Cellular switch
                val hasWifi = capabilities.hasTransport(NetworkCapabilities.TRANSPORT_WIFI)
                val hasCellular = capabilities.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR)
                Log.i(TAG, "capabilities changed — wifi=$hasWifi cellular=$hasCellular")
                triggerReconnect()
            }
        }

        val request = NetworkRequest.Builder()
            .addCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
            .build()

        connectivityManager?.registerNetworkCallback(request, networkCallback!!)
        Log.i(TAG, "network callback registered")
    }

    private fun unregisterNetworkCallback() {
        networkCallback?.let { connectivityManager?.unregisterNetworkCallback(it) }
        networkCallback = null
        Log.i(TAG, "network callback unregistered")
    }

    private fun triggerReconnect() {
        if (!running) return
        thread(name = "ConnectAlso-reconnect") {
            Log.i(TAG, "triggering reconnect...")
            val ok = RustBridge.reconnect()
            Log.i(TAG, "reconnect result: $ok")
        }
    }

    // ═══════════════════════════════════════════════════════════════
    // Battery optimization / 电池优化
    // ═══════════════════════════════════════════════════════════════

    /**
     * Acquire a partial wakelock to keep the CPU awake for packet forwarding.
     * 获取部分唤醒锁以保持 CPU 唤醒，确保数据包转发不中断。
     */
    private fun acquireWakelock() {
        val pm = getSystemService(POWER_SERVICE) as PowerManager
        wakelock = pm.newWakeLock(
            PowerManager.PARTIAL_WAKE_LOCK,
            "ConnectAlso:packet-forwarding"
        ).apply {
            acquire(10 * 60 * 1000L) // 10 min timeout, auto-release
        }
        Log.i(TAG, "wakelock acquired")
    }

    private fun releaseWakelock() {
        wakelock?.let {
            if (it.isHeld) it.release()
        }
        wakelock = null
        Log.i(TAG, "wakelock released")
    }

    /**
     * Adaptive polling: faster when traffic is active, slower when idle.
     *
     * The Rust engine's recvPacket already uses internal timeouts,
     * so we simply call it in a loop. The engine adjusts its own
     * poll interval based on traffic activity.
     *
     * For maximum battery savings, the Android Doze mode will
     * automatically defer network access during maintenance windows.
     */
    private fun adaptivePollInterval(): Long {
        // The Rust engine handles adaptive timing internally.
        // This method exists as a hook for future platform-specific
        // battery optimization (e.g., checking BatteryManager level).
        return 10 // ms — Rust engine adjusts actual wait
    }
}
