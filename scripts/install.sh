#!/usr/bin/env bash
# Install optionTerm into the system.
# Builds a release binary and installs it, the .desktop entry, and icons.
#
# Usage:
#   ./scripts/install.sh           # user install to ~/.local/bin
#   ./scripts/install.sh --system  # system-wide install to /usr/local/bin (needs sudo)

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP_ID="io.option.terminal"
DESKTOP_FILE="$APP_ID.desktop"

SYSTEM=false

while [[ $# -gt 0 ]]; do
  case "$1" in
    --system)
      SYSTEM=true
      shift
      ;;
    -h|--help)
      echo "Usage: $0 [--system]"
      exit 0
      ;;
    *)
      echo "Unknown option: $1" >&2
      echo "Usage: $0 [--system]" >&2
      exit 1
      ;;
  esac
done

if $SYSTEM; then
  PREFIX="${PREFIX:-/usr/local}"
  BIN_DIR="$PREFIX/bin"
  APP_DIR="$PREFIX/share/applications"
  ICONS_DIR="$PREFIX/share/icons/hicolor"
else
  PREFIX="${PREFIX:-$HOME/.local}"
  BIN_DIR="$PREFIX/bin"
  APP_DIR="$HOME/.local/share/applications"
  ICONS_DIR="$HOME/.local/share/icons/hicolor"
fi

echo "Building optionterm (release)..."
(
  cd "$ROOT"
  cargo build --release
)

if [[ ! -f "$ROOT/target/release/optionterm" ]]; then
  echo "error: build did not produce target/release/optionterm" >&2
  exit 1
fi

echo "Installing binary to $BIN_DIR..."
mkdir -p "$BIN_DIR"
install -Dm755 "$ROOT/target/release/optionterm" "$BIN_DIR/optionterm"
ln -sf optionterm "$BIN_DIR/option-term"

echo "Installing .desktop entry to $APP_DIR..."
mkdir -p "$APP_DIR"
install -Dm644 "$ROOT/packaging/$DESKTOP_FILE" "$APP_DIR/$DESKTOP_FILE"

echo "Installing icons to $ICONS_DIR..."
for s in 16 24 32 48 64 128 256 512; do
  mkdir -p "$ICONS_DIR/${s}x${s}/apps"
  if command -v magick >/dev/null 2>&1 && [[ -f "$ROOT/assets/default.png" ]]; then
    magick "$ROOT/assets/default.png" -resize "${s}x${s}" "$ICONS_DIR/${s}x${s}/apps/$APP_ID.png"
  elif [[ -f "$ROOT/assets/option-term-symbol.png" ]]; then
    install -Dm644 "$ROOT/assets/option-term-symbol.png" "$ICONS_DIR/${s}x${s}/apps/$APP_ID.png"
  fi
done

if [[ -f "$ROOT/assets/option-term-symbol.svg" ]]; then
  mkdir -p "$ICONS_DIR/symbolic/apps"
  install -Dm644 "$ROOT/assets/option-term-symbol.svg" "$ICONS_DIR/symbolic/apps/$APP_ID-symbolic.svg"
fi

rm -f "$ICONS_DIR/icon-theme.cache"

if command -v update-desktop-database >/dev/null 2>&1; then
  echo "Updating desktop database..."
  update-desktop-database "$APP_DIR" 2>/dev/null || true
fi

echo ""
echo "optionTerm installed."
echo "  binary: $BIN_DIR/optionterm (with an option-term symlink)"
echo "  desktop: $APP_DIR/$DESKTOP_FILE"
echo "  icons: $ICONS_DIR"
echo ""

if ! $SYSTEM; then
  if [[ ":$PATH:" != *":$BIN_DIR:"* ]]; then
    echo "Tip: $BIN_DIR is not on your PATH. Add this to your shell config:"
    echo "  export PATH=\"$BIN_DIR:\$PATH\""
  fi
  echo "Launch with: optionterm"
else
  echo "Launch with: optionterm (or from your applications menu)"
fi
