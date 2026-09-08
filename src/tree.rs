//! A minimal file-tree widget, "IDE-style".
//!
//! A `ListBox` of indented rows with per-type icons: clicking a folder
//! navigates into it (the tree re-roots), clicking a file opens it with the
//! system's default application. The chevron expands/collapses a folder in
//! place without navigating.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Duration, Instant};

use gtk4::gio;
use gtk4::prelude::*;

/// How many entries a directory contributes before showing a `…` marker.
const MAX_ENTRIES_PER_DIR: usize = 800;

/// Re-root the visible tree (folder navigation).
type NavigateFn = Rc<dyn Fn(PathBuf)>;
/// Re-render the visible tree.
type RebuildFn = Rc<dyn Fn()>;
/// Shared rebuild handle, so rows and recursion can reach the latest closure
/// even though it is constructed after the widget fields exist.
type RebuildHolder = Rc<RefCell<Option<RebuildFn>>>;
type Expanded = Rc<RefCell<HashSet<PathBuf>>>;
type DirectoryEntries = Vec<Option<(PathBuf, bool)>>;
type DirectoryListing = Result<DirectoryEntries, std::io::ErrorKind>;
const MAX_VISIBLE_ENTRIES: usize = 1600;
const MAX_TREE_DEPTH: usize = 32;
const TREE_ENTRY: &str = "optionterm-tree-entry";

struct TreeSnapshot {
    directories: HashMap<PathBuf, DirectoryListing>,
    created: Instant,
    scans: usize,
    limited: bool,
}

struct TreeRequest {
    root: PathBuf,
    expanded: HashSet<PathBuf>,
    cache: Option<Arc<TreeSnapshot>>,
    generation: Arc<AtomicU64>,
    revision: u64,
}

type TreeLoader =
    Arc<dyn Fn(TreeRequest) -> Result<TreeSnapshot, std::io::ErrorKind> + Send + Sync>;

/// Context-menu actions offered on a file-tree row.
#[derive(Clone)]
pub struct TreeActions {
    /// Open a file with the system's default application.
    pub open: Rc<dyn Fn(PathBuf)>,
    /// Open a new terminal tab rooted at a directory.
    pub open_in_terminal: Rc<dyn Fn(PathBuf)>,
    /// Copy an absolute path to the clipboard.
    pub copy_path: Rc<dyn Fn(&Path)>,
}

impl Default for TreeActions {
    fn default() -> Self {
        Self {
            open: Rc::new(|path| open_with_default(&path)),
            open_in_terminal: Rc::new(|_| {}),
            copy_path: Rc::new(|_| {}),
        }
    }
}

pub struct FileTree {
    pub widget: gtk4::ScrolledWindow,
    header: gtk4::Label,
    root: Rc<RefCell<Option<PathBuf>>>,
    rebuild: RebuildHolder,
    cache: Rc<RefCell<Option<Arc<TreeSnapshot>>>>,
    generation: Arc<AtomicU64>,
    alive: Rc<Cell<bool>>,
}

impl FileTree {
    pub fn new() -> Self {
        Self::with_loader(Arc::new(load_tree), TreeActions::default())
    }

    /// A file tree whose context menu runs the given actions.
    pub fn new_with_actions(actions: TreeActions) -> Self {
        Self::with_loader(Arc::new(load_tree), actions)
    }

