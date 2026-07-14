# ConnectAlso iOS 集成指南

将 ConnectAlso Rust 核心编译为静态库，集成到 iOS NetworkExtension 中。

---

## 架构

```
┌─────────────────────────────────────────────┐
│                 iOS App                      │
│  ┌───────────────────────────────────────┐  │
│  │  Container App                        │  │
│  │  • UI / Settings                      │  │
│  │  • Starts VPN via NETunnelProviderManager │
│  └───────────────┬───────────────────────┘  │
│                  │ IPC                       │
│  ┌───────────────▼───────────────────────┐  │
│  │  Network Extension (separate process) │  │
│  │  ┌─────────────────────────────────┐  │  │
│  │  │  PacketTunnelProvider.swift     │  │  │
│  │  │  • NEPacketTunnelNetworkSettings │  │  │
│  │  │  • packetFlow read/write        │  │  │
│  │  └─────────────┬───────────────────┘  │  │
│  │                │ FFI (C ABI)          │  │
│  │  ┌─────────────▼───────────────────┐  │  │
│  │  │  libconnectalso_mobile.a       │  │  │
│  │  │  (Rust static library)         │  │  │
│  │  │  • Key exchange                │  │  │
│  │  │  • Encrypted relay             │  │  │
│  │  │  • Peer management             │  │  │
│  │  └─────────────┬───────────────────┘  │  │
│  └────────────────┼──────────────────────┘  │
│                   │ UDP                     │
└───────────────────┼─────────────────────────┘
                    ▼
            Relay / P2P / Internet
```

## 构建

### 1. 安装 Rust 交叉编译工具链

```bash
rustup target add aarch64-apple-ios      # 真机
rustup target add aarch64-apple-ios-sim   # 模拟器 (Apple Silicon)
rustup target add x86_64-apple-ios        # 模拟器 (Intel Mac)
```

### 2. 编译静态库

```bash
# 真机 (arm64)
cargo build -p connectalso-mobile --release --target aarch64-apple-ios

# 模拟器 (Intel)
cargo build -p connectalso-mobile --release --target x86_64-apple-ios

# 模拟器 (Apple Silicon)
cargo build -p connectalso-mobile --release --target aarch64-apple-ios-sim

# 创建通用模拟器库 (x86_64 + arm64)
lipo -create \
  target/x86_64-apple-ios/release/libconnectalso_mobile.a \
  target/aarch64-apple-ios-sim/release/libconnectalso_mobile.a \
  -output target/ios-simulator/libconnectalso_mobile.a
```

产物位置：
- 真机: `target/aarch64-apple-ios/release/libconnectalso_mobile.a`
- 模拟器: `target/ios-simulator/libconnectalso_mobile.a`

## Xcode 集成

### 1. 创建 NetworkExtension Target

在 Xcode 中：
1. File → New → Target → Network Extension
2. 选择 "Packet Tunnel Provider"
3. 命名如 "ConnectAlsoExtension"

### 2. 添加静态库

1. 将 `libconnectalso_mobile.a` 拖入项目
2. 在 Build Settings → Library Search Paths 添加库路径
3. 在 Build Settings → Other Linker Flags 添加 `-lconnectalso_mobile -lresolv`

### 3. 添加 Bridging Header

`ConnectAlsoExtension-Bridging-Header.h`:
```c
#ifndef ConnectAlso_Bridging_Header_h
#define ConnectAlso_Bridging_Header_h

int32_t connectalso_init(const char *control_url,
                          const char *stun_server,
                          const char *relay_server,
                          const char *hostname);

int32_t connectalso_send_packet(const uint8_t *packet, uint32_t len,
                                 uint8_t *out, uint32_t out_len);

int32_t connectalso_recv_packet(uint8_t *out, uint32_t out_len);

int32_t connectalso_get_virtual_ip(char *out, uint32_t out_len);

void    connectalso_shutdown(void);

#endif
```

### 4. 替换默认 Provider

将 `deploy/ios/PacketTunnelProvider.swift` 复制到 `ConnectAlsoExtension/` 目录，
替换 Xcode 自动生成的 `PacketTunnelProvider.swift`。

### 5. 配置 Info.plist

在 NetworkExtension target 的 Info.plist 中添加：

```xml
<key>NSExtension</key>
<dict>
    <key>NSExtensionPointIdentifier</key>
    <string>com.apple.networkextension.packet-tunnel</string>
    <key>NSExtensionPrincipalClass</key>
    <string>$(PRODUCT_MODULE_NAME).PacketTunnelProvider</string>
</dict>
```

### 6. 配置 App Capabilities

在主 App target 中：
- 启用 "Personal VPN" capability
- 启用 "Network Extensions" capability

## 容器 App 集成

在主 App 中启动 VPN：

```swift
import NetworkExtension

func startVPN() {
    let manager = NETunnelProviderManager()
    manager.localizedDescription = "ConnectAlso"

    let config = NETunnelProviderProtocol()
    config.providerBundleIdentifier = "com.yourcompany.ConnectAlsoExtension"
    config.serverAddress = "connectalso"
    config.providerConfiguration = [
        "control_url": "http://your-server.com:3000",
        "relay_server": "your-server.com:33478",
        "hostname": UIDevice.current.name
    ]

    manager.protocolConfiguration = config
    manager.isEnabled = true

    manager.saveToPreferences { error in
        if error == nil {
            manager.loadFromPreferences { _ in
                try? manager.connection.startVPNTunnel()
            }
        }
    }
}

func stopVPN() {
    NETunnelProviderManager.loadAllFromPreferences { managers, _ in
        managers?.first?.connection.stopVPNTunnel()
    }
}
```

## FFI 接口说明

| 函数 | 说明 |
|------|------|
| `connectalso_init(url, stun, relay, name)` | 初始化引擎，注册设备，连接对等 |
| `connectalso_send_packet(pkt, len, out, out_len)` | 发送 TUN 包到网络 |
| `connectalso_recv_packet(out, out_len)` | 从网络接收包 |
| `connectalso_get_virtual_ip(out, out_len)` | 获取分配的虚拟 IP |
| `connectalso_shutdown()` | 关闭引擎 |

所有函数返回 `int32_t`：≥0 成功，-1 错误。

## 注意事项

1. **iOS 后台限制**: NetworkExtension 进程有严格的内存和 CPU 限制
2. **UDP 后台**: iOS 可能限制后台 UDP 通信，确保 keepalive 正常工作
3. **网络切换**: Wi-Fi ↔ 蜂窝网络切换时需重新建立连接
4. **省电**: 避免 busy-waiting，使用 `Thread.sleep` 或 `DispatchSource` 定时器
5. **App Group**: 通过 App Group 共享配置文件和 Keychain 数据
