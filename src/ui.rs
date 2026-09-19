//! Menus, context menu and dialogs (palette, preferences, shortcuts, about).

use std::{cell::RefCell, path::PathBuf, rc::Rc};

use gtk4::gdk;
use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;

use crate::{
    app::Pages,
    config::{
        Config, CursorStyle, MiddleClickTab, NewTabPosition, TabOverflow, TabWidth, TabsLocation,
        Theme,
    },
    keys::Bindings,
    session::slugify_session_name,
    terminal::TerminalView,
};

/// Menu behind the new-tab `+` button: grouped by what it opens.
pub fn tabs_menu(agents: &gio::Menu) -> gio::Menu {
    let menu = gio::Menu::new();

    // New surfaces.
    let tabs = gio::Menu::new();
    tabs.append(Some("New Tab"), Some("win.new-tab"));
    tabs.append(Some("Open Browser…"), Some("win.open-browser"));
    tabs.append(Some("File Explorer"), Some("win.file-tree"));
    menu.append_section(None, &tabs);

    // Splitting the focused pane (also reachable via the split button).
    menu.append_submenu(Some("Split"), &splits_menu());

    // One entry per installed agent; opens a dedicated agent tab.
    menu.append_submenu(Some("Agents"), agents);
    menu
}

/// Menu behind the splits button: split directions (the button's main click
/// re-runs the last-chosen direction, defaulting to Split Right).
pub fn splits_menu() -> gio::Menu {
    let split = gio::Menu::new();
    split.append(Some("Split Right"), Some("win.split-right"));
    split.append(Some("Split Down"), Some("win.split-down"));
    split.append(Some("Split Left"), Some("win.split-left"));
    split.append(Some("Split Up"), Some("win.split-up"));
    split.append(Some("Toggle Split Zoom"), Some("win.toggle-split-zoom"));
    split.append(Some("Equalize Splits"), Some("win.equalize-splits"));
    split
}

/// Right-click on a tab: rename, split, close.
pub fn tab_menu() -> gio::Menu {
    let menu = gio::Menu::new();

    let rename = gio::Menu::new();
    rename.append(Some("Rename…"), Some("win.rename-tab"));
    menu.append_section(None, &rename);

    menu.append_submenu(Some("Split"), &splits_menu());

    let close = gio::Menu::new();
    close.append(Some("Close Tab"), Some("win.close-tab"));
    menu.append_section(None, &close);
    menu
}

/// The `···` menu, deliberately short: appearance lives in Preferences and
/// tab/split/terminal actions live behind `+`, so what is left here is the
/// window-wide odds and ends plus the quick controls in [`quick_settings`].
fn main_menu() -> gio::Menu {
    let menu = gio::Menu::new();

    // Anchor for the theme/zoom widget built by `quick_settings`.
    let header = gio::MenuItem::new(None, None);
    header.set_attribute_value("custom", Some(&QUICK_SETTINGS_ID.to_variant()));
    let header_section = gio::Menu::new();
    header_section.append_item(&header);
    menu.append_section(None, &header_section);

    let edit = gio::Menu::new();
    edit.append(Some("Copy"), Some("win.copy"));
    edit.append(Some("Paste"), Some("win.paste"));
    edit.append(Some("Select All"), Some("win.select-all"));
    menu.append_section(None, &edit);

    let tools = gio::Menu::new();
    tools.append(Some("Find…"), Some("win.find"));
    tools.append(Some("Open Browser…"), Some("win.open-browser"));
    tools.append(Some("File Explorer"), Some("win.file-tree"));
    tools.append(Some("Command Palette"), Some("win.command-palette"));
    menu.append_section(None, &tools);

    // Named workspaces: whole-window state, so they get their own section.
    let sessions = gio::Menu::new();
    sessions.append(Some("Save Session As…"), Some("win.save-session-as"));
    sessions.append(Some("Load Session…"), Some("win.load-session"));
    menu.append_section(None, &sessions);

    let term = gio::Menu::new();
    term.append(Some("Restart Terminal"), Some("win.restart-tab"));
    menu.append_section(None, &term);

    let app = gio::Menu::new();
    app.append(Some("Preferences"), Some("win.preferences"));
    app.append(Some("Keyboard Shortcuts"), Some("win.shortcuts"));
    app.append(Some("About optionTerm"), Some("win.about"));
    menu.append_section(None, &app);

    let quit = gio::Menu::new();
    quit.append(Some("Quit"), Some("win.quit"));
    menu.append_section(None, &quit);

    menu
}

/// Name tying the custom widget to its slot in [`main_menu`].
const QUICK_SETTINGS_ID: &str = "quick-settings";

/// The `···` menu as a real popover, so the theme picker and zoom stepper can
/// be actual widgets. A `GMenu` can only hold text items.
pub fn main_popover() -> (gtk4::PopoverMenu, QuickSettings) {
    let popover = gtk4::PopoverMenu::from_model_full(&main_menu(), gtk4::PopoverMenuFlags::NESTED);
    let quick = QuickSettings::new();
    popover.add_child(&quick.widget, QUICK_SETTINGS_ID);
    (popover, quick)
}

/// A font-size stepper and the current grid size at the top of the main menu.
/// The theme is set in Preferences instead, so there is one place for it.
pub struct QuickSettings {
    pub widget: gtk4::Box,
    zoom_label: gtk4::Label,
    grid_label: gtk4::Label,
}

impl QuickSettings {
    fn new() -> Self {
        let widget = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
        widget.add_css_class("quick-settings");

        // --- Font size ---
        let zoom = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        let out = gtk4::Button::from_icon_name("list-remove-symbolic");
        out.add_css_class("circular");
        out.set_tooltip_text(Some("Decrease font size"));
        out.set_action_name(Some("win.zoom-out"));

        let zoom_label = gtk4::Label::new(Some("100%"));
        zoom_label.set_hexpand(true);
        zoom_label.add_css_class("heading");
        // Reset on click is the obvious meaning of pressing the readout.
        let reset = gtk4::Button::builder().child(&zoom_label).build();
        reset.add_css_class("flat");
        reset.set_hexpand(true);
        reset.set_tooltip_text(Some("Reset font size"));
        reset.set_action_name(Some("win.zoom-reset"));

        let into = gtk4::Button::from_icon_name("list-add-symbolic");
        into.add_css_class("circular");
        into.set_tooltip_text(Some("Increase font size"));
        into.set_action_name(Some("win.zoom-in"));

        zoom.append(&out);
        zoom.append(&reset);
        zoom.append(&into);
        widget.append(&zoom);

        // --- Grid size ---
        let grid_label = gtk4::Label::new(None);
        grid_label.add_css_class("dim-label");
        grid_label.add_css_class("caption");
        widget.append(&grid_label);

        Self {
            widget,
            zoom_label,
            grid_label,
        }
    }

    /// `size` against the size the window opened with, so 100% is the size the
    /// user configured rather than an arbitrary constant.
    pub fn set_font_size(&self, size: f32, base: f32) {
        let percent = if base > 0.0 {
            (size / base * 100.0).round()
        } else {
            100.0
        };
        self.zoom_label.set_text(&format!("{percent:.0}%"));
    }

    pub fn set_grid(&self, cols: u16, rows: u16) {
        self.grid_label.set_text(&format!("{cols} × {rows}"));
    }
}

/// Right-click inside a terminal. Deliberately short: clipboard, the two
/// terminal actions, and everything split-related folded into one submenu so
/// six directions do not crowd out the two entries people actually reach for.
fn context_menu() -> gio::Menu {
    let menu = gio::Menu::new();

    let edit = gio::Menu::new();
    edit.append(Some("Copy"), Some("win.copy"));
    edit.append(Some("Paste"), Some("win.paste"));
    menu.append_section(None, &edit);

    menu.append_submenu(Some("Split"), &splits_menu());

    let misc = gio::Menu::new();
    misc.append(Some("Select All"), Some("win.select-all"));
    misc.append(Some("Clear Terminal"), Some("win.clear-tab"));
    misc.append(Some("Find…"), Some("win.find"));
    menu.append_section(None, &misc);
    menu
}

pub fn attach_context_menu(view: &Rc<TerminalView>) {
    let popover = gtk4::PopoverMenu::from_model(Some(&context_menu()));
    popover.set_parent(view.widget());
    popover.set_has_arrow(false);
    popover.set_halign(gtk4::Align::Start);

    {
        let popover = popover.clone();
        view.widget().connect_destroy(move |_| popover.unparent());
    }

    let gesture = gtk4::GestureClick::new();
    gesture.set_button(gdk::BUTTON_SECONDARY);
    {
        let popover = popover.clone();
        gesture.connect_pressed(move |_, _, x, y| {
            popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            popover.popup();
        });
    }
    view.widget().add_controller(gesture);
}

/// Scrollback search bar (`Ctrl+Shift+F`).
///
/// Returned widget is meant to live in a `gtk::Revealer` above the terminal;
/// `current_view` is queried lazily so the bar always searches the focused
/// pane, even after the user switches tabs with the bar open.
pub struct SearchBar {
    pub widget: gtk4::SearchBar,
    entry: gtk4::SearchEntry,
    sync: Rc<dyn Fn() -> Option<Rc<TerminalView>>>,
    query: Rc<RefCell<String>>,
}