    fn with_loader(loader: TreeLoader, actions: TreeActions) -> Self {
        let header = gtk4::Label::new(Some("Files"));
        header.add_css_class("heading");
        header.add_css_class("caption");
        header.set_ellipsize(gtk4::pango::EllipsizeMode::Start);
        header.set_xalign(0.0);
        header.set_hexpand(true);
        header.set_margin_start(6);
        header.set_margin_end(6);

        let list = gtk4::ListBox::new();
        list.add_css_class("navigation-sidebar");
        list.set_selection_mode(gtk4::SelectionMode::Single);

        let scroller = gtk4::ScrolledWindow::new();
        scroller.set_vexpand(true);
        scroller.set_propagate_natural_height(true);
        scroller.set_child(Some(&list));

        let expanded: Expanded = Rc::new(RefCell::new(HashSet::new()));
        let root: Rc<RefCell<Option<PathBuf>>> = Rc::new(RefCell::new(None));
        let rebuild: RebuildHolder = Rc::new(RefCell::new(None));
        let cache = Rc::new(RefCell::new(None::<Arc<TreeSnapshot>>));
        let generation = Arc::new(AtomicU64::new(0));
        let alive = Rc::new(Cell::new(true));
        let loading = Rc::new(Cell::new(false));
        let last_root = Rc::new(RefCell::new(None::<PathBuf>));

        // Folder navigation: re-root the tree.
        let header_nav = header.clone();
        let root_nav = root.clone();
        let rebuild_nav = Rc::downgrade(&rebuild);
        let navigate: NavigateFn = Rc::new(move |dir: PathBuf| {
            *root_nav.borrow_mut() = Some(dir.clone());
            header_nav.set_text(&format!("Files — {}", dir.display()));
            if let Some(holder) = rebuild_nav.upgrade() {
                let rebuild = holder.borrow().clone();
                if let Some(rebuild) = rebuild {
                    rebuild();
                }
            }
        });

        bind_tree_activation(&list, navigate.clone(), actions.open.clone());
        {
            let list = list.downgrade();
            let root = root.clone();
            let expanded = expanded.clone();
            let actions = actions.clone();

            let holder = Rc::downgrade(&rebuild);
            let cache = cache.clone();
            let generation = generation.clone();
            let alive = alive.clone();
            let header = header.downgrade();
            *rebuild.borrow_mut() = Some(Rc::new(move || {
                if !alive.get() {
                    return;
                }
                let revision = generation.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
                let Some(requested_root) = root.borrow().clone() else {
                    return;
                };
                if loading.replace(true) {
                    return;
                }
                let request = TreeRequest {
                    expanded: expanded
                        .borrow()
                        .iter()
                        .filter(|path| path.starts_with(&requested_root))
                        .cloned()
                        .collect(),
                    root: requested_root.clone(),
                    cache: cache.borrow().clone(),
                    generation: generation.clone(),
                    revision,
                };
                let loader = loader.clone();
                let job = gio::spawn_blocking(move || loader(request));
                let list = list.clone();
                let holder = holder.clone();
                let expanded = expanded.clone();

                let root = root.clone();
                let cache = cache.clone();
                let generation = generation.clone();
                let alive = alive.clone();
                let loading = loading.clone();
                let last_root = last_root.clone();
                let header = header.clone();
                let actions = actions.clone();
                if let Some(list) = list.upgrade()
                    && list.first_child().is_none()
                {
                    append_status(&list, "Loading files…", 0);
                }
                gtk4::glib::spawn_future_local(async move {
                    let result = job.await.unwrap_or(Err(std::io::ErrorKind::Other));
                    loading.set(false);
                    if !alive.get() {
                        return;
                    }
                    let Some(holder) = holder.upgrade() else {
                        return;
                    };
                    if generation.load(Ordering::Relaxed) != revision {
                        let rebuild = holder.borrow().clone();
                        if let Some(rebuild) = rebuild {
                            rebuild();
                        }
                        return;
                    }
                    let (Some(list), Some(header)) = (list.upgrade(), header.upgrade()) else {
                        return;
                    };
                    match result {
                        Ok(snapshot) => {
                            tracing::debug!(
                                directories_read = snapshot.scans,
                                "file tree refreshed"
                            );
                            while let Some(child) = list.first_child() {
                                list.remove(&child);
                            }
                            expanded
                                .borrow_mut()
                                .retain(|path| path.starts_with(&requested_root));
                            append_dir_rows(
                                &list,
                                &requested_root,
                                0,
                                &expanded,
                                &holder,
                                &snapshot,
                                &actions,
                            );
                            if snapshot.limited {
                                append_status(&list, "More entries not shown", 0);
                            }
                            header.set_tooltip_text(None);
                            header.set_text(&format!("Files — {}", requested_root.display()));
                            *last_root.borrow_mut() = Some(requested_root);
                            *cache.borrow_mut() = Some(Arc::new(snapshot));
                        }
                        Err(_) => {
                            *root.borrow_mut() = last_root.borrow().clone();
                            let title = last_root
                                .borrow()
                                .as_ref()
                                .map(|path| format!("Files — {}", path.display()))
                                .unwrap_or_else(|| "Files".into());
                            header.set_text(&title);
                            header.set_tooltip_text(Some("Could not read the requested folder"));
                            if last_root.borrow().is_none() {
                                while let Some(child) = list.first_child() {
                                    list.remove(&child);
                                }
                                append_status(&list, "Could not read folder", 0);
                            }
                        }
                    }
                });
            }));
        }

        let tree = FileTree {
            widget: scroller,
            header,
            root,
            rebuild,
            cache,
            generation,
            alive,
        };
        tree.refresh();
        tree
    }

