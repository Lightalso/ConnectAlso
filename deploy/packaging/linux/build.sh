#!/bin/bash
# =============================================================================
# Script: build.sh
# Purpose: Builds ConnectAlso Linux binaries and packages them as DEB, RPM,
#          and a portable tarball (.tar.gz)
# 用途: 构建 ConnectAlso Linux 二进制文件，并打包为 DEB、RPM 和便携 tarball (.tar.gz)
#
# Prerequisites / 前置要求:
#   - Rust 1.85+ (https://rustup.rs)
#   - cargo-deb: cargo install cargo-deb
#   - cargo-rpm: cargo install cargo-generate-rpm (optional / 可选)
#   - dpkg-dev (for DEB), rpm-build (for RPM)
#
# Usage / 用法: ./deploy/packaging/linux/build.sh 0.1.0
# =============================================================================

set -e

VERSION="${1:-0.1.0}"
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
OUTDIR="$ROOT/target/release"
STAGE="$OUTDIR/connectalso-$VERSION"

echo -e "\033[36mConnectAlso Linux Build v$VERSION\033[0m"

# 1. Build / 构建
echo -e "\033[33m[1/5] Building Rust binaries...\033[0m"
cd "$ROOT"
cargo build --release \
    -p connectalso-control \
    -p connectalso-relay \
    -p connectalso-stun \
    -p connectalso-daemon \
    -p connectalso-cli \
    -p connectalso-desktop

# 2. Stage / 暂存文件
echo -e "\033[33m[2/5] Staging files...\033[0m"
rm -rf "$STAGE"
mkdir -p "$STAGE/usr/local/bin"
mkdir -p "$STAGE/etc/connectalso"
mkdir -p "$STAGE/lib/systemd/system"
mkdir -p "$STAGE/usr/share/doc/connectalso"
mkdir -p "$STAGE/DEBIAN"

cp "$OUTDIR/connectalso-control"  "$STAGE/usr/local/bin/"
cp "$OUTDIR/connectalso-relay"    "$STAGE/usr/local/bin/"
cp "$OUTDIR/connectalso-stun"     "$STAGE/usr/local/bin/"
cp "$OUTDIR/connectalso-daemon"   "$STAGE/usr/local/bin/"
cp "$OUTDIR/connectalso"          "$STAGE/usr/local/bin/"
cp "$OUTDIR/connectalso-desktop"  "$STAGE/usr/local/bin/" 2>/dev/null || true

cp "$ROOT/README.md" "$STAGE/usr/share/doc/connectalso/"
cp "$ROOT/LICENSE"   "$STAGE/usr/share/doc/connectalso/"
cp "$ROOT/deploy/systemd/connectalso-daemon.service" "$STAGE/lib/systemd/system/"

# 3. DEB package / DEB 软件包
echo -e "\033[33m[3/5] Building DEB package...\033[0m"

cat > "$STAGE/DEBIAN/control" << EOF
Package: connectalso
Version: $VERSION
Section: net
Priority: optional
Architecture: amd64
Maintainer: ConnectAlso Contributors
Description: Simple, secure cross-platform virtual networking
 ConnectAlso creates a virtual LAN overlay across distributed devices
 with automatic P2P hole punching, end-to-end encryption, and relay fallback.
Depends: libc6 (>= 2.28)
EOF

cat > "$STAGE/DEBIAN/postinst" << 'SCRIPT'
#!/bin/bash
set -e
# Enable and start systemd service
if command -v systemctl >/dev/null 2>&1; then
    systemctl daemon-reload
    systemctl enable connectalso-daemon 2>/dev/null || true
fi
echo "ConnectAlso installed. Configure at /etc/connectalso/"
echo "Start with: systemctl start connectalso-daemon"
SCRIPT

cat > "$STAGE/DEBIAN/prerm" << 'SCRIPT'
#!/bin/bash
set -e
if command -v systemctl >/dev/null 2>&1; then
    systemctl stop connectalso-daemon 2>/dev/null || true
    systemctl disable connectalso-daemon 2>/dev/null || true
fi
SCRIPT

chmod +x "$STAGE/DEBIAN/postinst" "$STAGE/DEBIAN/prerm"

dpkg-deb --build "$STAGE" "$OUTDIR/connectalso_${VERSION}_amd64.deb"

# 4. RPM package / RPM 软件包
echo -e "\033[33m[4/5] Building RPM package...\033[0m"
mkdir -p "$STAGE/rpmbuild/SPECS"

cat > "$STAGE/rpmbuild/SPECS/connectalso.spec" << EOF
Name:           connectalso
Version:        $VERSION
Release:        1%{?dist}
Summary:        Simple, secure cross-platform virtual networking
License:        GPL-3.0-only
URL:            https://github.com/Lightalso/ConnectAlso
BuildArch:      x86_64

%description
ConnectAlso creates a virtual LAN overlay across distributed devices
with automatic P2P hole punching, end-to-end encryption, and relay fallback.

%install
mkdir -p %{buildroot}/usr/local/bin
mkdir -p %{buildroot}/lib/systemd/system
cp -a $STAGE/usr/local/bin/* %{buildroot}/usr/local/bin/
cp -a $STAGE/lib/systemd/system/* %{buildroot}/lib/systemd/system/

%post
systemctl daemon-reload 2>/dev/null || true
systemctl enable connectalso-daemon 2>/dev/null || true

%preun
systemctl stop connectalso-daemon 2>/dev/null || true
systemctl disable connectalso-daemon 2>/dev/null || true

%files
/usr/local/bin/connectalso-control
/usr/local/bin/connectalso-relay
/usr/local/bin/connectalso-stun
/usr/local/bin/connectalso-daemon
/usr/local/bin/connectalso
/lib/systemd/system/connectalso-daemon.service
EOF

if command -v rpmbuild >/dev/null 2>&1; then
    rpmbuild -bb --define "_topdir $STAGE/rpmbuild" "$STAGE/rpmbuild/SPECS/connectalso.spec"
    cp "$STAGE/rpmbuild/RPMS/x86_64/"*.rpm "$OUTDIR/" 2>/dev/null || true
    echo "RPM: $OUTDIR/connectalso-${VERSION}-1.x86_64.rpm"
else
    echo "rpmbuild not found — skipping RPM"
fi

# 5. Tarball / Tarball 压缩包
echo -e "\033[33m[5/5] Creating tarball...\033[0m"
tar -czf "$OUTDIR/connectalso-${VERSION}-linux-x86_64.tar.gz" \
    -C "$STAGE" usr etc lib

echo ""
echo -e "\033[32mBuild complete!\033[0m"
echo "  DEB : $OUTDIR/connectalso_${VERSION}_amd64.deb"
echo "  TGZ : $OUTDIR/connectalso-${VERSION}-linux-x86_64.tar.gz"
