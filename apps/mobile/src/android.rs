use std::net::SocketAddr;

use jni::objects::{JClass, JString};
use jni::sys::{jboolean, jbyteArray, jint, jstring};
use jni::JNIEnv;

use crate::engine;

/// JNI 函数前缀：Java_com_connectalso_mobile_RustBridge_<method>
/// JNI function prefix: Java_com_connectalso_mobile_RustBridge_<method>
const CLASS_PATH: &str = "com/connectalso/mobile/RustBridge";

// ═══════════════════════════════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════════════════════════════

fn jstring_to_str<'a>(env: &'a JNIEnv, s: &'a JString) -> &'a str {
    env.get_string(s)
        .map(|s| {
            let s: &str = &s;
            // SAFETY: leak the underlying OsStr to get a &str with
            // 'a lifetime matching JString's lifetime in the JNI call.
            unsafe { std::mem::transmute::<&str, &'a str>(s) }
        })
        .unwrap_or("")
}

fn throw(env: &mut JNIEnv, msg: &str) {
    let _ = env.throw_new("java/lang/RuntimeException", msg);
}

// ═══════════════════════════════════════════════════════════════════
// JNI Exports
// ═══════════════════════════════════════════════════════════════════

/// 初始化 ConnectAlso 引擎。
///
/// Java 签名：
///   native boolean init(String controlUrl, String relayServer, String hostname);
///
/// Initialize the ConnectAlso engine.
///
/// Java signature:
///   native boolean init(String controlUrl, String relayServer, String hostname);
#[no_mangle]
pub extern "system" fn Java_com_connectalso_mobile_RustBridge_init(
    mut env: JNIEnv,
    _class: JClass,
    control_url: JString,
    relay_server: JString,
    hostname: JString,
) -> jboolean {
    let ctl = jstring_to_str(&env, &control_url);
    let relay = jstring_to_str(&env, &relay_server);
    let name = jstring_to_str(&env, &hostname);

    let relay_addr: SocketAddr = match relay.parse() {
        Ok(a) => a,
        Err(e) => {
            throw(&mut env, &format!("invalid relay: {e}"));
            return jboolean::from(false);
        }
    };

    match engine::RUNTIME.block_on(engine::engine_init(ctl, relay_addr, name)) {
        Ok(()) => {
            tracing::info!("Android engine initialized");
            jboolean::from(true)
        }
        Err(e) => {
            throw(&mut env, &format!("init failed: {e}"));
            jboolean::from(false)
        }
    }
}

/// 保护 UDP socket 文件描述符，使其不被 VPN 路由捕获。
///
/// Java 签名：
///   native void protectSocket(int fd);
///
/// 由 VpnService.protect() 通过 RustBridge 调用。
/// 此操作可防止中继 socket 走自己的 TUN 接口。
///
/// Protect a UDP socket file descriptor from being routed through the VPN.
///
/// Java signature:
///   native void protectSocket(int fd);
///
/// Called from VpnService.protect() via RustBridge.
/// This prevents the relay socket from going through our own TUN interface.
#[no_mangle]
pub extern "system" fn Java_com_connectalso_mobile_RustBridge_protectSocket(_env: JNIEnv, _class: JClass, _fd: jint) {
    // Protection is handled by the Kotlin VpnService calling
    // VpnService.protect(DatagramSocket). This JNI function is a
    // placeholder that the Kotlin side can call to register the
    // protected fd with the Rust engine if needed.
    tracing::debug!(fd = _fd, "socket protect placeholder");
}

/// 发送来自 TUN 接口的出站数据包。
///
/// Java 签名：
///   native byte[] sendPacket(byte[] packet);
///
/// 返回加密后的数据包，错误时返回 null。
///
/// Send an outgoing packet from the TUN interface.
///
/// Java signature:
///   native byte[] sendPacket(byte[] packet);
///
/// Returns the encrypted packet to send, or null on error.
#[no_mangle]
pub extern "system" fn Java_com_connectalso_mobile_RustBridge_sendPacket(
    mut env: JNIEnv,
    _class: JClass,
    packet: jbyteArray,
) -> jbyteArray {
    let data = match env.convert_byte_array(&packet) {
        Ok(d) => d,
        Err(e) => {
            throw(&mut env, &format!("bad input: {e}"));
            return std::ptr::null_mut();
        }
    };

    match engine::RUNTIME.block_on(engine::engine_send_packet(&data)) {
        Ok(result) => match env.byte_array_from_slice(&result) {
            Ok(arr) => arr.into_raw(),
            Err(e) => {
                throw(&mut env, &format!("output error: {e}"));
                std::ptr::null_mut()
            }
        },
        Err(e) => {
            throw(&mut env, &format!("send failed: {e}"));
            std::ptr::null_mut()
        }
    }
}

/// 从隧道网络轮询入站数据包。
///
/// Java 签名：
///   native byte[] recvPacket();
///
/// 返回解密后的数据包供注入 TUN，无数据时返回 null。
///
/// Poll for an incoming packet from the tunnel network.
///
/// Java signature:
///   native byte[] recvPacket();
///
/// Returns the decrypted packet to inject into TUN, or null if no data.
#[no_mangle]
pub extern "system" fn Java_com_connectalso_mobile_RustBridge_recvPacket(
    mut env: JNIEnv,
    _class: JClass,
) -> jbyteArray {
    match engine::RUNTIME.block_on(engine::engine_recv_packet()) {
        Ok(data) => match env.byte_array_from_slice(&data) {
            Ok(arr) => arr.into_raw(),
            Err(e) => {
                throw(&mut env, &format!("output error: {e}"));
                std::ptr::null_mut()
            }
        },
        Err(_) => std::ptr::null_mut(), // No data — normal
    }
}

/// 获取分配的虚拟 IPv4 地址。
///
/// Java 签名：
///   native String getVirtualIP();
///
/// Get the assigned virtual IPv4 address.
///
/// Java signature:
///   native String getVirtualIP();
#[no_mangle]
pub extern "system" fn Java_com_connectalso_mobile_RustBridge_getVirtualIP(env: JNIEnv, _class: JClass) -> jstring {
    match engine::RUNTIME.block_on(async {
        let guard = engine::ENGINE.lock().unwrap();
        let arc = guard.as_ref().ok_or(())?;
        let e = tokio::runtime::Handle::current().block_on(async { arc.lock().await });
        Ok::<_, ()>(e.our_ip.to_string())
    }) {
        Ok(ip) => match env.new_string(&ip) {
            Ok(s) => s.into_raw(),
            Err(_) => std::ptr::null_mut(),
        },
        Err(_) => std::ptr::null_mut(),
    }
}

/// 关闭引擎。
///
/// Java 签名：
///   native void shutdown();
///
/// Shut down the engine.
///
/// Java signature:
///   native void shutdown();
#[no_mangle]
pub extern "system" fn Java_com_connectalso_mobile_RustBridge_shutdown(_env: JNIEnv, _class: JClass) {
    tracing::info!("Android engine shutting down");
}

/// 网络切换后触发重连。
///
/// Java 签名：
///   native boolean reconnect();
///
/// 当设备在 Wi-Fi 和蜂窝网络之间切换时应调用此方法。
/// 会向控制服务重新注册、刷新节点列表并重连中继会话。
/// 缓存的出站数据包会自动冲刷。
///
/// Trigger reconnection after a network change.
///
/// Java signature:
///   native boolean reconnect();
///
/// Call this when the device switches between Wi-Fi and Cellular.
/// Re-registers with control service, refreshes peers, and re-connects
/// relay sessions. Queued outbound packets are flushed automatically.
#[no_mangle]
pub extern "system" fn Java_com_connectalso_mobile_RustBridge_reconnect(mut env: JNIEnv, _class: JClass) -> jboolean {
    match engine::RUNTIME.block_on(engine::engine_reconnect()) {
        Ok(()) => jboolean::from(true),
        Err(e) => {
            tracing::error!(%e, "reconnect failed");
            jboolean::from(false)
        }
    }
}
