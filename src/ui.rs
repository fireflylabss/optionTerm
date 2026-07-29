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
    config::{Config, CursorStyle, MiddleClickTab, NewTabPosition, TabsLocation, Theme},
    terminal::{Match, TerminalView},
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

/// Menu behind the `+` split button: everything that creates or acts on tabs
/// and splits, so the main menu does not have to carry any of it.
pub fn tiling_menu() -> gio::Menu {
    let menu = gio::Menu::new();

    let tabs = gio::Menu::new();
    tabs.append(Some("New Tab"), Some("win.new-tab"));
    tabs.append(Some("Rename Tab"), Some("win.rename-tab"));
    tabs.append(Some("Close Tab"), Some("win.close-tab"));
    menu.append_section(Some("Tabs"), &tabs);

    let nav = gio::Menu::new();
    nav.append(Some("All Tabs"), Some("win.tab-overview"));
    nav.append(Some("Next Tab"), Some("win.next-tab"));
    nav.append(Some("Previous Tab"), Some("win.prev-tab"));
    menu.append_section(None, &nav);

    menu.append_section(Some("Split"), &splits_menu());
    menu.append_section(Some("Terminal"), &terminal_menu());
    menu
}

/// Actions that act on the terminal currently in focus.
fn terminal_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    menu.append(Some("Clear Terminal"), Some("win.clear-tab"));
    menu.append(Some("Restart Terminal"), Some("win.restart-tab"));
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

/// The three theme swatches: a literal preview of System / Light / Dark with a
/// check badge on the active one.
///
/// Shared by the `···` menu and Preferences so both look identical.
pub fn theme_swatches() -> (gtk4::Box, [gtk4::ToggleButton; 3]) {
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 14);
    row.set_halign(gtk4::Align::Center);

    let swatches = [
        (Theme::System, "system", "Follow the system theme"),
        (Theme::Light, "light", "Light theme"),
        (Theme::Dark, "dark", "Dark theme"),
    ]
    .map(|(theme, class, tip)| {
        let button = gtk4::ToggleButton::builder()
            .tooltip_text(tip)
            .action_name("win.theme")
            .action_target(&theme.as_str().to_variant())
            .build();
        button.add_css_class("theme-swatch");
        button.add_css_class(class);

        // Badge in the corner, on top of the swatch, only while selected.
        let check = gtk4::Image::from_icon_name("object-select-symbolic");
        check.add_css_class("theme-check");
        check.set_halign(gtk4::Align::End);
        check.set_valign(gtk4::Align::End);
        check.set_visible(button.is_active());
        button
            .bind_property("active", &check, "visible")
            .sync_create()
            .build();

        let overlay = gtk4::Overlay::new();
        overlay.set_child(Some(&button));
        overlay.add_overlay(&check);
        row.append(&overlay);
        button
    });

    (row, swatches)
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

/// Theme swatches, a font-size stepper and the current grid size, mirroring the
/// quick controls FoxTerminal puts at the top of its menu.
pub struct QuickSettings {
    pub widget: gtk4::Box,
    swatches: [gtk4::ToggleButton; 3],
    zoom_label: gtk4::Label,
    grid_label: gtk4::Label,
}

impl QuickSettings {
    fn new() -> Self {
        let widget = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
        widget.add_css_class("quick-settings");

        // --- Theme ---
        let (themes, swatches) = theme_swatches();
        widget.append(&themes);

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
            swatches,
            zoom_label,
            grid_label,
        }
    }

    /// Reflect the active theme in the swatches.
    pub fn set_theme(&self, theme: Theme) {
        for (button, value) in self
            .swatches
            .iter()
            .zip([Theme::System, Theme::Light, Theme::Dark])
        {
            button.set_active(value == theme);
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

    // Same swatches as the `···` menu, so the two never disagree. They drive
    // the `win.theme` action, which already applies and persists the choice.
    let (theme_row_widget, theme_swatch_buttons) = theme_swatches();
    theme_row_widget.set_margin_top(6);
    theme_row_widget.set_margin_bottom(6);
    {
        let theme = config.borrow().theme;
        for (button, value) in
            theme_swatch_buttons
                .iter()
                .zip([Theme::System, Theme::Light, Theme::Dark])
        {
            button.set_active(value == theme);
        }
    }
    theme_group.add(&theme_row_widget);

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

    look_page.add(&theme_group);
    look_page.add(&font_group);

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
        .description("Remapping is not implemented yet; these are the built-in bindings")
        .build();
    for (label, _action, accel) in COMMANDS {
        let row = adw::ActionRow::builder().title(*label).build();
        if accel.is_empty() {
            let none = gtk4::Label::new(Some("—"));
            none.add_css_class("dim-label");
            row.add_suffix(&none);
        } else {
            let keys = gtk4::Label::new(Some(accel));
            keys.add_css_class("dim-label");
            keys.add_css_class("monospace");
            row.add_suffix(&keys);
        }
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
        .comments("GTK4 + libadwaita terminal powered by libghostty-vt")
        .build();
    about.add_acknowledgement_section(
        Some("Inspired by"),
        &[
            "Yacha (FoxTerminal) https://gitlab.com/OrangeFox/misc/FoxTerminal",
            "Ghostty https://ghostty.org",
        ],
    );
    about.add_legal_section(
        "FoxTerminal",
        // Ideas only: FoxTerminal is GPL-3.0-or-later and optionTerm is
        // Apache-2.0, so no code is shared between them.
        Some("optionTerm's sidebar-first workflow, its quick theme and font\ncontrols and the shape of its preferences were inspired by\nFoxTerminal by Yacha, whose terminal is why this one exists.\n\nFoxTerminal is licensed GPL-3.0-or-later. No FoxTerminal code is\nincluded in optionTerm; only ideas were borrowed."),
        gtk4::License::Custom,
        None,
    );
    about.present(Some(window));
}
