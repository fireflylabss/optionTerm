# Plan 002: Let users choose which Codex thread to export

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat 1fa11c6..HEAD -- src/app.rs src/codex.rs src/ui.rs`
> If any in-scope file changed since this plan was written, compare the
> "Current state" excerpts against the live code before proceeding; on a
> mismatch, treat it as a STOP condition.

## Status

- **Priority**: P2
- **Effort**: M
- **Risk**: LOW
- **Depends on**: none
- **Category**: direction
- **Planned at**: commit `1fa11c6`, 2026-09-08
- **Issue**: (none)

## Why this matters

The "Save Codex Thread" feature only exports the most recent saved
conversation. Users accumulate many Codex threads; the current behavior makes
the menu action unpredictable for archiving. Adding a chooser lets them pick the
desired thread, and a simple round-trip "Open saved transcript" makes the
terminal a usable conversation archive rather than a one-way exporter.

## Current state

- `src/app.rs:2045-2109` — the `win.save-codex-thread` action spawns a job that
  calls `codex::list_threads().next()` and immediately exports that single
  newest thread:
  ```rust
  let job = gio::spawn_blocking(move || {
      (codex::list_threads().next(), folder.filter(|d| d.is_dir()))
  });
  ```
- `src/codex.rs:93-103` — `list_threads()` returns all saved threads sorted
  newest-first.
- `src/codex.rs:170-231` — `render_markdown` renders a rollout to a readable
  Markdown document.
- `src/codex.rs:233-236` — `save_markdown` atomically writes the Markdown.
- `src/codex.rs:238-257` — `slugify` produces a safe filename.
- `src/ui.rs:1760-1768` — `agent_menu()` builds a `GMenu` of agents as an
  existing example of dynamic menu construction.
- The project already uses `gtk4::FileDialog` for saving (`src/app.rs:2076-2082`)
  and `gio::spawn_blocking` for Codex I/O.

## Commands you will need

| Purpose   | Command                                                  | Expected on success |
|-----------|----------------------------------------------------------|---------------------|
| Build     | `cargo build --release`                                  | exit 0              |
| Tests     | `cargo test --release`                                   | all pass            |
| Clippy    | `cargo clippy --release --all-targets -- -D warnings`    | exit 0              |

## Scope

**In scope**:
- `src/codex.rs` — add helpers for listing/slugging and a simple Markdown preview
  loader; add tests.
- `src/ui.rs` — add `show_codex_picker` dialog (an `adw::Dialog` or `gtk4::Dialog`
  with a searchable `ListBox` of threads).
- `src/app.rs` — replace the single `list_threads().next()` call with the picker
  flow; keep the existing `FileDialog` save path.

**Out of scope**:
- Editing or re-importing into Codex's private rollout format (Codex JSONL is
  not our format; we only read from it and write Markdown).
- Adding a dedicated Markdown viewer widget (a saved transcript can be opened
  with the default app or in a new terminal tab).
- Changing the `[[command]]` preset feature.

## Git workflow

- Branch: `advisor/002-codex-thread-picker`
- Commit per logical step, e.g. `feat(codex): list all threads for picker`,
  `feat(ui): add Codex thread picker`, `refactor(app): use picker on export`.
- Do NOT push unless instructed.

## Steps

### Step 1: Add a simple Markdown preview loader in `codex.rs`

Add to `src/codex.rs`:
- `pub fn read_markdown(path: &Path) -> Result<String>` — reads an existing
  Markdown transcript via `crate::storage::read` or `std::fs::read_to_string`
  with a reasonable size cap (e.g. 4 MiB).
- Add a `CodexThread::display_title(&self) -> String` helper that returns the
  thread title and, when present, a short `” — <cwd>”` suffix.

**Verify**: `cargo build --release` → exit 0.

### Step 2: Add `show_codex_picker` in `src/ui.rs`

Create a new `pub fn show_codex_picker(
    window: &adw::ApplicationWindow,
    threads: Vec<CodexThread>,
    export: Rc<dyn Fn(CodexThread)>,
    open: Rc<dyn Fn(PathBuf)>,
)`.

Implementation:
- Build an `adw::Dialog` titled "Codex Threads".
- Add a `gtk4::SearchEntry` and a `gtk4::ListBox`.
- For each `CodexThread`, build an `adw::ActionRow` with:
  - title = `thread.display_title()`
  - subtitle = a short relative timestamp or `thread.id`
  - an icon (reuse the existing `codex` agent icon if available, else a generic
    document icon).
- Filtering: split the search query on whitespace; a row is visible if all
  query words appear in the title/subtitle (case-insensitive), matching the
  command palette pattern in `src/ui.rs:557-568`.
- Row activation: close the dialog and call `export(thread)`.
- Secondary action (e.g. a small "Open" button on the row): if the user has
  already saved a Markdown for this thread, ask for it with a `FileDialog` and
  call `open(path)`. This is the "round-trip" import path.
- If the list is empty, show an empty-state row "No Codex threads found".

**Verify**: `cargo build --release` → exit 0.

### Step 3: Wire the picker in `src/app.rs`

Replace the `save-codex-thread` action (`src/app.rs:2045-2109`):
- Fetch all threads with `codex::list_threads().collect::<Vec<_>>()` (still in
  `gio::spawn_blocking`).
- If zero threads, toast "No Codex threads found".
- Else, call `show_codex_picker` with:
  - `export`: a closure that does the existing `FileDialog` save + Markdown
    write for the chosen thread.
  - `open`: a closure that opens the chosen Markdown file in the default app
    (`gio::AppInfo::launch_default_for_uri_async`) or, simpler, in a new
    terminal tab by running `LaunchRequest { command: Some(vec!["cat".into(),
    path.display().to_string()]), cwd: Some(path.parent().map_or_else(
    PathBuf::new, Path::to_path_buf)) }`.

Keep the existing `FileDialog` behavior for the export path.

**Verify**: `cargo build --release` → exit 0.

### Step 4: Add tests

In `src/codex.rs` `#[cfg(test)] mod tests`:
- Test `read_markdown` on a fixture file and assert the round-tripped content
  matches the original.