    /// A ready-to-show panel: header on top, the scrollable tree below. Used
    /// as the right-hand side of a per-tab `Paned`.
    pub fn panel(&self) -> gtk4::Box {
        let panel = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        panel.add_css_class("file-tree-panel");
        panel.set_width_request(200);
        let header_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        header_row.set_margin_top(4);
        header_row.set_margin_bottom(2);
        header_row.set_margin_start(6);
        header_row.set_margin_end(6);
        header_row.append(&self.header);
        panel.append(&header_row);
        panel.append(&self.widget);
        panel
    }

    /// Re-root the tree at `dir`. No-op unless `dir` is a real directory.
    pub fn set_root(&self, dir: PathBuf) {
        *self.root.borrow_mut() = Some(dir.clone());
        self.header.set_text(&format!("Files — {}", dir.display()));
        self.refresh();
    }

    /// Re-render from the stored root (call on tab focus / cwd change).
    pub fn refresh(&self) {
        self.cache.borrow_mut().take();
        let rebuild = self.rebuild.borrow().clone();
        if let Some(rebuild) = rebuild {
            rebuild();
        }
    }
}

impl Drop for FileTree {
    fn drop(&mut self) {
        self.alive.set(false);
        self.generation.fetch_add(1, Ordering::Relaxed);
        self.rebuild.borrow_mut().take();
    }
}

/// Sorted (dirs-first, then files) entries under `dir`.
fn entries_sorted(dir: &Path, cancelled: impl Fn() -> bool) -> DirectoryListing {
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    let mut truncated = false;
    let entries = std::fs::read_dir(dir).map_err(|err| err.kind())?;
    for (i, entry) in entries.enumerate() {
        if cancelled() {
            return Err(std::io::ErrorKind::Interrupted);
        }
        if i >= MAX_ENTRIES_PER_DIR {
            truncated = true;
            break;
        }
        let entry = entry.map_err(|err| err.kind())?;
        let p = entry.path();
        if entry
            .file_type()
            .is_ok_and(|kind| kind.is_dir() || (kind.is_symlink() && p.is_dir()))
        {
            dirs.push(p);
            continue;
        }
        files.push(p);
    }
    dirs.sort();
    files.sort();
    let mut out: Vec<Option<(PathBuf, bool)>> = dirs
        .into_iter()
        .map(|d| Some((d, true)))
        .chain(files.into_iter().map(|f| Some((f, false))))
        .collect();
    if truncated {
        out.push(None);
    }
    Ok(out)
}

fn load_tree(request: TreeRequest) -> Result<TreeSnapshot, std::io::ErrorKind> {
    let cached = request
        .cache
        .as_ref()
        .filter(|cache| !cache.limited && cache.created.elapsed() < Duration::from_secs(2));
    let mut snapshot = TreeSnapshot {
        directories: HashMap::new(),
        created: cached.map_or_else(Instant::now, |cache| cache.created),
        scans: 0,
        limited: false,
    };
    let mut pending = vec![(request.root.clone(), 0usize)];
    let mut remaining = MAX_VISIBLE_ENTRIES - 1;
    while let Some((dir, depth)) = pending.pop() {
        if request.generation.load(Ordering::Relaxed) != request.revision {
            return Err(std::io::ErrorKind::Interrupted);
        }
        if remaining == 0 {
            snapshot.limited = true;
            break;
        }
        if snapshot.directories.contains_key(&dir) {
            continue;
        }
        let mut listing = if depth > MAX_TREE_DEPTH {
            snapshot.limited = true;
            Err(std::io::ErrorKind::InvalidInput)
        } else if let Some(listing) = cached.and_then(|cache| cache.directories.get(&dir)) {
            listing.clone()
        } else {
            snapshot.scans += 1;
            entries_sorted(&dir, || {
                request.generation.load(Ordering::Relaxed) != request.revision
            })
        };
        if let Err(error) = listing {
            if dir == request.root || error == std::io::ErrorKind::Interrupted {
                return Err(error);
            }
            remaining -= 1;
        }
        if let Ok(entries) = &mut listing {
            if entries.len() > remaining {
                entries.truncate(remaining);
                if let Some(last) = entries.last_mut() {
                    *last = None;
                }
                snapshot.limited = true;
            }
            remaining -= entries.len().max(1).min(remaining);
            for (path, is_dir) in entries.iter().flatten().rev() {
                if *is_dir && request.expanded.contains(path) {
                    pending.push((path.clone(), depth + 1));
                }
            }
        }
        snapshot.directories.insert(dir, listing);
    }
    Ok(snapshot)
}

