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
    terminal::TerminalView,
};

/// (label, action, accel) for menus and the command palette.
pub const COMMANDS: &[(&str, &str, &str)] = &[
    ("New Tab", "win.new-tab", "Ctrl+Shift+T"),
    ("Close Tab", "win.close-tab", "Ctrl+Shift+W"),
    ("Next Tab", "win.next-tab", "Ctrl+PgDn"),
    ("Previous Tab", "win.prev-tab", "Ctrl+PgUp"),
    ("All Tabs", "win.tab-overview", "F1"),
    ("Split Right", "win.split-right", "Ctrl+Shift+O"),
    ("Split Down", "win.split-down", "Ctrl+Shift+E"),
    ("Split Left", "win.split-left", "Ctrl+Shift+L"),
    ("Split Up", "win.split-up", "Ctrl+Shift+U"),
    (
        "Toggle Split Zoom",
        "win.toggle-split-zoom",
        "Ctrl+Shift+Enter",
    ),
    ("Equalize Splits", "win.equalize-splits", ""),
    ("Focus Split Left", "win.focus-split-left", "Ctrl+Alt+←"),
    ("Focus Split Right", "win.focus-split-right", "Ctrl+Alt+→"),
    ("Focus Split Up", "win.focus-split-up", "Ctrl+Alt+↑"),
    ("Focus Split Down", "win.focus-split-down", "Ctrl+Alt+↓"),
    ("Previous Split", "win.focus-split-previous", "Ctrl+Super+["),
    ("Next Split", "win.focus-split-next", "Ctrl+Super+]"),
    ("Copy", "win.copy", "Ctrl+Shift+C"),
    ("Paste", "win.paste", "Ctrl+Shift+V"),
    ("Select All", "win.select-all", "Ctrl+Shift+A"),
    ("Clear Terminal", "win.clear-tab", "Ctrl+Shift+K"),
    ("Restart Terminal", "win.restart-tab", "Ctrl+Shift+R"),
    ("Increase Font Size", "win.zoom-in", "Ctrl++"),
    ("Decrease Font Size", "win.zoom-out", "Ctrl+-"),
    ("Default Font Size", "win.zoom-reset", "Ctrl+0"),
    ("Find in Scrollback", "win.find", "Ctrl+Shift+F"),
    ("Rename Tab", "win.rename-tab", "F2"),
    ("Reload Configuration", "win.reload-config", ""),
    ("Preferences", "win.preferences", "Ctrl+,"),
    ("Keyboard Shortcuts", "win.shortcuts", ""),
    ("About optionTerm", "win.about", ""),
    ("Quit", "win.quit", "Ctrl+Shift+Q"),
];

fn splits_menu() -> gio::Menu {
    let split = gio::Menu::new();
    split.append(Some("Split Right"), Some("win.split-right"));
    split.append(Some("Split Down"), Some("win.split-down"));
    split.append(Some("Split Left"), Some("win.split-left"));
    split.append(Some("Split Up"), Some("win.split-up"));
    split.append(Some("Toggle Split Zoom"), Some("win.toggle-split-zoom"));
    split.append(Some("Equalize Splits"), Some("win.equalize-splits"));
    split
}

/// Menu behind the `+` split button: the split directions and nothing else.
/// Clicking the button itself opens a tab, so tab actions live on the tab's own
/// context menu instead of being buried here.
pub fn tiling_menu() -> gio::Menu {
    splits_menu()
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
    tools.append(Some("Command Palette"), Some("win.command-palette"));
    menu.append_section(None, &tools);

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

        {
            let current_view = current_view.clone();
            entry.connect_search_changed(move |e| {
                let query = e.text().to_string();
                if let Some(view) = current_view() {
                    view.search_set_query(&query);
                    if !query.trim().is_empty() {
                        let _ = view.search_find_next();
                    }
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
                    if let Some(view) = current_view() {
                        view.search_set_query("");
                        view.focus();
                    }
                    return gtk4::glib::Propagation::Stop;
                }
                gtk4::glib::Propagation::Proceed
            });
            entry.add_controller(key);
        }

        Self { widget: bar, entry }
    }

    /// Open the bar and focus the entry; re-running the query if there is one.
    pub fn open(&self) {
        self.widget.set_search_mode(true);
        self.entry.grab_focus();
        self.entry.select_region(0, -1);
    }
}

