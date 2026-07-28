//! Menus, context menu and dialogs (palette, preferences, shortcuts, about).

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk4::gdk;
use gtk4::gio;
use gtk4::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;

use crate::{
    app::Pages,
    config::{Config, CursorStyle, TabsLocation, Theme},
    terminal::{Match, TerminalView},
};

/// (label, action, accel) for menus and the command palette.
pub const COMMANDS: &[(&str, &str, &str)] = &[
    ("New Tab", "win.new-tab", "Ctrl+Shift+T"),
    ("Close Tab", "win.close-tab", "Ctrl+Shift+W"),
    ("Next Tab", "win.next-tab", "Ctrl+PgDn"),
    ("Previous Tab", "win.prev-tab", "Ctrl+PgUp"),
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

pub fn tiling_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    menu.append_section(Some("Tiling in this window"), &splits_menu());
    menu
}

/// Actions that act on the terminal currently in focus.
fn terminal_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    menu.append(Some("Clear Terminal"), Some("win.clear-tab"));
    menu.append(Some("Restart Terminal"), Some("win.restart-tab"));
    menu
}

pub fn main_menu() -> gio::Menu {
    let menu = gio::Menu::new();

    // --- Tabs & splits ---
    let tabs = gio::Menu::new();
    tabs.append(Some("New Tab"), Some("win.new-tab"));
    tabs.append(Some("Close Tab"), Some("win.close-tab"));
    tabs.append(Some("Rename Tab"), Some("win.rename-tab"));
    tabs.append(Some("Next Tab"), Some("win.next-tab"));
    tabs.append(Some("Previous Tab"), Some("win.prev-tab"));
    menu.append_section(None, &tabs);
    menu.append_submenu(Some("Split"), &splits_menu());

    // --- Current terminal ---
    let edit = gio::Menu::new();
    edit.append(Some("Copy"), Some("win.copy"));
    edit.append(Some("Paste"), Some("win.paste"));
    edit.append(Some("Select All"), Some("win.select-all"));
    menu.append_section(Some("Edit"), &edit);
    menu.append_section(Some("Terminal"), &terminal_menu());

    // --- Appearance (submenu with live radios) ---
    let appearance = gio::Menu::new();

    let theme = gio::Menu::new();
    theme.append(Some("System"), Some("win.theme::system"));
    theme.append(Some("Light"), Some("win.theme::light"));
    theme.append(Some("Dark"), Some("win.theme::dark"));
    appearance.append_section(Some("Theme"), &theme);

    let tabs_pos = gio::Menu::new();
    tabs_pos.append(Some("Tabs on Top"), Some("win.tabs-pos::top"));
    tabs_pos.append(Some("Sidebar on the Left"), Some("win.tabs-pos::left"));
    tabs_pos.append(Some("Sidebar on the Right"), Some("win.tabs-pos::right"));
    tabs_pos.append(Some("Hidden Tabs"), Some("win.tabs-pos::hidden"));
    tabs_pos.append(Some("Always Show Sidebar"), Some("win.sidebar-always"));
    appearance.append_section(Some("Tabs"), &tabs_pos);

    let cursor = gio::Menu::new();
    cursor.append(Some("Block Cursor"), Some("win.cursor-shape::block"));
    cursor.append(Some("Bar Cursor"), Some("win.cursor-shape::bar"));
    cursor.append(
        Some("Underline Cursor"),
        Some("win.cursor-shape::underline"),
    );
    cursor.append(Some("Blinking Cursor"), Some("win.cursor-blink"));
    appearance.append_section(Some("Cursor"), &cursor);

    let zoom = gio::Menu::new();
    zoom.append(Some("Increase Font Size"), Some("win.zoom-in"));
    zoom.append(Some("Decrease Font Size"), Some("win.zoom-out"));
    zoom.append(Some("Default Font Size"), Some("win.zoom-reset"));
    appearance.append_section(Some("Font"), &zoom);

    menu.append_submenu(Some("Appearance"), &appearance);

    // --- Tools ---
    let tools = gio::Menu::new();
    tools.append(Some("Command Palette"), Some("win.command-palette"));
    tools.append(Some("Find in Scrollback"), Some("win.find"));
    tools.append(Some("Reload Configuration"), Some("win.reload-config"));
    tools.append(Some("Preferences"), Some("win.preferences"));
    menu.append_section(None, &tools);

    // --- Help ---
    let help = gio::Menu::new();
    help.append(Some("Keyboard Shortcuts"), Some("win.shortcuts"));
    help.append(Some("About optionTerm"), Some("win.about"));
    menu.append_section(Some("Help"), &help);

    let quit = gio::Menu::new();
    quit.append(Some("Quit"), Some("win.quit"));
    menu.append_section(None, &quit);

    menu
}

