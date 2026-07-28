#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# libghostty-vt 0.2.x builds against Zig 0.15.x
if [[ -x /tmp/zig151/zig-0.15.2/zig ]]; then
  export PATH="/tmp/zig151/zig-0.15.2:$PATH"
fi
cd "$ROOT"
exec cargo run --release "$@"
