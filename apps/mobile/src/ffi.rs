use std::ffi::{c_char, CStr, CString};
use std::net::SocketAddr;

use tokio::runtime::Runtime;

use crate::engine;

static RUNTIME: std::sync::OnceLock<Runtime> = std::sync::OnceLock::new();

fn runtime() -> &'static Runtime {
    RUNTIME.get_or_init(|| Runtime::new().expect("tokio runtime"))
}

/// 辅助函数：将 C 字符串转换为 Rust `&str`。
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

/// 初始化 ConnectAlso 移动端引擎。
///
/// 参数均为以 null 结尾的 C 字符串：
/// - control_url: 例如 "http://192.168.1.1:3000"
/// - stun_server: iOS 忽略（使用 relay）
/// - relay_server: 例如 "192.168.1.1:33478"
/// - hostname: 设备名称
///
/// 成功返回 0，失败返回 -1。
///
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

/// 处理来自 TUN 接口的出站 IP 数据包。
///
/// 从 `packet` 读取 `packet_len` 字节，加密后经中继转发至对应节点。
/// 将结果数据写入 `out_buf`（最多 `out_buf_len` 字节），返回写入的字节数。
/// 返回 0 表示数据包已转发（无本地投递）。
/// 返回 -1 表示错误。
///
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

/// 从隧道网络轮询一个入站数据包。
///
/// 将最多 `out_buf_len` 字节写入 `out_buf`，返回写入的字节数。
/// 返回 0 表示无数据。
/// 返回 -1 表示错误。
///
/// Poll for an incoming packet from the tunnel network.
///
/// Writes up to `out_buf_len` bytes to `out_buf` and returns the
/// number of bytes written. Returns 0 if no data is available.
/// Returns -1 on error.
#[no_mangle]
pub extern "C" fn connectalso_recv_packet(out_buf: *mut u8, out_buf_len: u32) -> i32 {
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

/// 关闭引擎并释放资源。
/// Shut down the engine and release resources.
#[no_mangle]
pub extern "C" fn connectalso_shutdown() {
    tracing::info!("iOS engine shutting down");
}

/// 网络切换后触发重连（Wi-Fi ↔ 蜂窝网络）。
///
/// 重新向控制服务注册、刷新节点列表、重连中继会话并
/// 冲刷缓存的外发包队列。
///
/// 成功返回 0，失败返回 -1。
///
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

/// 使用自适应超时轮询数据（用于电池优化）。
///
/// 返回写入的字节数，超时返回 0，错误返回 -1。
///
/// Poll for data with adaptive timeout for battery optimization.
///
/// Returns bytes written, 0 if timeout, -1 on error.
#[no_mangle]
pub extern "C" fn connectalso_recv_packet_timeout(out_buf: *mut u8, out_buf_len: u32, timeout_ms: u32) -> i32 {
    if out_buf.is_null() {
        return -1;
    }

    match runtime().block_on(async {
        tokio::time::timeout(std::time::Duration::from_millis(timeout_ms as u64), engine::engine_recv_packet()).await
    }) {
        Ok(Ok(data)) => {
            let len = data.len().min(out_buf_len as usize);
            unsafe {
                std::ptr::copy_nonoverlapping(data.as_ptr(), out_buf, len);
            }
            len as i32
        }
        _ => 0,
    }
}

/// 返回分配给本设备的虚拟 IP 地址。
///
/// 将一个以 null 结尾的字符串写入 `out_buf`（最多 `out_len` 字节）。
/// 成功返回字符串长度（不含 null），失败返回 -1。
///
/// Return the virtual IP assigned to this device.
///
/// Writes a null-terminated string to `out_buf` (max `out_len` bytes).
/// Returns the string length (excluding null) on success, -1 on error.
#[no_mangle]
pub extern "C" fn connectalso_get_virtual_ip(out_buf: *mut c_char, out_len: u32) -> i32 {
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