- Test that `CodexThread::display_title` behaves with and without a `cwd`.

In `src/ui.rs` tests (or a new `ui.rs` test module):
- Create a picker with a list of fake `CodexThread` values and verify filtering
  hides/shows rows correctly. If `gtk4::test` is needed, use `#[gtk4::test]`.

**Verify**: `cargo test --release` → all pass, including new tests.

### Step 5: Quality gates

Run:
- `cargo test --release`
- `cargo clippy --release --all-targets -- -D warnings`

**Verify**: both exit 0.

## Test plan

- `src/codex.rs` unit tests for `read_markdown` and display helpers.
- `src/ui.rs` GTK test for picker filtering using fake `CodexThread` data.
- Manual smoke test: run with a real `~/.codex` directory, open the picker, and
  export a non-most-recent thread.

## Done criteria

- [ ] `git diff --stat 1fa11c6..HEAD -- src/app.rs src/codex.rs src/ui.rs` matches the expected in-scope files.
- [ ] `cargo build --release` exits 0.
- [ ] `cargo test --release` exits 0.
- [ ] `cargo clippy --release --all-targets -- -D warnings` exits 0.
- [ ] `show_codex_picker` exists and is used by the `win.save-codex-thread` action.
- [ ] The picker shows all Codex threads, sorted newest-first.
- [ ] Selecting a thread opens the existing save dialog and exports the correct
      rollout.
- [ ] `plans/README.md` status row for 002 is updated to DONE.

## STOP conditions

Stop and report (do not improvise) if:
- `codex::list_threads()` no longer returns an iterator.
- The live `save-codex-thread` action no longer matches the excerpt.
- The picker must touch a Codex rollout file with write access; we only read
  and write Markdown, never modify Codex's data store.

## Maintenance notes

- If Codex changes its directory layout, `codex.rs` is the only module that
  needs to know about it.
- Future improvements (e.g. a side-by-side Markdown preview) should reuse
  `show_codex_picker`; do not build a second thread chooser.