pub fn show_command_palette(
    window: &adw::ApplicationWindow,
    config: &Rc<RefCell<Config>>,
    open_launch: Rc<dyn Fn(crate::launch::LaunchRequest)>,
) {
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

    #[derive(Clone)]
    enum PaletteAction {
        Win(&'static str),
        Launch(crate::launch::LaunchRequest),
    }

    let mut entries: Vec<(String, String, PaletteAction)> = COMMANDS
        .iter()
        .map(|(label, action, accel)| {
            (
                (*label).to_string(),
                (*accel).to_string(),
                PaletteAction::Win(action),
            )
        })
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

    let query: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));
    {
        let query = query.clone();
        let entries = entries.clone();
        list.set_filter_func(move |row| {
            let q = query.borrow();
            if q.is_empty() {
                return true;
            }
            entries
                .get(row.index() as usize)
                .map(|(label, _, _)| {
                    let needle = label.to_lowercase();
                    q.split_whitespace().all(|w| needle.contains(w))
                })
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
        let window = window.clone();
        let dialog = dialog.clone();
        let entries = entries.clone();
        let open_launch = open_launch.clone();
        Rc::new(move |row: &gtk4::ListBoxRow| {
            let Some((_, _, action)) = entries.get(row.index() as usize) else {
                return;
            };
            dialog.close();
            match action {
                PaletteAction::Win(action) => {
                    gtk4::prelude::WidgetExt::activate_action(&window, action, None).ok();
                }
                PaletteAction::Launch(req) => open_launch(req.clone()),
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

    dialog.present(Some(window));
    entry.grab_focus();
}

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
        Rc::new(move |f: Rc<dyn Fn(&mut Config)>| {
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
        let uri = format!("file://{}", source.display());
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
            font_row.set_value(config.borrow().font_size as f64);
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

    // --- Default terminal ---
    let default_page = adw::PreferencesPage::builder()
        .title("Default Terminal")
        .icon_name("utilities-terminal-symbolic")
        .build();
    let default_group = adw::PreferencesGroup::builder()
        .title("System Integration")
        .description(
            "There is no single setting for this. optionTerm writes the portable \
             xdg-terminals.list, plus your desktop's own key when it has one.",
        )
        .build();
    let default_row = adw::ActionRow::builder()
        .title("Set as Default Terminal")
        .build();
    let default_btn = gtk4::Button::new();
    default_btn.add_css_class("flat");
    default_btn.set_valign(gtk4::Align::Center);

    let refresh_default = {
        let row = default_row.clone();
        let button = default_btn.clone();
        Rc::new(move || {
            if crate::default_terminal::is_default() {
                row.set_subtitle("optionTerm is the preferred terminal");
                button.set_label("Set Again");
            } else {
                row.set_subtitle("Another terminal is preferred");
                button.set_label("Set as Default");
            }
        })
    };
    refresh_default();
    {
        let refresh_default = refresh_default.clone();
        let dialog = dialog.clone();
        default_btn.connect_clicked(move |_| {
            let toast = match crate::default_terminal::set_default() {
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
            refresh_default();
            dialog.add_toast(adw::Toast::builder().title(&toast).timeout(4).build());
        });
    }
    default_row.add_suffix(&default_btn);
    default_group.add(&default_row);
    default_page.add(&default_group);

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
            let builtin = COMMANDS
                .iter()
                .find(|(_, a, _)| a.trim_start_matches("win.") == name)
                .map(|(_, _, d)| *d)
                .unwrap_or("");
            reset.connect_clicked(move |_| {
                bindings.borrow_mut().set(&name, None);
                if let Err(err) = bindings.borrow().save() {
                    tracing::warn!("could not save shortcuts: {err:#}");
                }
                apply_bindings();
                button.set_label(if builtin.is_empty() {
                    "Unbound"
                } else {
                    builtin
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

    dialog.add(&look_page);
    dialog.add(&behavior_page);
    dialog.add(&sound_page);
    dialog.add(&shortcuts_page);
    dialog.add(&default_page);
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

pub fn show_shortcuts(window: &adw::ApplicationWindow) {
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

    for (label, _, accel) in COMMANDS.iter().filter(|(_, _, a)| !a.is_empty()) {
        let row = adw::ActionRow::builder().title(*label).build();
        let key = gtk4::Label::new(Some(accel));
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
