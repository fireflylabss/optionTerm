#!/usr/bin/env bash
# Bump packaging/aur for a tagged release (no makepkg required — CI-friendly).
# Usage: ./packaging/aur/bump.sh v0.2.1
#        ./packaging/aur/bump.sh 0.2.1
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
PKGBUILD="$ROOT/packaging/aur/PKGBUILD"
SRCINFO="$ROOT/packaging/aur/.SRCINFO"

TAG="${1:?usage: bump.sh <version|vVersion>}"
VER="${TAG#v}"
TARBALL_URL="https://github.com/fireflylabss/optionTerm/archive/refs/tags/v${VER}.tar.gz"

echo "==> waiting for $TARBALL_URL"
for _ in $(seq 1 12); do
  if curl -fsI "$TARBALL_URL" >/dev/null 2>&1; then
    break
  fi
  sleep 5
done

echo "==> hashing tarball"
SHA="$(curl -fsSL "$TARBALL_URL" | sha256sum | awk '{print $1}')"
echo "    sha256=$SHA"

echo "==> updating PKGBUILD → $VER"
sed -i "s/^pkgver=.*/pkgver=${VER}/" "$PKGBUILD"
sed -i "s/^pkgrel=.*/pkgrel=1/" "$PKGBUILD"
# Collapse multiline or single-line sha256sums= to one line.
perl -i -0pe "s/sha256sums=\(\s*'[^']*'\s*\)/sha256sums=('${SHA}')/s" "$PKGBUILD"

echo "==> writing .SRCINFO"
cat > "$SRCINFO" <<EOF
pkgbase = optionterm
	pkgdesc = Sidebar-first GTK4 terminal with tiling splits and Adwaita preferences
	pkgver = ${VER}
	pkgrel = 1
	url = https://github.com/fireflylabss/optionTerm
	arch = x86_64
	license = Apache-2.0
	makedepends = cargo
	makedepends = pkgconf
	depends = gcc-libs
	depends = glib2
	depends = glibc
	depends = gtk4
	depends = libadwaita
	depends = cairo
	depends = pango
	depends = vte4
	provides = option-term
	conflicts = option-term
	replaces = option-term
	source = optionterm-${VER}.tar.gz::https://github.com/fireflylabss/optionTerm/archive/refs/tags/v${VER}.tar.gz
	sha256sums = ${SHA}

pkgname = optionterm
EOF

echo "==> done (packaging/aur ready for AUR push)"
