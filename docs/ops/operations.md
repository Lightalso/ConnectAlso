# ConnectAlso 运维手册

日常运维操作、监控告警和故障处理指南。

---

## 日常操作

### 查看系统状态

```bash
# 服务端健康
curl http://localhost:3000/api/v1/health

# 在线设备列表
curl http://localhost:3000/api/v1/peers | jq

# IP 池使用情况
curl http://localhost:3000/api/v1/allocations | jq
# {"total":10, "used":3, "free":7, "network":"100.64.0.0/16"}

# 所有设备（含状态）
curl http://localhost:3000/api/v1/admin/peers | jq
```

### 审批新设备

```bash
# 查看待审批
curl http://localhost:3000/api/v1/register/pending | jq

# 审批
curl -X PUT http://localhost:3000/api/v1/register/<device_id>/approve

# 撤销
curl -X PUT http://localhost:3000/api/v1/register/<device_id>/revoke
```

### 备份

```bash
# API 备份
curl -X POST http://localhost:3000/api/v1/backup

# 定时备份 (crontab)
0 3 * * * curl -X POST http://localhost:3000/api/v1/backup
```

### 日志查看

```bash
# Docker
docker compose -f deploy/docker/docker-compose.yml logs -f --tail=100 control

# systemd
journalctl -u connectalso-control -f

# 设置日志级别
# 编辑 .env 文件:
RUST_LOG=debug  # 排查问题
RUST_LOG=warn   # 生产环境
RUST_LOG=error  # 仅错误
```

---

## 监控

### 健康检查端点

| 端点 | 用途 | 告警条件 |
|------|------|----------|
| `GET /api/v1/health` | 服务存活 | 非 200 |
| `GET /api/v1/allocations` | IP 池耗尽 | `free` = 0 |
| `GET /api/v1/peers` | 设备在线 | 设备数骤降 |

### 简单监控脚本

```bash
#!/bin/bash
# check-connectalso.sh — 用于 Nagios / cron

CONTROL_URL="http://localhost:3000"

# 健康检查
if ! curl -sf "${CONTROL_URL}/api/v1/health" > /dev/null; then
    echo "CRITICAL: Control service down"
    exit 2
fi

# IP 池检查
FREE=$(curl -sf "${CONTROL_URL}/api/v1/allocations" | jq -r '.free')
if [ "$FREE" -lt 5 ]; then
    echo "WARNING: Only $FREE IPs remaining"
    exit 1
fi

echo "OK: Service healthy, $FREE IPs free"
exit 0
```

### 关键指标

| 指标 | 正常范围 | 告警阈值 |
|------|----------|----------|
| 控制服务响应时间 | < 50ms | > 500ms |
| IP 池使用率 | < 80% | > 90% |
| 中继 UDP 丢包率 | < 0.1% | > 1% |
| 设备离线数 | 波动 | > 50% 设备同时离线 |
| 数据库大小 | 随设备增长 | > 1GB（考虑清理） |

---

## 告警配置

### Prometheus + Alertmanager（推荐）

```yaml
# prometheus.yml
scrape_configs:
  - job_name: connectalso
    metrics_path: /api/v1/health
    static_configs:
      - targets: ['localhost:3000']
```

### Uptime Kuma

添加 HTTP(s) 监控:
- URL: `http://your-server:3000/api/v1/health`
- 间隔: 60s
- 重试: 3 次

---

## 故障处理

### 服务无法启动

```bash
# 1. 检查端口占用
ss -tlnp | grep -E '3000|33478|3478'

# 2. 检查数据库完整性
sqlite3 /var/lib/connectalso/control.db "PRAGMA integrity_check;"

# 3. 检查磁盘空间
df -h /var/lib/connectalso/

# 4. 查看详细日志
docker compose -f deploy/docker/docker-compose.yml logs control
```

### 设备无法连接

```bash
# 1. 检查设备是否在线
curl http://localhost:3000/api/v1/peers | jq '.peers[] | select(.hostname=="alice")'

# 2. 检查设备是否待审批
curl http://localhost:3000/api/v1/register/pending | jq

# 3. 检查防火墙
sudo ufw status
sudo iptables -L -n | grep -E '3000|33478|3478'

# 4. 客户端诊断
connectalso diag
```

### IP 池耗尽

```bash
# 查看当前分配
curl http://localhost:3000/api/v1/allocations | jq

# 清理过期设备
# 自动: stale_timeout 配置项
# 手动: 撤销未使用的设备
curl -X PUT http://localhost:3000/api/v1/register/<id>/revoke

# 扩容: 修改 CONTROL_NETWORK 环境变量
# 例如从 /24 扩展到 /16
```

### 性能问题

```bash
# 检查中继延迟（客户端）
connectalso diag
# 查看 relay 延迟和 healthy 状态

# 检查数据库查询慢
# SQLite 无内置慢查询日志，检查文件大小
ls -lh /var/lib/connectalso/control.db

# 增加日志级别排查
RUST_LOG=debug docker compose up -d
```

---

## 定期维护

### 每日

- [ ] 检查健康端点
- [ ] 查看日志异常

### 每周

- [ ] 检查待审批设备
- [ ] 检查 IP 池使用
- [ ] 审核 ACL 规则

### 每月

- [ ] 数据库备份
- [ ] 清理过期日志
- [ ] 检查依赖更新 (`cargo audit`)
- [ ] 评估是否需要升级

### 每季度

- [ ] 安全审计
- [ ] 性能基准测试
- [ ] 灾难恢复演练

---

## 数据库维护

```bash
# 查看数据库大小
ls -lh /var/lib/connectalso/control.db

# 压缩数据库（SQLite VACUUM）
sqlite3 /var/lib/connectalso/control.db "VACUUM;"

# 备份
cp /var/lib/connectalso/control.db /backup/control-$(date +%Y%m%d).db

# 恢复
sudo systemctl stop connectalso-control
cp /backup/control-20260714.db /var/lib/connectalso/control.db
sudo systemctl start connectalso-control
```

---

## 扩容

### 水平扩展

- Control: **单实例**（SQLite，可通过 PostgreSQL 实现多实例）
- Relay: **可水平扩展**（无状态，前面加 UDP 负载均衡）
- STUN: **可水平扩展**（无状态，DNS 轮询或 Anycast）

### 多区域中继

```bash
# 服务端：启动多个中继实例
docker compose up -d --scale relay=3

# 客户端：配置多个中继地址
connectalso-daemon \
  --relay us-east.relay.example.com:33478 \
  --relay eu-west.relay.example.com:33478 \
  --relay ap-south.relay.example.com:33478
```
