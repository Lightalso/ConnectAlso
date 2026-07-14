#!/bin/bash
# NAT Gateway A — Port Restricted Cone NAT
#
# 模拟大多数家用路由器的 NAT 行为:
#   - 出站连接自动创建端口映射 (SNAT MASQUERADE)
#   - 入站仅允许已建立连接的回包 (conntrack)
#   - 外部 IP:Port 必须匹配才能回包 (Port Restricted)

set -e

echo "[gateway-a] Setting up Port Restricted Cone NAT..."

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
# 这是 Port Restricted 的关键: conntrack 精确匹配回包
iptables -A FORWARD -i eth0 -o eth1 \
    -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT

# NAT: 出站流量做源地址转换
iptables -t nat -A POSTROUTING -o eth0 -j MASQUERADE

echo "[gateway-a] Port Restricted Cone NAT activated"
echo "[gateway-a] Internal: eth1, External: eth0"