impl SearchBar {
    pub fn new(current_view: Rc<dyn Fn() -> Option<Rc<TerminalView>>>) -> Self {
        let entry = gtk4::SearchEntry::new();
        entry.set_placeholder_text(Some("Search scrollback…"));
        entry.set_hexpand(true);

        let prev = gtk4::Button::from_icon_name("go-up-symbolic");
        prev.set_tooltip_text(Some("Previous match (Shift+Enter)"));
        prev.add_css_class("flat");
        let next = gtk4::Button::from_icon_name("go-down-symbolic");
        next.set_tooltip_text(Some("Next match (Enter)"));
        next.add_css_class("flat");

        let boxed = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        boxed.append(&entry);
        boxed.append(&prev);
        boxed.append(&next);

        let bar = gtk4::SearchBar::builder()
            .search_mode_enabled(false)
            .show_close_button(true)
            .child(&boxed)
            .build();
        bar.connect_entry(&entry);
        let target: Rc<RefCell<Option<std::rc::Weak<TerminalView>>>> = Rc::new(RefCell::new(None));
        let last_query = Rc::new(RefCell::new(String::new()));
        let sync: Rc<dyn Fn() -> Option<Rc<TerminalView>>> = {
            let current_view = current_view.clone();
            let target = target.clone();
            let last_query = last_query.clone();
            let entry = entry.downgrade();
            Rc::new(move || {
                let next = current_view();
                let old = target.borrow().as_ref().and_then(std::rc::Weak::upgrade);
                let same = old
                    .as_ref()
                    .zip(next.as_ref())
                    .is_some_and(|(a, b)| Rc::ptr_eq(a, b));
                let query = entry.upgrade()?.text().to_string();
                if !same || *last_query.borrow() != query {
                    if let Some(old) = old {
                        old.search_set_query("");
                    }
                    if let Some(next) = next.as_ref() {
                        next.search_set_query(&query);
                    }
                    *target.borrow_mut() = next.as_ref().map(Rc::downgrade);
                    *last_query.borrow_mut() = query;
                }
                next
            })
        };
        {
            let current_view = current_view.clone();
            bar.connect_search_mode_enabled_notify(move |bar| {
                if !bar.is_search_mode() {
                    let previous = target.borrow_mut().take().and_then(|v| v.upgrade());
                    if let Some(previous) = previous {
                        previous.search_set_query("");
                    }
                    if let Some(view) = current_view() {
                        view.focus();
                    }
                }
            });
        }
        {
            let bar = bar.downgrade();
            entry.connect_stop_search(move |_| {
                if let Some(bar) = bar.upgrade() {
                    bar.set_search_mode(false);
                }
            });
        }
        let current_view = sync.clone();

        {
            let current_view = current_view.clone();
            let bar = bar.downgrade();
            entry.connect_search_changed(move |e| {
                if !bar.upgrade().is_some_and(|bar| bar.is_search_mode()) {
                    return;
                }
                let query = e.text().to_string();
                if let Some(view) = current_view()
                    && !query.is_empty()
                {
                    let _ = view.search_find_next();
                }
            });
        }
        {
            let current_view = current_view.clone();
            entry.connect_activate(move |_| {
                if let Some(view) = current_view() {
                    let _ = view.search_find_next();
                }
            });
        }
        {
            let current_view = current_view.clone();
            next.connect_clicked(move |_| {
                if let Some(view) = current_view() {
                    let _ = view.search_find_next();
                }
            });
        }
        {
            let current_view = current_view.clone();
            prev.connect_clicked(move |_| {
                if let Some(view) = current_view() {
                    let _ = view.search_find_previous();
                }
            });
        }
        {
            let bar_weak = bar.downgrade();
            let current_view = current_view.clone();
            let key = gtk4::EventControllerKey::new();
            key.set_propagation_phase(gtk4::PropagationPhase::Capture);
            key.connect_key_pressed(move |_, keyval, _, modifier| {
                if keyval == gdk::Key::Return && modifier.contains(gdk::ModifierType::SHIFT_MASK) {
                    if let Some(view) = current_view() {
                        let _ = view.search_find_previous();
                    }
                    return gtk4::glib::Propagation::Stop;
                }
                if keyval == gdk::Key::Escape {
                    if let Some(bar) = bar_weak.upgrade() {
                        bar.set_search_mode(false);
                    }
                    return gtk4::glib::Propagation::Stop;
                }
                gtk4::glib::Propagation::Proceed
            });
            entry.add_controller(key);
        }

        Self {
            widget: bar,
            entry,
            sync,
            query: last_query,
        }
    }

    pub fn sync_target(&self) {
        if self.widget.is_search_mode() {
            (self.sync)();
        }
    }

    /// Open the bar and focus the entry; re-running the query if there is one.
    pub fn open(&self) {
        if self.entry.text().is_empty() {
            let query = self.query.borrow().clone();
            self.entry.set_text(&query);
        }
        self.widget.set_search_mode(true);
        if let Some(view) = (self.sync)()
            && !self.entry.text().is_empty()
        {
            let _ = view.search_find_next();
        }
        self.entry.grab_focus();
        self.entry.select_region(0, -1);
    }
}

/// One actionable row of the command palette.
#[derive(Clone)]
enum PaletteAction {
    Win(&'static str),
    Launch(crate::launch::LaunchRequest),
    /// Whatever the user typed, tokenized on the last keystroke.
    Typed(Vec<String>),
}

/// Should the synthetic "Run: <query>" row be shown? It duplicates a
/// `[[command]]` preset exactly when the query spells that preset's name,
/// and then the preset row wins. Whitespace-only queries have nothing to
/// run; unparseable ones (unbalanced quotes) still show, so Enter is a
/// no-op instead of falling through to a preset.
fn typed_row_visible(query: &str, entries: &[(String, String, PaletteAction)]) -> bool {
    if query.trim().is_empty() {
        return false;
    }
    let typed = format!("Run: {query}").to_lowercase();
    !entries
        .iter()
        .any(|(label, _, _)| label.to_lowercase() == typed)
}

/// Build the palette dialog and the widgets that drive it, as
/// `(dialog, entry, list)`. Split from `show_command_palette` so tests can
/// exercise the palette without presenting the dialog.
fn build_command_palette(
    window: &adw::ApplicationWindow,
    config: &Rc<RefCell<Config>>,
    bindings: &Bindings,
    open_launch: Rc<dyn Fn(crate::launch::LaunchRequest)>,
    current_dir: Rc<dyn Fn() -> Option<PathBuf>>,
) -> (adw::Dialog, gtk4::SearchEntry, gtk4::ListBox) {
    let dialog = adw::Dialog::builder()
        .title("Command Palette")
        .content_width(460)
        .content_height(480)
        .build();

    let root = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    root.set_margin_top(6);
    root.set_margin_bottom(6);
    root.set_margin_start(6);
    root.set_margin_end(6);

    let entry = gtk4::SearchEntry::new();
    entry.set_placeholder_text(Some("Type a command…"));
    root.append(&entry);

    let list = gtk4::ListBox::new();
    list.set_selection_mode(gtk4::SelectionMode::Single);
    list.add_css_class("boxed-list");

    let mut entries: Vec<(String, String, PaletteAction)> = bindings
        .effective()
        .into_iter()
        .map(|(label, action, accel)| (label.to_string(), accel, PaletteAction::Win(action)))
        .collect();
    for cmd in &config.borrow().commands {
        entries.push((
            format!("Run: {}", cmd.name),
            String::new(),
            PaletteAction::Launch(crate::launch::LaunchRequest {
                cwd: cmd.cwd.as_ref().map(PathBuf::from),
                command: Some(cmd.argv.clone()),
            }),
        ));
    }
    let entries = Rc::new(entries);

    // Synthetic "Run: <typed query>" row, pinned to model index 0 so Enter
    // picks it first. Its label tracks the entry text; activating it runs
    // the tokenized query.
    let typed_row = gtk4::ListBoxRow::new();
    let typed_hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
    typed_hbox.set_margin_top(8);
    typed_hbox.set_margin_bottom(8);
    typed_hbox.set_margin_start(12);
    typed_hbox.set_margin_end(12);
    let typed_label = gtk4::Label::new(None);
    typed_label.set_halign(gtk4::Align::Start);
    typed_label.set_hexpand(true);
    typed_hbox.append(&typed_label);
    typed_row.set_child(Some(&typed_hbox));
    list.append(&typed_row);

    for (label, accel, _) in entries.iter() {
        let row = gtk4::ListBoxRow::new();
        let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
        hbox.set_margin_top(8);
        hbox.set_margin_bottom(8);
        hbox.set_margin_start(12);
        hbox.set_margin_end(12);
        let name = gtk4::Label::new(Some(label));
        name.set_halign(gtk4::Align::Start);
        name.set_hexpand(true);
        hbox.append(&name);
        if !accel.is_empty() {
            let key = gtk4::Label::new(Some(accel));
            key.add_css_class("dim-label");
            key.add_css_class("numeric");
            hbox.append(&key);
        }
        row.set_child(Some(&hbox));
        list.append(&row);
    }

    // Raw, case-preserving query: the typed row runs exactly what the user
    // typed, while the filter lowercases its own copy for matching.
    let query: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));
    let typed_action: Rc<RefCell<Option<PaletteAction>>> = Rc::new(RefCell::new(None));
    {
        let query = query.clone();
        let entries = entries.clone();
        list.set_filter_func(move |row| {
            let q = query.borrow();
            if row.index() == 0 {
                return typed_row_visible(&q, &entries);
            }
            if q.is_empty() {
                return true;
            }
            let lower = q.to_lowercase();
            entries
                .get(row.index() as usize - 1)
                .map(|(label, _, _)| {
                    let needle = label.to_lowercase();
                    lower.split_whitespace().all(|w| needle.contains(w))
                })
                .unwrap_or(false)
        });
    }
    {
        let query = query.clone();
        let list = list.clone();
        let typed_label = typed_label.clone();
        let typed_action = typed_action.clone();
        entry.connect_search_changed(move |e| {
            let text = e.text().to_string();
            typed_label.set_text(&format!("Run: {text}"));
            *typed_action.borrow_mut() =
                crate::launch::tokenize_shell(&text).map(PaletteAction::Typed);
            *query.borrow_mut() = text;
            list.invalidate_filter();
        });
    }

    let activate = {
        let window = window.clone();
        let dialog = dialog.clone();
        let entries = entries.clone();
        let open_launch = open_launch.clone();
        let typed_action = typed_action.clone();
        let current_dir = current_dir.clone();
        Rc::new(move |row: &gtk4::ListBoxRow| {
            if row.index() == 0 {
                // An unparseable query (e.g. unbalanced quotes) leaves the
                // dialog open so the user can fix it.
                let Some(PaletteAction::Typed(argv)) = typed_action.borrow().as_ref().cloned()
                else {
                    return;
                };
                dialog.close();
                let req = crate::launch::LaunchRequest {
                    cwd: current_dir(),
                    command: Some(argv),
                };
                open_launch(req);
                return;
            }
            let Some((_, _, action)) = entries.get(row.index() as usize - 1) else {
                return;
            };
            dialog.close();
            match action {
                PaletteAction::Win(action) => {
                    gtk4::prelude::WidgetExt::activate_action(&window, action, None).ok();
                }
                PaletteAction::Launch(req) => open_launch(req.clone()),
                PaletteAction::Typed(_) => unreachable!("typed actions live on the first row"),
            }
        })
    };

    {
        let activate = activate.clone();
        list.connect_row_activated(move |_, row| activate(row));
    }
    {
        let activate = activate.clone();
        let list = list.clone();
        entry.connect_activate(move |_| {
            let mut idx = 0;
            while let Some(row) = list.row_at_index(idx) {
                if row.is_mapped() {
                    activate(&row);
                    return;
                }
                idx += 1;
            }
        });
    }

    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_child(Some(&list));
    root.append(&scroll);

    dialog.set_child(Some(&root));

    // Escape has to be handled twice over. GtkSearchEntry swallows it to clear
    // its own text, so the dialog never sees the first press; and once the list
    // has focus the entry is not involved at all.
    {
        let dialog = dialog.clone();
        entry.connect_stop_search(move |_| {
            dialog.close();
        });
    }
    {
        let dialog_for_keys = dialog.clone();
        let keys = gtk4::EventControllerKey::new();
        keys.set_propagation_phase(gtk4::PropagationPhase::Capture);
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gdk::Key::Escape {
                dialog_for_keys.close();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        dialog.add_controller(keys);
    }
    // Clicking outside dismisses it.
    dialog.set_can_close(true);

    (dialog, entry, list)
}

