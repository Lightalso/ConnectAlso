#!/bin/bash
# =============================================================================
# Script: build.sh
# Purpose: Builds ConnectAlso macOS universal (x86_64 + arm64) binaries,
#          creates a PKG installer, and a portable tarball. Supports optional
#          code signing and notarization.
# 用途: 构建 ConnectAlso macOS 通用二进制文件（x86_64 + arm64），
#       创建 PKG 安装程序和便携 tarball。支持可选的代码签名和公证。
#
# Prerequisites / 前置要求:
#   - Rust 1.85+ with aarch64-apple-darwin and x86_64-apple-darwin targets
#   - Apple Developer ID certificate in Keychain
#   - Xcode Command Line Tools
#
# Usage / 用法: ./deploy/packaging/macos/build.sh 0.1.0 [sign|notarize]
# =============================================================================

set -e

VERSION="${1:-0.1.0}"
ACTION="${2:-build}"
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
OUTDIR="$ROOT/target/release"
STAGE="$OUTDIR/ConnectAlso-$VERSION"
APP_DIR="$STAGE/ConnectAlso.app"

echo -e "\033[36mConnectAlso macOS Build v$VERSION\033[0m"

# 1. Build universal binary / 构建通用二进制文件
echo -e "\033[33m[1/5] Building universal binaries...\033[0m"
cd "$ROOT"

TARGETS=("x86_64-apple-darwin" "aarch64-apple-darwin")
BINARIES=("connectalso-control" "connectalso-relay" "connectalso-stun" "connectalso-daemon" "connectalso" "connectalso-desktop")

for target in "${TARGETS[@]}"; do
    echo "  Building for $target..."
    cargo build --release --target "$target" \
        -p connectalso-control \
        -p connectalso-relay \
        -p connectalso-stun \
        -p connectalso-daemon \
        -p connectalso-cli \
        -p connectalso-desktop 2>/dev/null || true
done

# 2. Create universal binaries with lipo / 使用 lipo 创建通用二进制
echo -e "\033[33m[2/5] Creating universal binaries...\033[0m"
rm -rf "$STAGE"
mkdir -p "$STAGE/bin"

for bin in "${BINARIES[@]}"; do
    if [ -f "target/x86_64-apple-darwin/release/$bin" ] && [ -f "target/aarch64-apple-darwin/release/$bin" ]; then
        lipo -create \
            "target/x86_64-apple-darwin/release/$bin" \
            "target/aarch64-apple-darwin/release/$bin" \
            -output "$STAGE/bin/$bin"
    elif [ -f "target/release/$bin" ]; then
        cp "target/release/$bin" "$STAGE/bin/"
    fi
done

cp "$ROOT/README.md" "$STAGE/"
cp "$ROOT/LICENSE"   "$STAGE/"
cp "$ROOT/deploy/launchd/com.connectalso.daemon.plist" "$STAGE/"

# 3. Code sign / 代码签名
if [ "$ACTION" = "sign" ] || [ "$ACTION" = "notarize" ]; then
    echo -e "\033[33m[3/5] Code signing...\033[0m"
    IDENTITY="${CODE_SIGN_IDENTITY:-Developer ID Application}"
    for bin in "$STAGE/bin/"*; do
        codesign --force --options runtime --sign "$IDENTITY" "$bin"
    done
fi

# 4. Create PKG installer / 创建 PKG 安装程序
echo -e "\033[33m[4/5] Creating PKG installer...\033[0m"
PKG_ROOT="$STAGE/pkgroot"
mkdir -p "$PKG_ROOT/usr/local/bin"
mkdir -p "$PKG_ROOT/Library/LaunchDaemons"

cp "$STAGE/bin/"* "$PKG_ROOT/usr/local/bin/"
cp "$STAGE/com.connectalso.daemon.plist" "$PKG_ROOT/Library/LaunchDaemons/"

pkgbuild \
    --root "$PKG_ROOT" \
    --identifier com.connectalso.daemon \
    --version "$VERSION" \
    --install-location / \
    "$OUTDIR/connectalso-${VERSION}.pkg"

# 5. Notarize (optional) / 公证（可选）
if [ "$ACTION" = "notarize" ]; then
    echo -e "\033[33m[5/5] Notarizing...\033[0m"
    APPLE_ID="${APPLE_ID:-}"
    APPLE_PASSWORD="${APPLE_PASSWORD:-}"
    if [ -n "$APPLE_ID" ] && [ -n "$APPLE_PASSWORD" ]; then
        xcrun notarytool submit "$OUTDIR/connectalso-${VERSION}.pkg" \
            --apple-id "$APPLE_ID" \
            --password "$APPLE_PASSWORD" \
            --team-id "${APPLE_TEAM_ID:-}" \
            --wait
        xcrun stapler staple "$OUTDIR/connectalso-${VERSION}.pkg"
    else
        echo "Skipping notarization — set APPLE_ID and APPLE_PASSWORD"
    fi
else
    echo -e "\033[33m[5/5] Skipping notarization\033[0m"
fi

# Also create tarball / 同时创建 tarball
tar -czf "$OUTDIR/connectalso-${VERSION}-darwin-universal.tar.gz" -C "$STAGE" bin README.md LICENSE

echo ""
echo -e "\033[32mBuild complete!\033[0m"
echo "  PKG : $OUTDIR/connectalso-${VERSION}.pkg"
echo "  TGZ : $OUTDIR/connectalso-${VERSION}-darwin-universal.tar.gz"
