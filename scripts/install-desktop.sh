#!/usr/bin/env bash
# Install the optionTerm icons + .desktop entry for the current user.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP_ID="labs.firefly.optionTerm"
ICONS="$HOME/.local/share/icons/hicolor"

for s in 16 24 32 48 64 128 256 512; do
  mkdir -p "$ICONS/${s}x${s}/apps"
  magick "$ROOT/assets/default.png" -resize "${s}x${s}" "$ICONS/${s}x${s}/apps/$APP_ID.png"
done

mkdir -p "$ICONS/symbolic/apps"
cp "$ROOT/assets/option-term-symbol.svg" "$ICONS/symbolic/apps/$APP_ID-symbolic.svg"

# A stale cache without an index makes GTK miss the icons entirely.
rm -f "$ICONS/icon-theme.cache"

mkdir -p "$HOME/.local/share/applications"
cp "$ROOT/packaging/$APP_ID.desktop" "$HOME/.local/share/applications/"
update-desktop-database "$HOME/.local/share/applications" 2>/dev/null || true

echo "installed: icons ($ICONS) + desktop entry ($APP_ID.desktop)"
