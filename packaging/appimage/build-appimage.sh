#!/usr/bin/env bash
# Build an AppImage for optionTerm.
#
# GTK4/libadwaita apps are painful to bundle by hand (loaders, schemas,
# typelibs), so linuxdeploy's GTK plugin does the heavy lifting and the
# resulting AppImage carries its own GTK stack.
#
# Usage: ./packaging/appimage/build-appimage.sh [--out DIR]
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
APP_ID="io.option.terminal"
OUT_DIR="$ROOT/dist"
TOOLS="${APPIMAGE_TOOL_DIR:-$ROOT/target/appimage-tools}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --out) OUT_DIR="$2"; shift 2 ;;
    -h|--help) echo "Usage: $0 [--out DIR]"; exit 0 ;;
    *) echo "Unknown option: $1" >&2; exit 1 ;;
  esac
done

version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)"
[[ -n "$version" ]] || { echo "error: could not read version from Cargo.toml" >&2; exit 1; }

binary="$ROOT/target/release/option-term"
[[ -x "$binary" ]] || {
  echo "error: $binary not found — run 'cargo build --release' first" >&2
  exit 1
}

# appimagetool and linuxdeploy both need FUSE, which is usually missing in
# containers and CI; extracting them instead works everywhere.
export APPIMAGE_EXTRACT_AND_RUN=1
# linuxdeploy ships an old binutils whose `strip` chokes on the `.relr.dyn`
# sections modern distros emit; the size saving is not worth a failed build.
export NO_STRIP="${NO_STRIP:-1}"

mkdir -p "$TOOLS"
fetch() { # fetch <url> <dest>
  local url="$1" dest="$2"
  if [[ ! -x "$dest" ]]; then
    echo "downloading $(basename "$dest")"
    curl -fsSL -o "$dest" "$url"
    chmod +x "$dest"
  fi
}

base="https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous"
plugin="https://raw.githubusercontent.com/linuxdeploy/linuxdeploy-plugin-gtk/master/linuxdeploy-plugin-gtk.sh"
fetch "$base/linuxdeploy-x86_64.AppImage" "$TOOLS/linuxdeploy"
fetch "$plugin" "$TOOLS/linuxdeploy-plugin-gtk.sh"
fetch \
  "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage" \
  "$TOOLS/appimagetool"

appdir="$ROOT/target/AppDir"
rm -rf "$appdir"
install -Dm755 "$binary" "$appdir/usr/bin/option-term"

# The AppImage must not use the generic theme icon: give it a real one.
desktop="$appdir/usr/share/applications/$APP_ID.desktop"
install -Dm644 "$ROOT/packaging/$APP_ID.desktop" "$desktop"
sed -i "s/^Icon=.*/Icon=$APP_ID/" "$desktop"

for size in 16 24 32 48 64 128 256 512; do
  dir="$appdir/usr/share/icons/hicolor/${size}x${size}/apps"
  mkdir -p "$dir"
  if command -v magick >/dev/null 2>&1; then
    magick "$ROOT/assets/default.png" -resize "${size}x${size}" "$dir/$APP_ID.png"
  elif command -v convert >/dev/null 2>&1; then
    convert "$ROOT/assets/default.png" -resize "${size}x${size}" "$dir/$APP_ID.png"
  else
    cp "$ROOT/assets/default.png" "$dir/$APP_ID.png"
  fi
done

mkdir -p "$OUT_DIR"
export OUTPUT="$OUT_DIR/optionTerm-${version}-x86_64.AppImage"
rm -f "$OUTPUT"

PATH="$TOOLS:$PATH" "$TOOLS/linuxdeploy" \
  --appdir "$appdir" \
  --desktop-file "$desktop" \
  --icon-file "$appdir/usr/share/icons/hicolor/256x256/apps/$APP_ID.png" \
  --plugin gtk \
  --output appimage

echo "built: $OUTPUT"
