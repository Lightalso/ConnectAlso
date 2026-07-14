#!/bin/bash
# =============================================================================
# Script: gateway-fullcone.sh
# Purpose: Configures a Linux host as a Full Cone NAT gateway for NAT
#          traversal testing. This is the most permissive NAT type.
# 用途: 配置 Linux 主机作为全锥形 NAT 网关，用于 NAT 穿透测试。
#       这是最宽松的 NAT 类型。
#
# 模拟的 NAT 行为 / Simulated NAT behavior:
#   最宽松的 NAT: 一旦内部主机创建了端口映射，任何外部主机都可以
#   向该映射地址发送数据包。
#   Most permissive NAT: once a port mapping is created by an internal host,
#   any external host can send packets to that mapped address.
# =============================================================================

set -e

# Apply iptables rules for Full Cone NAT / 应用全锥形 NAT 的 iptables 规则
echo "[gateway] Setting up Full Cone NAT..."

echo 1 > /proc/sys/net/ipv4/ip_forward
echo 1 > /proc/sys/net/ipv4/conf/all/forwarding

iptables -F
iptables -t nat -F
iptables -t mangle -F

iptables -P INPUT ACCEPT
iptables -P FORWARD ACCEPT  # Full Cone: 允许所有入站转发
iptables -P OUTPUT ACCEPT

# NAT: 仅做源地址转换，不做入站过滤
iptables -t nat -A POSTROUTING -o eth0 -j MASQUERADE

echo "[gateway] Full Cone NAT activated"
