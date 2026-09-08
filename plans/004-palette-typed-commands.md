# Plan 004: Make the command palette run arbitrary typed commands

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat 1fa11c6..HEAD -- src/ui.rs src/launch.rs src/app.rs`
> If any in-scope file changed since this plan was written, compare the
> "Current state" excerpts against the live code before proceeding; on a
> mismatch, treat it as a STOP condition.

## Status

- **Priority**: P2
- **Effort**: S
- **Risk**: MED
- **Depends on**: none
- **Category**: direction
- **Planned at**: commit `1fa11c6`, 2026-09-08
- **Issue**: (none)

## Why this matters

The command palette (`Ctrl+Shift+P`) is a static catalog of bound actions and
named `[[command]]` presets. Power users often want to run a one-off command
without first editing `config.toml`. Letting the palette accept typed text and
run it as a shell command keeps the same shortcut useful for ad-hoc work.

## Current state

- `src/ui.rs:483-651` — `show_command_palette` builds a `ListBox` from
  `bindings.effective()` and `config.commands`; each row maps to
  `PaletteAction::Win` or `PaletteAction::Launch`.
- `src/ui.rs:515-529` — `entries` is an `Rc<Vec<(String, String, PaletteAction)>>`:
  ```rust
  let mut entries: Vec<(String, String, PaletteAction)> = bindings
      .effective()
      .into_iter()
      .map(|(label, action, accel)| (label.to_string(), accel, PaletteAction::Win(action)))
      .collect();
  for cmd in &config.borrow().commands {
      entries.push((
          format!("Run: {}", cmd.name),
          String::new(),
          PaletteAction::Launch(crate::launch::LaunchRequest { ... }),
      ));
  }
  ```
- `src/ui.rs:553-569` — the filter function matches each row against the query.
- `src/ui.rs:580-616` — activation runs the selected action; `entry` activate
  runs the first visible row.
- `src/app.rs:1907-1919` — the `command-palette` action builds an
  `open_launch` callback that passes a `LaunchRequest` to `add_tab`.
- `src/launch.rs:27-86` — `parse_args` handles `optionterm`'s own CLI; it does
  not implement general shell quoting.

## Commands you will need

| Purpose   | Command                                                  | Expected on success |
|-----------|----------------------------------------------------------|---------------------|
| Build     | `cargo build --release`                                  | exit 0              |
| Tests     | `cargo test --release`                                   | all pass            |
| Clippy    | `cargo clippy --release --all-targets -- -D warnings`    | exit 0              |

## Suggested executor toolkit

- Skill `better-ui` may help with the palette row styling.
- Reference `src/launch.rs` for how `LaunchRequest` and `parse_args` are
  structured.

## Scope

**In scope**:
- `src/ui.rs` — add a synthetic "Run: <query>" row, parse the typed query into
  an argv, and launch it.
- `src/launch.rs` — add a small `tokenize_shell(query: &str) -> Option<Vec<String>>`
  helper.

**Out of scope**:
- Changing `config.toml` or `[[command]]` semantics.
- Full POSIX shell expansion (no `$VAR`, `~`, glob, backticks, or pipes; only
  basic quoting so multi-word args work).
- Adding a command history.

## Git workflow

- Branch: `advisor/004-palette-typed-commands`
- Commit per step, e.g. `feat(launch): add shell tokenizer`,
  `feat(ui): typed commands in palette`.
- Do NOT push unless instructed.

## Steps

### Step 1: Add a small shell tokenizer in `src/launch.rs`

Add a public function:
```rust
/// Split a typed command into an argv, respecting single and double quotes.
/// Returns None for empty input or unbalanced quotes.
pub fn tokenize_shell(query: &str) -> Option<Vec<String>>;
```

Implementation rules:
- Skip leading/trailing whitespace.
- Split on unquoted whitespace.
- Support `"..."` and `'...'` quoted segments that may contain spaces.
- Support `\"` and `\'` as escaped quotes inside matching quotes.
- Return `None` if quotes are unbalanced or the result is empty.

Example tests:
- `"cargo build"` → `Some(vec!["cargo", "build"])`
- `"echo 'hello world'"` → `Some(vec!["echo", "hello world"])`
- `""` → `None`
- `"echo 'hello"` → `None`

