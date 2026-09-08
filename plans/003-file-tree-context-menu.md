# Plan 003: Add a context menu to the file tree

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat 1fa11c6..HEAD -- src/tree.rs src/app.rs`
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

The file tree is advertised as "IDE-style" in the README and module doc, but it
only reacts to single-click/Enter. Right-click is the expected affordance for
"Open in Terminal", "Copy Path", "Open With…", etc. Adding a context menu
closes the UX gap without changing the existing single-click behavior.

## Current state

- `src/tree.rs:404-418` — `bind_tree_activation` only wires `row-activated`
  (single click or Enter) to either `navigate` or `open`:
  ```rust
  list.connect_row_activated(move |_, row| {
      let entry = unsafe { ... };
      if let Some((path, directory)) = entry {
          if directory { navigate(path); } else { open(path); }
      }
  });
  ```
- `src/tree.rs:421-434` — `open_with_default` opens a file with the system's
  default app.
- `src/tree.rs:436-454` — `icon_name_for` returns an icon name for a file type.
- `src/ui.rs:258-300` — the terminal's context menu is already implemented as a
  `GtkPopoverMenu` driven by a `GMenuModel` and a secondary `GestureClick`.
- `src/app.rs:1936-1970` — the `win.file-tree` action toggles the file-tree
  panel and re-roots it to the focused pane's directory.
- `src/app.rs:1720-1744` — `win.new-tab` opens a new tab in the focused pane's
  directory or a provided one.

## Commands you will need

| Purpose   | Command                                                  | Expected on success |
|-----------|----------------------------------------------------------|---------------------|
| Build     | `cargo build --release`                                  | exit 0              |
| Tests     | `cargo test --release`                                   | all pass            |
| Clippy    | `cargo clippy --release --all-targets -- -D warnings`    | exit 0              |

## Suggested executor toolkit

- Skill `better-ui` may help with the menu styling and hit-areas.
- Reference the terminal context menu in `src/ui.rs:258-300` as the existing
  pattern.

## Scope

**In scope**:
- `src/tree.rs` — add a right-click `GestureClick` to each file-tree row,
  build a context `GMenu`, and handle "Open in Terminal", "Copy Path",
  and "Open" actions.
- `src/app.rs` — pass a `new_terminal_in_dir` callback to `FileTree::new` so
  the context menu can open a terminal in a selected directory.

**Out of scope**:
- A full "Open With" application chooser (do just "Open" with the default app).
- Drag-and-drop in the file tree.
- Renaming, deleting, or creating files in the tree.

## Git workflow

- Branch: `advisor/003-file-tree-context-menu`
- Commit per logical step, e.g. `feat(tree): right-click context menu` and
  `test(tree): context menu actions`.
- Do NOT push unless instructed.

## Steps

### Step 1: Add a `TreeAction` callback bundle

In `src/tree.rs`, extend `FileTree::with_loader` (and `FileTree::new`) to accept
a new `actions: TreeActions` struct or three closures:
- `open_with_default: Rc<dyn Fn(PathBuf)>` (already exists)
- `open_in_terminal: Rc<dyn Fn(PathBuf)>`
- `copy_path: Rc<dyn Fn(&Path)>`

Add a public builder, e.g.:
```rust
pub struct TreeActions {
    pub open: Rc<dyn Fn(PathBuf)>,
    pub open_in_terminal: Rc<dyn Fn(PathBuf)>,
    pub copy_path: Rc<dyn Fn(&Path)>,
}
impl Default for TreeActions {
    fn default() -> Self { ... } // use open_with_default, no-ops for the rest
}
```

Keep `FileTree::new()` working by defaulting to no-ops.

**Verify**: `cargo build --release` → exit 0.

### Step 2: Build the context menu model

In `src/tree.rs`:
- Add `fn tree_context_menu() -> gio::Menu` returning:
  ```rust
  let menu = gio::Menu::new();
  let open_group = gio::Menu::new();
  open_group.append(Some("Open"), Some("tree.open"));
  open_group.append(Some("Open in Terminal"), Some("tree.open-in-terminal"));
  menu.append_section(None, &open_group);
  let edit_group = gio::Menu::new();
  edit_group.append(Some("Copy Path"), Some("tree.copy-path"));
  menu.append_section(None, &edit_group);
  menu
  ```

**Verify**: `cargo build --release` → exit 0.

### Step 3: Attach the gesture to each row

In `append_dir_rows`, while creating each `ListBoxRow`:
- Build the `GtkPopoverMenu` from `tree_context_menu()`.
- Create a `gtk4::GestureClick`, set `set_button(gdk::BUTTON_SECONDARY)`, and
  on `pressed` show the popover at the pointer.
- Create a `gio::SimpleActionGroup`, add three `gio::SimpleAction`s named
  `open`, `open-in-terminal`, `copy-path`, and connect them to the closures for
  this row's `path`.
- Insert the action group into the row with `row.insert_action_group("tree",
  Some(&group))`.

Follow the pattern in `src/ui.rs:280-299` for popover placement and lifecycle.

**Verify**:
- `cargo build --release` → exit 0.
- `cargo clippy --release --all-targets -- -D warnings` → exit 0.

### Step 4: Pass real callbacks from `src/app.rs`

Find where the `FileTree` is created (in `build_window`, around the file-tree
panel setup). Pass a `TreeActions` with:
- `open` → existing `open_with_default`.
- `open_in_terminal` → a closure that calls `add_tab(LaunchRequest { cwd:
  Some(path), command: None })` to open a new terminal tab in that directory.
- `copy_path` → a closure that copies the absolute path to the `gtk4` clipboard:
  ```rust
  let display = gdk::Display::default()?;
  let clipboard = display.clipboard();
  clipboard.set_text(&path.display().to_string());
  ```

**Verify**: `cargo build --release` → exit 0.

### Step 5: Add tests

In `src/tree.rs` `#[cfg(test)] mod tests`:
- Build a `FileTree` with a custom `TreeActions` that increments a counter for
  each action.
