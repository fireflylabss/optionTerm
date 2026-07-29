# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.7] - 2026-07-29

### Changed

- **The package and binary are now named `optionterm`** (was `option-term`). On the AUR the package is [`optionterm`](https://aur.archlinux.org/packages/optionterm); the `.deb` is `optionterm_<version>_amd64.deb`.
- **Nothing breaks if you already had it installed.** Every install path — AUR, `.deb` and `scripts/install.sh` — also creates an `option-term` symlink pointing at the new binary, so existing aliases, scripts and launchers keep working. The AUR and Debian packages declare `replaces`/`conflicts`/`provides` against `option-term`, so upgrading migrates cleanly instead of leaving an orphaned binary behind.

## [0.1.6] - 2026-07-29

### Changed

- Application ID is now **`io.option.terminal`** (was `labs.firefly.optionTerm`). Desktop entry, install scripts, AppImage and `.deb` packaging follow the new ID. Reinstall the `.desktop` file if you used a previous install.
- **The grid is drawn in runs instead of cell by cell, cutting frame time by ~9x.** Adjacent cells that share a colour and font are now shaped as a single Pango run, and adjacent cells that share a background fill as a single rectangle. A full 86x24 screen went from 2063 Pango round trips per frame to 24 — one per row — and `paint` from 10.2 ms to 1.1 ms (scrolling: 9.7 ms to 1.1 ms; text changing colour every 8 cells: 10.0 ms to 2.4 ms). Nothing stopped being drawn: the number of glyphs per frame is unchanged. Only cells that provably advance exactly one column may join a run, so wide (CJK) cells, composed clusters, underlines and strikethroughs keep the previous per-cell path and their exact pixels.
- The cell loop now covers a row's backgrounds before its text. Besides letting both kinds of run merge, this fixes a latent issue in the old interleaved order, where a glyph that overhung its cell could be clipped by the next cell's background.

### Added

- **Render profiling** — `OPTION_TERM_PROFILE=1` reports `paint` timings per phase (background, setup, images, cells, cursor) with p50/p99, plus cells, glyphs and Pango runs per frame. Costs a single boolean check when unset.
- `scripts/bench-render.sh` — reproducible render benchmark. It runs against an isolated `HOME`, so config and session are generated fresh and the window always opens at the same size, which keeps the grid comparable between runs.

## [0.1.5] - 2026-07-28

### Added

- **Clickable links** — `Ctrl+click` opens OSC 8 hyperlinks, bare URLs (`https://`, `www.`, `user@host`) and filesystem paths. Paths only become links when they actually exist, resolved against the shell's OSC 7 directory, so ordinary words and version numbers stay inert.
- **Scrollback search** (`Ctrl+Shift+F`) — case-insensitive search across the screen and scrollback with a match counter, `Enter` / `Shift+Enter` to step (wrapping), and hits scrolled into view and selected. `Escape` closes and returns focus to the terminal.
- **`background_opacity`** (`0.15`–`1.0`) — translucent window background, also read from Ghostty's `background-opacity`. Cell backgrounds stay opaque so text remains readable.
- **Automatic config reload** — `config.toml` is watched with a `GFileMonitor` and re-applied on external edits (debounced, and rename/replace aware for editors that write atomically). Writes made from the UI are ignored so saving a preference cannot loop.
- **Session restore** (`window.session_restore`) — reopens tabs, their pane count and each pane's working directory from `~/.option/terminal/session.toml`. Only workspace shape is stored, never scrollback contents. Saved on window close and on `SIGTERM`/`SIGINT` so a logout does not lose it.
- **Tab renaming** — `F2` or double-click a sidebar row. A renamed tab keeps its title instead of following the shell; clearing the name restores automatic titles.
- **Tab reordering** — drag sidebar rows onto each other.
- **New splits inherit the focused pane's working directory.**
- **GitHub Actions CI** — build, tests, `cargo fmt --check` and `cargo clippy -D warnings` on every push and pull request.
- **AppImage and `.deb` release artifacts**, built on Ubuntu 24.04 and attached automatically on tag pushes. The AppImage bundles GTK 4 and libadwaita (verified on a system without GTK 4 installed) and requires glibc 2.38+.

### Changed

- `window.theme = "system"` now genuinely follows the desktop light/dark preference at runtime.
- Codebase is `cargo fmt` clean and free of `clippy` warnings, both enforced by CI.

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
- The app uses a single-instance `GApplication` (`io.option.terminal`).
- A benign `AdwTabBox reported min width -6` warning may appear on startup (known libadwaita behavior).
