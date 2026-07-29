#!/usr/bin/env bash
# Deterministic render baseline for `Session::paint`.
#
# Runs optionTerm against an isolated HOME so config.toml/session.toml are
# generated fresh every time: the window always opens at the app default
# (960x640) and the grid stays comparable between runs. `$SHELL` is pointed at a
# wrapper that replays a generated workload and exits, so no interaction is
# needed.
#
# Usage: scripts/bench-render.sh [dense|ansi|code] [repeats]
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="$ROOT/target/release/option-term"
WORKLOAD="${1:-dense}"
REPEATS="${2:-12}"
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
DATA="$OUT/$WORKLOAD.txt"

# 4000 lines of 200 columns. Deterministic (no RNG, no clock) so the byte stream
# is identical across runs and across machines.
if [[ ! -f "$DATA" ]]; then
  echo "generating $WORKLOAD workload..." >&2
  awk -v kind="$WORKLOAD" 'BEGIN {
    glyphs = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789#@$%&*+=~^<>/\\|"
    n = length(glyphs)
    for (row = 0; row < 4000; row++) {
      line = ""
      for (col = 0; col < 200; col++) {
        # Spaces are skipped by the cell loop, so a dense line is the worst case.
        line = line substr(glyphs, ((row * 31 + col * 7) % n) + 1, 1)
      }
      if (kind == "ansi") {
        # Force a style change every 8 cells: defeats any run batching and
        # exercises the fg/bg colour path.
        out = ""
        for (col = 0; col < 200; col += 8) {
          out = out sprintf("\033[%dm%s", 31 + ((row + col) % 7), substr(line, col + 1, 8))
        }
        print out "\033[0m"
      } else if (kind == "code") {
        # Roughly source-shaped: short lines, lots of spaces, long runs.
        printf "    let %s = compute(%d, \"%s\");\n", substr(line, 1, 6), row, substr(line, 7, 24)
      } else {
        print line
      }
    }
  }' > "$DATA"
fi

SHIM="$OUT/bench-shell.sh"
cat > "$SHIM" <<EOF
#!/bin/sh
# Stands in for \$SHELL so the workload runs without any interaction.
sleep 1.5           # let the window reach its final size before measuring
i=0
while [ \$i -lt $REPEATS ]; do
  cat "$DATA"
  i=\$((i + 1))
done
sleep 0.5
exit
EOF
chmod +x "$SHIM"

# GApplication is single-instance: a stale process would swallow the activation
# and the run would measure nothing.
pkill -x option-term 2>/dev/null || true
sleep 0.3

BENCH_HOME="$OUT/home"
rm -rf "$BENCH_HOME"
mkdir -p "$BENCH_HOME"

LOG="$OUT/$WORKLOAD.log"
echo "running $WORKLOAD x$REPEATS -> $LOG" >&2
env HOME="$BENCH_HOME" XDG_CONFIG_HOME="$BENCH_HOME/.config" SHELL="$SHIM" \
  OPTION_TERM_PROFILE=1 RUST_LOG=option_term=info \
  timeout 120 "$BIN" >"$LOG" 2>&1 || true

echo
grep -E "frames=|p50=" "$LOG" || {
  echo "no samples collected; full log:" >&2
  cat "$LOG" >&2
  exit 1
}
