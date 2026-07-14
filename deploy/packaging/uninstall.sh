#!/bin/bash
# ConnectAlso — Uninstall, Upgrade & Recovery
# ===========================================
#
# This script handles clean uninstall, version upgrade (preserving config),
# and recovery (reinstalling after system restore).
#
# Usage:
#   sudo ./uninstall.sh                    # Full uninstall
#   sudo ./uninstall.sh --upgrade 0.2.0    # Upgrade preserving config
#   sudo ./uninstall.sh --recover          # Recover from backup

set -e

ACTION="${1:-uninstall}"
VERSION="${2:-}"
BACKUP_DIR="${HOME}/.connectalso-backup"
CONFIG_DIR="${HOME}/.config/connectalso"

echo -e "\033[36mConnectAlso — Uninstall/Upgrade/Recovery\033[0m"

detect_os() {
    case "$(uname -s)" in
        Linux*)  echo "linux" ;;
        Darwin*) echo "macos" ;;
        CYGWIN*|MINGW*|MSYS*) echo "windows" ;;
        *)       echo "unknown" ;;
    esac
}

OS=$(detect_os)
echo "Detected OS: $OS"

# ── Backup config ──
backup_config() {
    echo -e "\033[33mBacking up configuration...\033[0m"
    mkdir -p "$BACKUP_DIR"
    if [ -d "$CONFIG_DIR" ]; then
        cp -r "$CONFIG_DIR" "$BACKUP_DIR/config-$(date +%Y%m%d-%H%M%S)"
    fi

    # Also backup control DB if running locally
    if [ -f "/var/lib/connectalso/control.db" ]; then
        cp "/var/lib/connectalso/control.db" "$BACKUP_DIR/control.db.$(date +%Y%m%d-%H%M%S)"
    fi

    echo "Backup saved to: $BACKUP_DIR"
}

# ── Stop services ──
stop_services() {
    echo -e "\033[33mStopping services...\033[0m"
    case "$OS" in
        linux)
            sudo systemctl stop connectalso-daemon 2>/dev/null || true
            sudo systemctl disable connectalso-daemon 2>/dev/null || true
            ;;
        macos)
            sudo launchctl unload /Library/LaunchDaemons/com.connectalso.daemon.plist 2>/dev/null || true
            ;;
        windows)
            sc.exe stop ConnectAlsoDaemon 2>/dev/null || true
            sc.exe delete ConnectAlsoDaemon 2>/dev/null || true
            ;;
    esac
}

# ── Remove files ──
remove_files() {
    local keep_config="${1:-false}"
    echo -e "\033[33mRemoving files...\033[0m"

    case "$OS" in
        linux|macos)
            # Binaries
            for bin in connectalso-control connectalso-relay connectalso-stun connectalso-daemon connectalso connectalso-desktop; do
                sudo rm -f "/usr/local/bin/$bin"
            done

            # Service files
            sudo rm -f /lib/systemd/system/connectalso-daemon.service
            sudo rm -f /Library/LaunchDaemons/com.connectalso.daemon.plist

            # Data (optional — keep on upgrade)
            if [ "$keep_config" = "false" ]; then
                sudo rm -rf /var/lib/connectalso
                sudo rm -rf /var/log/connectalso*
                rm -rf "$CONFIG_DIR"
            fi
            ;;
        windows)
            # Remove from Program Files
            rm -rf "/c/Program Files/ConnectAlso"
            if [ "$keep_config" = "false" ]; then
                rm -rf "$APPDATA/connectalso"
            fi
            ;;
    esac
}

# ── Restore from backup ──
restore_from_backup() {
    echo -e "\033[33mRestoring from backup...\033[0m"
    local latest=$(ls -dt "$BACKUP_DIR"/config-* 2>/dev/null | head -1)
    if [ -n "$latest" ]; then
        mkdir -p "$CONFIG_DIR"
        cp -r "$latest"/* "$CONFIG_DIR/"
        echo "Config restored from: $latest"
    fi

    local latest_db=$(ls -dt "$BACKUP_DIR"/control.db.* 2>/dev/null | head -1)
    if [ -n "$latest_db" ]; then
        sudo mkdir -p /var/lib/connectalso
        sudo cp "$latest_db" /var/lib/connectalso/control.db
        echo "Database restored from: $latest_db"
    fi
}

# ── Cleanup (TUN interfaces, routes) ──
cleanup_network() {
    echo -e "\033[33mCleaning up network interfaces...\033[0m"
    case "$OS" in
        linux)
            sudo ip link del connectalso 2>/dev/null || true
            sudo ip link del utun 2>/dev/null || true
            ;;
        macos)
            # utun interfaces are auto-removed when process exits
            sudo route delete -net 100.64.0.0/10 2>/dev/null || true
            ;;
        windows)
            # Wintun adapter removal
            ;;
    esac
}

# ── Main ──
case "$ACTION" in
    uninstall)
        echo "Full uninstall — all data will be removed."
        echo -n "Continue? [y/N] "; read -r confirm
        if [ "$confirm" != "y" ] && [ "$confirm" != "Y" ]; then exit 0; fi
        backup_config
        stop_services
        cleanup_network
        remove_files "false"
        echo -e "\033[32mConnectAlso fully removed.\033[0m"
        echo "Backups retained at: $BACKUP_DIR"
        ;;

    --upgrade)
        echo "Upgrading to v$VERSION..."
        backup_config
        stop_services
        remove_files "true"  # Keep config
        cleanup_network
        echo -e "\033[33mOld version removed (config preserved).\033[0m"
        echo "Install new version: sudo dpkg -i connectalso_${VERSION}_amd64.deb"
        ;;

    --recover)
        echo "Recovering from backup..."
        restore_from_backup
        echo -e "\033[33mReinstall ConnectAlso to complete recovery.\033[0m"
        ;;

    *)
        echo "Usage: $0 [uninstall|--upgrade <version>|--recover]"
        exit 1
        ;;
esac
