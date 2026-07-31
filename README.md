# optionTerm

Sidebar-first GTK4 + libadwaita terminal with tiling splits, a command palette,
and Adwaita preferences.

## Features

- **Tabs, three ways** — top tab bar, left/right sidebar, or hidden tabs (`window.tabs`), each with a `+` button that doubles as a tab, split and terminal menu.
- **Tab overview** — `F1` (or `Super+Tab`) opens a grid of live tab thumbnails to search and switch.
- **Quick settings** — the `···` menu opens with theme swatches, a font-size stepper showing the current zoom, and the live `columns × rows` of the focused pane.
- **Categorized preferences** — Appearance, Behavior, Sound, Shortcuts, Default Terminal and Advanced.
- **Editable shortcuts** — captured in the app and stored in `keys.toml`, separate from `config.toml`, with conflict detection and per-row reset.
- **Configurable tab shape** — tabs fill the bar or fit their title, and squeeze or scroll once they no longer fit.
- **Scrolling, your way** — optional scrollbar and jump-to-bottom button, and typing returns you to the prompt.
- **Audible bell** — rung on BEL, honoring your desktop's sound settings, with a test button.
- **Default terminal** — registers optionTerm through the portable `xdg-terminals.list` plus your desktop's own key.
- **Tab handling** — configurable position for new tabs, a middle-click action, and optional confirmation before closing a tab or a window.
- **Keep the system awake** while a command is running, so a long build is not interrupted by the screen locking.
- **Accents & input methods** — dead keys and compose sequences work (`´` + `a` → `á`), plus CJK/IBus engines.
- **Persistent settings** — anything changed from the menus or Preferences is written straight back to `config.toml`.
- **Clear & restart** — wipe the screen and scrollback (`Ctrl+Shift+K`) or respawn the shell in place keeping the split layout (`Ctrl+Shift+R`).
- **Clickable links** — `Ctrl+click` or `Shift+click` opens hyperlinks, bare URLs and existing file paths.
- **Scrollback search** — `Ctrl+Shift+F` with wrap-around next/previous.
- **Session restore** — reopen tabs, panes and their working directories on start (layout only).
- **Translucent background** — `background_opacity`, plus automatic config reload when `config.toml` changes.
- **Tiling splits** — split panes in any direction, directional focus, cycle focus, equalize, resize, and toggle zoom.
- **Command palette** — `Ctrl+Shift+P` to search every action and its shortcut.
- **Stylized cursor** — block, bar, underline, or hollow block; blinking and custom colors.
- **TOML configuration** — `~/.option/terminal/config.toml`, generated with defaults on first run.

## Requirements

- Rust **1.90+**
- GTK **4.14+**, libadwaita **1.5+**
- VTE **0.76+** (GTK4 package: `vte4` / `libvte-2.91-gtk4`)
- pkg-config, pango, cairo

### Arch / CachyOS

```bash
sudo pacman -S gtk4 libadwaita vte4 pango cairo pkgconf
```

### Debian / Ubuntu

```bash
sudo apt install libgtk-4-dev libadwaita-1-dev libvte-2.91-gtk4-dev \
  libpango1.0-dev libcairo2-dev pkg-config
```

## Build

```bash
cargo build --release
./target/release/optionterm
```

Or use the helper scripts:

```bash
./scripts/run.sh
./scripts/install.sh           # ~/.local
./scripts/install.sh --system  # /usr/local (needs sudo)
```

## Configuration

optionTerm reads `~/.option/terminal/config.toml`. On the first run it is
generated from built-in defaults (left sidebar for tabs).

```toml
[font]
family = "monospace"
size = 13
ligatures = true
use_system = false

[cursor]
style = "block"   # block | bar | underline | block_hollow
blink = true

[window]
tabs = "left"     # top | left | right | hidden
session_restore = true
inherit_working_directory = true
background_opacity = 1.0
```

Shortcuts live in `~/.option/terminal/keys.toml` (overrides only).

With `session_restore = true`, open tabs, pane count and each pane's working
directory are written to `~/.option/terminal/session.toml` on exit and restored
on the next start.

## Packaging

- **AUR** — `optionterm` (`packaging/aur/`)
- **.deb** — `./packaging/deb/build-deb.sh` after a release build
- **AppImage** — `./packaging/appimage/build-appimage.sh` (prefer Ubuntu 24.04 hosts)

`NOTICE` documents the LGPL VTE system dependency.

## License

Apache-2.0. See `LICENSE` and `NOTICE`.
