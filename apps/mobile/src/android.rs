use std::net::SocketAddr;

use jni::objects::{JClass, JString};
use jni::sys::{jboolean, jbyteArray, jint, jstring};
use jni::JNIEnv;

use crate::engine;

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

/// Protect a UDP socket file descriptor from being routed through the VPN.
///
/// Java signature:
///   native void protectSocket(int fd);
///
/// Called from VpnService.protect() via RustBridge.
/// This prevents the relay socket from going through our own TUN interface.
#[no_mangle]
pub extern "system" fn Java_com_connectalso_mobile_RustBridge_protectSocket(
    _env: JNIEnv,
    _class: JClass,
    _fd: jint,
) {
    // Protection is handled by the Kotlin VpnService calling
    // VpnService.protect(DatagramSocket). This JNI function is a
    // placeholder that the Kotlin side can call to register the
    // protected fd with the Rust engine if needed.
    tracing::debug!(fd = _fd, "socket protect placeholder");
}

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

/// Get the assigned virtual IPv4 address.
///
/// Java signature:
///   native String getVirtualIP();
#[no_mangle]
pub extern "system" fn Java_com_connectalso_mobile_RustBridge_getVirtualIP(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
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

/// Shut down the engine.
///
/// Java signature:
///   native void shutdown();
#[no_mangle]
pub extern "system" fn Java_com_connectalso_mobile_RustBridge_shutdown(
    _env: JNIEnv,
    _class: JClass,
) {
    tracing::info!("Android engine shutting down");
}

/// Trigger reconnection after a network change.
///
/// Java signature:
///   native boolean reconnect();
///
/// Call this when the device switches between Wi-Fi and Cellular.
/// Re-registers with control service, refreshes peers, and re-connects
/// relay sessions. Queued outbound packets are flushed automatically.
#[no_mangle]
pub extern "system" fn Java_com_connectalso_mobile_RustBridge_reconnect(
    mut env: JNIEnv,
    _class: JClass,
) -> jboolean {
    match engine::RUNTIME.block_on(engine::engine_reconnect()) {
        Ok(()) => jboolean::from(true),
        Err(e) => {
            tracing::error!(%e, "reconnect failed");
            jboolean::from(false)
        }
    }
}