- Build the GTK widgets (requires `#[gtk4::test]`), trigger a right-click
  gesture, and verify the menu model contains the three actions.
- If the popover is hard to test, at least verify that `tree_context_menu()`
  returns a `GMenu` with the expected number of items.

**Verify**: `cargo test --release` → all pass, including new tests.

### Step 6: Quality gates

Run:
- `cargo test --release`
- `cargo clippy --release --all-targets -- -D warnings`

**Verify**: both exit 0.

## Test plan

- `src/tree.rs` unit test for the menu model structure.
- `#[gtk4::test]` in `src/tree.rs` that creates a tree, expands a directory,
  and asserts a right-click gesture is connected to each row.
- Manual smoke test: open the file tree, right-click a file, and confirm
  "Copy Path" and "Open" work; right-click a folder and confirm "Open in
  Terminal" opens a new tab with that CWD.

## Done criteria

- [ ] `git diff --stat 1fa11c6..HEAD -- src/tree.rs src/app.rs` matches the
      expected in-scope files.
- [ ] `cargo build --release` exits 0.
- [ ] `cargo test --release` exits 0.
- [ ] `cargo clippy --release --all-targets -- -D warnings` exits 0.
- [ ] Each file-tree row has a secondary-click `GestureClick`.
- [ ] The context menu contains "Open", "Open in Terminal", and "Copy Path".
- [ ] "Open in Terminal" opens a new terminal tab in the selected directory.
- [ ] "Copy Path" copies the selected path to the clipboard.
- [ ] `plans/README.md` status row for 003 is updated to DONE.

## STOP conditions

Stop and report (do not improvise) if:
- `FileTree::new` or `with_loader` signature changes from the plan.
- You need to modify `src/ui.rs` to build the menu; the menu should live in
  `src/tree.rs`.
- The row-activated single-click behavior is broken.

## Maintenance notes

- Future tree actions (e.g. "Reveal in File Manager") should be added to
  `TreeActions` and the menu model; do not add one-off gesture handlers.
- If the row widget changes from `adw::ActionRow`/`ListBoxRow` to a custom
  widget, the action group insertion point will need updating but the `TreeActions`
  API can stay the same.
