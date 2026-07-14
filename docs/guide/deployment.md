# ConnectAlso 部署指南

面向非开发者用户的完整部署文档。从零开始，5 分钟搭建专属虚拟局域网。

---

## 目录

1. [快速入门（5 分钟）](#快速入门5-分钟)
2. [安装客户端](#安装客户端)
3. [服务端部署](#服务端部署)
4. [连接设备](#连接设备)
5. [验证连接](#验证连接)
6. [常用操作](#常用操作)
7. [故障排查](#故障排查)
8. [配置参考](#配置参考)

---

## 快速入门（5 分钟）

### 准备工作

- 一台有公网 IP 的云服务器（阿里云/腾讯云/AWS 均可，1核1G 够用）
- 需要互联的设备（电脑、NAS、树莓派等）
- 开放服务器端口：**3000** (TCP)、**33478** (UDP)、**3478** (UDP)

### 第一步：部署服务端

在云服务器上执行：

```bash
# 安装 Docker（如已安装跳过）
curl -fsSL https://get.docker.com | bash

# 下载配置文件
wget https://raw.githubusercontent.com/Lightalso/ConnectAlso/main/deploy/docker/docker-compose.yml
wget https://raw.githubusercontent.com/Lightalso/ConnectAlso/main/deploy/docker/.env.example -O .env

# 启动服务
docker compose up -d

# 验证
curl http://localhost:3000/api/v1/health
# 输出: {"status":"ok"}
```

> 如果不想用 Docker，也可以用预编译二进制直接运行，见[手动部署](#手动部署)。

### 第二步：安装客户端

在你的电脑上：

**Windows**:
```powershell
# 下载并安装 MSI
# https://github.com/Lightalso/ConnectAlso/releases/latest
msiexec /i connectalso-0.1.0-x64.msi
```

**macOS**:
```bash
# 下载 PKG 安装包
# https://github.com/Lightalso/ConnectAlso/releases/latest
sudo installer -pkg connectalso-0.1.0.pkg -target /
```

**Linux (Debian/Ubuntu)**:
```bash
wget https://github.com/Lightalso/ConnectAlso/releases/latest/download/connectalso_0.1.0_amd64.deb
sudo dpkg -i connectalso_0.1.0_amd64.deb
```

### 第三步：连接网络

```bash
# 替换 <服务器IP> 为你的云服务器公网 IP
connectalso start \
  --control-url http://<服务器IP>:3000 \
  --stun-server <服务器IP>:3478 \
  --relay-server <服务器IP>:33478 \
  --hostname my-laptop
```

### 第四步：验证

```bash
# 查看连接状态
connectalso status

# 输出示例:
# ConnectAlso Daemon
#   Device ID : 550e8400-e29b-...
#   Virtual IP: 100.64.0.1
#   Uptime    : 5m 30s
#   Peers     : 1
#
#   PEER         VIRTUAL IP     HOSTNAME
#   alice        100.64.0.2     my-nas
```

在另一台设备上重复第三步和第四步，两台设备即可互相访问对方的虚拟 IP。

---

## 安装客户端

### Windows

1. 从 [Releases](https://github.com/Lightalso/ConnectAlso/releases) 下载最新 `.msi` 安装包
2. 双击安装，或命令行静默安装：`msiexec /i connectalso-x64.msi /quiet`
3. 安装完成后，打开 PowerShell 或命令提示符：
   ```powershell
   connectalso status
   ```

**需要 Wintun 驱动**：安装包自动包含。如果 TUN 创建失败，从 https://www.wintun.net 下载 `wintun.dll` 放到 `C:\Windows\System32\`。

### macOS

1. 下载 `.pkg` 安装包
2. 双击安装或：`sudo installer -pkg connectalso.pkg -target /`
3. 打开终端：
   ```bash
   connectalso status
   ```

**权限提示**：首次运行需要允许网络扩展。在「系统设置 → 隐私与安全性」中批准。

### Linux

**Debian/Ubuntu**:
```bash
sudo dpkg -i connectalso_0.1.0_amd64.deb
sudo systemctl enable --now connectalso-daemon
```

**Fedora/RHEL**:
```bash
sudo rpm -i connectalso-0.1.0-1.x86_64.rpm
sudo systemctl enable --now connectalso-daemon
```

**通用 tarball**:
```bash
tar -xzf connectalso-linux-x86_64.tar.gz
sudo cp connectalso-*/bin/* /usr/local/bin/
```

Linux 版本需要 `/dev/net/tun` 支持（大多数发行版内核已包含）。确保当前用户在 `sudo` 组中或有 `CAP_NET_ADMIN` 权限。

---

## 服务端部署

### Docker 部署（推荐）

```bash
# 1. 创建目录
mkdir connectalso-server && cd connectalso-server

# 2. 下载配置
curl -O https://raw.githubusercontent.com/Lightalso/ConnectAlso/main/deploy/docker/docker-compose.yml
curl -O https://raw.githubusercontent.com/Lightalso/ConnectAlso/main/deploy/docker/.env.example
cp .env.example .env

# 3. 编辑配置（可选）
# 修改网络范围、端口等
nano .env

# 4. 启动
docker compose up -d

# 5. 查看日志
docker compose logs -f
```

### 手动部署

如果不使用 Docker，可以直接运行预编译二进制：

```bash
# 下载服务端二进制
wget https://github.com/Lightalso/ConnectAlso/releases/latest/download/connectalso-server-linux-x86_64.tar.gz
tar -xzf connectalso-server-linux-x86_64.tar.gz

# 启动三个服务（各开一个终端或使用 screen/tmux）
./connectalso-control --listen 0.0.0.0:3000 &
./connectalso-relay   --listen 0.0.0.0:33478 &
./connectalso-stun    --listen 0.0.0.0:3478 &
```

### 防火墙配置

确保以下端口在云服务器安全组和系统防火墙中开放：

| 端口 | 协议 | 用途 | 方向 |
|------|------|------|------|
| 3000 | TCP | 控制服务 API | 入站 |
| 33478 | UDP | 数据中继 | 入站 |
| 3478 | UDP | STUN NAT 探测 | 入站 |

**UFW (Ubuntu)**:
```bash
sudo ufw allow 3000/tcp
sudo ufw allow 33478/udp
sudo ufw allow 3478/udp
```

**firewalld (CentOS/RHEL)**:
```bash
sudo firewall-cmd --add-port=3000/tcp --permanent
sudo firewall-cmd --add-port=33478/udp --permanent
sudo firewall-cmd --add-port=3478/udp --permanent
sudo firewall-cmd --reload
```

---

## 连接设备

### 命令行连接

```bash
connectalso start \
  --control-url http://<服务器IP>:3000 \
  --stun-server <服务器IP>:3478 \
  --relay-server <服务器IP>:33478 \
  --hostname <设备名称>
```

### 开机自启

**Linux (systemd)**:
```bash
sudo systemctl enable connectalso-daemon
```

**macOS (LaunchDaemon)**:
```bash
sudo launchctl load /Library/LaunchDaemons/com.connectalso.daemon.plist
```

**Windows**:
```powershell
# 安装为 Windows 服务
sc.exe create ConnectAlsoDaemon binPath= "C:\Program Files\ConnectAlso\connectalso-daemon.exe --control-url http://<IP>:3000" start= auto
sc.exe start ConnectAlsoDaemon
```

### 系统托盘

Windows 和 macOS 支持系统托盘图标，实时显示连接状态：

```bash
connectalso-desktop &
```

- 🟢 绿色 = P2P 直连
- 🟠 橙色 = 中继连接
- ⚫ 灰色 = 未连接

---

## 验证连接

### 基本检查

```bash
# 查看自己的 IP 和在线对等
connectalso status

# 详细状态（含连接路径）
connectalso status --verbose

# 运行诊断
connectalso diag
```

### 网络连通性

```bash
# 假设你的虚拟 IP 是 100.64.0.1，对等是 100.64.0.2
ping 100.64.0.2

# SSH 到对等设备
ssh user@100.64.0.2

# 访问对等的 Web 服务
curl http://100.64.0.2:8080
```

---

## 常用操作

### 设备管理

```bash
# 查看待审批设备
connectalso admin pending

# 审批设备
connectalso admin approve <device-id>

# 撤销设备
connectalso admin revoke <device-id>

# 查看所有设备
connectalso admin peers
```

### 备份和恢复

```bash
# 创建备份
connectalso backup

# 恢复备份
connectalso restore
```

### 升级

```bash
# 1. 停止守护进程
connectalso stop

# 2. 升级服务端
docker compose pull && docker compose up -d

# 3. 升级客户端
sudo dpkg -i connectalso_NEWVERSION_amd64.deb

# 4. 重启
connectalso start --control-url http://<IP>:3000
```

---

## 故障排查

### 守护进程无法启动

```bash
# 检查控制服务是否可达
curl http://<服务器IP>:3000/api/v1/health

# 查看守护进程日志
# Linux: journalctl -u connectalso-daemon -f
# macOS: tail -f /var/log/connectalso-daemon.log
# Windows: 查看 C:\Users\<用户名>\AppData\Local\connectalso\logs\
```

### TUN 设备创建失败

**Linux**: 确保有 `/dev/net/tun` 且用户有权限
```bash
ls -la /dev/net/tun
sudo modprobe tun
```

**macOS**: 确保在「系统设置 → 隐私与安全性」中批准了网络扩展

**Windows**: 确保 Wintun 驱动已安装
```powershell
Get-NetAdapter | Where-Object {$_.InterfaceDescription -like "*Wintun*"}
```

### 设备待审批无法连接

新设备加入时需要管理员审批。运行：
```bash
connectalso admin pending
connectalso admin approve <device-id>
```

### 连接速度慢/走中继

检查是否成功建立 P2P：
```bash
connectalso status --verbose
```
如果显示 `path: relay` 而非 `direct`，说明未能 P2P 直连。常见原因：
- 两端 NAT 类型严格（Symmetric NAT）
- STUN 服务器不可达
- 防火墙阻止 UDP

### 常见错误码

| 错误 | 原因 | 解决 |
|------|------|------|
| `connection refused` | 控制服务未启动 | 检查服务器 docker compose 状态 |
| `device pending approval` | 未审批 | `connectalso admin approve` |
| `TUN creation failed` | 权限不足 | Linux: sudo; macOS: 批准网络扩展 |
| `STUN timeout` | STUN 端口未开 | 检查防火墙 UDP 3478 |
| `relay timeout` | 中继端口未开 | 检查防火墙 UDP 33478 |

---

## 配置参考

### 服务端环境变量

| 变量 | 默认值 | 说明 |
|------|--------|------|
| `CONTROL_PORT` | 3000 | 控制服务 TCP 端口 |
| `RELAY_PORT` | 33478 | 中继 UDP 端口 |
| `STUN_PORT` | 3478 | STUN UDP 端口 |
| `CONTROL_NETWORK` | `100.64.0.0/16` | 虚拟网络 IPv4 地址池 |
| `STALE_TIMEOUT` | 300 | 设备心跳超时（秒） |
| `RUST_LOG` | info | 日志级别 |

### 守护进程参数

| 参数 | 默认值 | 说明 |
|------|--------|------|
| `--control-url` | `http://127.0.0.1:3000` | 控制服务地址 |
| `--stun-server` | `127.0.0.1:3478` | STUN 服务器 |
| `--relay-server` | `127.0.0.1:33478` | 中继服务器 |
| `--hostname` | `unnamed` | 设备名称 |
| `--tun-name` | `connectalso` | TUN 接口名称 |

### 配置文件位置

| 平台 | 路径 |
|------|------|
| Linux | `~/.config/connectalso/config.json` |
| macOS | `~/Library/Application Support/connectalso/config.json` |
| Windows | `%APPDATA%\connectalso\config.json` |

### 日志位置

| 平台 | 路径 |
|------|------|
| Linux | `~/.local/share/connectalso/logs/` |
| macOS | `~/Library/Logs/connectalso/` |
| Windows | `%LOCALAPPDATA%\connectalso\logs\` |

---

> 如有问题，请提交 [GitHub Issue](https://github.com/Lightalso/ConnectAlso/issues)
