# ConnectAlso Android 集成指南

将 ConnectAlso Rust 核心编译为 `.so` 动态库，通过 JNI 集成到 Android VpnService 中。

---

## 架构

```
┌─────────────────────────────────────────────┐
│              Android App                     │
│  ┌───────────────────────────────────────┐  │
│  │  MainActivity                         │  │
│  │  • Start/Stop VPN                     │  │
│  │  • Settings UI                        │  │
│  └───────────────┬───────────────────────┘  │
│                  │ Intent                    │
│  ┌───────────────▼───────────────────────┐  │
│  │  ConnectAlsoVpnService                │  │
│  │  • VpnService.Builder                 │  │
│  │  • TUN fd (FileInputStream/OutputStream) │
│  │  • Foreground notification            │  │
│  └───────────────┬───────────────────────┘  │
│                  │ JNI                       │
│  ┌───────────────▼───────────────────────┐  │
│  │  libconnectalso_mobile.so             │  │
│  │  (Rust — android.rs → engine.rs)      │  │
│  │  • Key exchange                       │  │
│  │  • Encrypted relay                    │  │
│  │  • Peer management                    │  │
│  └───────────────┬───────────────────────┘  │
│                  │ UDP                       │
└──────────────────┼──────────────────────────┘
                   ▼
           Relay / P2P / Internet
```

## 构建

### 1. 安装 Rust Android 交叉编译工具链

```bash
rustup target add aarch64-linux-android    # ARM64 (大多数现代设备)
rustup target add armv7-linux-androideabi  # ARM32 (旧设备)
rustup target add x86_64-linux-android     # x86_64 模拟器
rustup target add i686-linux-android       # x86 模拟器
```

### 2. 安装 Android NDK

```bash
# 通过 Android Studio SDK Manager 或命令行
sdkmanager "ndk;27.0.12077973"
```

### 3. 配置 Cargo 链接器

`~/.cargo/config.toml`:
```toml
[target.aarch64-linux-android]
linker = "<NDK_PATH>/toolchains/llvm/prebuilt/linux-x86_64/bin/aarch64-linux-android26-clang"

[target.armv7-linux-androideabi]
linker = "<NDK_PATH>/toolchains/llvm/prebuilt/linux-x86_64/bin/armv7a-linux-androideabi26-clang"

[target.x86_64-linux-android]
linker = "<NDK_PATH>/toolchains/llvm/prebuilt/linux-x86_64/bin/x86_64-linux-android26-clang"
```

### 4. 编译

```bash
# ARM64 (真机)
cargo build -p connectalso-mobile --release --target aarch64-linux-android

# 多架构编译脚本
for target in aarch64-linux-android armv7-linux-androideabi x86_64-linux-android; do
    cargo build -p connectalso-mobile --release --target $target
done
```

产物位置：
- `target/aarch64-linux-android/release/libconnectalso_mobile.so`
- `target/armv7-linux-androideabi/release/libconnectalso_mobile.so`
- `target/x86_64-linux-android/release/libconnectalso_mobile.so`

## Android 项目集成

### 1. 目录结构

```
app/
├── src/main/
│   ├── java/com/connectalso/mobile/
│   │   ├── ConnectAlsoVpnService.kt     # VpnService 实现
│   │   ├── RustBridge.kt                # JNI 桥接
│   │   └── MainActivity.kt              # 主界面
│   ├── jniLibs/
│   │   ├── arm64-v8a/
│   │   │   └── libconnectalso_mobile.so
│   │   ├── armeabi-v7a/
│   │   │   └── libconnectalso_mobile.so
│   │   └── x86_64/
│   │       └── libconnectalso_mobile.so
│   └── AndroidManifest.xml
```

### 2. 复制文件

将 `deploy/android/` 中的文件复制到项目中：

```bash
cp deploy/android/ConnectAlsoVpnService.kt app/src/main/java/com/connectalso/mobile/
cp deploy/android/RustBridge.kt app/src/main/java/com/connectalso/mobile/
cp target/aarch64-linux-android/release/libconnectalso_mobile.so app/src/main/jniLibs/arm64-v8a/
```

