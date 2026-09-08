# Plan 001: Respect GNOME text-scaling-factor in the terminal font

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat 1fa11c6..HEAD -- src/terminal.rs src/app.rs src/ui.rs`
> If any in-scope file changed since this plan was written, compare the
> "Current state" excerpts against the live code before proceeding; on a
> mismatch, treat it as a STOP condition.

## Status

- **Priority**: P1
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: direction
- **Planned at**: commit `1fa11c6`, 2026-09-08
- **Issue**: (none)

## Why this matters

`AGENTS.md:81` lists this as the only open TODO. The app already honors GNOME's
`text-scaling-factor` and Xft DPI for the chrome in `apply_desktop_chrome`, but
the terminal glyph grid is built from `config.font_size` directly. Users with
non-default text scaling see chrome text scale while the terminal body stays at
the configured point size, making the app feel out of step with the rest of the
desktop and hurting accessibility.

## Current state

- `src/app.rs:2870-2908` — `apply_desktop_chrome` reads
  `org.gnome.desktop.interface` `text-scaling-factor` and `gtk-xft-dpi`,
  multiplies them, and builds a CSS font-size for the chrome.
- `src/terminal.rs:865-873` — `apply_font` builds a `FontDescription` from
  `config.font_size` with no scale applied:
  ```rust
  let desc = FontDescription::from_string(&format!("{family} {}", config.font_size));
  ```
- `src/terminal.rs:539-544` — `set_font_size` stores the requested size directly
  in `config.font_size` and then calls `apply_font`:
  ```rust
  pub fn set_font_size(&self, size: f32) -> f32 {
      let size = size.clamp(6.0, 40.0);
      self.config.borrow_mut().font_size = size;
      apply_font(&self.terminal, &self.config.borrow());
      size
  }
  ```
- `src/app.rs:1696-1719` — `apply_zoom` calls `view.set_font_size(size)`,
  stores the returned value back into `config.font_size`, and toasts it.
- `src/ui.rs:244-251` — `QuickSettings::set_font_size` displays the zoom
  percentage as `size / base * 100`.
- The project convention is to keep user-facing config values as the configured
  base, and to apply system-derived scales at render time (see `chrome_font_css`).

## Commands you will need

| Purpose   | Command                                                  | Expected on success |
|-----------|----------------------------------------------------------|---------------------|
| Build     | `cargo build --release`                                  | exit 0              |
| Tests     | `cargo test --release`                                   | all pass            |
| Clippy    | `cargo clippy --release --all-targets -- -D warnings`    | exit 0              |
| Smoke     | `timeout 5 ./target/release/optionterm`                  | clean startup       |

## Suggested executor toolkit

- Skill `better-accessibility` may help if you need to reason about desktop
  accessibility scaling conventions.
- Reference: `AGENTS.md` build/test notes and the VTE fork instructions.

## Scope

**In scope**:
- `src/terminal.rs` — add a per-view text scale, apply it in `apply_font` and
  `set_font_size`, and expose a setter.
- `src/app.rs` — extract a shared scale-reading helper, pass the scale into
  `TerminalView` at creation and on settings changes.
- `src/ui.rs` — ensure `QuickSettings` zoom percentage reflects the scaled size
  (it should already if `set_font_size` returns the scaled value).
- New tests in `src/terminal.rs`.

**Out of scope**:
- Adding a `config.toml` key for the scale (the scale must stay tied to the
  desktop, not persisted per-app).
- Changing the Preferences font row (it still sets the configured base size).
- Refactoring the chrome scale code beyond moving the helper.

## Git workflow

- Branch: `advisor/001-text-scaling-terminal`
- Commit per logical step, e.g. `feat(terminal): apply GNOME text-scaling-factor`
  and `test(terminal): scaled font size`.
- Do NOT push unless instructed.

## Steps

### Step 1: Extract a shared text-scale helper

Move the GNOME `text-scaling-factor` + Xft DPI multiplication out of
`apply_desktop_chrome` into a public `text_scale_factor() -> f64` function in
`src/app.rs` (or a new small module). It must return `1.0` when:
- `gio::SettingsSchemaSource` is `None`,
- the key is missing,
- or the value is non-finite/non-positive.

Use the existing logic in `src/app.rs:2870-2908` as the source of truth; do not
re-implement with different edge cases.

**Verify**: `cargo build --release` → exit 0.

### Step 2: Add a text scale field and setter to `TerminalView`

In `src/terminal.rs`:
- Add `text_scale: Cell<f64>` to `TerminalView`.
- Initialize it to `1.0` in `TerminalView::new`.
- Add `pub fn set_text_scale(&self, scale: f64)` that clamps to a sensible
  minimum (e.g. `0.5`) and maximum (e.g. `3.0`), stores the value, and calls
  `apply_font(&self.terminal, &self.config.borrow())`.

**Verify**: `cargo build --release` → exit 0.

### Step 3: Apply the scale in `apply_font` and `set_font_size`

In `src/terminal.rs`:
- Change `apply_font` to compute an effective size:
  ```rust
  let effective = (config.font_size as f64 * text_scale).clamp(6.0, 80.0) as f32;
  let desc = FontDescription::from_string(&format!("{family} {effective}"));
  ```
  `apply_font` needs access to the scale. The simplest path is to make
  `apply_font` a method on `TerminalView` (or pass the scale as a parameter).
  Keep `config.font_size` as the unscaled base.
- Change `set_font_size` so it still stores the *unscaled* size in
  `config.font_size` but returns the *scaled* size:
  ```rust
  pub fn set_font_size(&self, size: f32) -> f32 {
      let size = size.clamp(6.0, 40.0);
      self.config.borrow_mut().font_size = size;
      apply_font(&self.terminal, &self.config.borrow(), self.text_scale.get());
      (size as f64 * self.text_scale.get()).clamp(6.0, 80.0) as f32
  }
  ```

**Verify**: `cargo build --release` → exit 0.

### Step 4: Pass the scale from `build_window`

In `src/app.rs`:
- Compute the scale once after `apply_desktop_chrome` runs.
- When `TerminalView`s are created in `make_view` (the closure in `build_window`),
  call `view.set_text_scale(scale)` before or after spawning.
- On desktop settings changes that affect text scale, recompute the scale and
  call `set_text_scale` on every terminal view in `pages`.

**Verify**:
- `cargo build --release` → exit 0.
- `timeout 5 ./target/release/optionterm` → clean startup.

### Step 5: Update or add tests

Add a `#[gtk4::test]` in `src/terminal.rs`:
- Create a `TerminalView` with `Config::default()`.
- Call `view.set_text_scale(1.5)`.
- Call `view.set_font_size(10.0)` and assert the returned value is `15.0`.
- Call `view.set_font_size(40.0)` and assert the returned value is `60.0`.

