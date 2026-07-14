# M0-6: 数据面协议、依赖与许可证方案

> 状态：已完成  
> 日期：2026-07-14  
> 关联路线图：M0 — 立项与技术验证

---

## 一、当前依赖许可证审计

### 直接依赖（workspace 声明的 17 个外部 crate）

| 依赖 | 版本 | 许可证 | GPLv3 兼容 | 维护状态 |
|------|------|--------|------------|----------|
| `tokio` | 1.x | MIT | ✅ | 非常活跃 |
| `serde` | 1.x | MIT OR Apache-2.0 | ✅ | 非常活跃 |
| `serde_json` | 1.x | MIT OR Apache-2.0 | ✅ | 非常活跃 |
| `tracing` | 0.1 | MIT | ✅ | 活跃 |
| `tracing-subscriber` | 0.3 | MIT | ✅ | 活跃 |
| `anyhow` | 1.x | MIT OR Apache-2.0 | ✅ | 稳定 |
| `thiserror` | 2.x | MIT OR Apache-2.0 | ✅ | 活跃 |
| `uuid` | 1.x | MIT OR Apache-2.0 | ✅ | 稳定 |
| `rand` | 0.8 | MIT OR Apache-2.0 | ✅ | 活跃 |
| `clap` | 4.x | MIT OR Apache-2.0 | ✅ | 非常活跃 |
| `axum` | 0.8 | MIT | ✅ | 非常活跃 |
| `tower` | 0.5 | MIT | ✅ | 活跃 |
| `sqlx` | 0.8 | MIT OR Apache-2.0 | ✅ | 活跃 |
| `prost` | 0.13 | Apache-2.0 | ✅ | 活跃 |
| `bytes` | 1.x | MIT | ✅ | 非常活跃 |
| `ipnet` | 2.x | MIT OR Apache-2.0 | ✅ | 稳定 |
| **`tun2`** | **4.x** | **WTFPL** | ⚠️ 罕见 | 活跃 |
| **`x25519-dalek`** | **2.x** | **BSD-3-Clause** | ✅ | 稳定 |
| `chacha20poly1305` | 0.10 | MIT OR Apache-2.0 | ✅ | 稳定 |

### 许可证风险评估

| 风险等级 | 依赖 | 问题 | 建议 |
|----------|------|------|------|
| ⚠️ 中 | `tun2` (WTFPL) | WTFPL 是极不常见的许可证，缺乏法律审查先例。虽然实质等同于公共领域，但在某些司法管辖区可能不被认可为有效许可证。 | 监控替代方案：`boringtun` 的 TUN 抽象层，或自行实现平台 TUN 适配 |
| ✅ 低 | `x25519-dalek` (BSD-3) | BSD-3-Clause 与 GPLv3 兼容。与 MIT/Apache-2.0 组合良好。 | 无需操作 |
| ✅ 无 | 其余所有 | MIT 或 Apache-2.0 均为 GPLv3 兼容许可证。 | 维持现状 |

### 间接/传递依赖注意事项

- `tun2` → `wintun-bindings` (Windows 平台) → Wintun 驱动 (Microsoft 签名的第三方驱动，非 Rust crate)
- `x25519-dalek` → `curve25519-dalek` (BSD-3-Clause, 与父 crate 一致)
- `chacha20poly1305` → `aead`、`chacha20`、`poly1305` (均为 MIT OR Apache-2.0)
- `prost` → `protobuf` 生态 (Apache-2.0)
- `sqlx` → `libsqlite3-sys` (需要在发布时关注动态/静态链接的许可证义务)

---

## 二、数据面协议方案分析

### 方案对比

