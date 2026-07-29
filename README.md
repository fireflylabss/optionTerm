# optionTerm

A GTK4 + libadwaita terminal emulator powered by [libghostty-vt](https://ghostty.org) (Ghostty’s VT engine).

optionTerm gives you a fast, modern terminal with tabs, Ghostty-style tiling splits, a command palette, deep keyboard-driven controls, and its own TOML configuration while still understanding your existing Ghostty config.

## Features

- **Tabs, three ways** — top tab bar, left/right sidebar, or hidden tabs (`window.tabs`), each with a `+` button that doubles as a tab, split and terminal menu.
- **Tab overview** — `F1` (or `Super+Tab`) opens a grid of live tab thumbnails to search and switch.
- **Quick settings** — the `···` menu opens with theme swatches, a font-size stepper showing the current zoom, and the live `columns × rows` of the focused pane.
- **Kitty graphics protocol** — inline images (`timg`, `chafa --format=kitty`, plotting backends, previews), with PNG and raw pixel formats, scaling and z-layers.
- **Accents & input methods** — dead keys and compose sequences work (`´` + `a` → `á`), plus CJK/IBus engines via `GtkIMMulticontext`.
- **Persistent settings** — anything changed from the menus or Preferences is written straight back to `config.toml`.
- **Clear & restart** — wipe the screen and scrollback (`Ctrl+Shift+K`) or respawn the shell in place keeping the split layout (`Ctrl+Shift+R`).
- **Clickable links** — `Ctrl+click` or `Shift+click` opens OSC 8 hyperlinks, bare URLs and existing file paths.
- **Scrollback search** — `Ctrl+Shift+F`, with match counter and wrap-around navigation.
- **Session restore** — reopen tabs, panes and their working directories on start.
- **Translucent background** — `background_opacity`, plus automatic config reload when `config.toml` changes.
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
- `git` (the `libghostty-vt-sys` build script fetches the Ghostty sources)
- GTK **4.14+**, libadwaita **1.5+**
- pkg-config, pango, cairo

### Arch / CachyOS

```bash
sudo pacman -S gtk4 libadwaita pango cairo pkgconf git
# Make sure zig 0.15.x is on PATH, e.g.:
# export PATH="/tmp/zig151/zig-0.15.2:$PATH"
```

## Install

### Arch Linux (AUR)

```bash
yay -S optionterm
# or
paru -S optionterm
```

### Debian / Ubuntu

Download the `.deb` from the [latest release](https://github.com/fireflylabss/optionTerm/releases) and install it:

```bash
sudo apt install ./optionterm_*_amd64.deb
```

Requires GTK 4.14+ and libadwaita 1.5+ (Ubuntu 24.04+, Debian 13+).

### AppImage

Grab the `.AppImage` from the [latest release](https://github.com/fireflylabss/optionTerm/releases), make it executable and run it:

```bash
chmod +x optionTerm-*-x86_64.AppImage
./optionTerm-*-x86_64.AppImage
```

The AppImage bundles GTK 4 and libadwaita, so it runs on desktops that ship
an older GTK. It is built on Ubuntu 24.04 and therefore needs **glibc 2.38+**
(Ubuntu 24.04+, Debian 13+, Fedora 39+); on older systems use the AUR package
or build from source.

### From source

The install script builds a release binary, copies it into your `PATH`, and installs the `.desktop` entry + icons.

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
optionterm
```

Or from your applications menu / launcher (it appears as **optionTerm**).

> The package and command were called `option-term` up to 0.1.6. Installs still
> provide an `option-term` symlink, so existing aliases and scripts keep working.

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
tabs = "left"              # top | left | right | hidden
theme = "system"           # system | light | dark
sidebar_always = false     # show the sidebar even with a single tab
background_opacity = 1.0   # 0.15 .. 1.0, needs a compositor
session_restore = false    # reopen tabs/panes and their cwd on start
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

The file is watched: editing it in any editor re-applies the settings immediately. Settings changed from the menus or Preferences are saved back to it automatically.

With `session_restore = true`, the open tabs, their pane count and each pane's working directory are written to `~/.option/terminal/session.toml` on exit and restored on the next start. Scrollback contents are never stored.

## Keyboard shortcuts

### Tabs

| Action | Shortcut |
|--------|----------|
| New tab | `Ctrl+Shift+T` |
| Close tab | `Ctrl+Shift+W` |
| Next tab | `Ctrl+Tab` / `Ctrl+PageDown` |
| Previous tab | `Ctrl+Shift+Tab` / `Ctrl+PageUp` |
| Rename tab | `F2` (or double-click a sidebar row) |

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
| Clear terminal | `Ctrl+Shift+K` |
| Restart terminal | `Ctrl+Shift+R` |
| Find in scrollback | `Ctrl+Shift+F` |
| Open link under cursor | `Ctrl+Click` or `Shift+Click` |
| Tab overview | `F1` / `Super+Tab` |
| Increase font size | `Ctrl++` |
| Decrease font size | `Ctrl+-` |
| Reset font size | `Ctrl+0` |
| Command palette | `Ctrl+Shift+P` |
| Preferences | `Ctrl+,` |
| Reload configuration | (Command palette) |
| Quit | `Ctrl+Shift+Q` |

## Notes

- optionTerm is a single-instance `GApplication` (`io.option.terminal`).
- A benign `AdwTabBox reported min width -6` warning may appear on startup (known libadwaita behavior).
- Requires **Zig 0.15.2** at build time; `libghostty-vt-sys` does not yet support Zig 0.16.

## Acknowledgements

optionTerm exists because of **[FoxTerminal](https://gitlab.com/OrangeFox/misc/FoxTerminal)** by **Yacha** ([OrangeFox](https://orangefox.tech)). Her sidebar-first terminal is what convinced us to build this one, and its quick theme/font controls and the shape of its preferences directly inspired the UI here. If you want a GNOME terminal with containers, SSH hosts and agent sessions in one sidebar, use hers — it does far more than this one.

FoxTerminal is licensed GPL-3.0-or-later and optionTerm is Apache-2.0, so **no FoxTerminal code is included here**; only ideas were borrowed.

The VT engine is Ghostty's [libghostty-vt](https://ghostty.org), by Mitchell Hashimoto and the Ghostty contributors.

## License

[Apache-2.0](LICENSE)

## Links

- Repository: https://github.com/fireflylabss/optionTerm
- Issues: https://github.com/fireflylabss/optionTerm/issues