fn append_status(list: &gtk4::ListBox, message: &str, depth: usize) {
    let row = gtk4::ListBoxRow::new();
    row.set_activatable(false);
    row.set_selectable(false);
    let label = gtk4::Label::new(Some(message));
    label.add_css_class("dim-label");
    label.set_margin_start(4 + depth as i32 * 10);
    label.set_wrap(true);
    row.set_child(Some(&label));
    list.append(&row);
}

fn bind_tree_activation(list: &gtk4::ListBox, navigate: NavigateFn, open: Rc<dyn Fn(PathBuf)>) {
    list.set_activate_on_single_click(true);
    list.connect_row_activated(move |_, row| {
        let entry = unsafe {
            row.data::<(PathBuf, bool)>(TREE_ENTRY)
                .map(|entry| entry.as_ref().clone())
        };
        if let Some((path, directory)) = entry {
            if directory {
                navigate(path);
            } else {
                open(path);
            }
        }
    });
}

/// Open a file with the system's default application.
fn open_with_default(path: &Path) {
    let uri = gio::File::for_path(path).uri();
    gio::AppInfo::launch_default_for_uri_async(
        &uri,
        gio::AppLaunchContext::NONE,
        gio::Cancellable::NONE,
        |result| {
            if let Err(err) = result {
                tracing::warn!("opening file failed: {err}");
            }
        },
    );
}

/// A themed icon name for a path, picked from the file extension.
fn icon_name_for(path: &Path, is_dir: bool) -> &'static str {
    if is_dir {
        return "folder-symbolic";
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    match ext.as_deref() {
        Some("png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" | "bmp" | "avif") => {
            "image-x-generic-symbolic"
        }
        Some("mp3" | "ogg" | "wav" | "flac" | "m4a" | "opus") => "audio-x-generic-symbolic",
        Some("mp4" | "mkv" | "webm" | "mov" | "avi") => "video-x-generic-symbolic",
        Some("zip" | "tar" | "gz" | "xz" | "bz2" | "7z") => "package-x-generic-symbolic",
        _ => "text-x-generic-symbolic",
    }
}

/// Right-click context menu of a file-tree row: Open, Open in Terminal,
/// Copy Path, backed by per-row `tree.*` actions.
fn tree_context_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    let open_group = gio::Menu::new();
    open_group.append(Some("Open"), Some("tree.open"));
    open_group.append(Some("Open in Terminal"), Some("tree.open-in-terminal"));
    menu.append_section(None, &open_group);
    let edit_group = gio::Menu::new();
    edit_group.append(Some("Copy Path"), Some("tree.copy-path"));
    menu.append_section(None, &edit_group);
    menu
}

