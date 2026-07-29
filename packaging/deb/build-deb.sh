#!/usr/bin/env bash
# Build a .deb for optionTerm.
#
# Uses dpkg-deb when available (Debian/Ubuntu, CI) and falls back to plain
# ar+tar otherwise, so the package can also be produced on Arch/Fedora.
#
# Usage: ./packaging/deb/build-deb.sh [--out DIR]
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
APP_ID="io.option.terminal"
PKG="optionterm"
OUT_DIR="$ROOT/dist"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --out) OUT_DIR="$2"; shift 2 ;;
    -h|--help) echo "Usage: $0 [--out DIR]"; exit 0 ;;
    *) echo "Unknown option: $1" >&2; exit 1 ;;
  esac
done

version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)"
[[ -n "$version" ]] || { echo "error: could not read version from Cargo.toml" >&2; exit 1; }

binary="$ROOT/target/release/optionterm"
[[ -x "$binary" ]] || {
  echo "error: $binary not found — run 'cargo build --release' first" >&2
  exit 1
}

arch="amd64"
case "$(uname -m)" in
  x86_64) arch="amd64" ;;
  aarch64) arch="arm64" ;;
esac

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
root="$work/$PKG"

install -Dm755 "$binary" "$root/usr/bin/optionterm"
# The command was called option-term up to 0.1.6; keep it working.
ln -s optionterm "$root/usr/bin/option-term"
install -Dm644 "$ROOT/packaging/$APP_ID.desktop" \
  "$root/usr/share/applications/$APP_ID.desktop"
install -Dm644 "$ROOT/LICENSE" "$root/usr/share/doc/$PKG/copyright"
install -Dm644 "$ROOT/README.md" "$root/usr/share/doc/$PKG/README.md"
install -Dm644 "$ROOT/CHANGELOG.md" "$root/usr/share/doc/$PKG/changelog.md"
gzip -9n "$root/usr/share/doc/$PKG/changelog.md"

# Icons are optional: the .desktop falls back to the theme's terminal icon.
if command -v convert >/dev/null 2>&1 || command -v magick >/dev/null 2>&1; then
  magick_bin="$(command -v magick || command -v convert)"
  for size in 16 24 32 48 64 128 256 512; do
    dir="$root/usr/share/icons/hicolor/${size}x${size}/apps"
    mkdir -p "$dir"
    "$magick_bin" "$ROOT/assets/default.png" -resize "${size}x${size}" "$dir/$APP_ID.png"
  done
fi
if [[ -f "$ROOT/assets/option-term-symbol.svg" ]]; then
  install -Dm644 "$ROOT/assets/option-term-symbol.svg" \
    "$root/usr/share/icons/hicolor/symbolic/apps/$APP_ID-symbolic.svg"
fi

installed_kb="$(du -sk "$root" | cut -f1)"

mkdir -p "$root/DEBIAN"
cat > "$root/DEBIAN/control" <<EOF
Package: $PKG
Version: $version
Section: utils
Priority: optional
Architecture: $arch
Depends: libc6, libgtk-4-1 (>= 4.14), libadwaita-1-0 (>= 1.5), libpango-1.0-0, libcairo2, libglib2.0-0
Conflicts: option-term
Replaces: option-term
Provides: option-term
Maintainer: Firefly Labs <fireflylabss@users.noreply.github.com>
Homepage: https://github.com/fireflylabss/optionTerm
Installed-Size: $installed_kb
Description: GTK4 terminal emulator powered by libghostty-vt
 optionTerm is a GTK4 + libadwaita terminal emulator built on Ghostty's VT
 engine. It supports tabs and Ghostty-style tiling splits, the Kitty
 graphics protocol, a command palette, scrollback search, clickable links
 and its own TOML configuration.
EOF

mkdir -p "$OUT_DIR"
deb="$OUT_DIR/${PKG}_${version}_${arch}.deb"
rm -f "$deb"

if command -v dpkg-deb >/dev/null 2>&1; then
  dpkg-deb --root-owner-group --build "$root" "$deb" >/dev/null
else
  # Minimal ar-based .deb: debian-binary, control.tar.gz, data.tar.gz, in order.
  echo "dpkg-deb not found, assembling the archive with ar/tar" >&2
  ( cd "$root" && echo "2.0" > "$work/debian-binary" )
  tar --create --gzip --owner=root --group=root \
    --file "$work/control.tar.gz" -C "$root/DEBIAN" .
  tar --create --gzip --owner=root --group=root \
    --exclude=./DEBIAN --file "$work/data.tar.gz" -C "$root" .
  ( cd "$work" && ar rc "$deb" debian-binary control.tar.gz data.tar.gz )
fi

echo "built: $deb"