### 3. AndroidManifest.xml 配置

```xml
<manifest xmlns:android="http://schemas.android.com/apk/res/android">

    <!-- 权限 -->
    <uses-permission android:name="android.permission.INTERNET" />
    <uses-permission android:name="android.permission.FOREGROUND_SERVICE" />
    <uses-permission android:name="android.permission.FOREGROUND_SERVICE_SPECIAL_USE" />
    <uses-permission android:name="android.permission.POST_NOTIFICATIONS" />

    <application>
        <!-- VPN Service -->
        <service
            android:name=".ConnectAlsoVpnService"
            android:permission="android.permission.BIND_VPN_SERVICE"
            android:exported="false"
            android:foregroundServiceType="specialUse">
            <intent-filter>
                <action android:name="android.net.VpnService" />
            </intent-filter>
        </service>

        <!-- Main Activity -->
        <activity
            android:name=".MainActivity"
            android:exported="true">
            <intent-filter>
                <action android:name="android.intent.action.MAIN" />
                <category android:name="android.intent.category.LAUNCHER" />
            </intent-filter>
        </activity>
    </application>
</manifest>
```

### 4. MainActivity 启动 VPN

```kotlin
class MainActivity : AppCompatActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // ... UI setup ...

        findViewById<Button>(R.id.btn_start).setOnClickListener {
            val intent = Intent(this, ConnectAlsoVpnService::class.java).apply {
                action = ConnectAlsoVpnService.ACTION_START
                putExtra(ConnectAlsoVpnService.EXTRA_CONTROL_URL, "http://your-server:3000")
                putExtra(ConnectAlsoVpnService.EXTRA_RELAY_SERVER, "your-server:33478")
                putExtra(ConnectAlsoVpnService.EXTRA_HOSTNAME, Build.MODEL)
            }
            startService(intent)
        }

        findViewById<Button>(R.id.btn_stop).setOnClickListener {
            val intent = Intent(this, ConnectAlsoVpnService::class.java).apply {
                action = ConnectAlsoVpnService.ACTION_STOP
            }
            startService(intent)
        }
    }
}
```

## JNI 接口

| Kotlin 方法 | Rust JNI 函数 | 说明 |
|------------|--------------|------|
| `RustBridge.init(url, relay, name)` | `Java_..._init` | 初始化引擎 |
| `RustBridge.sendPacket(pkt)` | `Java_..._sendPacket` | TUN → 网络 |
| `RustBridge.recvPacket()` | `Java_..._recvPacket` | 网络 → TUN |
| `RustBridge.getVirtualIP()` | `Java_..._getVirtualIP` | 获取虚拟 IP |
| `RustBridge.protectSocket(fd)` | `Java_..._protectSocket` | 保护 socket |
| `RustBridge.shutdown()` | `Java_..._shutdown` | 关闭引擎 |

## 注意事项

1. **前台通知**: Android 要求 VpnService 在 5 秒内显示前台通知
2. **Socket 保护**: 中继 UDP socket 必须调用 `VpnService.protect()` 防止路由循环
3. **后台限制**: Android 8+ 限制后台服务，确保使用 foreground service
4. **网络切换**: WiFi ↔ 蜂窝网络切换时 VpnService 自动重连
5. **电池优化**: 请求 `REQUEST_IGNORE_BATTERY_OPTIMIZATIONS` 权限
6. **VPN 对话框**: 首次启动系统会弹出 VPN 授权对话框，用户必须批准
7. **TUN 阻塞模式**: 使用 `setBlocking(true)` 简化 I/O，线程处理即可

## 调试

```bash
# 查看 VPN 日志
adb logcat -s ConnectAlsoVPN:V

# 查看 Rust 日志 (需在 Rust 端配置 android_logger)
adb logcat -s ConnectAlsoRust:V

# 验证 VPN 接口
adb shell ip addr show tun0
adb shell ip route show table all | grep tun0
```