/// Give one tree row its context menu: a secondary-click `GestureClick`
/// popping a `PopoverMenu`, and a `tree.*` action group bound to the row's
/// path so menu entries operate on this row only.
fn attach_row_context_menu(row: &gtk4::ListBoxRow, path: &Path, is_dir: bool, actions: &TreeActions) {
    let popover = gtk4::PopoverMenu::from_model(Some(&tree_context_menu()));
    popover.set_parent(row);
    popover.set_has_arrow(false);
    popover.set_halign(gtk4::Align::Start);

    {
        let popover = popover.clone();
        row.connect_destroy(move |_| popover.unparent());
    }

    let group = gio::SimpleActionGroup::new();

    let open = gio::SimpleAction::new("open", None);
    {
        let actions = actions.clone();
        let path = path.to_path_buf();
        open.connect_activate(move |_, _| (actions.open)(path.clone()));
    }

    let open_in_terminal = gio::SimpleAction::new("open-in-terminal", None);
    {
        let actions = actions.clone();
        let path = path.to_path_buf();
        open_in_terminal.connect_activate(move |_, _| {
            // A file's terminal is its containing folder.
            let dir = if is_dir {
                path.clone()
            } else {
                path.parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| path.clone())
            };
            (actions.open_in_terminal)(dir);
        });
    }

    let copy_path = gio::SimpleAction::new("copy-path", None);
    {
        let actions = actions.clone();
        let path = path.to_path_buf();
        copy_path.connect_activate(move |_, _| (actions.copy_path)(&path));
    }

    group.add_action(&open);
    group.add_action(&open_in_terminal);
    group.add_action(&copy_path);
    row.insert_action_group("tree", Some(&group));

    let gesture = gtk4::GestureClick::new();
    gesture.set_button(gtk4::gdk::BUTTON_SECONDARY);
    {
        let popover = popover.clone();
        gesture.connect_pressed(move |_, _, x, y| {
            popover.set_pointing_to(Some(&gtk4::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            popover.popup();
        });
    }
    row.add_controller(gesture);
}

fn append_dir_rows(
    list: &gtk4::ListBox,
    dir: &Path,
    depth: usize,
    expanded: &Expanded,
    rebuild: &RebuildHolder,
    snapshot: &TreeSnapshot,
    actions: &TreeActions,
) {
    let entries = match snapshot.directories.get(dir) {
        Some(Ok(entries)) => entries,
        Some(Err(_)) => {
            append_status(list, "Folder unavailable", depth);
            return;
        }
        None => return,
    };
    if entries.is_empty() {
        append_status(list, "Empty folder", depth);
    }
    for entry in entries.iter().cloned() {
        // `…` truncation row.
        let Some((path, is_dir)) = entry else {
            let row = gtk4::ListBoxRow::new();
            let l = gtk4::Label::new(Some("…"));
            l.add_css_class("dim-label");
            l.set_margin_start(4 + (depth as i32) * 10);
            l.set_hexpand(true);
            row.set_child(Some(&l));
            list.append(&row);
            continue;
        };

        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let already_expanded = is_dir && expanded.borrow().contains(&path);

        // Expand/collapse chevron (folders only).
        let chevron = gtk4::Button::from_icon_name(if already_expanded {
            "go-down-symbolic"
        } else {
            "go-next-symbolic"
        });
        chevron.add_css_class("flat");
        chevron.set_tooltip_text(Some(if already_expanded {
            "Collapse folder"
        } else {
            "Expand folder"
        }));
        chevron.set_size_request(24, 24);
        if !is_dir {
            chevron.set_visible(false);
        }

        // Per-type icon.
        let icon = gtk4::Image::from_icon_name(icon_name_for(&path, is_dir));
        icon.set_pixel_size(13);

        let title = if is_dir { format!("{name}/") } else { name };
        let name_label = gtk4::Label::new(Some(&title));
        name_label.set_xalign(0.0);
        name_label.set_hexpand(true);
        name_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        if is_dir {
            name_label.add_css_class("heading");
        }

        let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        row.set_margin_start(4 + (depth as i32) * 10);
        row.set_margin_top(0);
        row.set_margin_bottom(0);
        row.append(&chevron);
        row.append(&icon);
        row.append(&name_label);

        let list_row = gtk4::ListBoxRow::new();
        list_row.set_child(Some(&row));
        list_row.set_activatable(true);

        // Chevron click: expand/collapse in place (claimed so the row below
        // doesn't also navigate).
        if is_dir {
            let chevron = chevron.clone();
            let c_path = path.clone();
            let c_expanded = expanded.clone();
            let c_holder = rebuild.clone();
            chevron.connect_clicked(move |_| {
                let mut ex = c_expanded.borrow_mut();
                if !ex.remove(&c_path) {
                    ex.insert(c_path.clone());
                }
                drop(ex);
                let rebuild = c_holder.borrow().clone();
                if let Some(rebuild) = rebuild {
                    rebuild();
                }
            });
        }

        // Single click: folder navigates, file opens with the default app.
        unsafe {
            list_row.set_data(TREE_ENTRY, (path.clone(), is_dir));
        }

        // Double-click / Enter behaves the same.

        // Right-click: Open / Open in Terminal / Copy Path for this row.
        attach_row_context_menu(&list_row, &path, is_dir, actions);

        list.append(&list_row);
        if already_expanded {
            append_dir_rows(
                list,
                &path,
                depth + 1,
                expanded,
                rebuild,
                snapshot,
                actions,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(
        root: &Path,
        expanded: HashSet<PathBuf>,
        cache: Option<Arc<TreeSnapshot>>,
    ) -> TreeRequest {
        TreeRequest {
            root: root.to_path_buf(),
            expanded,
            cache,
            generation: Arc::new(AtomicU64::new(0)),
            revision: 0,
        }
    }

    #[test]
    fn cached_expansion_avoids_reenumerating_directories() {
        let dir = crate::test_support::TestDir::new("tree-cache");
        std::fs::write(dir.path().join("file"), "").unwrap();
        let first = load_tree(request(dir.path(), HashSet::new(), None)).unwrap();
        assert_eq!(first.scans, 1);
        let second = load_tree(request(dir.path(), HashSet::new(), Some(Arc::new(first)))).unwrap();
        assert_eq!(second.scans, 0);
        assert_eq!(second.directories[dir.path()].as_ref().unwrap().len(), 1);
    }

    #[test]
    fn cancelled_requests_stop_before_directory_io() {
        let request = request(
            Path::new("/not-a-real-optionterm-test-path"),
            HashSet::new(),
            None,
        );
        request.generation.store(1, Ordering::Relaxed);
        assert!(matches!(
            load_tree(request),
            Err(std::io::ErrorKind::Interrupted)
        ));
    }

    #[test]
    fn total_visible_entries_are_bounded() {
        let dir = crate::test_support::TestDir::new("tree-budget");
        let mut expanded = HashSet::new();
        for i in 0..3 {
            let child = dir.path().join(format!("dir-{i}"));
            std::fs::create_dir(&child).unwrap();
            for j in 0..600 {
                std::fs::write(child.join(format!("file-{j}")), "").unwrap();
            }
            expanded.insert(child);
        }
        let snapshot = load_tree(request(dir.path(), expanded, None)).unwrap();
        let count: usize = snapshot
            .directories
            .values()
            .map(|listing| listing.as_ref().map_or(1, |entries| entries.len().max(1)))
            .sum();
        assert!(snapshot.limited);
        assert!(count < MAX_VISIBLE_ENTRIES);
    }

    #[gtk4::test]
    fn asynchronous_loading_coalesces_and_discards_stale_results() {
        let dir = crate::test_support::TestDir::new("tree-async");
        let first = dir.path().join("first");
        let last = dir.path().join("last");
        std::fs::create_dir(&first).unwrap();
        std::fs::create_dir(&last).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let release = std::sync::Mutex::new(Some(rx));
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = calls.clone();
        let tree = FileTree::with_loader(
            Arc::new(move |request| {
                count.fetch_add(1, Ordering::Relaxed);
                if let Some(rx) = release.lock().unwrap().take() {
                    assert!(
                        rx.recv_timeout(Duration::from_secs(2)).is_ok(),
                        "UI was blocked by the file tree"
                    );
                }
                load_tree(request)
            }),
            TreeActions::default(),
        );
        assert_eq!(calls.load(Ordering::Relaxed), 0);
        tree.set_root(first);
        tree.set_root(last.clone());
        gtk4::glib::idle_add_local_once(move || {
            let _ = tx.send(());
        });
        crate::test_support::spin_until(|| {
            tree.cache
                .borrow()
                .as_ref()
                .is_some_and(|cache| cache.directories.contains_key(&last))
        });
        assert_eq!(calls.load(Ordering::Relaxed), 2);
        assert_eq!(tree.root.borrow().as_ref(), Some(&last));
    }

    #[gtk4::test]
    fn dropping_tree_cancels_an_inflight_load() {
        let dir = crate::test_support::TestDir::new("tree-cancel");
        let (tx, rx) = std::sync::mpsc::channel();
        let release = std::sync::Mutex::new(Some(rx));
        let finished = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let done = finished.clone();
        let tree = FileTree::with_loader(
            Arc::new(move |request| {
                release
                    .lock()
                    .unwrap()
                    .take()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(2))
                    .unwrap();
                let result = load_tree(request);
                assert!(matches!(result, Err(std::io::ErrorKind::Interrupted)));
                done.store(true, Ordering::Relaxed);
                result
            }),
            TreeActions::default(),
        );
        tree.set_root(dir.path().to_path_buf());
        let widget = tree.widget.downgrade();
        drop(tree);
        assert!(widget.upgrade().is_none());
        tx.send(()).unwrap();
        crate::test_support::spin_until(|| finished.load(Ordering::Relaxed));
    }

    #[gtk4::test]
    fn invalid_navigation_preserves_last_loaded_root() {
        let dir = crate::test_support::TestDir::new("tree-invalid");
        let tree = FileTree::new();
        tree.set_root(dir.path().to_path_buf());
        crate::test_support::spin_until(|| tree.cache.borrow().is_some());
        tree.set_root(dir.path().join("missing"));
        crate::test_support::spin_until(|| tree.root.borrow().as_deref() == Some(dir.path()));
        assert!(tree.header.tooltip_text().is_some());
    }

    #[gtk4::test]
    fn dropping_tree_releases_rebuild_and_widgets() {
        let dir = crate::test_support::TestDir::new("tree-lifecycle");
        std::fs::create_dir(dir.path().join("folder")).unwrap();
        let tree = FileTree::new();
        tree.set_root(dir.path().to_path_buf());
        crate::test_support::spin_until(|| tree.cache.borrow().is_some());
        let widget = tree.widget.downgrade();
        let rebuild = Rc::downgrade(&tree.rebuild);
        drop(tree);
        assert!(widget.upgrade().is_none());
        assert!(rebuild.upgrade().is_none());
    }

    #[gtk4::test]
    fn expanded_folder_precedes_children() {
        let dir = crate::test_support::TestDir::new("tree-order");
        let folder = dir.path().join("parent");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(folder.join("child"), "").unwrap();
        let list = gtk4::ListBox::new();
        let expanded: Expanded = Rc::new(RefCell::new(HashSet::from([folder])));
        let navigate: NavigateFn = Rc::new(|_| {});
        let opened = Rc::new(Cell::new(0));
        let count = opened.clone();
        bind_tree_activation(
            &list,
            navigate,
            Rc::new(move |_| count.set(count.get() + 1)),
        );
        let rebuild: RebuildHolder = Rc::new(RefCell::new(None));
        let snapshot = load_tree(request(
            dir.path(),
            expanded.borrow().iter().cloned().collect(),
            None,
        ))
        .unwrap();
        append_dir_rows(
            &list,
            dir.path(),
            0,
            &expanded,
            &rebuild,
            &snapshot,
            &TreeActions::default(),
        );
        let titles: Vec<String> = (0..2)
            .map(|i| {
                list.row_at_index(i)
                    .unwrap()
                    .child()
                    .unwrap()
                    .last_child()
                    .unwrap()
                    .downcast::<gtk4::Label>()
                    .unwrap()
                    .text()
                    .to_string()
            })
            .collect();
        assert_eq!(titles, ["parent/", "child"]);
        let window = gtk4::Window::new();
        window.set_child(Some(&list));
        list.row_at_index(1)
            .unwrap()
            .emit_by_name::<()>("activate", &[]);
        assert_eq!(opened.get(), 1);
        let toggle = list
            .row_at_index(0)
            .unwrap()
            .child()
            .unwrap()
            .first_child()
            .unwrap()
            .downcast::<gtk4::Button>()
            .unwrap();
        toggle.emit_clicked();
        assert_eq!(opened.get(), 1);
        assert!(expanded.borrow().is_empty());
        window.destroy();
    }
}
