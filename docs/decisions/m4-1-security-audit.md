# M4-1: 安全审计与模糊测试

> 状态：已完成  
> 日期：2026-07-14  
> 关联：M4 — 公测与 1.0

---

## 一、模糊测试基础设施

### 目录结构

```
crates/
├── relay-proto/fuzz/
│   ├── Cargo.toml
│   └── fuzz_targets/
│       ├── fuzz_frame_decode.rs     # RelayFrame::decode() 任意输入
│       └── fuzz_frame_roundtrip.rs  # decode → encode 往返一致性
└── nat/fuzz/
    ├── Cargo.toml
    └── fuzz_targets/
        └── fuzz_stun_parse.rs       # STUN 响应解析器
```

### 模糊测试目标

| 目标 | 测试函数 | 输入 | 验证 |
|------|----------|------|------|
| `fuzz_frame_decode` | `RelayFrame::decode()` | 任意字节序列 | 永不 panic |
| `fuzz_frame_roundtrip` | `decode → encode → decode` | 任意字节 → 有效帧 | 往返一致性 |
| `fuzz_stun_parse` | `parse_response` + `parse_xor_mapped_address` | 任意字节序列 | 永不 panic |

### 运行

```bash
# 安装 cargo-fuzz
cargo install cargo-fuzz

# 运行 relay-proto 帧解码模糊测试（无时间限制）
cargo +nightly fuzz run -p relay-proto fuzz_frame_decode -- -max_total_time=300

# 运行往返一致性测试
cargo +nightly fuzz run -p relay-proto fuzz_frame_roundtrip -- -max_total_time=120

# 运行 STUN 解析模糊测试
cargo +nightly fuzz run -p nat fuzz_stun_parse -- -max_total_time=300

# 复现 crash（如果发现）
cargo +nightly fuzz run -p relay-proto fuzz_frame_decode -- artifact-123456
```

---

## 二、安全审计清单

### 密码学

| 检查项 | 状态 | 说明 |
|--------|:----:|------|
| 密钥生成使用 CSPRNG | ✅ | `EphemeralSecret::random()` → OS CSPRNG |
| 私钥仅在内存中 | ✅ | `EphemeralSecret` 内部管理，不暴露原始字节 |
| Nonce 不重用 | ✅ | 发送/接收方向使用独立 nonce 空间 (offset=2^32) |
| AEAD 认证 | ✅ | ChaCha20-Poly1305: 篡改/伪造包自动检测丢弃 |
| DH 前向安全性 | ✅ | 临时密钥每会话生成，长期私钥不在数据面使用 |
| 协议降级保护 | ✅ | 硬编码密码套件，无协商 |

### 输入验证

| 检查项 | 状态 | 说明 |
|--------|:----:|------|
| RelayFrame 长度检查 | ✅ | `HEADER_LEN` + payload 边界验证 |
| STUN 响应长度检查 | ✅ | 20 字节最小长度检查 |
| payload 大小限制 | ✅ | `MAX_PAYLOAD = 2048` 拒绝超大帧 |
| 版本号校验 | ✅ | 未知版本返回 `UnknownVersion` 错误 |
| 消息类型校验 | ✅ | 未知类型返回 `UnknownType` 错误 |
| IP 包头解析 | ✅ | 长度 ≥ 20 字节 + IPv4 版本检查 |

### 内存安全

| 检查项 | 状态 | 说明 |
|--------|:----:|------|
| unsafe 代码 | ✅ | `workspace.lints.rust.unsafe_code = "deny"` |
| 数组边界 | ✅ | `copy_from_slice` 固定大小，编译器边界检查 |
| 堆分配 | ✅ | `Vec` 分配在 `MAX_PAYLOAD` 限制内 |
| 整数溢出 | ✅ | `wrapping_add` 用于 nonce 计数器 |

### 网络

| 检查项 | 状态 | 说明 |
|--------|:----:|------|
| UDP 包大小限制 | ✅ | 最大 65536 字节缓冲区 |
| DoS 保护 (中继) | ⚠️ | 缺少速率限制（计划 M4 中期） |
| 重放保护 | ⚠️ | Nonce 递增但未强制单调性检查（计划 M4 中期） |
| TLS 控制面 | ⚠️ | HTTP 明文（生产环境需 TLS/反向代理） |

### 平台

| 检查项 | 状态 | 说明 |
|--------|:----:|------|
| TUN 权限 | ✅ | Linux: CAP_NET_ADMIN; macOS: NetworkExtension; Windows: Wintun |
| 私钥存储 | ⚠️ | M1 阶段明文存储（M4 计划接入 Keychain/Keystore/DPAPI） |
| 日志清理 | ✅ | 日志不含私钥/Token/业务流量 |

---

## 三、已知风险与缓解

### 高风险

| 风险 | 影响 | 缓解 | 计划 |
|------|------|------|------|
| 私钥明文存储 | 设备失窃 → 私钥泄露 | 系统 Keychain/Keystore | M4 |
| 控制面明文 | MITM → 配置篡改 | TLS + 证书固定 | M4 |
| Nonce 重放 | 旧包重放 | 单调性检查 + 窗口 | M4 中期 |

### 中风险

| 风险 | 影响 | 缓解 | 计划 |
|------|------|------|------|
| 中继 DoS | 服务中断 | 速率限制 | M4 中期 |
| 大规模负载 | 内存耗尽 | `MAX_PAYLOAD` 已限制 | 已实施 |

### 低风险

| 风险 | 影响 | 缓解 | 计划 |
|------|------|------|------|
| 流量分析 | 隐私泄露 | 中继仅见密文+帧头 | 持续评估 |
| `unsafe` FFI | 内存损坏 | JNI/C ABI 边界已最小化 | 持续审计 |

---

## 四、模糊测试最佳实践

### 持续集成

建议将模糊测试加入 CI 作为定期任务（非每次 PR）：

```yaml
# .github/workflows/fuzz.yml
name: Fuzz
on:
  schedule:
    - cron: '0 6 * * 1'  # 每周一 UTC 6:00

jobs:
  fuzz:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@nightly
      - run: cargo install cargo-fuzz
      - run: cargo fuzz run fuzz_frame_decode -- -max_total_time=600
      - run: cargo fuzz run fuzz_stun_parse -- -max_total_time=600
```

### 语料库管理

```bash
# 保存有趣的语料（发现新代码路径的输入）
cp fuzz/artifacts/fuzz_frame_decode/* fuzz/corpus/fuzz_frame_decode/

# 最小化 crash 用例
cargo fuzz tmin fuzz_frame_decode crash-xxxxx
```

### 覆盖率报告

```bash
cargo fuzz coverage fuzz_frame_decode
# 生成覆盖率报告:
# target/x86_64-unknown-linux-gnu/coverage/.../index.html
```

---

## 五、审计结论

ConnectAlso 在 M0-M3 开发阶段已实施以下安全措施：

1. ✅ 密码学使用标准库 (`x25519-dalek`, `chacha20poly1305`)
2. ✅ 所有外部输入有边界检查和错误处理
3. ✅ 工作区级别 `unsafe_code = "deny"`
4. ✅ 3 个模糊测试目标覆盖关键解析路径
5. ✅ 依赖许可证审计 (`cargo-deny`)
6. ⚠️ 生产部署需添加 TLS 和私钥安全存储
7. ⚠️ 中继需添加速率限制和重放保护

**总体评级**: M4 公测前需完成 TLS + 私钥存储 + 速率限制三项高优先级任务。