**Verify**: `cargo test --release` → all pass, including the new test.

### Step 6: Run the quality gates

Run:
- `cargo test --release`
- `cargo clippy --release --all-targets -- -D warnings`

**Verify**: both exit 0.

## Test plan

- New test in `src/terminal.rs` (model after `src/terminal.rs:1128-1199`) covering
  `set_text_scale` and the scaled return value from `set_font_size`.
- Optional smoke test: run `timeout 5 ./target/release/optionterm` on a GNOME
  desktop with `text-scaling-factor > 1.0` and visually confirm the terminal
  font is larger than the configured point size.

## Done criteria

- [ ] `git diff --stat 1fa11c6..HEAD -- src/terminal.rs src/app.rs src/ui.rs` matches the expected in-scope files.
- [ ] `cargo build --release` exits 0.
- [ ] `cargo test --release` exits 0.
- [ ] `cargo clippy --release --all-targets -- -D warnings` exits 0.
- [ ] `TerminalView` exposes `set_text_scale` and stores an unscaled
      `config.font_size`.
- [ ] `apply_font` multiplies `config.font_size` by the view's text scale.
- [ ] The quick-settings zoom percentage still displays correctly
      (size returned by `set_font_size` divided by `base_font_size`).
- [ ] `plans/README.md` status row for 001 is updated to DONE.

## STOP conditions

Stop and report (do not improvise) if:
- The excerpts in "Current state" do not match the live code.
- `cargo clippy` fails with an error that touches code outside the in-scope list.
- `QuickSettings` no longer displays the zoom percentage or the label is stale.
- You need to add a new crate dependency to solve this; escalate first.

## Maintenance notes

- Future GTK/libadwaita versions may expose a built-in text-scale API; if so,
  the desktop helper can be simplified but the `TerminalView` API should stay
  the same.
- Any new font-related config (e.g. per-tab font size) should still go through
  `set_font_size` so the scale is applied consistently.
