#!/bin/bash
# =============================================================================
# Script: entrypoint.sh
# Purpose: Docker container entrypoint — dispatches to the correct ConnectAlso
#          binary (control, relay, stun, daemon, or CLI)
# 用途: Docker 容器入口点 — 将启动命令分发到正确的 ConnectAlso 二进制文件
#       （控制端、中继、STUN、守护进程或 CLI）
# =============================================================================
set -e

BINARY="$1"
shift 2>/dev/null || true

# Dispatch to the requested binary / 分发到请求的二进制文件
case "$BINARY" in
    connectalso-control)
        exec /usr/local/bin/connectalso-control "$@"
        ;;
    connectalso-relay)
        exec /usr/local/bin/connectalso-relay "$@"
        ;;
    connectalso-stun)
        exec /usr/local/bin/connectalso-stun "$@"
        ;;
    connectalso-daemon)
        exec /usr/local/bin/connectalso-daemon "$@"
        ;;
    connectalso)
        exec /usr/local/bin/connectalso "$@"
        ;;
    *)
        echo "Usage: docker run ... <binary> [args]"
        echo "Binaries: connectalso-control | connectalso-relay | connectalso-stun | connectalso-daemon | connectalso"
        exit 1
        ;;
esac