| 维度 | 方案 A: 继续自定义 | 方案 B: Noise Protocol | 方案 C: WireGuard/boringtun |
|------|--------------------|------------------------|----------------------------|
| **当前进度** | 已实现 X25519 + ChaCha20Poly1305 | 未开始 | 未开始 |
| **协议标准** | 无 | Noise Framework (RFC draft) | WireGuard (RFC, 广泛部署) |
| **Rust 实现** | 自研 (~200行) | `snow` v0.9 (2.8K 行, Apache-2.0/MIT) | `boringtun` v0.7 (5.1K 行, BSD-3) |
| **安全审计** | 未审计 | 框架已审计，具体组合需验证 | 已审计 (Cloudflare + 第三方) |
| **互操作性** | 无 | 有限 (Noise 生态) | 广泛 (WireGuard 生态) |
| **许可证兼容** | ✅ GPLv3 | ✅ (Apache-2.0/MIT) | ✅ (BSD-3-Clause) |
| **前向安全性** | 是 (临时密钥 DH) | 是 | 是 |
| **身份认证** | 未实现 | IK/XX 模式原生支持 | 预共享密钥或外部身份 |
| **密钥轮换** | 未实现 | 会话重握手 | 每 2 分钟 rekey (内置) |
| **代码量** | 最小 | 中等 | 较大 |
| **控制复杂度** | 最大 (需自行设计) | 中等 | 小 (协议已标准化) |

### 详细分析

#### 方案 A: 继续自定义协议 (当前路径)

**当前状态：**
- `connectalso-crypto`: X25519 密钥交换 + ChaCha20Poly1305 AEAD
- `connectalso-tunnel`: 加密 UDP 隧道（发送/接收封装）
- 原生 nonce 管理、双向独立密钥空间

**优势：**
- 最小依赖、最小攻击面
- 完全可控的协议演进
- 与项目中其他 crate 紧密集成

**劣势：**
- 缺乏标准化握手协议
- 需要自行设计身份认证、前向安全性、密钥轮换
- 无法与现有 VPN 生态互操作
- 安全审计成本高

**建议演进方向：** 参考 Noise 框架的 `IK` 握手模式（交互式密钥交换 + 双向静态密钥认证），逐步引入标准化的握手流程。

#### 方案 B: 采用 `snow` crate (Noise Protocol)

**`snow` crate 信息：**
- 版本：0.9.7 (latest stable)
- 许可证：Apache-2.0 OR MIT (GPLv3 兼容)
- 代码量：~2,800 行 Rust
- 依赖：`x25519-dalek`、`chacha20poly1305`、`blake2`、`rand`（与现有依赖一致）

**推荐 Noise 模式：`IK`**
```
IK:
  <- s (对端静态公钥由控制面下发)
  ...
  -> e, es, s, ss (发起方：临时密钥 + 静态密钥 DH)
  <- e, ee, se     (响应方：临时密钥 + 双向 DH)
```
- `s`: 静态密钥（设备长期身份）
- `e`: 临时密钥（每次会话重新生成，保证前向安全性）
- `es`, `ss`, `se`, `ee`: 各种 DH 组合

**优势：**
- 标准化、可审计的握手协议
- 复用现有 X25519/ChaCha20Poly1305 依赖
- 支持前向安全性和双向身份认证
- `snow` 维护良好（最近更新 2025+）

**劣势：**
- 增加一个外部依赖
- 需要将现有自定义协议改造为 Noise 模式
- Nonce 管理需要与 Noise 的 CipherState 对齐

#### 方案 C: 采用 `boringtun` (WireGuard)

**`boringtun` crate 信息：**
- 版本：0.7.1 (May 2026 更新)
- 许可证：BSD-3-Clause (GPLv3 兼容)
- 代码量：~5,100 行 Rust + 64 行 C Header
- 由 Cloudflare 维护

**WireGuard 协议特点：**
- Noise_IKpsk2 握手（IK + 预共享密钥）
- 每 2 分钟自动 rekey
- 内置重放保护、cookie 机制防御 DoS
- 内核级和用户态实现均经过广泛审计

**优势：**
- 最成熟的方案，安全记录优秀
- 自动密钥轮换和重放保护
- 可与标准 WireGuard 客户端互操作
- 大厂维护 (Cloudflare)

**劣势：**
- 依赖较重（5K+ 行代码）
- 握手模式固定（IKpsk2），与 ConnectAlso 控制面驱动模型不完全匹配
- 需要桥接身份/密钥管理系统
- 可能带来不必要的协议复杂性

### 推荐方案

**MVP (M1-M2) 阶段：** 维持方案 A（自定义），但向方案 B 演进

1. **立即**：保持当前 X25519 + ChaCha20-Poly1305 实现不变，专注于完成 M0 剩余验证和 M1 桌面 Alpha
2. **M1 后期**：引入 Noise IK 握手模式，使用 `snow` 或参考其实现自行编码
3. **M3 后评估**：如有需要与 WireGuard 生态互操作，再评估集成 `boringtun`