pub fn show_command_palette(
    window: &adw::ApplicationWindow,
    config: &Rc<RefCell<Config>>,
    bindings: &Bindings,
    open_launch: Rc<dyn Fn(crate::launch::LaunchRequest)>,
    current_dir: Rc<dyn Fn() -> Option<PathBuf>>,
) {
    let (dialog, entry, _list) =
        build_command_palette(window, config, bindings, open_launch, current_dir);
    dialog.present(Some(window));
    entry.grab_focus();
}

/// A Codex thread row: title, search subtitle and preview targets.
type CodexThreadRows = Vec<(String, String)>;

/// Searchable picker over the saved Codex conversations: activating a row
/// exports that thread as Markdown; the per-row "Open" button round-trips an
/// already saved transcript.
pub fn show_codex_picker(
    window: &adw::ApplicationWindow,
    threads: Vec<crate::codex::CodexThread>,
    export: Rc<dyn Fn(crate::codex::CodexThread)>,
    open: Rc<dyn Fn(PathBuf)>,
) {
    let dialog = adw::Dialog::builder()
        .title("Codex Threads")
        .content_width(520)
        .content_height(480)
        .build();
    let (root, entry, _list) = codex_picker(&dialog, window.downgrade(), threads, export, open);
    dialog.set_child(Some(&root));

    // Escape has to be handled twice over (see show_command_palette).
    {
        let dialog = dialog.clone();
        entry.connect_stop_search(move |_| {
            dialog.close();
        });
    }
    {
        let dialog_for_keys = dialog.clone();
        let keys = gtk4::EventControllerKey::new();
        keys.set_propagation_phase(gtk4::PropagationPhase::Capture);
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gdk::Key::Escape {
                dialog_for_keys.close();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        dialog.add_controller(keys);
    }
    // Clicking outside dismisses it.
    dialog.set_can_close(true);

    dialog.present(Some(window));
    entry.grab_focus();
}

/// Build the picker contents: the root box, the search entry and the thread
/// list. Split from [`show_codex_picker`] so tests can exercise the rows and
/// filtering without presenting a dialog over a live window. `window` is
/// weak: it is only needed when the user clicks a row's "Open" button, to
/// parent the transcript chooser.
fn codex_picker(
    dialog: &adw::Dialog,
    window: glib::WeakRef<adw::ApplicationWindow>,
    threads: Vec<crate::codex::CodexThread>,
    export: Rc<dyn Fn(crate::codex::CodexThread)>,
    open: Rc<dyn Fn(PathBuf)>,
) -> (gtk4::Box, gtk4::SearchEntry, gtk4::ListBox) {
    let root = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    root.set_margin_top(6);
    root.set_margin_bottom(6);
    root.set_margin_start(6);
    root.set_margin_end(6);

    let entry = gtk4::SearchEntry::new();
    entry.set_placeholder_text(Some("Search Codex threads…"));
    root.append(&entry);

    let list = gtk4::ListBox::new();
    list.set_selection_mode(gtk4::SelectionMode::Single);
    list.add_css_class("boxed-list");

    let threads = Rc::new(threads);
    let rows: CodexThreadRows = if threads.is_empty() {
        let row = adw::ActionRow::builder()
            .title("No Codex threads found")
            .activatable(false)
            .build();
        list.append(&row);
        Vec::new()
    } else {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(i64::MAX);
        let icon_name = codex_thread_icon_name();
        let rows: CodexThreadRows = threads
            .iter()
            .map(|thread| {
                let subtitle = if thread.time > 0 {
                    crate::codex::format_relative_time(thread.time, now)
                } else {
                    thread.id.clone()
                };
                (thread.display_title(), subtitle)
            })
            .collect();
        for (thread, (title, subtitle)) in threads.iter().zip(&rows) {
            let row = adw::ActionRow::builder()
                .title(title.clone())
                .subtitle(subtitle.clone())
                .activatable(true)
                .build();
            row.add_prefix(&gtk4::Image::from_icon_name(icon_name));

            // Round trip: open an already saved Markdown for this thread.
            let open_button = gtk4::Button::builder()
                .label("Open")
                .valign(gtk4::Align::Center)
                .css_classes(["flat"])
                .build();
            {
                let dialog = dialog.clone();
                let window = window.clone();
                let open = open.clone();
                let thread = thread.clone();
                open_button.connect_clicked(move |_| {
                    let Some(window) = window.upgrade() else {
                        return;
                    };
                    let chooser = gtk4::FileDialog::builder()
                        .title("Open saved transcript")
                        .initial_name(format!("{}.md", crate::codex::slugify(&thread.title)))
                        .build();
                    // Per-click clones: the response callback is FnOnce,
                    // the button itself can be clicked again.
                    let dialog = dialog.clone();
                    let open = open.clone();
                    chooser.open(Some(&window), None::<&gio::Cancellable>, move |res| {
                        let Ok(file) = res else {
                            return; // cancelled
                        };
                        let Some(path) = file.path() else {
                            return;
                        };
                        dialog.close();
                        open(path);
                    });
                });
            }
            row.add_suffix(&open_button);
            list.append(&row);
        }
        rows
    };

    // Filtering follows the command palette: a row is visible when every
    // whitespace-separated query word appears in its title or subtitle.
    let labels = Rc::new(
        rows.iter()
            .map(|(title, subtitle)| format!("{title} {subtitle}").to_lowercase())
            .collect::<Vec<_>>(),
    );
    let query: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));
    {
        let query = query.clone();
        let labels = labels.clone();
        list.set_filter_func(move |row| {
            let q = query.borrow();
            if q.is_empty() {
                return true;
            }
            labels
                .get(row.index() as usize)
                .map(|haystack| q.split_whitespace().all(|w| haystack.contains(w)))
                .unwrap_or(false)
        });
    }
    {
        let query = query.clone();
        let list = list.clone();
        entry.connect_search_changed(move |e| {
            *query.borrow_mut() = e.text().to_lowercase();
            list.invalidate_filter();
        });
    }

    let activate = {
        let dialog = dialog.clone();
        let threads = threads.clone();
        let export = export.clone();
        Rc::new(move |row: &gtk4::ListBoxRow| {
            let Some(thread) = threads.get(row.index() as usize) else {
                return;
            };
            dialog.close();
            export(thread.clone());
        })
    };
    {
        let activate = activate.clone();
        list.connect_row_activated(move |_, row| activate(row));
    }
    {
        let activate = activate.clone();
        let list = list.clone();
        entry.connect_activate(move |_| {
            let mut idx = 0;
            while let Some(row) = list.row_at_index(idx) {
                if row.is_mapped() {
                    activate(&row);
                    return;
                }
                idx += 1;
            }
        });
    }

    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_child(Some(&list));
    root.append(&scroll);

    (root, entry, list)
}

/// The Codex agent logo when the icon theme has it, else a generic document
/// glyph.
fn codex_thread_icon_name() -> &'static str {
    let Some(display) = gdk::Display::default() else {
        return "text-x-generic-symbolic";
    };
    let theme = gtk4::IconTheme::for_display(&display);
    let dark = adw::StyleManager::default().is_dark();
    let preferred = crate::agents::AgentKind::Codex.theme_icon_name(dark);
    if theme.has_icon(preferred) {
        preferred
    } else {
        "text-x-generic-symbolic"
    }
}

