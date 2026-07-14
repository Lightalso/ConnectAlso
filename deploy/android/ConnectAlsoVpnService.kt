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
import android.util.Log
import java.io.FileInputStream
import java.io.FileOutputStream
import java.net.DatagramSocket
import kotlin.concurrent.thread

/**
 * ConnectAlso VpnService for Android.
 *
 * Creates a TUN interface via VpnService.Builder and forwards packets
 * through the Rust engine (libconnectalso_mobile.so via JNI).
 *
 * ## Setup in AndroidManifest.xml
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
 * ## Required permissions
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
    private var relaySocket: DatagramSocket? = null
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
    // VPN Control
    // ═══════════════════════════════════════════════════════════════

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

        // Register network change listener
        registerNetworkCallback()

        // Start packet forwarding threads
        startPacketForwarding()

        Log.i(TAG, "VPN started ($virtualIp)")
    }

    private fun stopVpn() {
        running = false

        // Unregister network callback
        unregisterNetworkCallback()

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
    // Packet Forwarding
    // ═══════════════════════════════════════════════════════════════

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
    // Notification
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
    // Network change detection (Wi-Fi ↔ Cellular)
    // ═══════════════════════════════════════════════════════════════

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
}
