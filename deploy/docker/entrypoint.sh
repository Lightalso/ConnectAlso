#!/bin/bash
# ConnectAlso Docker entrypoint
# Usage: docker run ... connectalso-control [args]
set -e

BINARY="$1"
shift 2>/dev/null || true

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