/// Named-workspace picker: save the current session under a typed name, then
/// load or delete existing profiles. `current` prefills the name row when a
/// profile is active. The callbacks receive profile names verbatim (typed
/// text for save, listed slug for load/delete).
#[allow(clippy::too_many_arguments)]
pub fn show_session_profile_picker(
    window: &adw::ApplicationWindow,
    profiles: Vec<String>,
    current: Option<String>,
    on_save: Rc<dyn Fn(String)>,
    on_load: Rc<dyn Fn(String)>,
    on_delete: Rc<dyn Fn(String)>,
) {
    let dialog = adw::Dialog::builder()
        .title("Session Profiles")
        .content_width(440)
        .content_height(480)
        .build();

    let root = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    root.set_margin_top(6);
    root.set_margin_bottom(6);
    root.set_margin_start(6);
    root.set_margin_end(6);

    // --- Save the current workspace under a new name ---
    let save_group = adw::PreferencesGroup::new();
    let name_row = adw::EntryRow::builder().title("Profile name").build();
    if let Some(current) = current.as_deref().filter(|c| !c.is_empty()) {
        name_row.set_text(current);
    }
    save_group.add(&name_row);

    let list = gtk4::ListBox::new();
    list.set_selection_mode(gtk4::SelectionMode::None);
    list.add_css_class("boxed-list");

    // Slugs in row order; the filter maps `row.index()` into this list, so
    // every row addition/removal has to keep it in sync.
    let names: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(profiles));

    // One existing profile: name, Load, Delete. Loading closes the picker
    // first — it swaps the whole workspace underneath.
    let make_row: Rc<dyn Fn(&str) -> gtk4::ListBoxRow> = {
        let names = names.clone();
        let list = list.clone();
        let dialog = dialog.clone();
        let on_load = on_load.clone();
        let on_delete = on_delete.clone();
        Rc::new(move |name: &str| {
            let name = name.to_string();
            let row = gtk4::ListBoxRow::new();
            let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
            hbox.set_margin_top(8);
            hbox.set_margin_bottom(8);
            hbox.set_margin_start(12);
            hbox.set_margin_end(12);
            let label = gtk4::Label::new(Some(name.as_str()));
            label.set_halign(gtk4::Align::Start);
            label.set_hexpand(true);
            hbox.append(&label);

            let load = gtk4::Button::with_label("Load");
            load.add_css_class("flat");
            {
                let dialog = dialog.clone();
                let on_load = on_load.clone();
                let name = name.clone();
                load.connect_clicked(move |_| {
                    dialog.close();
                    on_load(name.clone());
                });
            }
            hbox.append(&load);

            let delete = gtk4::Button::from_icon_name("user-trash-symbolic");
            delete.add_css_class("flat");
            delete.add_css_class("destructive-action");
            delete.set_tooltip_text(Some("Delete profile"));
            {
                let on_delete = on_delete.clone();
                let names = names.clone();
                let list = list.clone();
                let row_for_delete = row.clone();
                let name = name.clone();
                delete.connect_clicked(move |_| {
                    on_delete(name.clone());
                    let index = row_for_delete.index();
                    if index >= 0 {
                        names.borrow_mut().remove(index as usize);
                        list.remove(&row_for_delete);
                    }
                });
            }
            hbox.append(&delete);

            row.set_child(Some(&hbox));
            row
        })
    };

    for name in names.borrow().iter() {
        let row = make_row(name);
        list.append(&row);
    }

    let save_row = gtk4::Button::with_label("Save Current Session");
    save_row.add_css_class("suggested-action");
    save_row.set_hexpand(true);
    {
        let name_entry = name_row.clone();
        let names = names.clone();
        let list = list.clone();
        let on_save = on_save.clone();
        let make_row = make_row.clone();
        let do_save: Rc<dyn Fn()> = Rc::new(move || {
            let name = name_entry.text().trim().to_string();
            if name.is_empty() {
                name_entry.add_css_class("error");
                return;
            }
            name_entry.remove_css_class("error");
            on_save(name.clone());
            // Show the saved profile right away so it can be loaded without
            // reopening the picker.
            let slug = slugify_session_name(&name);
            let mut names = names.borrow_mut();
            if !names.contains(&slug) {
                names.push(slug.clone());
                drop(names);
                let row = make_row(&slug);
                list.append(&row);
            }
        });
        {
            let do_save = do_save.clone();
            save_row.connect_clicked(move |_| do_save());
        }
        {
            let do_save = do_save.clone();
            name_row.connect_entry_activated(move |_| do_save());
        }
    }
    root.append(&save_group);
    root.append(&save_row);

    // --- Existing profiles, filtered as you type ---
    let search = gtk4::SearchEntry::new();
    search.set_placeholder_text(Some("Search profiles…"));
    root.append(&search);

    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_child(Some(&list));
    root.append(&scroll);

    // Same split-on-whitespace matching as the command palette.
    let query: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));
    {
        let query = query.clone();
        let names = names.clone();
        list.set_filter_func(move |row| {
            let q = query.borrow();
            if q.is_empty() {
                return true;
            }
            names
                .borrow()
                .get(row.index() as usize)
                .map(|name| {
                    let needle = name.to_lowercase();
                    q.split_whitespace().all(|w| needle.contains(w))
                })
                .unwrap_or(false)
        });
    }
    {
        let query = query.clone();
        let list = list.clone();
        search.connect_search_changed(move |e| {
            *query.borrow_mut() = e.text().to_lowercase();
            list.invalidate_filter();
        });
    }

    // Activating a row (or Enter in the search) loads it.
    let activate_row: Rc<dyn Fn(&gtk4::ListBoxRow)> = {
        let names = names.clone();
        let dialog = dialog.clone();
        let on_load = on_load.clone();
        Rc::new(move |row: &gtk4::ListBoxRow| {
            let Some(name) = names.borrow().get(row.index() as usize).cloned() else {
                return;
            };
            dialog.close();
            on_load(name);
        })
    };
    {
        let activate_row = activate_row.clone();
        list.connect_row_activated(move |_, row| activate_row(row));
    }
    {
        let activate_row = activate_row.clone();
        let list = list.clone();
        search.connect_activate(move |_| {
            let mut idx = 0;
            while let Some(row) = list.row_at_index(idx) {
                if row.is_mapped() {
                    activate_row(&row);
                    return;
                }
                idx += 1;
            }
        });
    }

    dialog.set_child(Some(&root));

    // Escape has to be handled twice over (see the command palette).
    {
        let dialog = dialog.clone();
        search.connect_stop_search(move |_| {
            dialog.close();
        });
    }
    {
        let dialog_for_keys = dialog.clone();
        let keys = gtk4::EventControllerKey::new();
        keys.set_propagation_phase(gtk4::PropagationPhase::Capture);
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gdk::Key::Escape {
                dialog_for_keys.close();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        dialog.add_controller(keys);
    }
    // Clicking outside dismisses it.
    dialog.set_can_close(true);

    dialog.present(Some(window));
    name_row.grab_focus();
}

type ConfigUpdater = Rc<dyn Fn(&Config)>;
pub type ConfigObservers = Rc<RefCell<Vec<(glib::WeakRef<adw::PreferencesDialog>, ConfigUpdater)>>>;

/// Everything Preferences needs to push a change back into the live window.
#[derive(Clone)]
pub struct PrefsHooks {
    pub apply_zoom: Rc<dyn Fn(f32)>,
    pub set_tabs_location: Rc<dyn Fn(TabsLocation)>,
    pub apply_tab_shape: Rc<dyn Fn()>,
    pub set_search_visible: Rc<dyn Fn(bool)>,
    pub save_config: Rc<dyn Fn()>,
    pub bindings: Rc<RefCell<Bindings>>,
    pub apply_bindings: Rc<dyn Fn()>,
    pub applying_config: Rc<std::cell::Cell<bool>>,
    pub config_observers: ConfigObservers,
}

