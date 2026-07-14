#!/bin/bash
# =============================================================================
# Script: gateway-b.sh
# Purpose: Configures a Linux host as a Symmetric NAT gateway for NAT
#          traversal testing. Simulates the strictest NAT behavior found
#          in enterprise networks.
# 用途: 配置 Linux 主机作为对称 NAT 网关，用于 NAT 穿透测试。
#       模拟企业网络中最严格的 NAT 行为。
#
# 模拟的 NAT 行为 / Simulated NAT behavior:
#   - 每个目标 IP:Port 组合分配不同的源端口 (random MASQUERADE)
#     Different source port assigned per destination IP:Port
#   - 入站仅允许已建立连接的回包
#     Inbound only allows replies to established connections
#   - 外部主机必须使用与出站包完全相同的 IP:Port
#     External host must use the exact same IP:Port as the outbound packet
# =============================================================================

set -e

# Apply iptables rules for Symmetric NAT / 应用对称 NAT 的 iptables 规则
echo "[gateway-b] Setting up Symmetric NAT..."

# 启用转发
echo 1 > /proc/sys/net/ipv4/ip_forward
echo 1 > /proc/sys/net/ipv4/conf/all/forwarding

# 清除已有规则
iptables -F
iptables -t nat -F
iptables -t mangle -F

# 默认策略
iptables -P INPUT ACCEPT
iptables -P FORWARD DROP
iptables -P OUTPUT ACCEPT

# 允许来自内部网络的转发 (出站)
iptables -A FORWARD -i eth1 -o eth0 -j ACCEPT

# 允许已建立/相关的回包 (入站)
iptables -A FORWARD -i eth0 -o eth1 \
    -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT

# NAT: 使用 --random 模拟 Symmetric NAT 的随机端口分配
iptables -t nat -A POSTROUTING -o eth0 -j MASQUERADE --random

echo "[gateway-b] Symmetric NAT activated"
echo "[gateway-b] Internal: eth1, External: eth0"
