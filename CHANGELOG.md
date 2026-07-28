# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.4] - 2026-07-28

### Added

- **Kitty graphics protocol** — images sent with `ESC _G …` are decoded and rendered inline, so tools like `timg`, `chafa --format=kitty`, `matplotlib`'s kitty backend and image previews in file managers work. Supports PNG (`f=100`) plus raw RGB/RGBA/gray buffers, source rectangles, cell offsets, scaling and the three z-layers (below background, below text, above text). Decoded frames are cached as Cairo surfaces and evicted per frame.

### Fixed

- **Cursor did not blink with `cursor.blink = true`.** The config value was only kept application-side and never pushed into the VT, so the render snapshot always reported a static cursor and the blink timer was disabled. `cursor-style` and `cursor-style-blink` are now applied as the DECSCUSR defaults, both at startup and when toggled at runtime.

## [0.1.3] - 2026-07-28

### Added

- **Input method support** — dead keys and compose sequences now work, so `´` + `a` types `á`. Accented characters, cedilla and CJK/IBus engines are routed through a `GtkIMMulticontext` and encoded as event text, so key protocols (Kitty/xterm) stay intact. `Ctrl`/`Alt`/`Super` combos bypass the IM and reach the encoder untouched.
- **Settings persistence** — every change made from the menus or the Preferences dialog is written back to `~/.option/terminal/config.toml` (theme, tab position, always-show-sidebar, font size, padding, cursor style and blink).
- **`window.theme`** config key (`system` | `light` | `dark`), applied before the window is mapped so there is no theme flash on start; also parsed from Ghostty's `window-theme`.
- **Tiling dropdown on the header `+`** — the top tab bar now uses the same `AdwSplitButton` as the sidebar, so splits are reachable without switching to sidebar tabs.
- **Clear Terminal** (`Ctrl+Shift+K`) — clears the screen and the scrollback of the focused pane.
- **Restart Terminal** (`Ctrl+Shift+R`) — kills the child process and respawns a fresh shell in place, preserving the split layout.
- Richer main menu: Edit (copy/paste/select all), Terminal (clear/restart), next/previous tab, and a labelled Help section. Clear/restart are also in the context menu.
- Round-trip unit test asserting every persisted setting survives a save/load cycle.

## [0.1.2] - 2026-07-28

### Added

- Proper system install script (`scripts/install.sh`): builds release binary, installs it to `~/.local/bin` (or `/usr/local/bin` with `--system`), installs `.desktop` entry and icons, and auto-detects Zig 0.15.x.
- Application icon in the About dialog (`utilities-terminal`), matching the system `.desktop` icon.

### Changed

- README expanded with an **Install** section, system-wide/user-wide install instructions, and dev/build clarification.
- Bumped version to **0.1.2**.

## [0.1.0] - 2026-07-28

### Added

- Initial public release of **optionTerm** — a GTK4 + libadwaita terminal powered by `libghostty-vt`.
- Own TOML configuration at `~/.option/terminal/config.toml`, auto-generated from the system Ghostty config on first run.
- Config sections: `[font]`, `[cursor]`, `[window]`, `[colors]` — including palette arrays, tab position, padding, cursor style/blink/color.
- Tab bar placements: **top**, **left sidebar**, **right sidebar**, or **hidden** (`window.tabs`).
- Ghostty-style tiling: split **right/down/left/up**, directional focus, cycle focus, resize, equalize, and toggle split zoom.
- Ghostty-style keyboard shortcuts: `Ctrl+Shift+O/E/L/U`, `Ctrl+Alt+arrows`, `Ctrl+Super+[/]`, `Ctrl+Shift+Enter`, and more.
- Command palette (`Ctrl+Shift+P`) listing every command and its accelerator.
- Stylized cursor: **block**, **bar**, **underline**, **block-hollow**; blink timer; focus-aware hollow state; custom cursor/text colors.
- Context menu with copy, paste, select all, and split actions.
- Preferences dialog with theme, tab position, font size, padding, cursor style/blink, and one-click config file edit.
- About dialog with version, license, and repository links.
- Resize toast showing `cols × rows`, deduplicated and scoped to the focused pane.
- Font resolution with fallback to `monospace` when the configured font is missing or not monospaced (`FiraCode Nerd Font` used by default).
- Font/metric caching (`FontSet`) to avoid recreating resources every frame.
- PTY layer with write polling, anti-flood, SIGHUP/SIGKILL cleanup, and title sanitization.
- Application icon assets under `assets/` and a `scripts/install-desktop.sh` helper for local icon/desktop installation.
- `AGENTS.md` documenting architecture, build setup, and smoke-test workflow.

### Notes

- Requires **Zig 0.15.2** in `PATH` at build time (`libghostty-vt-sys`).
- The app uses a single-instance `GApplication` (`labs.firefly.optionTerm`).
- A benign `AdwTabBox reported min width -6` warning may appear on startup (known libadwaita behavior).