pub fn show_preferences(
    window: &adw::ApplicationWindow,
    config: &Rc<RefCell<Config>>,
    pages: &Pages,
    hooks: PrefsHooks,
) {
    let PrefsHooks {
        apply_zoom,
        set_tabs_location,
        apply_tab_shape,
        set_search_visible,
        save_config,
        bindings,
        apply_bindings,
        applying_config,
        config_observers,
    } = hooks;
    let dialog = adw::PreferencesDialog::builder()
        .title("Preferences")
        .build();
    let look_page = adw::PreferencesPage::builder()
        .title("Appearance")
        .icon_name("applications-graphics-symbolic")
        .build();
    let behavior_page = adw::PreferencesPage::builder()
        .title("Behavior")
        .icon_name("preferences-system-symbolic")
        .build();
    let advanced_page = adw::PreferencesPage::builder()
        .title("Advanced")
        .icon_name("preferences-other-symbolic")
        .build();

    // Apply a config mutation to every live terminal.
    let update_all = {
        let pages = pages.clone();
        let config = config.clone();
        let save_config = save_config.clone();
        let applying_config = applying_config.clone();
        Rc::new(move |f: Rc<dyn Fn(&mut Config)>| {
            if applying_config.get() {
                return;
            }
            f(&mut config.borrow_mut());
            for (_, views) in pages.borrow().iter() {
                for view in views {
                    view.update_config(|cfg| f(cfg));
                }
            }
            save_config();
        })
    };

    // --- Appearance ---
    let theme_group = adw::PreferencesGroup::builder().title("Theme").build();
    let font_group = adw::PreferencesGroup::builder().title("Font").build();
    let tabs_group = adw::PreferencesGroup::builder().title("Tabs").build();

    let theme_row = adw::ComboRow::builder()
        .title("Theme")
        .subtitle("Application interface style")
        .model(&gtk4::StringList::new(&["System", "Light", "Dark"]))
        .build();
    theme_row.set_selected(match config.borrow().theme {
        Theme::Light => 1,
        Theme::Dark => 2,
        Theme::System => 0,
    });
    {
        let config = config.clone();
        let save_config = save_config.clone();
        theme_row.connect_selected_notify(move |row| {
            let theme = match row.selected() {
                1 => Theme::Light,
                2 => Theme::Dark,
                _ => Theme::System,
            };
            adw::StyleManager::default().set_color_scheme(match theme {
                Theme::Light => adw::ColorScheme::ForceLight,
                Theme::Dark => adw::ColorScheme::ForceDark,
                Theme::System => adw::ColorScheme::Default,
            });
            config.borrow_mut().theme = theme;
            save_config();
        });
    }
    theme_group.add(&theme_row);

    let tabs_row = adw::ComboRow::builder()
        .title("Tab Position")
        .subtitle("config.toml: window.tabs")
        .model(&gtk4::StringList::new(&[
            "Top",
            "Bottom",
            "Sidebar (left)",
            "Sidebar (right)",
            "Hidden",
        ]))
        .build();
    tabs_row.set_selected(match config.borrow().tabs_location {
        TabsLocation::Top => 0,
        TabsLocation::Bottom => 1,
        TabsLocation::Left => 2,
        TabsLocation::Right => 3,
        TabsLocation::Hidden => 4,
    });
    {
        let config = config.clone();
        let set_tabs_location = set_tabs_location.clone();
        let save_config = save_config.clone();
        tabs_row.connect_selected_notify(move |row| {
            let location = match row.selected() {
                1 => TabsLocation::Bottom,
                2 => TabsLocation::Left,
                3 => TabsLocation::Right,
                4 => TabsLocation::Hidden,
                _ => TabsLocation::Top,
            };
            config.borrow_mut().tabs_location = location;
            set_tabs_location(location);
            save_config();
        });
    }
    tabs_group.add(&tabs_row);

    let sidebar_always_row = adw::SwitchRow::builder()
        .title("Always Show Sidebar")
        .subtitle("Show the tab sidebar even with a single tab")
        .build();
    sidebar_always_row.set_active(config.borrow().sidebar_always);
    {
        let config = config.clone();
        let set_tabs_location = set_tabs_location.clone();
        let save_config = save_config.clone();
        sidebar_always_row.connect_active_notify(move |row| {
            config.borrow_mut().sidebar_always = row.is_active();
            let location = config.borrow().tabs_location;
            set_tabs_location(location);
            save_config();
        });
    }
    tabs_group.add(&sidebar_always_row);

    let new_tab_row = adw::ComboRow::builder()
        .title("New Tab Position")
        .subtitle("Where a new tab is inserted")
        .model(&gtk4::StringList::new(&[
            "After Current",
            "Before Current",
            "End",
            "Start",
        ]))
        .build();
    new_tab_row.set_selected(match config.borrow().new_tab_position {
        NewTabPosition::AfterCurrent => 0,
        NewTabPosition::BeforeCurrent => 1,
        NewTabPosition::End => 2,
        NewTabPosition::Start => 3,
    });
    {
        let config = config.clone();
        let save_config = save_config.clone();
        new_tab_row.connect_selected_notify(move |row| {
            config.borrow_mut().new_tab_position = match row.selected() {
                1 => NewTabPosition::BeforeCurrent,
                2 => NewTabPosition::End,
                3 => NewTabPosition::Start,
                _ => NewTabPosition::AfterCurrent,
            };
            save_config();
        });
    }
    tabs_group.add(&new_tab_row);

    let tab_width_row = adw::ComboRow::builder()
        .title("Tab Width")
        .subtitle("Share the bar between tabs, or keep each as wide as its title")
        .model(&gtk4::StringList::new(&["Fill the Bar", "Fit the Title"]))
        .build();
    tab_width_row.set_selected(match config.borrow().tab_width {
        TabWidth::Fill => 0,
        TabWidth::Natural => 1,
    });
    let tab_overflow_row = adw::ComboRow::builder()
        .title("When Tabs Do Not Fit")
        .subtitle("Keep shrinking them, or hold a readable width and scroll")
        .model(&gtk4::StringList::new(&["Squeeze", "Scroll"]))
        .build();
    tab_overflow_row.set_selected(match config.borrow().tab_overflow {
        TabOverflow::Squeeze => 0,
        TabOverflow::Scroll => 1,
    });
    {
        let config = config.clone();
        let save_config = save_config.clone();
        let apply_tab_shape = apply_tab_shape.clone();
        tab_width_row.connect_selected_notify(move |row| {
            config.borrow_mut().tab_width = match row.selected() {
                1 => TabWidth::Natural,
                _ => TabWidth::Fill,
            };
            apply_tab_shape();
            save_config();
        });
    }
    {
        let config = config.clone();
        let save_config = save_config.clone();
        let apply_tab_shape = apply_tab_shape.clone();
        tab_overflow_row.connect_selected_notify(move |row| {
            config.borrow_mut().tab_overflow = match row.selected() {
                1 => TabOverflow::Scroll,
                _ => TabOverflow::Squeeze,
            };
            apply_tab_shape();
            save_config();
        });
    }
    tabs_group.add(&tab_width_row);
    tabs_group.add(&tab_overflow_row);

    let search_btn_row = adw::SwitchRow::builder()
        .title("Show the Search Button")
        .subtitle("Magnifier in the header that opens the command palette")
        .build();
    search_btn_row.set_active(config.borrow().show_search_button);
    {
        let config = config.clone();
        let save_config = save_config.clone();
        let set_search_visible = set_search_visible.clone();
        search_btn_row.connect_active_notify(move |row| {
            config.borrow_mut().show_search_button = row.is_active();
            set_search_visible(row.is_active());
            save_config();
        });
    }
    tabs_group.add(&search_btn_row);

    let middle_row = adw::ComboRow::builder()
        .title("Middle Click on a Tab")
        .subtitle("Action bound to the middle mouse button")
        .model(&gtk4::StringList::new(&["Nothing", "New Tab", "Close Tab"]))
        .build();
    middle_row.set_selected(match config.borrow().middle_click_tab {
        MiddleClickTab::Ignore => 0,
        MiddleClickTab::NewTab => 1,
        MiddleClickTab::CloseTab => 2,
    });
    {
        let config = config.clone();
        let save_config = save_config.clone();
        middle_row.connect_selected_notify(move |row| {
            config.borrow_mut().middle_click_tab = match row.selected() {
                1 => MiddleClickTab::NewTab,
                2 => MiddleClickTab::CloseTab,
                _ => MiddleClickTab::Ignore,
            };
            save_config();
        });
    }
    tabs_group.add(&middle_row);

    let font_row = adw::SpinRow::with_range(6.0, 40.0, 1.0);
    font_row.set_title("Font Size");
    font_row.set_subtitle("Applies to all open tabs");
    font_row.set_value(config.borrow().font_size as f64);
    {
        let apply_zoom = apply_zoom.clone();
        font_row.connect_value_notify(move |row| {
            apply_zoom(row.value() as f32);
        });
    }
    font_group.add(&font_row);

    let padding_row = adw::SpinRow::with_range(0.0, 32.0, 1.0);
    padding_row.set_title("Window Padding");
    padding_row.set_subtitle("Terminal inner margin, in pixels");
    padding_row.set_value(config.borrow().padding_x);
    {
        let update_all = update_all.clone();
        padding_row.connect_value_notify(move |row| {
            let v = row.value();
            update_all(Rc::new(move |cfg| {
                cfg.padding_x = v;
                cfg.padding_y = v;
            }));
        });
    }
    theme_group.add(&padding_row);

    let opacity_row = adw::SpinRow::with_range(15.0, 100.0, 5.0);
    opacity_row.set_title("Background Opacity");
    opacity_row.set_subtitle("Requires a compositor; applies on reload");
    opacity_row.set_value(config.borrow().background_opacity * 100.0);
    {
        let update_all = update_all.clone();
        opacity_row.connect_value_notify(move |row| {
            let v = (row.value() / 100.0).clamp(0.15, 1.0);
            update_all(Rc::new(move |cfg| cfg.background_opacity = v));
        });
    }
    theme_group.add(&opacity_row);

    let ligature_row = adw::SwitchRow::builder()
        .title("Ligatures")
        .subtitle("Shape ->, => and != as single glyphs")
        .build();
    ligature_row.set_active(config.borrow().font_ligatures);
    {
        let update_all = update_all.clone();
        ligature_row.connect_active_notify(move |row| {
            let on = row.is_active();
            update_all(Rc::new(move |cfg| cfg.font_ligatures = on));
        });
    }
    font_group.add(&ligature_row);

    let system_font_row = adw::SwitchRow::builder()
        .title("Use the System Monospace Font")
        .subtitle("Ignores the family above and follows your desktop setting")
        .build();
    system_font_row.set_active(config.borrow().use_system_font);
    {
        let update_all = update_all.clone();
        system_font_row.connect_active_notify(move |row| {
            let on = row.is_active();
            update_all(Rc::new(move |cfg| cfg.use_system_font = on));
        });
    }
    font_group.add(&system_font_row);

    // --- Scrolling ---
    let scroll_group = adw::PreferencesGroup::builder().title("Scrolling").build();

    let scroll_bar_row = adw::SwitchRow::builder()
        .title("Show a Scrollbar")
        .subtitle("Appears only once there is scrollback to reach")
        .build();
    scroll_bar_row.set_active(config.borrow().scroll_bar);
    {
        let update_all = update_all.clone();
        scroll_bar_row.connect_active_notify(move |row| {
            let on = row.is_active();
            update_all(Rc::new(move |cfg| cfg.scroll_bar = on));
        });
    }
    scroll_group.add(&scroll_bar_row);

    let scroll_btn_row = adw::SwitchRow::builder()
        .title("Jump-to-Bottom Button")
        .subtitle("Floating button while scrolled up")
        .build();
    scroll_btn_row.set_active(config.borrow().scroll_button);
    {
        let update_all = update_all.clone();
        scroll_btn_row.connect_active_notify(move |row| {
            let on = row.is_active();
            update_all(Rc::new(move |cfg| cfg.scroll_button = on));
        });
    }
    scroll_group.add(&scroll_btn_row);

    let scroll_keys_row = adw::SwitchRow::builder()
        .title("Typing Returns to the Prompt")
        .subtitle("Jumps to the bottom as soon as you type while scrolled up")
        .build();
    scroll_keys_row.set_active(config.borrow().scroll_on_keystroke);
    {
        let update_all = update_all.clone();
        scroll_keys_row.connect_active_notify(move |row| {
            let on = row.is_active();
            update_all(Rc::new(move |cfg| cfg.scroll_on_keystroke = on));
        });
    }
    scroll_group.add(&scroll_keys_row);

    let scroll_lines_row = adw::SpinRow::builder()
        .title("Scrollback Lines")
        .subtitle("config.toml: scroll.lines — 0 disables scrollback")
        .adjustment(
            &gtk4::Adjustment::builder()
                .lower(0.0)
                .upper(1_000_000.0)
                .step_increment(1_000.0)
                .page_increment(10_000.0)
                .value(config.borrow().scroll_lines as f64)
                .build(),
        )
        .digits(0)
        .build();
    {
        let update_all = update_all.clone();
        scroll_lines_row.connect_changed(move |row| {
            let lines = row.value() as i64;
            update_all(Rc::new(move |cfg| cfg.scroll_lines = lines));
        });
    }
    scroll_group.add(&scroll_lines_row);

    look_page.add(&theme_group);
    look_page.add(&font_group);
    look_page.add(&scroll_group);

    // --- Session ---
    let session_group = adw::PreferencesGroup::builder().title("Session").build();
    let restore_row = adw::SwitchRow::builder()
        .title("Restore Tabs on Start")
        .subtitle("Reopens tabs, nested splits, working directories and window size")
        .build();
    restore_row.set_active(config.borrow().session_restore);
    {
        let config = config.clone();
        let save_config = save_config.clone();
        restore_row.connect_active_notify(move |row| {
            config.borrow_mut().session_restore = row.is_active();
            save_config();
        });
    }
    session_group.add(&restore_row);

    let inherit_row = adw::SwitchRow::builder()
        .title("Inherit Working Directory")
        .subtitle("New tabs and splits open in the focused pane's directory")
        .build();
    inherit_row.set_active(config.borrow().inherit_working_directory);
    {
        let config = config.clone();
        let save_config = save_config.clone();
        inherit_row.connect_active_notify(move |row| {
            config.borrow_mut().inherit_working_directory = row.is_active();
            save_config();
        });
    }
    session_group.add(&inherit_row);

    let awake_row = adw::SwitchRow::builder()
        .title("Keep the System Awake")
        .subtitle("Blocks idle and screen blanking while a command is running")
        .build();
    awake_row.set_active(config.borrow().keep_awake);
    {
        let config = config.clone();
        let save_config = save_config.clone();
        awake_row.connect_active_notify(move |row| {
            config.borrow_mut().keep_awake = row.is_active();
            save_config();
        });
    }
    session_group.add(&awake_row);
    behavior_page.add(&session_group);

    // --- Confirmations ---
    let confirm_group = adw::PreferencesGroup::builder()
        .title("Confirmations")
        .build();

    let confirm_tab_row = adw::SwitchRow::builder()
        .title("Confirm Closing a Tab")
        .build();
    confirm_tab_row.set_active(config.borrow().confirm_close_tab);
    {
        let config = config.clone();
        let save_config = save_config.clone();
        confirm_tab_row.connect_active_notify(move |row| {
            config.borrow_mut().confirm_close_tab = row.is_active();
            save_config();
        });
    }
    confirm_group.add(&confirm_tab_row);

    let confirm_quit_row = adw::SwitchRow::builder()
        .title("Confirm Closing the Window")
        .subtitle("Only asked when more than one tab is open")
        .build();
    confirm_quit_row.set_active(config.borrow().confirm_quit);
    {
        let config = config.clone();
        let save_config = save_config.clone();
        confirm_quit_row.connect_active_notify(move |row| {
            config.borrow_mut().confirm_quit = row.is_active();
            save_config();
        });
    }
    confirm_group.add(&confirm_quit_row);
    behavior_page.add(&confirm_group);

    // --- Cursor ---
    let cursor_group = adw::PreferencesGroup::builder().title("Cursor").build();

    let cursor_row = adw::ComboRow::builder()
        .title("Cursor Style")
        .subtitle("Default shape (apps may override it)")
        .model(&gtk4::StringList::new(&["Block", "Bar", "Underline"]))
        .build();
    cursor_row.set_selected(match config.borrow().cursor_style {
        CursorStyle::Bar => 1,
        CursorStyle::Underline => 2,
        _ => 0,
    });
    {
        let update_all = update_all.clone();
        cursor_row.connect_selected_notify(move |row| {
            let style = match row.selected() {
                1 => CursorStyle::Bar,
                2 => CursorStyle::Underline,
                _ => CursorStyle::Block,
            };
            update_all(Rc::new(move |cfg| cfg.cursor_style = style));
        });
    }
    cursor_group.add(&cursor_row);

    let blink_row = adw::SwitchRow::builder()
        .title("Blinking Cursor")
        .subtitle("Blinks while the terminal is focused")
        .build();
    blink_row.set_active(config.borrow().cursor_blink);
    {
        let update_all = update_all.clone();
        blink_row.connect_active_notify(move |row| {
            let v = row.is_active();
            update_all(Rc::new(move |cfg| cfg.cursor_blink = v));
        });
    }
    cursor_group.add(&blink_row);
    look_page.add(&cursor_group);

    // --- Config file ---
    let cfg_group = adw::PreferencesGroup::builder()
        .title("Configuration")
        .description("~/.option/terminal/config.toml")
        .build();

    let source = config.borrow().source.clone();
    let file_row = adw::ActionRow::builder()
        .title("Configuration File")
        .subtitle(source.display().to_string())
        .build();
    let open_btn = gtk4::Button::from_icon_name("document-edit-symbolic");
    open_btn.set_tooltip_text(Some("Open in the default editor"));
    open_btn.set_valign(gtk4::Align::Center);
    open_btn.add_css_class("flat");
    {
        let uri = gio::File::for_path(&source).uri();
        open_btn.connect_clicked(move |_| {
            if let Err(err) =
                gio::AppInfo::launch_default_for_uri(&uri, gio::AppLaunchContext::NONE)
            {
                tracing::warn!("failed to open config file: {err}");
            }
        });
    }
    file_row.add_suffix(&open_btn);
    file_row.set_activatable_widget(Some(&open_btn));
    cfg_group.add(&file_row);

    let reload_row = adw::ActionRow::builder()
        .title("Reload Configuration")
        .subtitle("Re-applies colors, font and padding to all tabs")
        .build();
    let reload_btn = gtk4::Button::from_icon_name("view-refresh-symbolic");
    reload_btn.set_valign(gtk4::Align::Center);
    reload_btn.add_css_class("flat");
    reload_btn.set_action_name(Some("win.reload-config"));
    {
        let config = config.clone();
        let font_row = font_row.clone();
        reload_btn.connect_clicked(move |_| {
            let size = config.borrow().font_size as f64;
            font_row.set_value(size);
        });
    }
    reload_row.add_suffix(&reload_btn);
    reload_row.set_activatable_widget(Some(&reload_btn));
    cfg_group.add(&reload_row);
    advanced_page.add(&cfg_group);

    behavior_page.add(&tabs_group);

    // --- Sound ---
    let sound_page = adw::PreferencesPage::builder()
        .title("Sound")
        .icon_name("audio-volume-high-symbolic")
        .build();
    let bell_group = adw::PreferencesGroup::builder()
        .title("Bell")
        .description("Programs ring the bell by writing the BEL character")
        .build();
    let bell_row = adw::SwitchRow::builder()
        .title("Audible Bell")
        .subtitle("Rings the system bell, honoring your desktop's sound settings")
        .build();
    bell_row.set_active(config.borrow().bell_sound);
    {
        let update_all = update_all.clone();
        bell_row.connect_active_notify(move |row| {
            let on = row.is_active();
            update_all(Rc::new(move |cfg| cfg.bell_sound = on));
        });
    }
    bell_group.add(&bell_row);

    let test_row = adw::ActionRow::builder()
        .title("Test the Bell")
        .subtitle("Plays it once, so you can tell whether your system has one")
        .build();
    let test_btn = gtk4::Button::with_label("Play");
    test_btn.add_css_class("flat");
    test_btn.set_valign(gtk4::Align::Center);
    test_btn.connect_clicked(|_| {
        if let Some(display) = gdk::Display::default() {
            display.beep();
        }
    });
    test_row.add_suffix(&test_btn);
    test_row.set_activatable_widget(Some(&test_btn));
    bell_group.add(&test_row);
    sound_page.add(&bell_group);

    let notify_group = adw::PreferencesGroup::builder()
        .title("Notifications")
        .build();
    let done_row = adw::SwitchRow::builder()
        .title("Command Finished")
        .subtitle("Sounds when a command ends while the window is not focused")
        .build();
    done_row.set_active(config.borrow().command_finished_sound);
    {
        let config = config.clone();
        let save_config = save_config.clone();
        done_row.connect_active_notify(move |row| {
            config.borrow_mut().command_finished_sound = row.is_active();
            save_config();
        });
    }
    notify_group.add(&done_row);
    sound_page.add(&notify_group);

    // --- Default terminal (lives in Advanced) ---
    let default_group = adw::PreferencesGroup::builder()
        .title("Default Terminal")
        .description(
            "There is no single setting for this. optionTerm writes the portable \
             xdg-terminals.list, plus your desktop's own key when it has one.",
        )
        .build();
    let default_row = adw::ActionRow::builder()
        .title("Set as Default Terminal")
        .build();
    let default_btn = gtk4::Button::with_label("Set as Default");
    default_btn.add_css_class("flat");
    default_btn.set_valign(gtk4::Align::Center);

    let refresh_default = {
        let row = default_row.downgrade();
        let button = default_btn.downgrade();
        let revision = Rc::new(std::cell::Cell::new(0u64));
        Rc::new(move || {
            let current = revision.get().wrapping_add(1);
            revision.set(current);
            let revision = revision.clone();
            let row = row.clone();
            let button = button.clone();
            let job = gio::spawn_blocking(crate::default_terminal::is_default);
            glib::spawn_future_local(async move {
                let result = job.await;
                if revision.get() != current {
                    return;
                }
                let (Some(row), Some(button)) = (row.upgrade(), button.upgrade()) else {
                    return;
                };
                match result {
                    Ok(true) => {
                        row.set_subtitle("optionTerm is the preferred terminal");
                        button.set_label("Set Again");
                    }
                    Ok(false) => {
                        row.set_subtitle("Another terminal is preferred");
                        button.set_label("Set as Default");
                    }
                    Err(_) => row.set_subtitle("Could not check the default terminal"),
                }
            });
        })
    };
    refresh_default();
    {
        let refresh_default = refresh_default.clone();
        let dialog = dialog.downgrade();
        default_btn.connect_clicked(move |button| {
            if !button.is_sensitive() {
                return;
            }
            button.set_sensitive(false);
            let button = button.downgrade();
            let dialog = dialog.clone();
            let refresh_default = refresh_default.clone();
            let job = gio::spawn_blocking(crate::default_terminal::set_default);
            glib::spawn_future_local(async move {
                let result = job
                    .await
                    .unwrap_or_else(|_| Err(anyhow::anyhow!("default-terminal worker failed")));
                let toast = match result {
                    // Say what actually changed: silently claiming success would
                    // hide a desktop we could not reach.
                    Ok(applied) if applied.is_empty() => {
                        "Nothing could be set on this desktop".to_string()
                    }
                    Ok(applied) => format!("Updated {}", applied.join(", ")),
                    Err(err) => {
                        tracing::warn!("could not set default terminal: {err:#}");
                        "Could not set the default terminal".to_string()
                    }
                };
                if let Some(button) = button.upgrade() {
                    button.set_sensitive(true);
                }
                refresh_default();
                if let Some(dialog) = dialog.upgrade() {
                    dialog.add_toast(adw::Toast::builder().title(&toast).timeout(4).build());
                }
            });
        });
    }
    default_row.add_suffix(&default_btn);
    default_group.add(&default_row);
    advanced_page.add(&default_group);

    // --- Shortcuts ---
    let shortcuts_page = adw::PreferencesPage::builder()
        .title("Shortcuts")
        .icon_name("preferences-desktop-keyboard-symbolic")
        .build();
    let shortcuts_group = adw::PreferencesGroup::builder()
        .title("Keyboard Shortcuts")
        .description("Overrides are stored in keys.toml, separately from config.toml")
        .build();
    for (label, action, accel) in bindings.borrow().effective() {
        let name = action.trim_start_matches("win.").to_string();
        let row = adw::ActionRow::builder().title(label).build();

        let shown = if accel.is_empty() {
            "Unbound".to_string()
        } else {
            accel.clone()
        };
        let button = gtk4::Button::with_label(&shown);
        button.add_css_class("flat");
        button.set_valign(gtk4::Align::Center);
        button.set_tooltip_text(Some("Click, then press the new shortcut"));

        let reset = gtk4::Button::from_icon_name("edit-undo-symbolic");
        reset.add_css_class("flat");
        reset.set_valign(gtk4::Align::Center);
        reset.set_tooltip_text(Some("Restore the default"));
        reset.set_visible(bindings.borrow().get(&name).is_some());

        {
            let bindings = bindings.clone();
            let apply_bindings = apply_bindings.clone();
            let button_c = button.clone();
            let reset_c = reset.clone();
            let dialog_c = dialog.clone();
            let name = name.clone();
            button.connect_clicked(move |_| {
                let bindings = bindings.clone();
                let apply_bindings = apply_bindings.clone();
                let button = button_c.clone();
                let reset = reset_c.clone();
                let dialog = dialog_c.clone();
                let parent = dialog_c.clone();
                let name = name.clone();
                capture_shortcut(&parent, move |accel| {
                    // Two actions on one key means one of them silently never
                    // fires, so refuse rather than let it happen.
                    if let Some(other) = bindings.borrow().conflict(&accel, &name) {
                        dialog.add_toast(
                            adw::Toast::builder()
                                .title(format!("Already used by {other}"))
                                .timeout(3)
                                .build(),
                        );
                        return;
                    }
                    bindings.borrow_mut().set(&name, Some(&accel));
                    if let Err(err) = bindings.borrow().save() {
                        tracing::warn!("could not save shortcuts: {err:#}");
                    }
                    apply_bindings();
                    button.set_label(&accel);
                    reset.set_visible(true);
                });
            });
        }
        {
            let bindings = bindings.clone();
            let apply_bindings = apply_bindings.clone();
            let button = button.clone();
            let reset_c = reset.clone();
            let name = name.clone();
            let builtin = Bindings::default().display(&name);
            let dialog = dialog.downgrade();
            reset.connect_clicked(move |_| {
                let mut proposed = bindings.borrow().clone();
                proposed.set(&name, None);
                if let Some(conflict) = proposed
                    .accels(&name)
                    .iter()
                    .find_map(|accel| proposed.conflict(accel, &name))
                {
                    if let Some(dialog) = dialog.upgrade() {
                        dialog.add_toast(adw::Toast::new(&format!("Already used by {conflict}")));
                    }
                    return;
                }
                *bindings.borrow_mut() = proposed;
                if let Err(err) = bindings.borrow().save() {
                    tracing::warn!("could not save shortcuts: {err:#}");
                }
                apply_bindings();
                button.set_label(if builtin.is_empty() {
                    "Unbound"
                } else {
                    &builtin
                });
                reset_c.set_visible(false);
            });
        }

        row.add_suffix(&button);
        row.add_suffix(&reset);
        row.set_activatable_widget(Some(&button));
        shortcuts_group.add(&row);
    }
    shortcuts_page.add(&shortcuts_group);

    let mut updates: Vec<ConfigUpdater> = Vec::new();
    macro_rules! observe {
        ($widget:ident, $method:ident, $value:expr) => {{
            let weak = $widget.downgrade();
            updates.push(Rc::new(move |config: &Config| {
                if let Some(widget) = weak.upgrade() {
                    widget.$method(($value)(config));
                }
            }));
        }};
    }
    observe!(theme_row, set_selected, |c: &Config| match c.theme {
        Theme::System => 0,
        Theme::Light => 1,
        Theme::Dark => 2,
    });
    observe!(tabs_row, set_selected, |c: &Config| match c.tabs_location {
        TabsLocation::Top => 0,
        TabsLocation::Bottom => 1,
        TabsLocation::Left => 2,
        TabsLocation::Right => 3,
        TabsLocation::Hidden => 4,
    });
    observe!(
        new_tab_row,
        set_selected,
        |c: &Config| match c.new_tab_position {
            NewTabPosition::AfterCurrent => 0,
            NewTabPosition::BeforeCurrent => 1,
            NewTabPosition::End => 2,
            NewTabPosition::Start => 3,
        }
    );
    observe!(tab_width_row, set_selected, |c: &Config| u32::from(
        c.tab_width == TabWidth::Natural
    ));
    observe!(tab_overflow_row, set_selected, |c: &Config| u32::from(
        c.tab_overflow == TabOverflow::Scroll
    ));
    observe!(
        middle_row,
        set_selected,
        |c: &Config| match c.middle_click_tab {
            MiddleClickTab::Ignore => 0,
            MiddleClickTab::NewTab => 1,
            MiddleClickTab::CloseTab => 2,
        }
    );
    observe!(
        cursor_row,
        set_selected,
        |c: &Config| match c.cursor_style {
            CursorStyle::Bar => 1,
            CursorStyle::Underline => 2,
            _ => 0,
        }
    );
    observe!(font_row, set_value, |c: &Config| c.font_size as f64);
    observe!(padding_row, set_value, |c: &Config| c.padding_x);
    observe!(opacity_row, set_value, |c: &Config| c.background_opacity
        * 100.0);
    observe!(scroll_lines_row, set_value, |c: &Config| c.scroll_lines
        as f64);
    observe!(sidebar_always_row, set_active, |c: &Config| c
        .sidebar_always);
    observe!(search_btn_row, set_active, |c: &Config| c
        .show_search_button);
    observe!(ligature_row, set_active, |c: &Config| c.font_ligatures);
    observe!(system_font_row, set_active, |c: &Config| c.use_system_font);
    observe!(scroll_bar_row, set_active, |c: &Config| c.scroll_bar);
    observe!(scroll_btn_row, set_active, |c: &Config| c.scroll_button);
    observe!(scroll_keys_row, set_active, |c: &Config| c
        .scroll_on_keystroke);
    observe!(restore_row, set_active, |c: &Config| c.session_restore);
    observe!(inherit_row, set_active, |c: &Config| c
        .inherit_working_directory);
    observe!(awake_row, set_active, |c: &Config| c.keep_awake);
    observe!(confirm_tab_row, set_active, |c: &Config| c
        .confirm_close_tab);
    observe!(confirm_quit_row, set_active, |c: &Config| c.confirm_quit);
    observe!(blink_row, set_active, |c: &Config| c.cursor_blink);
    observe!(bell_row, set_active, |c: &Config| c.bell_sound);
    observe!(done_row, set_active, |c: &Config| c.command_finished_sound);
    config_observers.borrow_mut().push((
        dialog.downgrade(),
        Rc::new(move |config| {
            for update in &updates {
                update(config);
            }
        }),
    ));
    dialog.connect_closed(move |dialog| {
        config_observers
            .borrow_mut()
            .retain(|(owner, _)| owner.upgrade().as_ref() != Some(dialog));
    });

    dialog.add(&look_page);
    dialog.add(&behavior_page);
    dialog.add(&sound_page);
    dialog.add(&shortcuts_page);
    dialog.add(&advanced_page);
    dialog.present(Some(window));
}

