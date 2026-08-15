#!/usr/bin/env bash
# Build the patched VTE (kitty graphics protocol) into ./vte-dist.
#
# The stock vte4 from crates.io links against the system libvte, which has no
# kitty graphics protocol support. This script compiles VTE 0.84.1 with
# vte-fork/patches/kitty-graphics.patch applied, installs it under vte-dist/,
# and writes .cargo/config.toml so pkg-config and the rpath point at it.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

VTE_VERSION="0.84.1"
VTE_URL="https://download.gnome.org/sources/vte/0.84/vte-${VTE_VERSION}.tar.xz"
PATCH="vte-fork/patches/kitty-graphics.patch"
PREFIX="$ROOT/vte-dist"
BUILD_DIR="$(mktemp -d)"
trap 'rm -rf "$BUILD_DIR"' EXIT

echo "==> downloading VTE ${VTE_VERSION}"
curl -fL --retry 3 -o "$BUILD_DIR/vte.tar.xz" "$VTE_URL"
tar xf "$BUILD_DIR/vte.tar.xz" -C "$BUILD_DIR"
SRC="$BUILD_DIR/vte-${VTE_VERSION}"

echo "==> applying kitty graphics patch"
patch -p1 -d "$SRC" < "$PATCH"

echo "==> configuring meson"
meson setup "$SRC/build" "$SRC" \
    --prefix="$PREFIX" \
    --libdir=lib \
    -Dgtk3=false -Dgtk4=true \
    -Ddocs=false -Dgir=false -Dvapi=false \
    -D_systemd=false

echo "==> building and installing"
ninja -C "$SRC/build"
ninja -C "$SRC/build" install

echo "==> writing .cargo/config.toml"
mkdir -p "$ROOT/.cargo"
cat > "$ROOT/.cargo/config.toml" <<EOF
[env]
# Use the patched VTE build (vte-fork) that adds the kitty graphics protocol
# instead of the system vte4. Run scripts/build-vte.sh first to produce
# vte-dist/. The absolute path matches that script's PREFIX.
PKG_CONFIG_PATH = "$PREFIX/lib/pkgconfig"

[build]
# Make the binary find the patched libvte in runtime (no LD_LIBRARY_PATH needed).
# The release binary lives in target/release/, test binaries in target/release/deps/,
# so cover both with \$ORIGIN hops back to the project root.
rustflags = [
    "-C", "link-arg=-Wl,-rpath,\$ORIGIN/../../vte-dist/lib",
    "-C", "link-arg=-Wl,-rpath,\$ORIGIN/../../../vte-dist/lib",
]
EOF

echo "==> done. Patched VTE installed at ${PREFIX}"