fn context_menu() -> gio::Menu {
    let menu = gio::Menu::new();

    let edit = gio::Menu::new();
    edit.append(Some("Copy"), Some("win.copy"));
    edit.append(Some("Paste"), Some("win.paste"));
    edit.append(Some("Select All"), Some("win.select-all"));
    menu.append_section(None, &edit);

    menu.append_section(None, &splits_menu());
    menu.append_section(None, &terminal_menu());

    let tabs = gio::Menu::new();
    tabs.append(Some("New Tab"), Some("win.new-tab"));
    tabs.append(Some("Close Tab"), Some("win.close-tab"));
    menu.append_section(None, &tabs);

    let misc = gio::Menu::new();
    misc.append(Some("Find in Scrollback"), Some("win.find"));
    misc.append(Some("Command Palette"), Some("win.command-palette"));
    misc.append(Some("Preferences"), Some("win.preferences"));
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

        let counter = gtk4::Label::new(None);
        counter.add_css_class("dim-label");
        counter.add_css_class("numeric");

        let prev = gtk4::Button::from_icon_name("go-up-symbolic");
        prev.set_tooltip_text(Some("Previous match (Shift+Enter)"));
        prev.add_css_class("flat");
        let next = gtk4::Button::from_icon_name("go-down-symbolic");
        next.set_tooltip_text(Some("Next match (Enter)"));
        next.add_css_class("flat");

        let boxed = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        boxed.append(&entry);
        boxed.append(&counter);
        boxed.append(&prev);
        boxed.append(&next);

        let bar = gtk4::SearchBar::builder()
            .search_mode_enabled(false)
            .show_close_button(true)
            .child(&boxed)
            .build();
        bar.connect_entry(&entry);

        // Matches are computed once per query and then just indexed into, so
        // stepping through hits never re-scans the scrollback.
        let matches: Rc<RefCell<Vec<Match>>> = Rc::new(RefCell::new(Vec::new()));
        let index = Rc::new(Cell::new(0usize));

        let refresh_counter = {
            let counter = counter.clone();
            let matches = matches.clone();
            let index = index.clone();
            Rc::new(move |query_empty: bool| {
                let total = matches.borrow().len();
                counter.set_text(&if query_empty {
                    String::new()
                } else if total == 0 {
                    "0/0".into()
                } else {
                    format!("{}/{}", index.get() + 1, total)
                });
            })
        };

        let step = {
            let matches = matches.clone();
            let index = index.clone();
            let current_view = current_view.clone();
            let refresh_counter = refresh_counter.clone();
            Rc::new(move |delta: isize| {
                let total = matches.borrow().len();
                if total == 0 {
                    return;
                }
                let cur = index.get() as isize;
                // Wrap around in both directions.
                let next = (cur + delta).rem_euclid(total as isize) as usize;
                index.set(next);
                if let (Some(view), Some(m)) = (current_view(), matches.borrow().get(next).copied())
                {
                    view.reveal_match(&m);
                }
                refresh_counter(false);
            })
        };

        {
            let matches = matches.clone();
            let index = index.clone();
            let current_view = current_view.clone();
            let refresh_counter = refresh_counter.clone();
            entry.connect_search_changed(move |e| {
                let query = e.text().to_string();
                let found = match current_view() {
                    Some(view) if !query.trim().is_empty() => view.search(&query),
                    _ => Vec::new(),
                };
                let first = found.first().copied();
                *matches.borrow_mut() = found;
                index.set(0);
                if let (Some(view), Some(m)) = (current_view(), first) {
                    view.reveal_match(&m);
                }
                refresh_counter(query.trim().is_empty());
            });
        }
        {
            let step = step.clone();
            entry.connect_activate(move |_| step(1));
        }
        {
            let step = step.clone();
            next.connect_clicked(move |_| step(1));
        }
        {
            let step = step.clone();
            prev.connect_clicked(move |_| step(-1));
        }
        {
            // Shift+Enter walks backwards; Escape closes and returns focus to
            // the terminal. The bar has no key-capture widget, so both have to
            // be handled here.
            let step = step.clone();
            let bar_weak = bar.downgrade();
            let current_view = current_view.clone();
            let key = gtk4::EventControllerKey::new();
            key.connect_key_pressed(move |_, keyval, _, modifier| {
                if keyval == gdk::Key::Return && modifier.contains(gdk::ModifierType::SHIFT_MASK) {
                    step(-1);
                    return gtk4::glib::Propagation::Stop;
                }
                if keyval == gdk::Key::Escape {
                    if let Some(bar) = bar_weak.upgrade() {
                        bar.set_search_mode(false);
                    }
                    if let Some(view) = current_view() {
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

pub fn show_command_palette(window: &adw::ApplicationWindow) {
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

    // Rows map 1:1 onto COMMANDS by index; filtering never reorders them.
    for (label, _, accel) in COMMANDS {
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
        list.set_filter_func(move |row| {
            let q = query.borrow();
            if q.is_empty() {
                return true;
            }
            COMMANDS
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
        Rc::new(move |row: &gtk4::ListBoxRow| {
            let Some((_, action, _)) = COMMANDS.get(row.index() as usize) else {
                return;
            };
            dialog.close();
            gtk4::prelude::WidgetExt::activate_action(&window, action, None).ok();
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
    dialog.present(Some(window));
    entry.grab_focus();
}

pub fn show_preferences(
    window: &adw::ApplicationWindow,
    config: &Rc<RefCell<Config>>,
    pages: &Pages,
    apply_zoom: Rc<dyn Fn(f32)>,
    set_tabs_location: Rc<dyn Fn(TabsLocation)>,
    save_config: Rc<dyn Fn()>,
) {
    let dialog = adw::PreferencesDialog::builder()
        .title("Preferences")
        .build();
    let page = adw::PreferencesPage::builder()
        .title("General")
        .icon_name("preferences-system-symbolic")
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
    let appearance = adw::PreferencesGroup::builder().title("Appearance").build();

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
            let scheme = match theme {
                Theme::Light => adw::ColorScheme::ForceLight,
                Theme::Dark => adw::ColorScheme::ForceDark,
                Theme::System => adw::ColorScheme::Default,
            };
            adw::StyleManager::default().set_color_scheme(scheme);
            config.borrow_mut().theme = theme;
            save_config();
        });
    }
    appearance.add(&theme_row);

    let tabs_row = adw::ComboRow::builder()
        .title("Tab Position")
        .subtitle("config.toml: window.tabs")
        .model(&gtk4::StringList::new(&[
            "Top",
            "Sidebar (left)",
            "Sidebar (right)",
            "Hidden",
        ]))
        .build();
    tabs_row.set_selected(match config.borrow().tabs_location {
        TabsLocation::Top => 0,
        TabsLocation::Left => 1,
        TabsLocation::Right => 2,
        TabsLocation::Hidden => 3,
    });
    {
        let config = config.clone();
        let set_tabs_location = set_tabs_location.clone();
        let save_config = save_config.clone();
        tabs_row.connect_selected_notify(move |row| {
            let location = match row.selected() {
                1 => TabsLocation::Left,
                2 => TabsLocation::Right,
                3 => TabsLocation::Hidden,
                _ => TabsLocation::Top,
            };
            config.borrow_mut().tabs_location = location;
            set_tabs_location(location);
            save_config();
        });
    }
    appearance.add(&tabs_row);

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
    appearance.add(&sidebar_always_row);

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
    appearance.add(&font_row);

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
    appearance.add(&padding_row);

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
    appearance.add(&opacity_row);
    page.add(&appearance);

    // --- Session ---
    let session_group = adw::PreferencesGroup::builder().title("Session").build();
    let restore_row = adw::SwitchRow::builder()
        .title("Restore Tabs on Start")
        .subtitle("Reopens tabs, panes and their working directories")
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
    page.add(&session_group);

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
    page.add(&cursor_group);

    // --- Config file ---
    let cfg_group = adw::PreferencesGroup::builder()
        .title("Configuration")
        .description("~/.option/terminal/config.toml (generated from Ghostty on first run)")
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
    page.add(&cfg_group);

    dialog.add(&page);
    dialog.present(Some(window));
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
        .developer_name("AE Firefly Labs")
        .version(env!("CARGO_PKG_VERSION"))
        .license_type(gtk4::License::Apache20)
        .website("https://github.com/fireflylabss/optionTerm")
        .issue_url("https://github.com/fireflylabss/optionTerm/issues")
        .comments("GTK4 + libadwaita terminal powered by libghostty-vt")
        .build();
    about.present(Some(window));
}
