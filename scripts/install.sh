#!/usr/bin/env bash
# Install optionTerm into the system.
# Builds a release binary and installs it, the .desktop entry, and icons.
#
# Usage:
#   ./scripts/install.sh           # user install to ~/.local/bin
#   ./scripts/install.sh --system  # system-wide install to /usr/local/bin (needs sudo)

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP_ID="labs.firefly.optionTerm"
DESKTOP_FILE="$APP_ID.desktop"

# ---------------------------------------------------------------------------
# Parse flags
# ---------------------------------------------------------------------------
SYSTEM=false
ZIG_PATH=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --system)
      SYSTEM=true
      shift
      ;;
    --zig)
      ZIG_PATH="$2"
      shift 2
      ;;
    -h|--help)
      echo "Usage: $0 [--system] [--zig /path/to/zig-0.15.x]"
      exit 0
      ;;
    *)
      echo "Unknown option: $1" >&2
      echo "Usage: $0 [--system] [--zig /path/to/zig-0.15.x]" >&2
      exit 1
      ;;
  esac
done

# ---------------------------------------------------------------------------
# Resolve install destinations
# ---------------------------------------------------------------------------
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

# ---------------------------------------------------------------------------
# Find Zig 0.15.x
# ---------------------------------------------------------------------------
find_zig() {
  if [[ -n "$ZIG_PATH" ]]; then
    if [[ -x "$ZIG_PATH" ]]; then
      echo "$ZIG_PATH"
      return
    fi
    echo "error: --zig path is not executable: $ZIG_PATH" >&2
    exit 1
  fi

  # Prefer an explicit /tmp location commonly used for optionTerm builds.
  if [[ -x "/tmp/zig151/zig-0.15.2/zig" ]]; then
    echo "/tmp/zig151/zig-0.15.2/zig"
    return
  fi

  # Otherwise search PATH for zig and check its version.
  if command -v zig >/dev/null 2>&1; then
    local ver
    ver="$(zig version 2>/dev/null | cut -d. -f1-2)"
    if [[ "$ver" == "0.15" ]]; then
      command -v zig
      return
    fi
  fi

  echo "error: Zig 0.15.x not found." >&2
  echo "       Install it or pass --zig /path/to/zig-0.15.2/zig" >&2
  exit 1
}

ZIG="$(find_zig)"
echo "Using Zig: $ZIG ($($ZIG version))"

# ---------------------------------------------------------------------------
# Build release binary
# ---------------------------------------------------------------------------
echo "Building option-term (release)..."
(
  cd "$ROOT"
  PATH="$(dirname "$ZIG"):$PATH" cargo build --release
)

if [[ ! -f "$ROOT/target/release/option-term" ]]; then
  echo "error: build did not produce target/release/option-term" >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# Install binary
# ---------------------------------------------------------------------------
echo "Installing binary to $BIN_DIR..."
mkdir -p "$BIN_DIR"
if $SYSTEM; then
  install -Dm755 "$ROOT/target/release/option-term" "$BIN_DIR/option-term"
else
  install -Dm755 "$ROOT/target/release/option-term" "$BIN_DIR/option-term"
fi

# ---------------------------------------------------------------------------
# Install .desktop entry
# ---------------------------------------------------------------------------
echo "Installing .desktop entry to $APP_DIR..."
mkdir -p "$APP_DIR"
install -Dm644 "$ROOT/packaging/$DESKTOP_FILE" "$APP_DIR/$DESKTOP_FILE"

# ---------------------------------------------------------------------------
# Install icons (default generic icon + optional custom assets)
# ---------------------------------------------------------------------------
echo "Installing icons to $ICONS_DIR..."
for s in 16 24 32 48 64 128 256 512; do
  mkdir -p "$ICONS_DIR/${s}x${s}/apps"
  # The generic public release uses the utilities-terminal icon from the theme,
  # but we still ship custom icon assets for users that want them.
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

# Remove stale icon cache so GTK picks up the new icons.
rm -f "$ICONS_DIR/icon-theme.cache"

# ---------------------------------------------------------------------------
# Update desktop database
# ---------------------------------------------------------------------------
if command -v update-desktop-database >/dev/null 2>&1; then
  echo "Updating desktop database..."
  update-desktop-database "$APP_DIR" 2>/dev/null || true
fi

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
echo ""
echo "optionTerm installed."
echo "  binary: $BIN_DIR/option-term"
echo "  desktop: $APP_DIR/$DESKTOP_FILE"
echo "  icons: $ICONS_DIR"
echo ""

if ! $SYSTEM; then
  if [[ ":$PATH:" != *":$BIN_DIR:"* ]]; then
    echo "Tip: $BIN_DIR is not on your PATH. Add this to your shell config:"
    echo "  export PATH=\"$BIN_DIR:\$PATH\""
  fi
  echo "Launch with: option-term"
else
  echo "Launch with: option-term (or from your applications menu)"
fi