/// Ask the user to press a shortcut, reporting it in GTK accelerator syntax.
///
/// Modifier-only presses are ignored: they are what the user is holding on the
/// way to the real key.
fn capture_shortcut(parent: &impl IsA<gtk4::Widget>, on_captured: impl Fn(String) + 'static) {
    let dialog = adw::AlertDialog::new(
        Some("Press the new shortcut"),
        Some("Escape cancels, Backspace unbinds the action."),
    );
    dialog.add_responses(&[("cancel", "Cancel")]);
    dialog.set_close_response("cancel");

    let keys = gtk4::EventControllerKey::new();
    {
        let dialog = dialog.clone();
        keys.connect_key_pressed(move |_, key, _, mods| {
            // Only the modifiers that take part in accelerators.
            let mods = mods
                & (gdk::ModifierType::CONTROL_MASK
                    | gdk::ModifierType::SHIFT_MASK
                    | gdk::ModifierType::ALT_MASK
                    | gdk::ModifierType::SUPER_MASK);
            match key {
                gdk::Key::Escape => {
                    dialog.close();
                    return glib::Propagation::Stop;
                }
                gdk::Key::BackSpace => {
                    on_captured(String::new());
                    dialog.close();
                    return glib::Propagation::Stop;
                }
                gdk::Key::Control_L
                | gdk::Key::Control_R
                | gdk::Key::Shift_L
                | gdk::Key::Shift_R
                | gdk::Key::Alt_L
                | gdk::Key::Alt_R
                | gdk::Key::Super_L
                | gdk::Key::Super_R => return glib::Propagation::Stop,
                _ => {}
            }
            let name = key.name().unwrap_or_default();
            if name.is_empty() {
                return glib::Propagation::Stop;
            }
            let mut accel = String::new();
            if mods.contains(gdk::ModifierType::CONTROL_MASK) {
                accel.push_str("<Control>");
            }
            if mods.contains(gdk::ModifierType::SHIFT_MASK) {
                accel.push_str("<Shift>");
            }
            if mods.contains(gdk::ModifierType::ALT_MASK) {
                accel.push_str("<Alt>");
            }
            if mods.contains(gdk::ModifierType::SUPER_MASK) {
                accel.push_str("<Super>");
            }
            accel.push_str(&name);
            on_captured(accel);
            dialog.close();
            glib::Propagation::Stop
        });
    }
    dialog.add_controller(keys);
    dialog.present(Some(parent));
}

