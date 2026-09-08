# Plan 005: Add named session profiles (workspaces)

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat 1fa11c6..HEAD -- src/session.rs src/app.rs src/ui.rs src/config.rs`
> If any in-scope file changed since this plan was written, compare the
> "Current state" excerpts against the live code before proceeding; on a
> mismatch, treat it as a STOP condition.

## Status

- **Priority**: P3
- **Effort**: L
- **Risk**: MED
- **Depends on**: none
- **Category**: direction
- **Planned at**: commit `1fa11c6`, 2026-09-08
- **Issue**: (none)

## Why this matters

Session restore is already reliable after the v0.2.11 stability pass. Power
users keep multiple projects open; saving and restoring named workspaces (e.g.
"work", "dotfiles", "client") is the natural next step once a single unnamed
workspace works.

## Current state

- `src/session.rs:119-127` — `Session` is a single anonymous struct:
  ```rust
  pub struct Session {
      pub tabs: Vec<TabState>,
      pub active: usize,
      pub width: Option<i32>,
      pub height: Option<i32>,
      pub maximized: bool,
  }
  ```
- `src/session.rs:134-137` — `Session::path()` returns
  `option_sdk::App::TERMINAL.session_toml()`.
- `src/session.rs:139-150` — `Session::load()` reads that single file.
- `src/session.rs:152-170` — `Session::save()` writes the single file via
  `crate::storage::atomic_write`.
- `src/app.rs:2470-2503` — `save_session` closure calls `capture_session` and
  `session.save()`.
- `src/app.rs:2598-2644` — `build_window` restores from `SessionState::load()`.
- `src/config.rs:186-187` — `session_restore` is a boolean.

## Commands you will need

| Purpose   | Command                                                  | Expected on success |
|-----------|----------------------------------------------------------|---------------------|
| Build     | `cargo build --release`                                  | exit 0              |
| Tests     | `cargo test --release`                                   | all pass            |
| Clippy    | `cargo clippy --release --all-targets -- -D warnings`    | exit 0              |

## Suggested executor toolkit

- Skill `better-interface` can review the picker and dialog design.
- Reference `src/session.rs` tests for the existing save/load round-trip pattern.

## Scope

**In scope**:
- `src/session.rs` — add named profile storage, a slug helper, save/load by name,
  and list/delete profiles.
- `src/app.rs` — add actions for "Save Session As…", "Load Session…",
  "Manage Sessions…"; continue to load the default unnamed session on startup.
- `src/ui.rs` — add a picker/chooser for profiles.

**Out of scope**:
- Scrollback restore (explicitly removed in 0.2.0).
- Cloud or encrypted sync.
- Session auto-backup/rotation.
- CLI flag `--profile` (can be added later without changing the core).

## Git workflow

- Branch: `advisor/005-named-session-profiles`
- Commit per step, e.g. `feat(session): named profile storage`,
  `feat(ui): session profile picker`, `feat(app): save/load session actions`.
- Do NOT push unless instructed.

## Steps

### Step 1: Add named profile storage to `src/session.rs`

- Add a `SessionProfile` struct or use the existing `Session` with a
  `name: Option<String>` field:
  ```rust
  pub struct Session {
      pub name: Option<String>,  // new
      pub tabs: Vec<TabState>,
      pub active: usize,
      pub width: Option<i32>,
      pub height: Option<i32>,
      pub maximized: bool,
  }
  ```
- Add `pub fn default_path() -> PathBuf` returning the existing `session.toml`.
- Add `pub fn profile_dir() -> PathBuf` returning `config_dir().join("sessions")`.
- Add `pub fn path_for(name: Option<&str>) -> PathBuf`:
  - `None` or empty → `default_path()`
  - `Some(name)` → `profile_dir().join(format!("{slug}.toml", slug =
    slugify_session_name(name)))`
- Add `pub fn slugify_session_name(name: &str) -> String` that keeps
  ASCII letters, digits, `-`, `_`, and `.`, collapsing everything else to `-`.
- Update `load()`, `save()`, `save_to()` to accept `name: Option<&str>`.
  Default to `None`.
- Add `pub fn list_profiles() -> Vec<String>` that lists `.toml` files in
  `profile_dir()`, strips the extension, and returns sorted names.
- Add `pub fn delete_profile(name: &str) -> Result<()>`.

**Verify**:
- `cargo build --release` → exit 0.
- `cargo test --release session` → all pass.

### Step 2: Keep the default load path backward-compatible

- `Session::load()` should still default to the existing `session.toml` and
  ignore the new `name` field in older files.
- `Session::parse` should be tolerant of a missing `name` (use `Option<String>`
  and default to `None`).

**Verify**: `cargo test --release` → all existing session tests pass.

### Step 3: Add the session profile picker in `src/ui.rs`

Create `pub fn show_session_profile_picker(
    window: &adw::ApplicationWindow,
    profiles: Vec<String>,
    current: Option<String>,
    on_save: Rc<dyn Fn(String)>,
    on_load: Rc<dyn Fn(String)>,
    on_delete: Rc<dyn Fn(String)>,
)`:

- Use an `adw::Dialog` titled "Session Profiles".
- Include:
  - An `adw::EntryRow` to type a new profile name.
  - A "Save current session" button.
  - A `gtk4::ListBox` of existing profiles.
- Each row shows the profile name and has "Load" and "Delete" buttons.
- Filter the list with a search entry, reusing the same split-on-whitespace
  logic as the command palette.

**Verify**: `cargo build --release` → exit 0.

### Step 4: Wire actions in `src/app.rs`

In `build_window`, after the existing `save_session` and `shutdown` closures:

- Add `win.save-session-as` action that:
  - Gathers `SessionState::list_profiles()`.
  - Opens the picker with `on_save` set to a closure that captures the current
    session, sets `name`, and calls `session.save_to(Session::path_for(Some(&name)))`.
- Add `win.load-session` action that:
  - Opens the picker with `on_load` set to a closure that:
    1. Saves the *current* unnamed session to a backup profile (optional but
       safe) by calling `SessionState::save` before switching.
    2. Loads the named profile.
    3. Calls `shutdown()` to close all tabs, then rebuilds the workspace from
       the loaded session (`build_layout_widget` + `add_browser_tab`).
- Add `win.manage-sessions` action (optional) or combine it with
  `win.load-session` (the picker can both load and delete).
- Add these actions to the `COMMANDS` table in `src/ui.rs` with accelerator and
  palette entry.

**Verify**:
- `cargo build --release` → exit 0.
- `cargo clippy --release --all-targets -- -D warnings` → exit 0.

### Step 5: Add menu and palette entries

- Add a new section in `src/ui.rs::main_menu()` (or the file-tree `+` menu if
  more appropriate) with "Save Session As…" and "Load Session…".
- Add `(label, action, accel)` tuples to `COMMANDS` in `src/ui.rs:23-84`.

**Verify**: `cargo build --release` → exit 0.

### Step 6: Add tests

In `src/session.rs` tests:
- Test `path_for(None)` returns the default `session.toml`.
- Test `path_for(Some("My Work"))` returns `sessions/my-work.toml`.
- Test `list_profiles()` and `delete_profile()` round-trip.
- Test loading a named profile and round-tripping the same `Session`.

In `src/app.rs` (optional, harder because it needs a full window):
- Test that `save_session` can save to a profile and restore from it.

**Verify**: `cargo test --release` → all pass.

### Step 7: Quality gates

Run:
- `cargo test --release`
- `cargo clippy --release --all-targets -- -D warnings`

**Verify**: both exit 0.

## Test plan

- Unit tests in `src/session.rs` for profile path, list, save, load, delete.
- Manual smoke test:
  1. Open a few tabs/splits.
  2. "Save Session As…" → "work".
  3. Close and reopen optionTerm.
  4. "Load Session…" → "work"; confirm tabs/splits/cwds are restored.
- Manual edge test: a profile name with spaces and symbols is slugified safely.

## Done criteria

- [ ] `git diff --stat 1fa11c6..HEAD -- src/session.rs src/app.rs src/ui.rs src/config.rs`
      matches the expected in-scope files.
- [ ] `cargo build --release` exits 0.
- [ ] `cargo test --release` exits 0.
- [ ] `cargo clippy --release --all-targets -- -D warnings` exits 0.
- [ ] `Session` has an optional `name` field.
- [ ] `Session::path_for` returns the correct default or profile path.
- [ ] `Session::list_profiles` and `delete_profile` exist and work.
- [ ] The picker lets the user save, load, and delete named profiles.
- [ ] The default startup still loads `session.toml` when `session_restore` is on.
- [ ] `plans/README.md` status row for 005 is updated to DONE.

## STOP conditions

Stop and report (do not improvise) if:
- Existing session tests fail because the `Session` TOML format changed in a
  non-backward-compatible way.
- Loading a named profile would require refactoring `build_window` beyond the
  existing `build_layout_widget` + `add_browser_tab` helpers; if those helpers
  cannot be reused, escalate.
- You need to add a CLI argument or new config key for this feature.

## Maintenance notes

- A future `--profile` CLI option only needs to call
  `Session::load_from(Session::path_for(Some(name)))` before `build_window`
  starts restoring.
- If session auto-save is added, it should continue to target the currently
  active profile (or the default if no profile is active).
- Profile names are user-facing; the slug is internal. Display the original
  name from the filename only when no separate metadata file exists.