**Verify**:
- `cargo build --release` → exit 0.
- `cargo test --release tokenize_shell` → all pass.

### Step 2: Add `PaletteAction::Typed` and the synthetic row

In `src/ui.rs`:
- Extend `PaletteAction`:
  ```rust
  enum PaletteAction {
      Win(&'static str),
      Launch(crate::launch::LaunchRequest),
      Typed(Vec<String>),  // new
  }
  ```
- Before building the `ListBox`, if the query is non-empty after building the
  filtered list, prepend a synthetic row at the top of the model (index 0) when
  the query itself is not already matched by an existing "Run: ..." preset:
  - label: `format!("Run: {}", query)`
  - accel: empty
  - action: `PaletteAction::Typed(tokenized_argv)`
- Update the filter function so the synthetic row is always visible when the
  query is non-empty.

**Verify**: `cargo build --release` → exit 0.

### Step 3: Activate typed commands

In `src/ui.rs`:
- In the `activate` closure, handle `PaletteAction::Typed(argv)` by building a
  `LaunchRequest`:
  ```rust
  let req = crate::launch::LaunchRequest {
      cwd: /* focused pane's cwd or None */,
      command: Some(argv),
  };
  open_launch(req);
  ```
- `show_command_palette` needs the focused pane's cwd. Change its signature to
  accept a `current_dir: Rc<dyn Fn() -> Option<PathBuf>>` (or pass the existing
  `current_view` getter). `src/app.rs` already has `current_view: Rc<dyn Fn() ->
  Option<Rc<TerminalView>>>` in `build_window`; you can pass
  `Rc::new(move || current_view().and_then(|v| v.pwd().map(PathBuf::from)))`.

**Verify**:
- `cargo build --release` → exit 0.
- `cargo clippy --release --all-targets -- -D warnings` → exit 0.

### Step 4: Use the query as the default row on Enter

In `src/ui.rs` `entry.connect_activate`:
- Currently it runs the first visible row. With the synthetic row always at the
  top when the query is non-empty, this already works. Make sure the synthetic
  row is *first* in the filtered list, not last.

**Verify**: `cargo build --release` → exit 0.

### Step 5: Add tests

In `src/launch.rs` `mod tests`:
- Unit tests for `tokenize_shell` covering empty, simple, quoted, escaped, and
  unbalanced inputs.

In `src/ui.rs` tests (or a new `ui.rs` test module):
- Build a palette with a `PaletteAction::Typed` and assert activation creates a
  `LaunchRequest` with the right argv. If a headless GTK test is too heavy, add
  the tokenize tests and a manual smoke test note.

**Verify**: `cargo test --release` → all pass.

### Step 6: Quality gates

Run:
- `cargo test --release`
- `cargo clippy --release --all-targets -- -D warnings`

**Verify**: both exit 0.

## Test plan

- Unit tests for `tokenize_shell` in `src/launch.rs`.
- Manual smoke test: open the palette, type `echo hello world`, press Enter, and
  confirm a new tab runs `echo hello world` and prints `hello world`.
- Manual edge test: `echo 'hello world'` passes a single two-word argument.

## Done criteria

- [ ] `git diff --stat 1fa11c6..HEAD -- src/ui.rs src/launch.rs src/app.rs`
      matches the expected in-scope files.
- [ ] `cargo build --release` exits 0.
- [ ] `cargo test --release` exits 0.
- [ ] `cargo clippy --release --all-targets -- -D warnings` exits 0.
- [ ] `tokenize_shell` exists and handles whitespace and simple quotes.
- [ ] The palette shows a "Run: <query>" row at the top when the typed query
      does not match an existing row.
- [ ] Pressing Enter with a query runs the typed command in a new tab.
- [ ] `plans/README.md` status row for 004 is updated to DONE.

## STOP conditions

Stop and report (do not improvise) if:
- The typed row accidentally replaces or breaks the existing "Run: <preset>"
  rows.
- You need to introduce a full shell parser or new dependency; keep it to a
  small tokenizer.
- `show_command_palette`'s signature cannot be changed because other callers
  exist; the only caller is in `src/app.rs:1916`.

## Maintenance notes

- If a command history is added later, the synthetic row should be the model for
  re-running a previous command.
- Adding tab-completion in the palette will need to share the tokenizer; put any
  new shared parsing in `src/launch.rs`.
