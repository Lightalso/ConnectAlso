# ConnectAlso Docker 自建部署

使用 Docker Compose 一键部署 ConnectAlso 基础设施（控制服务 + 中继 + STUN）。

## 快速开始

```bash
# 1. 克隆仓库
git clone https://github.com/Lightalso/ConnectAlso.git
cd ConnectAlso

# 2. 配置环境变量（可选）
cp deploy/docker/.env.example deploy/docker/.env
# 编辑 deploy/docker/.env 修改端口和网络范围

# 3. 构建并启动
docker compose -f deploy/docker/docker-compose.yml up -d

# 4. 验证
curl http://localhost:3000/api/v1/health
# → {"status":"ok"}
```

## 服务说明

| 服务 | 端口 | 协议 | 用途 |
|------|------|------|------|
| Control | 3000 | TCP | 设备注册、IPv4 分配、对等发现、候选交换 |
| Relay | 33478 | UDP | 加密流量中继（P2P 失败时的降级路径） |
| STUN | 3478 | UDP | NAT 类型探测、公网地址发现 |

## 客户端连接

部署完成后，客户端配置如下：

```bash
# Linux/macOS 客户端
connectalso-daemon \
  --control-url http://<服务器IP>:3000 \
  --stun-server <服务器IP>:3478 \
  --relay-server <服务器IP>:33478 \
  --hostname my-device

# 或使用 CLI 工具
connectalso start \
  --control-url http://<服务器IP>:3000 \
  --stun-server <服务器IP>:3478 \
  --relay-server <服务器IP>:33478 \
  --hostname my-device
```

## 运维命令

```bash
# 查看日志
docker compose -f deploy/docker/docker-compose.yml logs -f

# 查看特定服务日志
docker compose -f deploy/docker/docker-compose.yml logs -f control

# 重启服务
docker compose -f deploy/docker/docker-compose.yml restart relay

# 停止
docker compose -f deploy/docker/docker-compose.yml down

# 停止并删除数据卷
docker compose -f deploy/docker/docker-compose.yml down -v
```

## 数据持久化

- **SQLite 数据库**: Docker volume `connectalso-data`，挂载到 `/var/lib/connectalso/`
- 包含设备注册信息、IPv4 分配表、候选地址
- 删除 volume 会丢失所有设备注册数据

## 防火墙配置

如果使用云服务器，需开放以下端口：

| 端口 | 协议 | 方向 | 说明 |
|------|------|------|------|
| 3000 | TCP | 入站 | 控制服务 API |
| 33478 | UDP | 入站 | 中继流量 |
| 3478 | UDP | 入站 | STUN 查询 |

```bash
# UFW 示例
sudo ufw allow 3000/tcp
sudo ufw allow 33478/udp
sudo ufw allow 3478/udp
```

## 自定义网络范围

默认使用 `100.64.0.0/16`（CGNAT 地址段，65534 个可用 IP）。

如需修改，编辑 `.env` 中的 `CONTROL_NETWORK`：

```env
# 小规模部署（254 个 IP）
CONTROL_NETWORK=10.99.0.0/24

# 中等规模（65534 个 IP）
CONTROL_NETWORK=100.64.0.0/16

# 大规模（约 1600 万个 IP）
CONTROL_NETWORK=10.0.0.0/8
```

## 生产部署建议

1. **反向代理**: 在 Control 服务前放置 nginx/Caddy 提供 TLS
2. **高可用**: Relay 和 STUN 可水平扩展（无状态），Control 需共享数据库
3. **备份**: 定期备份 Docker volume 中的 `control.db`
4. **监控**: 通过 `/api/v1/health` 和 `/api/v1/allocations` 端点监控
5. **日志**: 设置 `RUST_LOG=warn` 减少日志量，或 `RUST_LOG=debug` 排查问题

## 开发模式

```bash
# 构建单个服务并挂载源码（热重载需手动重建）
docker compose -f deploy/docker/docker-compose.yml build control

# 本地开发不使用 Docker，直接 cargo run
cargo run -p connectalso-control
cargo run -p connectalso-relay
cargo run -p connectalso-stun
```
