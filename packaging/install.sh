#!/usr/bin/env bash
# Installs mouseshare from an extracted release tarball.
#   ./install.sh                 install for the current user (~/.local)
#   sudo ./install.sh --system   install system-wide (/usr/local)
#   ./install.sh --uninstall     remove (add --system if you installed that way)
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PREFIX="$HOME/.local"
UNINSTALL=0
for arg in "$@"; do
  case "$arg" in
    --system) PREFIX="/usr/local" ;;
    --prefix=*) PREFIX="${arg#--prefix=}" ;;
    --uninstall) UNINSTALL=1 ;;
    -h|--help) sed -n '2,6p' "${BASH_SOURCE[0]}"; exit 0 ;;
    *) echo "unknown option: $arg" >&2; exit 2 ;;
  esac
done

FILES=(
  "bin/mouseshare"
  "bin/mouseshare-gui"
  "share/applications/mouseshare.desktop"
  "share/icons/hicolor/scalable/apps/mouseshare.svg"
  "share/icons/hicolor/256x256/apps/mouseshare.png"
)

refresh_caches() {
  command -v update-desktop-database >/dev/null && update-desktop-database "$PREFIX/share/applications" 2>/dev/null || true
  command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t "$PREFIX/share/icons/hicolor" 2>/dev/null || true
}

if [ "$UNINSTALL" = 1 ]; then
  for f in "${FILES[@]}"; do rm -f "$PREFIX/$f"; done
  rm -rf "$PREFIX/share/doc/mouseshare"
  refresh_caches
  echo "mouseshare removed from $PREFIX"
  exit 0
fi

for f in "${FILES[@]}"; do
  mode=644; [[ "$f" == bin/* ]] && mode=755
  install -Dm"$mode" "$HERE/$f" "$PREFIX/$f"
done
install -d "$PREFIX/share/doc/mouseshare"
cp -a "$HERE/share/doc/mouseshare/." "$PREFIX/share/doc/mouseshare/"
# The desktop entry runs `mouseshare-gui` by name; pin it to the install path
# so it works even when $PREFIX/bin is not on the desktop session's PATH.
sed -i "s|^Exec=.*|Exec=$PREFIX/bin/mouseshare-gui|" "$PREFIX/share/applications/mouseshare.desktop"
refresh_caches

echo "mouseshare installed to $PREFIX"
case ":$PATH:" in
  *":$PREFIX/bin:"*) ;;
  *) echo "note: add $PREFIX/bin to your PATH to run it from a terminal" ;;
esac
echo "Open \"mouseshare\" from your application menu, or run: mouseshare-gui"