**理由：**
- 当前自定义实现足够 M0-M1 阶段使用（功能验证 + 桌面 Alpha）
- 三种方案共享底层密码原语（X25519 + ChaCha20-Poly1305），迁移成本可控
- `snow` 与现有依赖高度重叠，是最自然的演进路径
- `boringtun` 作为远期备选，不阻塞当前开发节奏

---

## 三、许可证合规方案

### 项目许可证

ConnectAlso 采用 **GPL-3.0-only** 许可证。

### GPLv3 兼容性矩阵

| 许可证 | GPLv3 兼容 | 说明 |
|--------|------------|------|
| MIT | ✅ | 广泛兼容 |
| Apache-2.0 | ✅ | 与 GPLv3 单向兼容（Apache-2.0 → GPLv3） |
| BSD-3-Clause | ✅ | 兼容 |
| BSD-2-Clause | ✅ | 兼容 |
| WTFPL | ⚠️ | FSF 认为 GPL 兼容，但法律效力不确定 |
| Unlicense | ⚠️ | FSF 认为 GPL 兼容（公共领域奉献） |

### 合规措施

1. **自动检查**：在 CI 中添加 `cargo deny check license` (使用 `cargo-deny`)
2. **禁止许可证**：`cargo-deny` 配置中拒绝 GPL 不兼容的许可证
3. **SBOM 生成**：`cargo cyclonedx` 或 `cargo sbom` 生成软件物料清单
4. **定期审计**：每个 Milestone 开始前审计传递依赖变更
5. **WTFPL 监控**：`tun2` 的 WTFPL 许可证作为例外接受，同时评估替代方案

### 关于 `tun2` 的 WTFPL 许可证

`tun2` v4 采用 WTFPL (Do What The F*ck You Want To Public License)。
- FSF 将其归类为 "lax permissive license"，与 GPL 兼容
- 但在某些司法管辖区可能不被承认为有效许可证
- **缓解措施**：
  - M0-M1 阶段接受此风险（功能验证阶段）
  - M2 后评估 `boringtun` 自带的平台 TUN 抽象，或用条件编译直接调用 OS API
  - `tun2` 的替代方案：Linux 用 `nix::fcntl` + ioctl，macOS 用 `utun` socket，Windows 用 `wintun` FFI

### cargo-deny 配置建议

```toml
# deny.toml
[licenses]
allow = [
    "MIT",
    "Apache-2.0",
    "BSD-3-Clause",
    "BSD-2-Clause",
    "ISC",
    "Unicode-3.0",
]
# 例外：tun2 的 WTFPL
exceptions = [
    { name = "tun2", allow = ["WTFPL"] },
]
```

---

## 四、依赖维护策略

### 版本锁定 vs 浮动

| 依赖类型 | 策略 | 说明 |
|----------|------|------|
| 核心密码 (`x25519-dalek`, `chacha20poly1305`) | 浮动 `"2"`, `"0.10"` | 密码库更新通常包含安全修复 |
| 框架 (`tokio`, `axum`, `tower`) | 浮动 `"1"`, `"0.8"`, `"0.5"` | 主流框架向后兼容性好 |
| 工具 (`clap`, `serde`, `thiserror`) | 浮动 major | 接口稳定 |
| 平台 (`tun2`) | 锁定 `"4"` | API 可能变动，升级前需测试 |

### 安全更新流程

1. `cargo audit` 每周运行（CI 定时任务）
2. 发现漏洞 → 评估影响 → 升级或打补丁
3. 密码相关依赖的安全公告优先级最高

---

## 五、总结

| 决策项 | 结论 |
|--------|------|
| **数据面协议** | MVP 阶段继续自定义 X25519+ChaCha20Poly1305；M1 后期引入 Noise IK 握手 |
| **WireGuard 集成** | M3 后评估 `boringtun`，不作为短期目标 |
| **许可证审计** | 当前全部 GPLv3 兼容；`tun2` (WTFPL) 为可接受例外 |
| **合规工具** | 引入 `cargo-deny` + `cargo audit` 到 CI |
| **依赖策略** | 核心密码库允许 minor 自动升级，平台库锁定 major |