pub fn show_shortcuts(window: &adw::ApplicationWindow, bindings: &Bindings) {
    let dialog = adw::Dialog::builder()
        .title("Keyboard Shortcuts")
        .content_width(420)
        .content_height(520)
        .build();

    let list = gtk4::ListBox::new();
    list.set_selection_mode(gtk4::SelectionMode::None);
    list.add_css_class("boxed-list");
    list.set_margin_top(12);
    list.set_margin_bottom(12);
    list.set_margin_start(12);
    list.set_margin_end(12);

    for (label, _, accel) in bindings
        .effective()
        .into_iter()
        .filter(|(_, _, a)| !a.is_empty())
    {
        let row = adw::ActionRow::builder().title(label).build();
        let key = gtk4::Label::new(Some(&accel));
        key.add_css_class("dim-label");
        key.add_css_class("numeric");
        row.add_suffix(&key);
        list.append(&row);
    }

    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_child(Some(&list));

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(&scroll));
    dialog.set_child(Some(&toolbar));
    dialog.present(Some(window));
}

pub fn show_about(window: &adw::ApplicationWindow) {
    let about = adw::AboutDialog::builder()
        .application_name("optionTerm")
        .application_icon("utilities-terminal")
        .developer_name("Firefly Labs")
        .version(env!("CARGO_PKG_VERSION"))
        .license_type(gtk4::License::Apache20)
        .website("https://github.com/fireflylabss/optionTerm")
        .issue_url("https://github.com/fireflylabss/optionTerm/issues")
        .comments(
            "Sidebar-first GTK4 terminal with tiling splits, Adwaita preferences, \
             and a keyboard-driven workflow.",
        )
        .build();
    about.add_acknowledgement_section(
        Some("Inspired by"),
        &["Yacha (FoxTerminal) https://gitlab.com/OrangeFox/misc/FoxTerminal"],
    );
    about.add_legal_section(
        "FoxTerminal",
        // Ideas only: FoxTerminal is GPL-3.0-or-later and optionTerm is
        // Apache-2.0, so no code is shared between them.
        Some("optionTerm's sidebar-first workflow, its quick theme and font\ncontrols and the shape of its preferences were inspired by\nFoxTerminal by Yacha, whose terminal is why this one exists.\n\nFoxTerminal is licensed GPL-3.0-or-later. No FoxTerminal code is\nincluded in optionTerm; only ideas were borrowed."),
        gtk4::License::Custom,
        None,
    );
    about.add_legal_section(
        "VTE",
        Some(
            "optionTerm dynamically links the system VTE library\n\
             (LGPL-3.0-or-later). See NOTICE and your distribution's\n\
             VTE package for license details.",
        ),
        gtk4::License::Custom,
        None,
    );
    about.present(Some(window));
}

