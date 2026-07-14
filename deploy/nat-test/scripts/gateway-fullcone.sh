#!/bin/bash
# NAT Gateway — Full Cone NAT
#
# 最宽松的 NAT: 一旦内部主机创建了端口映射,
# 任何外部主机都可以向该映射地址发送数据包。

set -e

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
