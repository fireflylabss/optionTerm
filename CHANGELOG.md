# Changelog

We follow [Semantic Versioning](https://semver.org/) and [Keep a Changelog](https://keepachangelog.com/). optionTerm is a single GNOME surface.

<details>
<summary>To see more about versioning, expand this.</summary>

Every version string starts with `v` (required), e.g. `v0.2.4-stable`, `v0.2.3-stable`.

The installable surface is **GNOME** (GTK4 + libadwaita), delivered as the `optionterm` desktop terminal.

| Part | What you install | Example |
| --- | --- | --- |
| **GNOME** | `optionterm` desktop terminal | `v0.2.4-stable` |

With one surface there is no `m` in the changelog version and no per-surface sections — just the version notes.

Each release heading is the version and date (`## v0.2.4-stable · 11/08/2026`); its short summary ends by naming the GNOME surface, channel, date, and version again.

### What the channel suffix means

| Suffix | In plain words |
| --- | --- |
| **-alpha** | Very early. Expect missing pieces and lots of bugs. |
| **-beta** | Mostly there, but still rough. Fine to try; not the official install. |
| **-stable** | Ready for daily use. This is what we put on GitHub Releases and the AUR. |

We only call something **stable** when we mean it. While a change is being validated, builds stay **beta**.

</details>

## v0.2.9-stable · 18/08/2026

Reliable full-pane Kitty Graphics rendering for terminal-browser. This version was made for GNOME with a stable release channel on 18/08/2026 (v0.2.9-stable).

- Replaced the limited local VTE patch with the complete FoxTerminal VTE 0.84.1 fork, pinned to commit `7ed5a96ccc0305b03695ac18af15f96b92805126` for reproducible builds.
- `terminal-browser` frames now use the fork's natural-size placement and repaint behavior, so pages remain visible when clients omit explicit cell dimensions (`c=` / `r=`).
- Kitty Graphics keeps raw RGB/RGBA, zlib, chunked payloads and query support while adding the fork's complete direct, file, temporary-file and shared-memory transport paths.
- Arch, Debian and AppImage packaging now bundle the same Kitty-capable VTE used by local builds instead of falling back to the distro library at runtime.
- CI is clean under warnings-as-errors again, including current Clippy releases.

## v0.2.8-stable · 18/08/2026

Fuller Kitty Graphics support: raw RGB/RGBA, deflate, chunked transfers and a bounded image cache. This version was made for GNOME with a stable release channel on 18/08/2026 (v0.2.8-stable).

- Kitty Graphics now renders raw `f=24` (RGB) and `f=32` (RGBA) payloads, not just PNG — so `kitten icat`, `chafa` and `terminal-browser` draw unpacked pixels directly.
- `o=z` zlib deflate payloads are decoded, bounded by the exact decompressed size for raw formats and a global cap for PNG.
- Chunked transmissions (`m=1`) join their payloads before decoding, so a client may split its base64 anywhere.
- Capability queries (`a=q`) now actually carry out the transmission and answer `Gi=<id>;OK` or `Gi=<id>;EINVAL:...` truthfully, instead of echoing a capability the terminal cannot honour.
- Image memory is a bounded budget with least-recently-used eviction, so a buggy client cannot grow the image table without limit.
- Reply quiet flags (`q=1`/`q=2`) are honoured per the spec. The parsing/decode/reply structure is informed by FoxTerminal's forked VTE, credited in NOTICE.

## v0.2.7-stable · 18/08/2026

Reliable Kitty Graphics capability detection for terminal-browser and related tools. This version was made for GNOME with a stable release channel on 18/08/2026 (v0.2.7-stable).

- Kitty Graphics capability queries (`a=q`) now reply in the exact `Gi=<id>;OK` form required by clients, so `terminal-browser` can recognise optionTerm as an image-capable terminal.
- The AUR source checksum was refreshed to match the published `v0.2.6` archive, making local package rebuilds reproducible again.

## v0.2.6-stable · 14/08/2026

Kitty graphics protocol support through a patched VTE, with file-based image transfer. This version was made for GNOME with a stable release channel on 14/08/2026 (v0.2.6-stable).

- Kitty graphics protocol images render inline again: a patched VTE (built by `scripts/build-vte.sh` into `vte-dist/`) answers the `a=q` query and draws `a=T` / `a=p` placements on the cell grid.
- File-based transmit (`t=f`) is supported, so tools that send a base64 file path instead of pixel data work too — including optionFiles previews and `kitten icat`.
- Image numbers (`I=`), placement ids (`p=`), replace-in-place placement updates and delete-by-number (`a=d,d=n`) follow the kitty spec.
- The VTE string parser cap was raised from 4k to 64k so single-chunk PNG payloads are no longer silently dropped.

## v0.2.5-stable · 12/08/2026

Align the shared family SDK on the latest optionSDK. This version was made for GNOME with a stable release channel on 12/08/2026 (v0.2.5-stable).

- Bumped `optionSDK` from `0.1.2` to `0.1.3` to match the rest of the family.
- Bumped crate version from `0.2.4` to `0.2.5`.

## v0.2.4-stable · 11/08/2026

Full-bleed terminal panes with no visual frame around the terminal surface. This version was made for GNOME with a stable release channel on 11/08/2026 (v0.2.4-stable).

- Terminal panes now expand through the complete content area below the chrome, including nested app containers.
- Removed the terminal surface's CSS frame, corner radius, shadow and outer spacing while preserving the optional scrollbar and jump-to-bottom control.
- New installations default to zero terminal padding; the Appearance preference remains available for an intentional inner margin.

## v0.2.3-stable · 03/08/2026

Integrated browser tabs, borderless terminal panes and browser navigation controls. This version was made for GNOME with a stable release channel on 03/08/2026 (v0.2.3-stable).

- Browser opens as a native GTK4/WebKitGTK6 tab inside optionTerm, using the same tab surface and Adwaita chrome.
- Browser tabs expose back, forward, reload and address navigation while split and tiling actions stay disabled.
- The `+` menu now includes **Open Browser…** alongside terminal split actions.
- Terminal panes fill the available content area without the ScrolledWindow frame inset.
- Various other UI polish

## v0.2.2-stable · 03/08/2026

Shared Option paths and atomic terminal state persistence. This version was made for GNOME with a stable release channel on 03/08/2026 (v0.2.2-stable).

- Adopt `optionSDK` 0.1.3 for `~/.option/terminal` identity and session paths.
- Write config and session files through a flushed, synced sibling temporary file.
- Keep the existing `dirs` integration for desktop default-terminal discovery only.

## [0.2.1-stable] - 2026-08-01

### Added

- **Split-tree session restore** — nested pane orientation and divider ratios
  are saved in `session.toml` (legacy flat `panes` lists still load as a
  horizontal chain).
- **Window geometry in the session** — width, height and maximized state round-
  trip with the tabs.
- **CLI launch surface** — `--working-directory` / `-d`, `-e` / `--command` /
  `--`, and a directory positional. Second instances open a new tab in the
  primary window (`GApplication` command-line).
- **`tabs = "bottom"`** — tab bar under the content; Preferences and
  `window.tabs` accept `bottom`.
- **`scroll.lines`** — configurable VTE scrollback length (default 10 000).
- **Named `[[command]]` presets** — `name` + `argv` (+ optional `cwd`) in
  `config.toml`, listed in the command palette as “Run: …”.
- **Desktop chrome fidelity** — follow `gtk-decoration-layout`, chrome font
  from `gtk-font-name` / DPI; leave `gtk-enable-animations` alone.

### Changed

- GNOME default-terminal registration sets `exec-arg` to `-e` again (now
  supported).
- Release channel labeling follows optionMusic (`x.y.z-stable` in the
  changelog; Cargo/git tags stay numeric).

## [0.2.0] - 2026-07-31
### Changed
- **Engine rewrite** — terminal surface now embeds stock system VTE via
  `vte4` instead of `libghostty-vt` + custom Cairo paint. No Zig at build
  time. Public copy focuses on sidebar-first Adwaita chrome (no Ghostty /
  “powered by VTE” taglines). `NOTICE` documents the LGPL VTE dependency.
- **Config** — first run writes built-in defaults only; Ghostty
  `~/.config/ghostty/config` import and key=value parser are gone. Existing
  optionTerm `config.toml` keys are kept.
- **Search** — rewired to VTE `search_set_regex` / find next/previous
  (match-count UI removed).
- **Session restore** — layout + cwd only. Scrollback-content restore
  (`session_restore_scrollback` / VT dumps) removed; leftover dumps from
  ≤0.1.x are deleted on save/restore.
- **Environment** — `TERM=xterm-256color` (was `xterm-ghostty`).

### Removed
- Kitty graphics protocol support (`graphics.rs`).
- OSC 52 clipboard-write path (upstream VTE refuses).
- Custom PTY read loop / paint profiler / input encoder modules.
- Zig / `libghostty-vt` / `png` crate build dependencies.

## [0.1.14] - 2026-07-31
### Added
- **`TERM_PROGRAM` / `TERM_PROGRAM_VERSION`** — set on every PTY so CLIs can
  recognise optionTerm (Grok `/doctor`, OpenCode, …). `XTVERSION` now includes
  the package version as well.
- **OSC 52 clipboard writes** — `on_clipboard_write` copies into the GTK
  clipboard (or primary selection), so tools that copy via escape sequences
  no longer fail silently.
- **`window.session_restore_scrollback`** (default off) — when session restore
  is on, also persist each pane's screen/scrollback as a VT dump under
  `~/.option/terminal/scrollback/`. Opt-in because history can hold secrets.

## [0.1.13] - 2026-07-30
### Fixed
- **Terminal cell geometry now retains Pango's fractional advance.** Rounding it to whole pixels could report the wrong column count for FiraCode and shift full-screen TUIs relative to Ghostty.
## [0.1.12] - 2026-07-29

### Added

- **`scroll.on_keystroke`** (default on) — typing while scrolled up jumps straight back to the prompt. Key releases are excluded, so the view does not move on its own.
- **`scroll.show_bar`** (default on) — a scrollbar beside the terminal, shown only once there is scrollback to reach, so a fresh shell does not sit next to a dead one. Dragging it moves the viewport by rows.
- **`scroll.show_button`** (default off) — a floating jump-to-bottom button, visible only while scrolled up.

All three read the viewport position from the terminal rather than from a counter of our own: libghostty-vt has no getter for the scroll offset, so the top-left visible cell is converted from viewport space into screen space, which is by definition how far down the scrollback the view sits. A tracked counter would have drifted the moment the terminal scrolled without being asked.

## [0.1.11] - 2026-07-29

### Fixed

- **Middle click on a tab closed it even when set to open a new one.** `AdwTabBar` closes tabs on middle click by itself, so the configured action ran *and* libadwaita closed the tab under the pointer — and "Nothing" still closed it. The gesture now runs ahead of libadwaita's and claims the event, which makes the setting authoritative for all three choices.
- **Escape now closes the command palette.** It needed handling twice: `GtkSearchEntry` swallows the key to clear its own text, so the dialog never saw the first press, and once the list has focus the entry is not involved at all. Clicking outside dismisses it too.
- **A configured font that is not installed** no longer silently resolves to whatever Fontconfig picks — which could be proportional and wreck the grid. It falls back to monospace and says so in the log.

### Added

- **Editable keyboard shortcuts**, stored in **`~/.option/terminal/keys.toml`**, separate from `config.toml`: bindings are edited far more often than colours, and a typo there must not cost you the rest of your configuration. Only overrides are saved, so new built-in defaults still reach existing installs. The Shortcuts page captures a key combination, refuses one already taken by another action (two actions on one key means one silently never fires), and offers a per-row reset.
- **`sound.command_finished`** (default off) — plays when a command ends while the window is *not* focused, detected from the terminal's foreground process group going idle. No shell integration needed.
- **`font.use_system`** (default off) — follow the desktop's monospace font instead of the configured family.
- **`window.tab_width`** — tabs either share the bar (default) or stay as wide as their title.
- **`window.tab_overflow`** — once tabs no longer fit, keep squeezing them (default) or hold a readable width and scroll.
- **`window.show_search_button`** (default off) — the magnifier in the header that opens the command palette.
- **Right-click on a tab** — rename, split, close.

### Changed

- **The `+` dropdown carries only the split directions.** Tab actions moved to the tab's own context menu.
- **The terminal context menu is shorter** — clipboard, select all, clear and find, with the six split directions folded into one submenu.
- **The theme picker is a dropdown in Preferences only.** Having it in the menu as well meant two controls for one setting.

## [0.1.10] - 2026-07-29

### Added

- **Theme swatches in Preferences too**, the same control as the `···` menu instead of a dropdown, so the two cannot disagree. Both were also reworked to match the intended design: bigger circles, no button frame, and a check badge on the selected one.
- **Sound page** — an audible bell rung when a program writes BEL, honoring your desktop's sound settings, plus a button to play it once so you can tell whether your system has one at all. `sound.bell`, default on.
- **Default Terminal page** — registers optionTerm as the preferred terminal. There is no single mechanism for this, so it writes the portable `xdg-terminals.list` (keeping your other choices below ours) plus your desktop's own key when it has one, and reports exactly which ones it managed to update instead of claiming success blindly.
- **Shortcuts page** listing every action and its binding.
- **New tab position** (`window.new_tab_position`) — after the current tab (default), before it, at the end, or at the start.
- **Middle click on a tab** (`window.middle_click_tab`) — nothing (default), new tab, or close tab. It acts on the tab actually under the pointer.
- **Confirmations** — before closing a tab (`window.confirm_close_tab`, default off) and before closing a window with more than one tab open (`window.confirm_quit`, default on).
- **Keep the system awake** (`window.keep_awake`, default off) — blocks idle and screen blanking while any pane has a foreground job, so a long build does not get interrupted by the screen locking.

### Changed

- **Session restore is on by default.** Restoring tabs, panes and their directories is what a terminal with tabs and splits is expected to do.
- The developer name is now **Firefly Labs**.

## [0.1.9] - 2026-07-29

Thanks to **Yacha** ([FoxTerminal](https://gitlab.com/OrangeFox/misc/FoxTerminal), [OrangeFox](https://orangefox.tech)) — her terminal is the reason optionTerm exists, and this release borrows the shape of its quick controls and preferences. FoxTerminal is GPL-3.0-or-later and optionTerm is Apache-2.0, so no code is shared; only ideas were.

### Added

- **Tab overview** — `F1` or `Super+Tab` opens a grid of live tab thumbnails, with search and a `+` of its own. A tab-count button in the header opens it too.
- **Quick settings at the top of the `···` menu** — theme swatches (System / Light / Dark), a font-size stepper showing the current zoom as a percentage of your configured size, and the focused pane's live `columns × rows`. The `···` menu is a real popover now, because a `GMenu` can only hold text items.
- **Preferences for the two settings added in 0.1.8** — `Ligatures` and `Inherit Working Directory` were only editable by hand in `config.toml`.

### Changed

- **Preferences are split into pages** — Appearance (theme, font, cursor), Behavior (session, tabs) and Advanced (config file), instead of one long scroll.
- **The `···` menu is much shorter.** Appearance moved into Preferences and the quick settings; tab, split and terminal actions moved behind `+`, which now carries Tabs, All Tabs / Next / Previous, Split and Terminal. What is left in `···` is Copy/Paste/Select All, Find, Command Palette, Preferences, Shortcuts, About and Quit.

## [0.1.8] - 2026-07-29

### Fixed

- **The mouse wheel did nothing.** High-resolution wheels report a fraction of a detent per event (libinput uses 1/120 steps), and each event was rounded to whole lines on its own, so every one of them rounded to zero. Wheel travel is now accumulated and the remainder carried, which fixes both high-resolution wheels and slow scrolling on ordinary ones.
- **The wheel did nothing inside full-screen programs either**, for a second and unrelated reason: mouse events were never reported to the application at all. The wheel now goes to whichever destination the running program expects — reported as mouse buttons when it enables mouse tracking (`htop`, `vim` with `set mouse=a`), turned into cursor keys on the alternate screen (`less`, `man`), and otherwise moving the scrollback viewport.
- **New tabs never inherited the working directory**, and splits only did when the shell was set up to emit OSC 7, which most are not. Both now fall back to the kernel's view of the terminal's foreground process group, so inheritance works with any shell and also follows a `cd` made inside a running program, which OSC 7 cannot report.
- **Ligatures never rendered.** They were disabled unconditionally, because up to 0.1.6 each cell was shaped as its own Pango layout and a single cell has nothing to ligate with. Now that cells are shaped as runs, `->`, `=>` and `!=` form as intended. Programming fonts keep a ligature's advance equal to the sum of the glyphs it replaces, so the grid still lines up; there is a test asserting exactly that.

### Added

- **`Shift+click` opens links**, alongside the existing `Ctrl+click`.
- **`font.ligatures`** (default `true`) — also read from Ghostty's `font-feature = -liga`.
- **`window.inherit_working_directory`** (default `true`, matching Ghostty) — also read from Ghostty's `window-inherit-working-directory`. Set it to `false` for new tabs and splits to open wherever the shell would start on its own.

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
