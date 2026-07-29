#!/usr/bin/env python3
"""Deterministic terminal workloads for scripts/bench-render.sh.

Runs as the terminal's child, so it sizes itself from the PTY and every mode is
reproducible: no RNG, no clock-derived content.

Modes:
  static  full-screen redraw at a fixed pace. One write -> one repaint, so the
          per-frame numbers are `Session::paint` on a full grid.
  scroll  paced scrolling, the `cat`-like case, still slow enough to repaint.
  flood   unpaced dump. Measures throughput, not paint: the PTY source starves
          the frame clock, so very few frames happen.
"""

import argparse
import shutil
import sys
import time

# No spaces: the cell loop skips them, so a dense line is the worst case.
GLYPHS = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789#@$%&*+=~^<>/|"


def line(row: int, cols: int) -> str:
    n = len(GLYPHS)
    return "".join(GLYPHS[(row * 31 + col * 7) % n] for col in range(cols))


def styled(text: str, row: int) -> str:
    """A colour change every 8 cells: defeats run batching, exercises fg/bg."""
    out = []
    for col in range(0, len(text), 8):
        out.append(f"\033[{31 + ((row + col) % 7)}m{text[col:col + 8]}")
    return "".join(out) + "\033[0m"


def body(cols: int, rows: int, ansi: bool, offset: int) -> str:
    # The last row stops one cell short so a full-width write cannot scroll.
    lines = []
    for r in range(rows):
        width = cols if r < rows - 1 else cols - 1
        text = line(r + offset, width)
        lines.append(styled(text, r + offset) if ansi else text)
    return "\r\n".join(lines)


def pace(started: float, period: float) -> None:
    remaining = period - (time.monotonic() - started)
    if remaining > 0:
        time.sleep(remaining)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("mode", choices=("static", "scroll", "flood"))
    ap.add_argument("--frames", type=int, default=150)
    ap.add_argument("--period", type=float, default=0.033, help="seconds per frame")
    ap.add_argument("--ansi", action="store_true")
    ap.add_argument("--settle", type=float, default=1.5)
    args = ap.parse_args()

    # The window is still reaching its final size right after mapping; measuring
    # through the resize would mix grids of different sizes into one window.
    time.sleep(args.settle)
    cols, rows = shutil.get_terminal_size((80, 24))
    out = sys.stdout

    if args.mode == "static":
        out.write("\033[2J")
        for i in range(args.frames):
            started = time.monotonic()
            out.write("\033[H" + body(cols, rows, args.ansi, i))
            out.flush()
            pace(started, args.period)
    elif args.mode == "scroll":
        for i in range(args.frames):
            started = time.monotonic()
            # A screenful per tick: the same amount of new text as `static`,
            # but every row moves.
            out.write(body(cols, rows, args.ansi, i * rows) + "\r\n")
            out.flush()
            pace(started, args.period)
    else:
        blob = "\r\n".join(
            body(cols, rows, args.ansi, i * rows) for i in range(32)
        ) + "\r\n"
        for _ in range(args.frames):
            out.write(blob)
        out.flush()

    # Let the last frame land before the shell exits and the pane closes.
    time.sleep(0.5)
    return 0


if __name__ == "__main__":
    sys.exit(main())
