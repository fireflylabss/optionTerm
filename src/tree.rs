//! A minimal file-tree widget, "IDE-style".
//!
//! A `ListBox` of indented rows with per-type icons: clicking a folder
//! navigates into it (the tree re-roots), clicking a file opens it with the
//! system's default application. The chevron expands/collapses a folder in
//! place without navigating.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

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

pub struct FileTree {
    pub widget: gtk4::ScrolledWindow,
    header: gtk4::Label,
    root: Rc<RefCell<Option<PathBuf>>>,
    rebuild: RebuildHolder,
}

impl FileTree {
    pub fn new() -> Self {
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

        let expanded: Rc<RefCell<Vec<PathBuf>>> = Rc::new(RefCell::new(Vec::new()));
        let root: Rc<RefCell<Option<PathBuf>>> = Rc::new(RefCell::new(None));
        let rebuild: RebuildHolder = Rc::new(RefCell::new(None));

        // Folder navigation: re-root the tree.
        let header_nav = header.clone();
        let root_nav = root.clone();
        let rebuild_nav = rebuild.clone();
        let navigate: NavigateFn = Rc::new(move |dir: PathBuf| {
            if !dir.is_dir() {
                return;
            }
            *root_nav.borrow_mut() = Some(dir.clone());
            header_nav.set_text(&format!("Files — {}", dir.display()));
            if let Some(r) = rebuild_nav.borrow().as_ref() {
                r();
            }
        });

        {
            let list = list.clone();
            let root = root.clone();
            let expanded = expanded.clone();
            let navigate = navigate.clone();
            let holder = rebuild.clone();
            *rebuild.borrow_mut() = Some(Rc::new(move || {
                while let Some(child) = list.first_child() {
                    list.remove(&child);
                }
                let Some(root) = root.borrow().clone() else {
                    return;
                };
                expanded.borrow_mut().retain(|d| d.starts_with(&root));
                append_dir_rows(&list, &root, 0, &expanded, &navigate, &holder);
            }));
        }

        let tree = FileTree {
            widget: scroller,
            header,
            root,
            rebuild,
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
        if !dir.is_dir() {
            return;
        }
        *self.root.borrow_mut() = Some(dir.clone());
        self.header.set_text(&format!("Files — {}", dir.display()));
        self.refresh();
    }

    /// Re-render from the stored root (call on tab focus / cwd change).
    pub fn refresh(&self) {
        if let Some(rebuild) = self.rebuild.borrow().as_ref() {
            rebuild();
        }
    }
}

/// Sorted (dirs-first, then files) entries under `dir`.
fn entries_sorted(dir: &Path) -> Vec<Option<(PathBuf, bool)>> {
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    let mut truncated = false;
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    for (i, e) in entries.flatten().enumerate() {
        if i >= MAX_ENTRIES_PER_DIR {
            truncated = true;
            break;
        }
        let p = e.path();
        if let Ok(md) = std::fs::metadata(&p)
            && md.is_dir()
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
    out
}

/// Open a file with the system's default application.
fn open_with_default(path: &Path) {
    let uri = gio::File::for_path(path).uri();
    if let Err(err) = gio::AppInfo::launch_default_for_uri(&uri, gio::AppLaunchContext::NONE) {
        tracing::warn!("opening {uri} failed: {err}");
    }
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

fn append_dir_rows(
    list: &gtk4::ListBox,
    dir: &Path,
    depth: usize,
    expanded: &Rc<RefCell<Vec<PathBuf>>>,
    navigate: &NavigateFn,
    rebuild: &RebuildHolder,
) {
    for entry in entries_sorted(dir) {
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
        let already_expanded = is_dir && expanded.borrow().iter().any(|d| d == &path);

        // Expand/collapse chevron (folders only).
        let chevron = gtk4::Image::from_icon_name(if already_expanded {
            "go-down-symbolic"
        } else {
            "go-next-symbolic"
        });
        chevron.set_pixel_size(10);
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
            let gesture = gtk4::GestureClick::new();
            gesture.set_button(gtk4::gdk::BUTTON_PRIMARY);
            gesture.set_propagation_phase(gtk4::PropagationPhase::Capture);
            gesture.connect_pressed(move |g, _, _, _| {
                g.set_state(gtk4::EventSequenceState::Claimed);
                let mut ex = c_expanded.borrow_mut();
                if ex.iter().any(|d| d == &c_path) {
                    ex.retain(|d| d != &c_path);
                } else {
                    ex.push(c_path.clone());
                }
                drop(ex);
                if let Some(r) = c_holder.borrow().as_ref() {
                    r();
                }
            });
            chevron.add_controller(gesture);
        }

        // Single click: folder navigates, file opens with the default app.
        let row_path = path.clone();
        let nav = navigate.clone();
        let gesture = gtk4::GestureClick::new();
        gesture.set_button(gtk4::gdk::BUTTON_PRIMARY);
        gesture.connect_pressed(move |_, _, _, _| {
            if is_dir {
                nav(row_path.clone());
            } else {
                open_with_default(&row_path);
            }
        });
        list_row.add_controller(gesture);

        // Double-click / Enter behaves the same.
        let act_path = path.clone();
        let act_nav = navigate.clone();
        list_row.connect_activate(move |_| {
            if is_dir {
                act_nav(act_path.clone());
            } else {
                open_with_default(&act_path);
            }
        });

        if already_expanded {
            append_dir_rows(list, &path, depth + 1, expanded, navigate, rebuild);
        }
        list.append(&list_row);
    }
}
