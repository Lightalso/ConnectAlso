# ConnectAlso 升级指南

本文档涵盖服务端和客户端从旧版本升级到新版本的完整流程。

---

## 版本兼容性

| 组件 | 向后兼容 | 说明 |
|------|:--------:|------|
| 控制服务 API | ✅ | 新版本兼容旧客户端 |
| 中继协议 | ✅ | `PROTO_VERSION` 检查，不兼容拒绝 |
| 数据面加密 | ✅ | X25519 + ChaCha20-Poly1305 锁定 |
| 数据库 Schema | ⚠️ | 自动迁移（ALTER TABLE 容错） |
| 配置文件 | ✅ | JSON 向后兼容（`serde(default)`） |

### 兼容性矩阵

| 服务端 / 客户端 | 0.1.x | 0.2.x | 1.0.x |
|----------------|-------|-------|-------|
| 0.1.x | ✅ | ✅ | ✅ |
| 0.2.x | ✅ | ✅ | ✅ |
| 1.0.x | ✅ | ✅ | ✅ |

> 相同主版本内完全兼容。协议版本检查确保不兼容版本不会错误通信。

---

## 服务端升级

### Docker 部署（推荐）

```bash
# 1. 进入部署目录
cd connectalso-server

# 2. 拉取新版本
docker compose -f deploy/docker/docker-compose.yml pull

# 3. 备份数据库
curl -X POST http://localhost:3000/api/v1/backup

# 4. 滚动更新（零停机）
docker compose -f deploy/docker/docker-compose.yml up -d --no-deps control
docker compose -f deploy/docker/docker-compose.yml up -d --no-deps relay
docker compose -f deploy/docker/docker-compose.yml up -d --no-deps stun

# 5. 验证
curl http://localhost:3000/api/v1/health
```

### 手动部署

```bash
# 1. 备份
connectalso backup --control-url http://localhost:3000

# 2. 停止服务
sudo systemctl stop connectalso-control connectalso-relay connectalso-stun

# 3. 替换二进制
sudo cp connectalso-control /usr/local/bin/
sudo cp connectalso-relay   /usr/local/bin/
sudo cp connectalso-stun    /usr/local/bin/

# 4. 启动服务
sudo systemctl start connectalso-control connectalso-relay connectalso-stun

# 5. 验证
curl http://localhost:3000/api/v1/health
```

---

## 客户端升级

### Windows

```powershell
# 1. 停止守护进程
connectalso stop

# 2. 下载新版 MSI
# https://github.com/Lightalso/ConnectAlso/releases/latest

# 3. 升级安装（保留配置）
msiexec /i connectalso-1.0.0-x64.msi REINSTALL=ALL REINSTALLMODE=vomus

# 4. 重启
connectalso start --control-url http://<IP>:3000
```

### macOS

```bash
# 1. 停止
connectalso stop

# 2. 安装新版
sudo installer -pkg connectalso-1.0.0.pkg -target /

# 3. 重启
connectalso start --control-url http://<IP>:3000
```

### Linux

```bash
# 1. 停止
connectalso stop

# 2. 升级包
sudo dpkg -i connectalso_1.0.0_amd64.deb

# 3. 重启
connectalso start --control-url http://<IP>:3000
```

---

## 预升级检查清单

- [ ] 已备份数据库（`POST /api/v1/backup`）
- [ ] 已备份配置文件（`~/.config/connectalso/`）
- [ ] 已确认新版本 Release Notes
- [ ] 已在一个非关键客户端上测试
- [ ] 已准备回滚方案
- [ ] 服务端有足够磁盘空间
- [ ] Docker 版本 ≥ 20.10

---

## 升级后验证

```bash
# 1. 服务健康检查
curl http://localhost:3000/api/v1/health

# 2. 查看已连接设备
curl http://localhost:3000/api/v1/peers

# 3. 客户端状态
connectalso status

# 4. 连通性测试（从客户端 ping 对等虚拟 IP）
ping 100.64.0.2
```
