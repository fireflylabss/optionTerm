#!/usr/bin/env bash
# Deterministic render baseline for `Session::paint`.
#
# Runs optionTerm against an isolated HOME so config.toml/session.toml are
# generated fresh every time: the window always opens at the app default
# (960x640) and the grid stays comparable between runs. `$SHELL` is pointed at a
# wrapper that drives scripts/bench-workload.py, so no interaction is needed.
#
# Usage: scripts/bench-render.sh [static|scroll|flood] [--ansi]
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="$ROOT/target/release/option-term"
MODE="${1:-static}"
shift || true
OUT="$ROOT/target/bench"

if [[ ! -x "$BIN" ]]; then
  echo "build first: PATH=\"/tmp/zig151/zig-0.15.2:\$PATH\" cargo build --release" >&2
  exit 1
fi
if [[ -z "${WAYLAND_DISPLAY:-}${DISPLAY:-}" ]]; then
  echo "no display: this benchmark needs a graphical session" >&2
  exit 1
fi

mkdir -p "$OUT"

SHIM="$OUT/bench-shell.sh"
cat > "$SHIM" <<EOF
#!/bin/sh
# Stands in for \$SHELL so the workload runs without any interaction.
exec python3 "$ROOT/scripts/bench-workload.py" $MODE $*
EOF
chmod +x "$SHIM"

# GApplication is single-instance: a stale process would swallow the activation
# and the run would measure nothing.
pkill -x option-term 2>/dev/null || true
sleep 0.3

BENCH_HOME="$OUT/home"
rm -rf "$BENCH_HOME"
mkdir -p "$BENCH_HOME"

LOG="$OUT/$MODE.log"
echo "running $MODE $* -> $LOG" >&2
env HOME="$BENCH_HOME" XDG_CONFIG_HOME="$BENCH_HOME/.config" SHELL="$SHIM" \
  OPTION_TERM_PROFILE=1 RUST_LOG=option_term=info \
  timeout 120 "$BIN" >"$LOG" 2>&1 || true

echo
if ! sed 's/\x1b\[[0-9;]*m//g' "$LOG" | grep -E "frames=|p50="; then
  echo "no samples collected; full log:" >&2
  sed 's/\x1b\[[0-9;]*m//g' "$LOG" >&2
  exit 1
fi
