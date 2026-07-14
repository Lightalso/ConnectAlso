use std::ffi::{c_char, CStr, CString};
use std::net::SocketAddr;

use tokio::runtime::Runtime;

use crate::engine;

static RUNTIME: std::sync::OnceLock<Runtime> = std::sync::OnceLock::new();

fn runtime() -> &'static Runtime {
    RUNTIME.get_or_init(|| Runtime::new().expect("tokio runtime"))
}

/// Helper: convert C string to Rust &str.
unsafe fn cstr_to_str<'a>(ptr: *const c_char) -> &'a str {
    if ptr.is_null() {
        return "";
    }
    unsafe { CStr::from_ptr(ptr) }.to_str().unwrap_or("")
}

// ═══════════════════════════════════════════════════════════════════
// FFI Exports
// ═══════════════════════════════════════════════════════════════════

/// Initialize the ConnectAlso mobile engine.
///
/// Parameters are null-terminated C strings:
/// - control_url: e.g. "http://192.168.1.1:3000"
/// - stun_server: ignored on iOS (uses relay)
/// - relay_server: e.g. "192.168.1.1:33478"
/// - hostname: device name
///
/// Returns 0 on success, -1 on error.
#[no_mangle]
pub extern "C" fn connectalso_init(
    control_url: *const c_char,
    _stun_server: *const c_char,
    relay_server: *const c_char,
    hostname: *const c_char,
) -> i32 {
    let ctl = unsafe { cstr_to_str(control_url) };
    let relay = unsafe { cstr_to_str(relay_server) };
    let name = unsafe { cstr_to_str(hostname) };

    let relay_addr: SocketAddr = match relay.parse() {
        Ok(a) => a,
        Err(e) => {
            tracing::error!(%e, "invalid relay address");
            return -1;
        }
    };

    match runtime().block_on(engine::engine_init(ctl, relay_addr, name)) {
        Ok(()) => {
            tracing::info!("iOS engine initialized");
            0
        }
        Err(e) => {
            tracing::error!(%e, "engine init failed");
            -1
        }
    }
}

/// Process an outgoing IP packet from the TUN interface.
///
/// Reads `packet_len` bytes from `packet`, encrypts and forwards to
/// the appropriate peer via relay. Writes any resulting data to `out_buf`
/// (up to `out_buf_len` bytes) and returns the number of bytes written.
/// Returns 0 if the packet was forwarded (no local delivery).
/// Returns -1 on error.
#[no_mangle]
pub extern "C" fn connectalso_send_packet(
    packet: *const u8,
    packet_len: u32,
    out_buf: *mut u8,
    out_buf_len: u32,
) -> i32 {
    if packet.is_null() || out_buf.is_null() {
        return -1;
    }
    let data = unsafe { std::slice::from_raw_parts(packet, packet_len as usize) };

    match runtime().block_on(engine::engine_send_packet(data)) {
        Ok(result) => {
            let len = result.len().min(out_buf_len as usize);
            unsafe {
                std::ptr::copy_nonoverlapping(result.as_ptr(), out_buf, len);
            }
            len as i32
        }
        Err(e) => {
            tracing::error!(%e, "send_packet failed");
            -1
        }
    }
}

/// Poll for an incoming packet from the tunnel network.
///
/// Writes up to `out_buf_len` bytes to `out_buf` and returns the
/// number of bytes written. Returns 0 if no data is available.
/// Returns -1 on error.
#[no_mangle]
pub extern "C" fn connectalso_recv_packet(
    out_buf: *mut u8,
    out_buf_len: u32,
) -> i32 {
    if out_buf.is_null() {
        return -1;
    }

    match runtime().block_on(engine::engine_recv_packet()) {
        Ok(data) => {
            let len = data.len().min(out_buf_len as usize);
            unsafe {
                std::ptr::copy_nonoverlapping(data.as_ptr(), out_buf, len);
            }
            len as i32
        }
        Err(_) => 0, // No data available — not an error
    }
}

/// Shut down the engine and release resources.
#[no_mangle]
pub extern "C" fn connectalso_shutdown() {
    tracing::info!("iOS engine shutting down");
}

/// Trigger reconnection after a network change (Wi-Fi ↔ Cellular).
///
/// This re-registers with the control service, refreshes the peer list,
/// re-connects relay sessions, and flushes queued outbound packets.
///
/// Returns 0 on success, -1 on error.
#[no_mangle]
pub extern "C" fn connectalso_reconnect() -> i32 {
    match runtime().block_on(engine::engine_reconnect()) {
        Ok(()) => 0,
        Err(e) => {
            tracing::error!(%e, "reconnect failed");
            -1
        }
    }
}

/// Return the virtual IP assigned to this device.
///
/// Writes a null-terminated string to `out_buf` (max `out_len` bytes).
/// Returns the string length (excluding null) on success, -1 on error.
#[no_mangle]
pub extern "C" fn connectalso_get_virtual_ip(
    out_buf: *mut c_char,
    out_len: u32,
) -> i32 {
    if out_buf.is_null() {
        return -1;
    }

    match runtime().block_on(async {
        let guard = engine::ENGINE.lock().unwrap();
        let engine = guard.as_ref().ok_or(())?;
        let e = engine.lock().await;
        Ok::<_, ()>(e.our_ip.to_string())
    }) {
        Ok(ip) => {
            let c_str = CString::new(ip).unwrap_or_default();
            let bytes = c_str.as_bytes_with_nul();
            let len = bytes.len().min(out_len as usize);
            unsafe {
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), out_buf as *mut u8, len);
            }
            (bytes.len() - 1) as i32 // exclude null terminator
        }
        Err(_) => -1,
    }
}
