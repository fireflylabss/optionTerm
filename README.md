# optionTerm

A GTK4 + libadwaita terminal emulator powered by [libghostty-vt](https://ghostty.org) (Ghostty’s VT engine).

optionTerm gives you a fast, modern terminal with tabs, Ghostty-style tiling splits, a command palette, deep keyboard-driven controls, and its own TOML configuration while still understanding your existing Ghostty config.

## Features

- **Tabs, three ways** — top tab bar, left/right sidebar, or hidden tabs (`window.tabs`).
- **Ghostty-style tiling** — split panes in any direction, directional focus, cycle focus, equalize, resize, and toggle zoom.
- **Command palette** — `Ctrl+Shift+P` to search every action and its shortcut.
- **Stylized cursor** — block, bar, underline, or hollow block; blinking and custom colors.
- **Rich keyboard shortcuts** — new/close tabs, splits, zoom, font zoom, copy/paste, and more.
- **TOML configuration** — own config file at `~/.option/terminal/config.toml`, auto-generated from Ghostty on first run.
- **Preferences & About dialogs** — live theme, tab position, font size, cursor style, and one-click config edit.
- **Resize toast** — shows `cols × rows` while resizing or creating panes.
- **Font fallback** — defaults to `FiraCode Nerd Font` and falls back to `monospace` if unavailable.
- **PTY hardening** — write polling, anti-flood, SIGHUP/SIGKILL cleanup, and title sanitization.

## Requirements

- Rust **1.90+**
- Zig **0.15.x** (required by `libghostty-vt-sys`; 0.16 will not work)
- GTK **4.14+**, libadwaita **1.5+**
- pkg-config, pango, cairo

### Arch / CachyOS

```bash
sudo pacman -S gtk4 libadwaita pango cairo pkgconf
# Make sure zig 0.15.x is on PATH, e.g.:
# export PATH="/tmp/zig151/zig-0.15.2:$PATH"
```

## Install

The easiest way to install optionTerm on your system is the provided install script. It builds a release binary, copies it into your `PATH`, and installs the `.desktop` entry + icons.

```bash
# User install (no sudo) → ~/.local/bin, ~/.local/share/applications
./scripts/install.sh

# System-wide install (needs sudo) → /usr/local/bin, /usr/local/share
sudo ./scripts/install.sh --system

# If Zig 0.15.x is not on PATH, point to it:
./scripts/install.sh --zig /tmp/zig151/zig-0.15.2/zig
```

After installing, launch from the terminal:

```bash
option-term
```

Or from your applications menu / launcher (it appears as **optionTerm**).

## Build & run (development)

```bash
cargo run --release
```

Optional: point at a local Ghostty checkout (must match the crate’s pinned API):

```bash
export GHOSTTY_SOURCE_DIR=/path/to/ghostty
cargo run --release
```

There is also a convenience script that ensures Zig 0.15.2 is on `PATH` for a quick dev run:

```bash
./scripts/run.sh
```

## Desktop integration

`./scripts/install.sh` already installs the `.desktop` entry and icons. If you only want to refresh the desktop files without rebuilding:

```bash
./scripts/install-desktop.sh
```

That copies icon sizes to `~/.local/share/icons/hicolor/` and the `.desktop` file to `~/.local/share/applications/`.

> The public release uses the generic `utilities-terminal` icon; custom icon assets are kept under `assets/` if you want to install them manually.

## Configuration

optionTerm reads `~/.option/terminal/config.toml`. On the first run it is generated from your system Ghostty config (`~/.config/ghostty/config`) and defaults to a left sidebar for tabs.

Example:

```toml
[font]
family = "FiraCode Nerd Font"
size = 13

[window]
tabs = "left"
padding_x = 4
padding_y = 4

[cursor]
style = "block"
blink = true
color = "#ff7b72"
text = "#000000"

[colors]
background = "#0d1117"
foreground = "#e6edf3"
palette = [
  "#000000", "#ff7b72", "#3fb950", "#d29922",
  "#58a6ff", "#f778ba", "#56d4dd", "#b0b8bf",
  "#5a6573", "#ff7b72", "#3fb950", "#d29922",
  "#58a6ff", "#f778ba", "#56d4dd", "#ffffff",
]
```

Use `Ctrl+Shift+P` → “Reload Configuration” or the Preferences dialog to re-apply changes live.

## Keyboard shortcuts

### Tabs

| Action | Shortcut |
|--------|----------|
| New tab | `Ctrl+Shift+T` |
| Close tab | `Ctrl+Shift+W` |
| Next tab | `Ctrl+Tab` / `Ctrl+PageDown` |
| Previous tab | `Ctrl+Shift+Tab` / `Ctrl+PageUp` |

### Tiling

| Action | Shortcut |
|--------|----------|
| Split right | `Ctrl+Shift+O` |
| Split down | `Ctrl+Shift+E` |
| Split left | `Ctrl+Shift+L` |
| Split up | `Ctrl+Shift+U` |
| Focus split left | `Ctrl+Alt+←` |
| Focus split right | `Ctrl+Alt+→` |
| Focus split up | `Ctrl+Alt+↑` |
| Focus split down | `Ctrl+Alt+↓` |
| Previous split | `Ctrl+Super+[` |
| Next split | `Ctrl+Super+]` |
| Equalize splits | (Command palette) |
| Toggle split zoom | `Ctrl+Shift+Enter` |

### Edit & view

| Action | Shortcut |
|--------|----------|
| Copy | `Ctrl+Shift+C` |
| Paste | `Ctrl+Shift+V` |
| Select all | `Ctrl+Shift+A` |
| Increase font size | `Ctrl++` |
| Decrease font size | `Ctrl+-` |
| Reset font size | `Ctrl+0` |
| Command palette | `Ctrl+Shift+P` |
| Preferences | `Ctrl+,` |
| Reload configuration | (Command palette) |
| Quit | `Ctrl+Shift+Q` |

## Notes

- optionTerm is a single-instance `GApplication` (`labs.firefly.optionTerm`).
- A benign `AdwTabBox reported min width -6` warning may appear on startup (known libadwaita behavior).
- Requires **Zig 0.15.2** at build time; `libghostty-vt-sys` does not yet support Zig 0.16.

## License

[Apache-2.0](LICENSE)

## Links

- Repository: https://github.com/fireflylabss/optionTerm
- Issues: https://github.com/fireflylabss/optionTerm/issues