pub fn agent_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    populate_agent_menu(&menu, || {
        crate::agents::AgentKind::ALL
            .into_iter()
            .filter(|kind| crate::agents::is_installed(*kind))
            .collect()
    });
    menu
}

fn populate_agent_menu(
    menu: &gio::Menu,
    probe: impl FnOnce() -> Vec<crate::agents::AgentKind> + Send + 'static,
) {
    menu.remove_all();
    menu.append(Some("Detecting agents…"), None);
    let menu = menu.downgrade();
    let job = gio::spawn_blocking(probe);
    glib::spawn_future_local(async move {
        let result = job.await;
        let Some(menu) = menu.upgrade() else { return };
        menu.remove_all();
        match result {
            Ok(kinds) if kinds.is_empty() => menu.append(Some("No installed agents found"), None),
            Ok(kinds) => {
                for kind in kinds {
                    menu.append(
                        Some(kind.label()),
                        Some(&format!("win.agent-{}", kind.as_str())),
                    );
                }
            }
            Err(_) => menu.append(Some("Agent discovery failed"), None),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use option_term_core::commands::COMMANDS;

    #[gtk4::test]
    fn search_tracks_panes_and_clears_on_close_and_escape() {
        let first = Rc::new(TerminalView::new(Config::default(), None, None).unwrap());
        let second = Rc::new(TerminalView::new(Config::default(), None, None).unwrap());
        let selected = Rc::new(RefCell::new(first.clone()));
        let current = selected.clone();
        let search = SearchBar::new(Rc::new(move || Some(current.borrow().clone())));
        search.entry.set_text("needle");
        search.open();
        assert!(first.has_search());
        *selected.borrow_mut() = second.clone();
        search.sync_target();
        assert!(!first.has_search());
        assert!(second.has_search());
        search.widget.set_search_mode(false);
        assert!(!second.has_search());
        search.open();
        assert!(second.has_search());
        search.entry.emit_by_name::<()>("stop-search", &[]);
        assert!(!search.widget.is_search_mode());
        assert!(!second.has_search());
    }

    #[gtk4::test]
    fn tab_menus_share_one_agent_model() {
        let agents = gio::Menu::new();
        let first = tabs_menu(&agents);
        let second = tabs_menu(&agents);
        let first_agents = first.item_link(2, "submenu").unwrap();
        let second_agents = second.item_link(2, "submenu").unwrap();
        assert_eq!(first_agents, second_agents);
        agents.append(Some("Codex"), Some("win.agent-codex"));
        assert_eq!(first_agents.n_items(), 1);
        assert_eq!(second_agents.n_items(), 1);
    }

    #[gtk4::test]
    fn agent_discovery_does_not_block_the_main_loop() {
        let menu = gio::Menu::new();
        let (tx, rx) = std::sync::mpsc::channel();
        glib::idle_add_local_once(move || {
            let _ = tx.send(());
        });
        populate_agent_menu(&menu, move || {
            assert!(
                rx.recv_timeout(std::time::Duration::from_secs(1)).is_ok(),
                "main loop was blocked by agent discovery"
            );
            vec![crate::agents::AgentKind::Codex]
        });
        crate::test_support::spin_until(|| {
            menu.item_attribute_value(0, "label", None)
                .and_then(|v| v.get::<String>())
                .as_deref()
                == Some("Codex")
        });
    }

    fn fake_codex_thread(id: &str, title: &str, cwd: Option<&str>) -> crate::codex::CodexThread {
        crate::codex::CodexThread {
            id: id.to_string(),
            title: title.to_string(),
            file: PathBuf::from(format!("/rollout-{id}.jsonl")),
            cwd: cwd.map(PathBuf::from),
            // time = 0 makes the subtitle the id, so filtering is
            // deterministic regardless of when the test runs.
            time: 0,
        }
    }

    #[gtk4::test]
    fn codex_picker_filters_rows_by_all_query_words() {
        let threads = vec![
            fake_codex_thread("t1", "Fix the parser", None),
            fake_codex_thread("t2", "Write docs", Some("/tmp/docs")),
        ];
        let dialog = adw::Dialog::new();
        let (root, entry, list) = codex_picker(
            &dialog,
            glib::WeakRef::new(),
            threads,
            Rc::new(|_| {}),
            Rc::new(|_| {}),
        );
        // Mount the picker in a real window: the ListBox filter hides rows by
        // unmapping them, which only happens once the list is onscreen.
        let win = gtk4::Window::new();
        win.set_child(Some(&root));
        win.present();
        crate::test_support::spin_until(|| list.row_at_index(1).is_some_and(|r| r.is_mapped()));
        let visible = |i: i32| list.row_at_index(i).is_some_and(|r| r.is_mapped());
        assert!(visible(0) && visible(1));

        entry.set_text("fix parser");
        crate::test_support::spin_until(|| !visible(1));
        assert!(visible(0));
        assert!(!visible(1));

        entry.set_text("docs");
        crate::test_support::spin_until(|| !visible(0));
        assert!(!visible(0));
        assert!(visible(1));

        // Every query word must match, as in the command palette.
        entry.set_text("fix docs");
        crate::test_support::spin_until(|| !visible(1));
        assert!(!visible(0) && !visible(1));

        entry.set_text("");
        crate::test_support::spin_until(|| visible(0) && visible(1));
    }

    #[gtk4::test]
    fn codex_picker_empty_list_shows_placeholder() {
        let dialog = adw::Dialog::new();
        let (_, _, list) = codex_picker(
            &dialog,
            glib::WeakRef::new(),
            Vec::new(),
            Rc::new(|_| {}),
            Rc::new(|_| {}),
        );
        let row = list.row_at_index(0).unwrap();
        let row = row.downcast::<adw::ActionRow>().unwrap();
        assert_eq!(row.title(), "No Codex threads found");
        assert!(!row.is_activatable());
        assert!(list.row_at_index(1).is_none());
    }

    #[test]
    fn typed_row_visibility_rules() {
        let row = |label: &str| {
            (
                label.to_string(),
                String::new(),
                PaletteAction::Win("win.noop"),
            )
        };
        let entries = vec![row("Run: build"), row("Zoom In")];
        assert!(!typed_row_visible("", &entries));
        assert!(!typed_row_visible("   ", &entries));
        assert!(typed_row_visible("cargo build", &entries));
        assert!(!typed_row_visible("build", &entries), "preset name wins");
        assert!(!typed_row_visible("Build", &entries), "case-insensitive");
        assert!(typed_row_visible("build clean", &entries));
    }

    #[gtk4::test]
    fn palette_runs_typed_commands_and_presets() {
        let app = gtk4::Application::builder()
            .application_id("io.option.terminal.palette-test")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(gio::Cancellable::NONE).unwrap();
        let window = adw::ApplicationWindow::new(&app);
        let config = Rc::new(RefCell::new(Config {
            commands: vec![crate::config::CommandPreset {
                name: "build".into(),
                argv: vec!["cargo".into(), "build".into()],
                cwd: None,
            }],
            ..Config::default()
        }));
        let launched = Rc::new(RefCell::new(Vec::<crate::launch::LaunchRequest>::new()));
        let open_launch = {
            let launched = launched.clone();
            Rc::new(move |req: crate::launch::LaunchRequest| {
                launched.borrow_mut().push(req);
            })
        };
        let current_dir = Rc::new(|| Some(PathBuf::from("/focused/pane")));
        let (_dialog, entry, list) = build_command_palette(
            &window,
            &config,
            &Bindings::default(),
            open_launch,
            current_dir,
        );

        // Simulate typing: the delayed search-changed signal feeds the
        // synthetic row. Handlers run in connect order, so once our probe
        // fires the palette's own handler has already run.
        let fired = Rc::new(std::cell::Cell::new(false));
        entry.connect_search_changed({
            let fired = fired.clone();
            move |_| fired.set(true)
        });
        entry.set_text("echo 'hello world'");
        crate::test_support::spin_until(|| fired.get());

        // Enter drives row-activated on the first row; do the same here.
        let typed_row = list.row_at_index(0).unwrap();
        list.emit_by_name::<()>("row-activated", &[&typed_row]);
        assert_eq!(
            launched.borrow().as_slice(),
            [crate::launch::LaunchRequest {
                cwd: Some(PathBuf::from("/focused/pane")),
                command: Some(vec!["echo".into(), "hello world".into()]),
            }]
        );

        // The synthetic row shifted preset rows by one; they must still map
        // to their own actions (model index 1 + all binding rows).
        launched.borrow_mut().clear();
        let preset_row = list.row_at_index(1 + COMMANDS.len() as i32).unwrap();
        list.emit_by_name::<()>("row-activated", &[&preset_row]);
        assert_eq!(
            launched.borrow().as_slice(),
            [crate::launch::LaunchRequest {
                cwd: None,
                command: Some(vec!["cargo".into(), "build".into()]),
            }]
        );

        // An unparseable query keeps the dialog open and launches nothing.
        launched.borrow_mut().clear();
        let fired = Rc::new(std::cell::Cell::new(false));
        entry.connect_search_changed({
            let fired = fired.clone();
            move |_| fired.set(true)
        });
        entry.set_text("echo 'unclosed");
        crate::test_support::spin_until(|| fired.get());
        list.emit_by_name::<()>("row-activated", &[&typed_row]);
        assert!(launched.borrow().is_empty());
    }
}
