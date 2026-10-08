#!/usr/bin/env bash
# Builds release artifacts into dist/:
#   mouseshare-<version>-x86_64-linux.tar.gz   (binaries + install script)
#   mouseshare_<version>_amd64.deb
#   SHA256SUMS
# Usage: packaging/build.sh        (run from anywhere)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
ARCH_DEB="amd64"
ARCH_TAR="x86_64-linux"
DIST="$ROOT/dist"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT

echo "==> building release binaries ($VERSION)"
cargo build --release --locked -p mouseshare -p mouseshare-gui

rm -rf "$DIST"
mkdir -p "$DIST"

# ---- common payload ---------------------------------------------------------
PAYLOAD="$STAGE/payload"
install -Dm755 target/release/mouseshare "$PAYLOAD/bin/mouseshare"
install -Dm755 target/release/mouseshare-gui "$PAYLOAD/bin/mouseshare-gui"
install -Dm644 packaging/mouseshare.desktop "$PAYLOAD/share/applications/mouseshare.desktop"
install -Dm644 packaging/mouseshare.svg "$PAYLOAD/share/icons/hicolor/scalable/apps/mouseshare.svg"
install -Dm644 packaging/mouseshare-256.png "$PAYLOAD/share/icons/hicolor/256x256/apps/mouseshare.png"
install -Dm644 LICENSE "$PAYLOAD/share/doc/mouseshare/LICENSE"
install -Dm644 README.md "$PAYLOAD/share/doc/mouseshare/README.md"
install -Dm644 README.zh-CN.md "$PAYLOAD/share/doc/mouseshare/README.zh-CN.md"
install -Dm644 layout.example.toml "$PAYLOAD/share/doc/mouseshare/layout.example.toml"

# ---- tarball ----------------------------------------------------------------
TAR_DIR="$STAGE/mouseshare-$VERSION-$ARCH_TAR"
mkdir -p "$TAR_DIR"
cp -a "$PAYLOAD/." "$TAR_DIR/"
install -m755 packaging/install.sh "$TAR_DIR/install.sh"
tar -C "$STAGE" -czf "$DIST/mouseshare-$VERSION-$ARCH_TAR.tar.gz" "mouseshare-$VERSION-$ARCH_TAR"

# ---- .deb -------------------------------------------------------------------
DEB="$STAGE/deb"
mkdir -p "$DEB/usr" "$DEB/DEBIAN"
cp -a "$PAYLOAD/." "$DEB/usr/"
gzip -9n -c CHANGELOG.md > "$DEB/usr/share/doc/mouseshare/changelog.gz"
chmod 0644 "$DEB/usr/share/doc/mouseshare/changelog.gz"
find "$DEB" -type d -exec chmod 0755 {} +
INSTALLED_KB="$(du -sk "$DEB/usr" | cut -f1)"
cat > "$DEB/DEBIAN/control" <<CONTROL
Package: mouseshare
Version: $VERSION
Section: utils
Priority: optional
Architecture: $ARCH_DEB
Installed-Size: $INSTALLED_KB
Depends: libc6, libgl1, libx11-6, libxcursor1, libxi6, libxkbcommon0, libxkbcommon-x11-0
Recommends: fonts-noto-cjk | fonts-wqy-microhei
Maintainer: PelerYuan <noreply@users.noreply.github.com>
Homepage: https://github.com/PelerYuan/mouseshare
Description: Share one mouse, keyboard and clipboard across your computers
 mouseshare lets the mouse and keyboard of one Linux/X11 computer drive
 others on the same network. Move the cursor off the edge of a screen and it
 appears on the next computer; the clipboard follows. Connections are
 authenticated with a short pairing code and encrypted.
 .
 Includes a graphical app (mouseshare-gui) and a command-line tool
 (mouseshare).
CONTROL
dpkg-deb --root-owner-group -Zxz --build "$DEB" "$DIST/mouseshare_${VERSION}_${ARCH_DEB}.deb" >/dev/null

(cd "$DIST" && sha256sum * > SHA256SUMS)
echo "==> artifacts in $DIST"
ls -lh "$DIST"
