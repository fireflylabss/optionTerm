# optionTerm AI

Terminal GTK4 + libadwaita powered by [libghostty-vt](https://ghostty.org) (Ghostty’s VT engine).

Uses your existing Ghostty config (`~/.config/ghostty/config`) for fonts, colors, and padding.

## Requirements

- Rust 1.90+
- Zig **0.15.x** (builds vendored `libghostty-vt`)
- GTK 4.14+, libadwaita 1.5+
- pkg-config, pango, cairo

```bash
# Arch / CachyOS
sudo pacman -S gtk4 libadwaita pango cairo pkgconf
# Zig 0.15.x on PATH (not 0.16 — required by libghostty-vt 0.2.x)
```

## Build & run

```bash
cargo run --release
```

Optional: point at a local Ghostty checkout (must match the crate’s pinned API):

```bash
export GHOSTTY_SOURCE_DIR=/path/to/ghostty
cargo run --release
```

## Shortcuts

| Action | Shortcut |
|--------|----------|
| New tab | `Ctrl+Shift+T` |
| Close tab | `Ctrl+Shift+W` |
| Next tab | `Ctrl+Tab` / `Ctrl+PageDown` |
| Previous tab | `Ctrl+Shift+Tab` / `Ctrl+PageUp` |
| Paste | `Ctrl+Shift+V` |
| Quit | `Ctrl+Shift+Q` |

The header bar has a **+** button (`tab-new-symbolic`) that also opens a new tab.
Font, size, colors, and padding come from `~/.config/ghostty/config`.
