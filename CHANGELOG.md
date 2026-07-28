# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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

### Changed

- License: **MIT → Apache-2.0**.
- Repository links updated to `https://github.com/fireflylabss/optionTerm`.
- Reverted the `.desktop`/app icon to `utilities-terminal` for the public release; custom assets remain available under `assets/` for local installs.

### Notes

- Requires **Zig 0.15.2** in `PATH` at build time (`libghostty-vt-sys`).
- The app uses a single-instance `GApplication` (`labs.firefly.optionTerm`).
- A benign `AdwTabBox reported min width -6` warning may appear on startup (known libadwaita behavior).
