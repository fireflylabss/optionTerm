#!/usr/bin/env bash
# Smoke test for optionterm-next (phase 2a).
# Runs the binary for a few seconds and reports the exit code.
# Expected: 124 (timeout kill) with no panic in the log.
set -u
cd "$(dirname "$0")/../../.."
BIN=target/release/optionterm-next
[ -x "$BIN" ] || cargo build -p option-term-gpui --release || exit 1
RUST_LOG=info timeout 8 "./$BIN"
echo "exit=$?"
