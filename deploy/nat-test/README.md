# ConnectAlso NAT 穿越测试环境

基于 Docker Compose 的 NAT 穿透测试基础设施。

## 拓扑

```
                    ┌─────────────────────────────┐
                    │     Public Network           │
                    │     172.30.0.0/24            │
                    │                              │
                    │  ┌──────┐    ┌──────┐        │
                    │  │ STUN │    │Relay │        │
                    │  │ .10  │    │ .20  │        │
                    │  └──────┘    └──────┘        │
                    │                              │
                    │  ┌──────────┐ ┌──────────┐   │
                    │  │Gateway A │ │Gateway B │   │
                    │  │  .100    │ │  .200    │   │
                    │  │Port Rstr.│ │Symmetric │   │
                    │  └────┬─────┘ └────┬─────┘   │
                    └───────┼────────────┼─────────┘
                            │            │
         ┌──────────────────┼──┐    ┌────┼──────────────┐
         │  Net A (10.0.1.0/24)│    │ Net B (10.0.2.0/24)│
         │                     │    │                    │
         │  ┌────────┐         │    │    ┌────────┐     │
         │  │ Peer A │         │    │    │ Peer B │     │
         │  │ .10    │         │    │    │  .10   │     │
         │  └────────┘         │    │    └────────┘     │
         └─────────────────────┘    └───────────────────┘
```

## NAT 类型配置

| 网关 | NAT 类型 | 脚本 | P2P 可打洞 |
|------|----------|------|------------|
| Gateway A | Port Restricted Cone | `gateway-a.sh` | ✅ 是 |
| Gateway B | Symmetric | `gateway-b.sh` | ❌ 否 (需中继) |
| 备用 | Full Cone | `gateway-fullcone.sh` | ✅ 极易 |

## 启动

```bash
# 从项目根目录启动
docker compose -f deploy/nat-test/docker-compose.yml up -d

# 查看运行状态
docker compose -f deploy/nat-test/docker-compose.yml ps

# 进入 Peer A
docker exec -it nat-peer-a bash

# 进入 Peer B
docker exec -it nat-peer-b bash
```

## 运行测试

在 Peer A 中:
```bash
# 编译项目 (挂载源码卷时)
cd /app
cargo build -p connectalso-nat

# 测试 STUN 发现
# STUN 服务器在 172.30.0.10:3478
```

在 Peer B 中:
```bash
# 同上
```

## 网络验证

```bash
# 从 Peer A 测试到 Peer B 的连通性
docker exec nat-peer-a ping -c 3 10.0.2.10    # 内部不互通 ✓

# 从 Peer A 测试到公网 STUN 服务器
docker exec nat-peer-a ping -c 3 172.30.0.10  # 通过 NAT 可达 ✓

# 在 Gateway A 上查看 NAT 表
docker exec nat-gateway-a iptables -t nat -L -n -v
docker exec nat-gateway-a conntrack -L
```

## 测试场景

### 场景 1: STUN 地址发现
```
Peer A → Gateway A (SNAT) → STUN Server → 返回公网地址
```
验证: STUN 返回的地址 ≠ Peer A 的本地地址

### 场景 2: P2P Hole Punching (Port Restricted ↔ Port Restricted)
```
Peer A → Gateway A → Internet → Gateway B → Peer B (打洞包)
Peer B → Gateway B → Internet → Gateway A → Peer A (打洞包)
```
预期: Port Restricted Cone 之间可以通过同时打洞建立 P2P

### 场景 3: 中继降级 (Port Restricted ↔ Symmetric)
```
Peer A → Gateway A → Relay → Gateway B → Peer B
```
预期: Symmetric NAT 无法 P2P 打洞，必须通过中继

### 场景 4: 路径恢复
```
1. 建立 P2P 直连
2. 在 Gateway A 上模拟故障: iptables -A FORWARD -j DROP
3. 验证自动降级到中继
4. 恢复: iptables -D FORWARD -j DROP
5. 验证自动恢复直连
```

## 停止

```bash
docker compose -f deploy/nat-test/docker-compose.yml down
```
