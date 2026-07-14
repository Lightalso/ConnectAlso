# ConnectAlso 回滚指南

当升级出现问题时的紧急回滚流程。

---

## 快速回滚（Docker）

### 情况 1: 服务端升级失败

```bash
# 1. 查看失败日志
docker compose -f deploy/docker/docker-compose.yml logs control

# 2. 恢复到上一个镜像版本
docker compose -f deploy/docker/docker-compose.yml down
export CONNECTALSO_VERSION=0.1.0  # 上一个已知好的版本
docker compose -f deploy/docker/docker-compose.yml up -d

# 3. 恢复数据库（如果 schema 被新版本修改）
docker compose -f deploy/docker/docker-compose.yml exec control \
  curl -X POST http://localhost:3000/api/v1/restore

# 4. 验证
curl http://localhost:3000/api/v1/health
```

### 情况 2: 客户端升级失败

```bash
# Linux: 安装旧版本
sudo dpkg -i connectalso_0.1.0_amd64.deb

# macOS: 安装旧版本 PKG
sudo installer -pkg connectalso-0.1.0.pkg -target /

# Windows: 卸载新版 → 安装旧版 MSI

# 重启守护进程
connectalso start --control-url http://<IP>:3000
```

---

## 数据库回滚

### 从 API 备份恢复

```bash
# 1. 确认存在备份
ls -la connectalso.db.backup

# 2. 触发恢复（控制服务需在运行）
curl -X POST http://localhost:3000/api/v1/restore

# 3. 验证设备列表
curl http://localhost:3000/api/v1/admin/peers
```

### 从文件备份恢复（Docker）

```bash
# 1. 停止控制服务
docker compose -f deploy/docker/docker-compose.yml stop control

# 2. 手动替换数据库文件
docker cp connectalso.db.backup connectalso-control:/var/lib/connectalso/control.db

# 3. 启动控制服务
docker compose -f deploy/docker/docker-compose.yml start control
```

### 从文件备份恢复（手动部署）

```bash
sudo systemctl stop connectalso-control
sudo cp connectalso.db.backup /var/lib/connectalso/control.db
sudo systemctl start connectalso-control
```

---

## 配置回滚

```bash
# 恢复配置文件
cp ~/.config/connectalso/config.json.bak ~/.config/connectalso/config.json

# 重启守护进程以加载旧配置
connectalso stop
connectalso start --control-url http://<IP>:3000
```

---

## 回滚后验证

```bash
# 1. 确认服务版本
connectalso-control --version

# 2. 确认设备在线
connectalso status

# 3. 确认数据库完整
curl http://localhost:3000/api/v1/admin/peers | jq '.peers | length'

# 4. 确认连通性
ping 100.64.0.2
```

---

## 应急预案

### 完全重建（最坏情况）

```bash
# 1. 完全停止
docker compose -f deploy/docker/docker-compose.yml down -v

# 2. 恢复数据库
cp /backup/connectalso.db /var/lib/connectalso/control.db

# 3. 从已知好的版本重新部署
export CONNECTALSO_VERSION=0.1.0
docker compose -f deploy/docker/docker-compose.yml up -d

# 4. 客户端重新连接
connectalso stop
connectalso start --control-url http://<IP>:3000
```

### 紧急联系人

- 项目维护者: [GitHub Issues](https://github.com/Lightalso/ConnectAlso/issues)
- 安全漏洞: 通过 GitHub 私下报告
