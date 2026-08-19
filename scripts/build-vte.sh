#!/usr/bin/env bash
# Build the FoxTerminal VTE fork (Kitty graphics protocol) into ./vte-dist.
#
# The stock vte4 Rust crate links against the system libvte, which has no Kitty
# graphics support. FoxTerminal's VTE fork restores VTE's image model and adds
# the protocol implementation used here. Pinning the exact reviewed commit
# keeps local, CI and package builds reproducible.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

VTE_VERSION="0.84.1"
VTE_URL="https://gitlab.com/OrangeFox/misc/foxterminal-vte.git"
VTE_COMMIT="7ed5a96ccc0305b03695ac18af15f96b92805126"
PREFIX="$ROOT/vte-dist"
BUILD_DIR="$(mktemp -d)"
trap 'rm -rf "$BUILD_DIR"' EXIT

echo "==> fetching FoxTerminal VTE ${VTE_VERSION} (${VTE_COMMIT:0:12})"
SRC="$BUILD_DIR/foxterminal-vte"
git init -q "$SRC"
git -C "$SRC" remote add origin "$VTE_URL"
git -C "$SRC" fetch --depth 1 origin "$VTE_COMMIT"
git -C "$SRC" checkout -q --detach FETCH_HEAD
test "$(git -C "$SRC" rev-parse HEAD)" = "$VTE_COMMIT"

# VTE 0.84.1 uses std::out_ptr (C++23, needs GCC 14+/libstdc++14). Prefer
# gcc-14/g++-14 when present (e.g. ubuntu-24.04 CI) and fall back to default.
if command -v gcc-14 >/dev/null 2>&1 && command -v g++-14 >/dev/null 2>&1; then
  echo "==> using gcc-14/g++-14"
  export CC=gcc-14 CXX=g++-14
fi

echo "==> configuring meson"
meson setup "$SRC/build" "$SRC" \
    --prefix="$PREFIX" \
    --libdir=lib \
    -Dgtk3=false -Dgtk4=true \
    -Dsixel=true \
    -Ddocs=false -Dgir=false -Dvapi=false \
    -D_systemd=false

echo "==> building and installing"
ninja -C "$SRC/build"
ninja -C "$SRC/build" install

echo "==> writing .cargo/config.toml"
mkdir -p "$ROOT/.cargo"
cat > "$ROOT/.cargo/config.toml" <<EOF
[env]
# Use the pinned FoxTerminal VTE fork that adds Kitty graphics instead of the
# system vte4. Run scripts/build-vte.sh first to produce vte-dist/. The
# absolute path matches that script's PREFIX.
PKG_CONFIG_PATH = "$PREFIX/lib/pkgconfig"
# The rpath entries live in build.rs (cargo:rustc-link-arg) so they survive
# makepkg/paru, which set RUSTFLAGS and would otherwise clobber a
# [build] rustflags key here.
EOF

echo "==> done. FoxTerminal VTE ${VTE_COMMIT:0:12} installed at ${PREFIX}"
