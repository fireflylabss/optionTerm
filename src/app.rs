//! Adwaita application shell with TabView, splits and command palette.

use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::{Rc, Weak},
};

use gtk4::gdk;
use gtk4::gio::{self, prelude::*};
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;

use crate::{
    browser,
    config::{
        Config, CursorStyle, MiddleClickTab, NewTabPosition, TabOverflow, TabWidth, TabsLocation,
        Theme,
    },
    launch::LaunchRequest,
    session::{PaneLayout, Session as SessionState, SplitOrientation, TabState},
    terminal::TerminalView,
    ui::{
        PrefsHooks, SearchBar, attach_context_menu, main_popover, show_about, show_command_palette,
        show_preferences, show_shortcuts, tab_menu, tiling_menu,
    },
};

const APP_ID: &str = "io.option.terminal";

/// How long after our own `config.toml` write the file monitor stays quiet.
const SELF_WRITE_GRACE: std::time::Duration = std::time::Duration::from_millis(1500);

pub type Pages = Rc<RefCell<Vec<(adw::TabPage, Vec<Rc<TerminalView>>)>>>;
type Toast = Rc<dyn Fn(&str)>;
type Focused = Rc<RefCell<Option<Weak<TerminalView>>>>;
type MakeViewFn = Rc<
    dyn Fn(
        Rc<RefCell<Option<adw::TabPage>>>,
        Option<PathBuf>,
        Option<Vec<String>>,
    ) -> anyhow::Result<Rc<TerminalView>>,
>;

/// Shared between `command-line` and the open window so a second instance can
/// open a tab in the primary process.
struct SharedLaunch {
    pending: RefCell<Vec<LaunchRequest>>,
    open_in_window: RefCell<Option<Rc<dyn Fn(LaunchRequest)>>>,
}

pub fn run() -> anyhow::Result<()> {
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();

    let shared = Rc::new(SharedLaunch {
        pending: RefCell::new(Vec::new()),
        open_in_window: RefCell::new(None),
    });

    {
        let shared = shared.clone();
        app.connect_command_line(move |app, cmdline| {
            let args: Vec<String> = cmdline
                .arguments()
                .iter()
                .map(|a| a.to_string_lossy().into_owned())
                .collect();
            if args.iter().any(|a| a == "-h" || a == "--help") {
                let help = "Usage: optionterm [OPTIONS] [DIRECTORY]\n\n\
Options:\n\
  -d, --working-directory DIR   Start in DIR\n\
  -e, --command CMD [ARGS…]     Run CMD instead of the shell\n\
  -- CMD [ARGS…]                Same as -e\n\
  -h, --help                    Show this help\n";
                eprintln!("{help}");
                return 0;
            }
            let req = crate::launch::parse_args(&args);
            if let Some(open) = shared.open_in_window.borrow().clone() {
                open(req);
            } else {
                shared.pending.borrow_mut().push(req);
            }
            app.activate();
            0
        });
    }

    {
        let shared = shared.clone();
        app.connect_activate(move |app| {
            if !app.windows().is_empty() {
                if let Some(win) = app.active_window() {
                    win.present();
                } else if let Some(win) = app.windows().into_iter().next() {
                    win.present();
                }
                return;
            }
            let initial = shared.pending.borrow_mut().pop().unwrap_or_default();
            if let Err(err) = build_window(app, shared.clone(), initial) {
                tracing::error!("failed to open window: {err:#}");
            }
        });
    }

    let code = app.run();
    if code == glib::ExitCode::SUCCESS {
        Ok(())
    } else {
        anyhow::bail!("application exited with {code:?}")
    }
}

/// Apply a config theme to the Adwaita style manager.
///
/// `System` leaves `ColorScheme::Default` in place, which is what makes
/// libadwaita follow the desktop's light/dark preference live.
fn apply_theme(theme: Theme) {
    let scheme = match theme {
        Theme::Light => adw::ColorScheme::ForceLight,
        Theme::Dark => adw::ColorScheme::ForceDark,
        Theme::System => adw::ColorScheme::Default,
    };
    adw::StyleManager::default().set_color_scheme(scheme);
}

/// Make the window itself translucent when `background-opacity < 1`.
///
/// The terminal already paints its own background with alpha; without
/// clearing the Adwaita window background the compositor would still see an
/// opaque surface underneath.
fn apply_window_opacity(window: &adw::ApplicationWindow, opacity: f64) {
    const CSS_CLASS: &str = "transparent-bg";
    if opacity >= 1.0 {
        window.remove_css_class(CSS_CLASS);
    } else {
        window.add_css_class(CSS_CLASS);
    }
}

/// Install the stylesheet backing `apply_window_opacity` once per display.
fn install_css(display: &gdk::Display) {
    let provider = gtk4::CssProvider::new();
    provider.load_from_string(
        "window.transparent-bg,
         window.transparent-bg > * ,
         window.transparent-bg .terminal { background-color: transparent; }

         .quick-settings { padding: 8px 14px 4px 14px; }",
    );
    gtk4::style_context_add_provider_for_display(
        display,
        &provider,
        gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

/// The tab under a point in the tab bar, if any.
///
/// libadwaita exposes neither hit-testing nor a middle-click signal on
/// `AdwTabBar`, so this picks the widget under the pointer and walks up looking
/// for the internal tab widget, which carries the page it represents as a
/// property. Reading it by name keeps this working without the private type
/// being public; if a future libadwaita renames it, the lookup simply fails and
/// middle click becomes a no-op rather than acting on the wrong tab.
fn tab_page_at(tab_bar: &adw::TabBar, x: f64, y: f64) -> Option<adw::TabPage> {
    let mut widget = tab_bar.pick(x, y, gtk4::PickFlags::DEFAULT)?;
    loop {
        // `property_value` panics on an unknown property, and most widgets on
        // the way up do not have one, so check before reading.
        if widget.has_property("page", Some(adw::TabPage::static_type()))
            && let Ok(page) = widget.property_value("page").get::<adw::TabPage>()
        {
            return Some(page);
        }
        widget = widget.parent()?;
    }
}

/// Key used to remember that the user renamed a tab by hand.
const RENAMED_KEY: &str = "option-term-renamed";
/// Key used to keep browser pages out of terminal split operations.
const BROWSER_TAB_KEY: &str = "option-term-browser";

/// Whether the user gave this tab a custom title, in which case the shell's
/// OSC title updates must not overwrite it.
fn tab_is_renamed(page: &adw::TabPage) -> bool {
    // SAFETY: the key is only ever written with a `bool` below.
    unsafe { page.data::<bool>(RENAMED_KEY).map(|v| *v.as_ref()) }.unwrap_or(false)
}

fn set_tab_renamed(page: &adw::TabPage, renamed: bool) {
    unsafe { page.set_data(RENAMED_KEY, renamed) };
}

fn is_browser_tab(page: &adw::TabPage) -> bool {
    unsafe { page.data::<bool>(BROWSER_TAB_KEY).map(|v| *v.as_ref()) }.unwrap_or(false)
}

fn set_browser_tab(page: &adw::TabPage) {
    unsafe { page.set_data(BROWSER_TAB_KEY, true) };
}

/// Ask for a new tab title. Clearing the field restores the shell's title.
fn rename_tab_dialog(anchor: &impl IsA<gtk4::Widget>, page: &adw::TabPage) {
    let dialog = adw::AlertDialog::new(Some("Rename Tab"), None);
    let entry = gtk4::Entry::builder()
        .text(page.title())
        .activates_default(true)
        .build();
    dialog.set_extra_child(Some(&entry));
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("rename", "Rename");
    dialog.set_response_appearance("rename", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("rename"));
    dialog.set_close_response("cancel");

    let page = page.clone();
    let entry_c = entry.clone();
    dialog.connect_response(None, move |_, response| {
        if response != "rename" {
            return;
        }
        let title = entry_c.text().trim().to_string();
        if title.is_empty() {
            // Empty means "go back to following the shell".
            set_tab_renamed(&page, false);
        } else {
            set_tab_renamed(&page, true);
            page.set_title(&title);
        }
    });
    dialog.present(Some(anchor));
    entry.grab_focus();
}

/// Swap `old` for `new` in old's parent (tab root Box or a Paned).
fn replace_in_parent(old: &gtk4::Widget, new: &gtk4::Widget) {
    let Some(parent) = old.parent() else { return };
    if let Some(bx) = parent.downcast_ref::<gtk4::Box>() {
        bx.remove(old);
        bx.append(new);
    } else if let Some(paned) = parent.downcast_ref::<gtk4::Paned>() {
        if paned.start_child().as_ref() == Some(old) {
            paned.set_start_child(Some(new));
        } else {
            paned.set_end_child(Some(new));
        }
    }
}

/// Remove a terminal widget from its split, collapsing the Paned around it.
fn collapse_split(widget: &gtk4::Widget) {
    let Some(parent) = widget.parent() else {
        return;
    };
    if let Some(paned) = parent.downcast_ref::<gtk4::Paned>() {
        let sibling = if paned.start_child().as_ref() == Some(widget) {
            paned.end_child()
        } else {
            paned.start_child()
        };
        paned.set_start_child(gtk4::Widget::NONE);
        paned.set_end_child(gtk4::Widget::NONE);
        if let Some(sibling) = sibling {
            replace_in_parent(paned.upcast_ref(), &sibling);
        }
    } else if let Some(bx) = parent.downcast_ref::<gtk4::Box>() {
        bx.remove(widget);
    }
}

fn build_window(
    app: &adw::Application,
    shared: Rc<SharedLaunch>,
    initial: LaunchRequest,
) -> anyhow::Result<()> {
    let config = Rc::new(RefCell::new(Config::load()?));
    let base_font_size = Rc::new(Cell::new(config.borrow().font_size));

    // Shortcut overrides live in their own file. Only overridden actions are
    // touched, so new built-in defaults still reach existing installs.
    let bindings = Rc::new(RefCell::new(crate::keys::Bindings::load()));
    let apply_bindings: Rc<dyn Fn()> = {
        let app = app.clone();
        let bindings = bindings.clone();
        Rc::new(move || {
            for (_, action, _) in crate::ui::COMMANDS {
                let name = action.trim_start_matches("win.");
                let Some(accel) = bindings.borrow().get(name).map(str::to_string) else {
                    continue;
                };
                if accel.is_empty() {
                    app.set_accels_for_action(action, &[]);
                } else {
                    let gtk_accel = crate::keys::to_gtk_accel(&accel);
                    app.set_accels_for_action(action, &[&gtk_accel]);
                }
            }
        })
    };
    tracing::info!(
        "loaded config from {} (font={} size={})",
        config.borrow().source.display(),
        config.borrow().font_family,
        config.borrow().font_size
    );

    // Timestamp of our own last write to config.toml, so the file monitor can
    // ignore the change it causes instead of reloading in a loop.
    let self_write = Rc::new(Cell::new(std::time::Instant::now() - SELF_WRITE_GRACE));

    // Persist settings changed from the UI so they survive a restart.
    let save_config: Rc<dyn Fn()> = {
        let config = config.clone();
        let self_write = self_write.clone();
        Rc::new(move || {
            self_write.set(std::time::Instant::now());
            if let Err(err) = config.borrow().save() {
                tracing::warn!("could not persist configuration: {err:#}");
            }
        })
    };

    // Honor the saved theme before the window is mapped to avoid a flash.
    apply_theme(config.borrow().theme);
    if let Some(display) = gdk::Display::default() {
        install_css(&display);
    }

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("optionTerm")
        .default_width(960)
        .default_height(640)
        .build();
    apply_window_opacity(&window, config.borrow().background_opacity);

    let toolbar = adw::ToolbarView::new();

    let header = adw::HeaderBar::new();
    header.add_css_class("option-chrome");
    // Title-button packing follows `gtk-decoration-layout` (see
    // `apply_desktop_chrome` below).
    header.set_show_start_title_buttons(true);
    header.set_show_end_title_buttons(true);

    let window_title = adw::WindowTitle::new("optionTerm", "");
    {
        let window_title = window_title.clone();
        window.connect_notify_local(Some("title"), move |w, _| {
            window_title.set_title(&w.title().unwrap_or_else(|| "optionTerm".into()));
        });
    }

    let tab_view = adw::TabView::new();
    tab_view.set_vexpand(true);
    tab_view.set_hexpand(true);

    let tab_bar = adw::TabBar::new();
    tab_bar.set_view(Some(&tab_view));
    tab_bar.set_autohide(false);
    tab_bar.set_hexpand(true);

    // Tab shape. `expand-tabs` is a genuine AdwTabBar property. Squeeze-versus-
    // scroll is not exposed at all, so it is expressed as a minimum tab width:
    // once tabs cannot shrink past it, AdwTabBar scrolls on its own.
    let apply_tab_shape: Rc<dyn Fn()> = {
        let tab_bar = tab_bar.clone();
        let config = config.clone();
        let provider = gtk4::CssProvider::new();
        if let Some(display) = gdk::Display::default() {
            gtk4::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
            );
        }
        Rc::new(move || {
            let (width, overflow) = {
                let cfg = config.borrow();
                (cfg.tab_width, cfg.tab_overflow)
            };
            tab_bar.set_expand_tabs(width == TabWidth::Fill);
            provider.load_from_string(match overflow {
                TabOverflow::Scroll => "tabbar tabbox > tab { min-width: 150px; }",
                TabOverflow::Squeeze => "",
            });
        })
    };
    apply_tab_shape();
    header.set_title_widget(Some(&tab_bar));

    // `+` with the 4-direction tiling dropdown, same as the sidebar variant,
    // so splits are reachable with the tab bar on top too.
    let new_tab_btn = adw::SplitButton::builder()
        .icon_name("tab-new-symbolic")
        .tooltip_text("New Tab (Ctrl+Shift+T)")
        .menu_model(&tiling_menu())
        .build();
    new_tab_btn.set_action_name(Some("win.new-tab"));
    header.pack_start(&new_tab_btn);

    let palette_btn = gtk4::Button::from_icon_name("system-search-symbolic");
    palette_btn.set_tooltip_text(Some("Command Palette (Ctrl+Shift+P)"));
    palette_btn.add_css_class("flat");
    palette_btn.set_action_name(Some("win.command-palette"));
    palette_btn.set_visible(config.borrow().show_search_button);
    header.pack_start(&palette_btn);

    // Shows the tab count and opens the overview, the way libadwaita expects
    // AdwTabOverview to be reached.
    let tab_button = adw::TabButton::builder().view(&tab_view).build();
    tab_button.set_tooltip_text(Some("All Tabs (F1)"));
    tab_button.set_action_name(Some("win.tab-overview"));
    header.pack_end(&tab_button);

    let (menu_popover, quick_settings) = main_popover();
    let quick_settings = Rc::new(quick_settings);
    let menu_btn = gtk4::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .tooltip_text("Main Menu")
        .build();
    menu_btn.add_css_class("flat");
    menu_btn.set_popover(Some(&menu_popover));
    header.pack_end(&menu_btn);

    // Search bar sits above the tabs so it spans whichever pane is focused.
    let content_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    content_box.append(&tab_view);

    let toast_overlay = adw::ToastOverlay::new();
    toast_overlay.set_child(Some(&content_box));

    // Tabs-as-sidebar support
    let sidebar_list = gtk4::ListBox::new();
    sidebar_list.add_css_class("navigation-sidebar");
    sidebar_list.set_selection_mode(gtk4::SelectionMode::Single);

    let sidebar_scroll = gtk4::ScrolledWindow::new();
    sidebar_scroll.set_vexpand(true);
    sidebar_scroll.set_child(Some(&sidebar_list));

    // Raised split button,
    // with the 4-direction tiling dropdown.
    let sidebar_new_btn = adw::SplitButton::builder()
        .icon_name("tab-new-symbolic")
        .tooltip_text("New Tab (Ctrl+Shift+T)")
        .menu_model(&tiling_menu())
        .build();
    sidebar_new_btn.set_action_name(Some("win.new-tab"));

    // Sidebar copies of the header buttons (palette + main menu).
    let sidebar_palette_btn = gtk4::Button::from_icon_name("system-search-symbolic");
    sidebar_palette_btn.set_tooltip_text(Some("Command Palette (Ctrl+Shift+P)"));
    sidebar_palette_btn.add_css_class("flat");
    sidebar_palette_btn.set_action_name(Some("win.command-palette"));
    sidebar_palette_btn.set_visible(config.borrow().show_search_button);

    let sidebar_menu_btn = gtk4::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .tooltip_text("Menu")
        .build();
    sidebar_menu_btn.add_css_class("flat");
    // A popover belongs to one parent, so the sidebar gets its own instance.
    let (sidebar_quick_popover, sidebar_quick) = main_popover();
    let sidebar_quick = Rc::new(sidebar_quick);
    sidebar_menu_btn.set_popover(Some(&sidebar_quick_popover));

    // Both menus show the same quick controls, so they are refreshed together.
    let last_grid = Rc::new(Cell::new((0u16, 0u16)));
    let sync_quick: Rc<dyn Fn()> = {
        let config = config.clone();
        let base_font_size = base_font_size.clone();
        let last_grid = last_grid.clone();
        let quicks = [quick_settings.clone(), sidebar_quick.clone()];
        Rc::new(move || {
            let size = config.borrow().font_size;
            let (cols, rows) = last_grid.get();
            for quick in &quicks {
                quick.set_font_size(size, base_font_size.get());
                if cols > 0 {
                    quick.set_grid(cols, rows);
                }
            }
        })
    };
    sync_quick();

    // A real HeaderBar inside the sidebar: window controls get the exact
    // system styling (theme CSS targets `headerbar windowcontrols`) and the
    // bar is natively draggable. [+ ▾] layout: [● ● ●] [+ ▾] … [🔍] [☰]
    let sidebar_header = adw::HeaderBar::new();
    sidebar_header.add_css_class("flat");
    sidebar_header.set_show_start_title_buttons(true);
    sidebar_header.set_show_end_title_buttons(true);
    sidebar_header.set_title_widget(Some(&gtk4::Box::new(gtk4::Orientation::Horizontal, 0)));
    sidebar_header.pack_start(&sidebar_new_btn);
    sidebar_header.pack_end(&sidebar_menu_btn);
    sidebar_header.pack_end(&sidebar_palette_btn);

    let sidebar_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    sidebar_box.append(&sidebar_header);
    sidebar_box.append(&sidebar_scroll);

    let split_view = adw::OverlaySplitView::new();
    split_view.set_sidebar_width_fraction(0.22);
    split_view.set_max_sidebar_width(280.0);
    split_view.set_min_sidebar_width(160.0);
    split_view.set_show_sidebar(false);
    split_view.set_sidebar(Some(&sidebar_box));
    // Grid of tab thumbnails (pages already set live thumbnails).
    let tab_overview = adw::TabOverview::builder()
        .view(&tab_view)
        .enable_new_tab(true)
        .child(&toast_overlay)
        .build();
    split_view.set_content(Some(&tab_overview));

    // Bottom tab strip holder — empty until `tabs = "bottom"`.
    let bottom_tabs = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    bottom_tabs.add_css_class("option-chrome");
    bottom_tabs.set_visible(false);

    toolbar.add_top_bar(&header);
    toolbar.add_bottom_bar(&bottom_tabs);
    toolbar.set_content(Some(&split_view));
    window.set_content(Some(&toolbar));

    // Follow desktop GTK settings for chrome font / scale / decorations.
    apply_desktop_chrome(&header, &window);

    // Rebuild the sidebar rows from the current TabView pages.
    let sidebar_syncing = Rc::new(Cell::new(false));
    let rebuild_sidebar = {
        let sidebar_list = sidebar_list.clone();
        let tab_view = tab_view.clone();
        let sidebar_syncing = sidebar_syncing.clone();
        Rc::new(move || {
            sidebar_syncing.set(true);
            while let Some(child) = sidebar_list.first_child() {
                sidebar_list.remove(&child);
            }
            let selected = tab_view.selected_page();
            for i in 0..tab_view.n_pages() {
                let page = tab_view.nth_page(i);
                let row = adw::ActionRow::builder()
                    .title(page.title())
                    .activatable(true)
                    .build();

                let close = gtk4::Button::from_icon_name("window-close-symbolic");
                close.add_css_class("flat");
                close.set_valign(gtk4::Align::Center);
                {
                    let tab_view = tab_view.clone();
                    let page = page.clone();
                    close.connect_clicked(move |_| {
                        tab_view.close_page(&page);
                    });
                }
                row.add_suffix(&close);

                // Double-click a row to rename the tab.
                {
                    let page = page.clone();
                    let row_weak = row.downgrade();
                    let gesture = gtk4::GestureClick::new();
                    gesture.set_button(gdk::BUTTON_PRIMARY);
                    gesture.connect_pressed(move |_, n, _, _| {
                        if n == 2
                            && let Some(row) = row_weak.upgrade()
                        {
                            rename_tab_dialog(&row, &page);
                        }
                    });
                    row.add_controller(gesture);
                }

                // Drag a row onto another to reorder the tab.
                {
                    let source = gtk4::DragSource::new();
                    source.set_actions(gdk::DragAction::MOVE);
                    let idx = i;
                    source.connect_prepare(move |_, _, _| {
                        Some(gdk::ContentProvider::for_value(&idx.to_value()))
                    });
                    row.add_controller(source);

                    let target = gtk4::DropTarget::new(i32::static_type(), gdk::DragAction::MOVE);
                    let tab_view = tab_view.clone();
                    let dest = i;
                    target.connect_drop(move |_, value, _, _| {
                        let Ok(from) = value.get::<i32>() else {
                            return false;
                        };
                        if from == dest || from < 0 || from >= tab_view.n_pages() {
                            return false;
                        }
                        let page = tab_view.nth_page(from);
                        tab_view.reorder_page(&page, dest);
                        true
                    });
                    row.add_controller(target);
                }

                // Keep the row title in sync without keeping the row alive.
                {
                    let weak_row = glib::object::WeakRef::<adw::ActionRow>::new();
                    weak_row.set(Some(&row));
                    page.connect_notify_local(Some("title"), move |p, _| {
                        if let Some(row) = weak_row.upgrade() {
                            row.set_title(&p.title());
                        }
                    });
                }

                sidebar_list.append(&row);
                if selected.as_ref() == Some(&page) {
                    sidebar_list.select_row(Some(&row));
                }
            }
            sidebar_syncing.set(false);
        })
    };

    {
        let tab_view = tab_view.clone();
        let sidebar_syncing = sidebar_syncing.clone();
        sidebar_list.connect_row_activated(move |_, row| {
            if sidebar_syncing.get() {
                return;
            }
            let idx = row.index();
            if idx >= 0 && idx < tab_view.n_pages() {
                let page = tab_view.nth_page(idx);
                tab_view.set_selected_page(&page);
            }
        });
    }
    {
        let rebuild_sidebar = rebuild_sidebar.clone();
        tab_view.connect_page_attached(move |_, _, _| rebuild_sidebar());
    }
    {
        let rebuild_sidebar = rebuild_sidebar.clone();
        tab_view.connect_page_detached(move |_, _, _| rebuild_sidebar());
    }
    {
        let rebuild_sidebar = rebuild_sidebar.clone();
        tab_view.connect_page_reordered(move |_, _, _| rebuild_sidebar());
    }

    // Switch between top/bottom tab bar, sidebar tab list or hidden tabs.
    // The sidebar auto-hides with a single tab unless `sidebar_always` is set.
    let set_tabs_location = {
        let split_view = split_view.clone();
        let tab_bar = tab_bar.clone();
        let header = header.clone();
        let window_title = window_title.clone();
        let rebuild_sidebar = rebuild_sidebar.clone();
        let tab_view = tab_view.clone();
        let config = config.clone();
        let bottom_tabs = bottom_tabs.clone();
        Rc::new(move |location: TabsLocation| {
            let sidebar_mode = matches!(location, TabsLocation::Left | TabsLocation::Right);
            let show_sidebar =
                sidebar_mode && (config.borrow().sidebar_always || tab_view.n_pages() > 1);
            // When the sidebar is visible the whole header moves into it
            // (system window controls, new tab, palette, menu).
            header.set_visible(!show_sidebar);
            // Detach the tab bar from whichever parent currently holds it.
            if let Some(parent) = tab_bar.parent() {
                if let Some(bx) = parent.downcast_ref::<gtk4::Box>() {
                    bx.remove(&tab_bar);
                } else if let Some(hb) = parent.downcast_ref::<adw::HeaderBar>() {
                    // Clearing the title widget releases the tab bar.
                    if hb.title_widget().as_ref() == Some(tab_bar.upcast_ref()) {
                        hb.set_title_widget(gtk4::Widget::NONE);
                    }
                }
            }
            bottom_tabs.set_visible(false);
            match location {
                TabsLocation::Top => {
                    split_view.set_show_sidebar(false);
                    tab_bar.set_visible(true);
                    header.set_title_widget(Some(&tab_bar));
                }
                TabsLocation::Bottom => {
                    split_view.set_show_sidebar(false);
                    tab_bar.set_visible(true);
                    header.set_title_widget(Some(&window_title));
                    bottom_tabs.append(&tab_bar);
                    bottom_tabs.set_visible(true);
                }
                TabsLocation::Hidden => {
                    split_view.set_show_sidebar(false);
                    tab_bar.set_visible(false);
                    header.set_title_widget(Some(&window_title));
                }
                TabsLocation::Left | TabsLocation::Right => {
                    tab_bar.set_visible(false);
                    header.set_title_widget(Some(&window_title));
                    split_view.set_sidebar_position(if location == TabsLocation::Left {
                        gtk4::PackType::Start
                    } else {
                        gtk4::PackType::End
                    });
                    rebuild_sidebar();
                    split_view.set_show_sidebar(show_sidebar);
                }
            }
        })
    };

    // Re-evaluate sidebar visibility whenever the tab count changes.
    let refresh_tabs = {
        let config = config.clone();
        let set_tabs_location = set_tabs_location.clone();
        Rc::new(move || set_tabs_location(config.borrow().tabs_location))
    };
    {
        let refresh_tabs = refresh_tabs.clone();
        tab_view.connect_page_attached(move |_, _, _| refresh_tabs());
    }
    {
        let refresh_tabs = refresh_tabs.clone();
        tab_view.connect_page_detached(move |_, _, _| refresh_tabs());
    }

    let toast: Toast = {
        let overlay = toast_overlay.clone();
        Rc::new(move |msg: &str| {
            overlay.add_toast(adw::Toast::builder().title(msg).timeout(1).build());
        })
    };

    // Resize indicator (`cols × rows` overlay), deduplicated:
    // dragging a divider updates one toast instead of stacking dozens.
    let resize_toast: Rc<RefCell<Option<adw::Toast>>> = Rc::new(RefCell::new(None));
    let show_resize = {
        let overlay = toast_overlay.clone();
        let resize_toast = resize_toast.clone();
        let last_grid = last_grid.clone();
        let sync_quick = sync_quick.clone();
        Rc::new(move |cols: u16, rows: u16| {
            last_grid.set((cols, rows));
            sync_quick();
            let title = format!("{cols} × {rows}");
            if let Some(t) = resize_toast.borrow().as_ref() {
                // Reuse the visible toast if it is still alive.
                t.set_title(&title);
                return;
            }
            let t = adw::Toast::builder().title(&title).timeout(1).build();
            {
                let resize_toast = resize_toast.clone();
                t.connect_dismissed(move |_| {
                    *resize_toast.borrow_mut() = None;
                });
            }
            *resize_toast.borrow_mut() = Some(t.clone());
            overlay.add_toast(t);
        })
    };

    // Keep strong refs to TerminalViews keyed by page. A page may hold
    // multiple terminals when split.
    let pages: Pages = Rc::new(RefCell::new(Vec::new()));
    let focused: Focused = Rc::new(RefCell::new(None));

    // --- Split zoom  (toggle split zoom) ---
    // Zooming hides every sibling pane so the focused split fills the tab.
    let zoom_hidden: Rc<RefCell<Vec<glib::WeakRef<gtk4::Widget>>>> =
        Rc::new(RefCell::new(Vec::new()));
    let unzoom = {
        let zoom_hidden = zoom_hidden.clone();
        Rc::new(move || {
            for weak in zoom_hidden.borrow_mut().drain(..) {
                if let Some(w) = weak.upgrade() {
                    w.set_visible(true);
                }
            }
        })
    };

    // Build a TerminalView wired to a page (title/exit/focus/context menu).
    let make_view: MakeViewFn = {
        let unzoom = unzoom.clone();
        let toast = toast.clone();
        let show_resize = show_resize.clone();
        let tab_view = tab_view.clone();
        let config = config.clone();
        let pages = pages.clone();
        let window = window.clone();
        let focused = focused.clone();
        Rc::new(
            move |page_slot: Rc<RefCell<Option<adw::TabPage>>>,
                  cwd: Option<PathBuf>,
                  command: Option<Vec<String>>|
                  -> anyhow::Result<Rc<TerminalView>> {
                let cfg = config.borrow().clone();
                let view = Rc::new(TerminalView::new(cfg, cwd, command)?);
                attach_context_menu(&view);

                {
                    // Ctrl+click or Shift+click on a hyperlink / URL / path.
                    let toast = toast.clone();
                    view.set_on_link(move |uri| {
                        match gio::AppInfo::launch_default_for_uri(
                            &uri,
                            gio::AppLaunchContext::NONE,
                        ) {
                            Ok(()) => toast(&format!("Opening {uri}")),
                            Err(err) => {
                                tracing::warn!("could not open {uri}: {err}");
                                toast("No application to open that link");
                            }
                        }
                    });
                }

                {
                    let view_weak = Rc::downgrade(&view);
                    let focused = focused.clone();
                    view.set_on_focus(move || {
                        *focused.borrow_mut() = Some(view_weak.clone());
                    });
                }

                {
                    // Show `cols × rows` while resizing, but only for the
                    // focused pane so splits don't double-report.
                    let view_weak = Rc::downgrade(&view);
                    let focused = focused.clone();
                    let show_resize = show_resize.clone();
                    view.set_on_resize(move |cols, rows| {
                        let is_focused = focused
                            .borrow()
                            .as_ref()
                            .and_then(Weak::upgrade)
                            .zip(view_weak.upgrade())
                            .map(|(f, v)| Rc::ptr_eq(&f, &v))
                            .unwrap_or(false);
                        if is_focused {
                            show_resize(cols, rows);
                        }
                    });
                }

                {
                    let page_slot = page_slot.clone();
                    let window = window.clone();
                    let tab_view = tab_view.clone();
                    view.set_on_title_changed(move |title| {
                        if let Some(page) = page_slot.borrow().as_ref() {
                            // A hand-renamed tab keeps its title.
                            if tab_is_renamed(page) {
                                return;
                            }
                            page.set_title(&title);
                            if tab_view.selected_page().as_ref() == Some(page) {
                                window.set_title(Some(&title));
                            }
                        }
                    });
                }

                {
                    let page_slot = page_slot.clone();
                    let tab_view = tab_view.clone();
                    let pages = pages.clone();
                    let view_weak = Rc::downgrade(&view);
                    let unzoom = unzoom.clone();
                    view.set_on_exit(move || {
                        unzoom();
                        let Some(page) = page_slot.borrow().clone() else {
                            return;
                        };
                        let Some(view) = view_weak.upgrade() else {
                            return;
                        };
                        let remaining = {
                            let mut pgs = pages.borrow_mut();
                            if let Some((_, views)) = pgs.iter_mut().find(|(p, _)| p == &page) {
                                views.retain(|v| !Rc::ptr_eq(v, &view));
                                views.first().cloned()
                            } else {
                                None
                            }
                        };
                        match remaining {
                            Some(next) => {
                                collapse_split(view.widget().upcast_ref());
                                next.focus();
                            }
                            None => {
                                tab_view.close_page(&page);
                                pages.borrow_mut().retain(|(p, _)| p != &page);
                            }
                        }
                    });
                }

                Ok(view)
            },
        )
    };

    let add_tab = {
        let tab_view = tab_view.clone();
        let pages = pages.clone();
        let make_view = make_view.clone();
        let config = config.clone();
        Rc::new(
            move |launch: LaunchRequest| -> anyhow::Result<adw::TabPage> {
                let page_slot: Rc<RefCell<Option<adw::TabPage>>> = Rc::new(RefCell::new(None));
                let view = make_view(page_slot.clone(), launch.cwd, launch.command)?;

                let root = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
                root.set_hexpand(true);
                root.set_vexpand(true);
                root.append(view.widget());

                let page = match config.borrow().new_tab_position {
                    NewTabPosition::End => tab_view.append(&root),
                    NewTabPosition::Start => tab_view.insert(&root, 0),
                    // Relative positions need the current tab; with none
                    // selected there is nothing to be relative to.
                    NewTabPosition::AfterCurrent => match tab_view.selected_page() {
                        Some(current) => {
                            tab_view.insert(&root, tab_view.page_position(&current) + 1)
                        }
                        None => tab_view.append(&root),
                    },
                    NewTabPosition::BeforeCurrent => match tab_view.selected_page() {
                        Some(current) => tab_view.insert(&root, tab_view.page_position(&current)),
                        None => tab_view.append(&root),
                    },
                };
                page.set_title("Terminal");
                page.set_live_thumbnail(true);
                *page_slot.borrow_mut() = Some(page.clone());

                pages.borrow_mut().push((page.clone(), vec![view.clone()]));
                tab_view.set_selected_page(&page);
                view.focus();
                Ok(page)
            },
        )
    };

    let add_browser_tab = {
        let tab_view = tab_view.clone();
        Rc::new(move || {
            let root = browser::new_tab();
            let page = tab_view.append(&root);
            set_browser_tab(&page);
            page.set_title("Browser");
            page.set_live_thumbnail(true);
            tab_view.set_selected_page(&page);
        })
    };

    // Restore a tab from a nested split tree.
    let add_tab_layout = {
        let tab_view = tab_view.clone();
        let pages = pages.clone();
        let make_view = make_view.clone();
        let config = config.clone();
        Rc::new(
            move |title: Option<String>, layout: &PaneLayout| -> anyhow::Result<adw::TabPage> {
                let page_slot: Rc<RefCell<Option<adw::TabPage>>> = Rc::new(RefCell::new(None));
                let (child, views) = build_layout_widget(layout, &make_view, &page_slot)?;

                let root = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
                root.set_hexpand(true);
                root.set_vexpand(true);
                root.append(&child);

                let page = match config.borrow().new_tab_position {
                    NewTabPosition::End => tab_view.append(&root),
                    NewTabPosition::Start => tab_view.insert(&root, 0),
                    NewTabPosition::AfterCurrent => match tab_view.selected_page() {
                        Some(current) => {
                            tab_view.insert(&root, tab_view.page_position(&current) + 1)
                        }
                        None => tab_view.append(&root),
                    },
                    NewTabPosition::BeforeCurrent => match tab_view.selected_page() {
                        Some(current) => tab_view.insert(&root, tab_view.page_position(&current)),
                        None => tab_view.append(&root),
                    },
                };
                if let Some(title) = title {
                    set_tab_renamed(&page, true);
                    page.set_title(&title);
                } else {
                    page.set_title("Terminal");
                }
                page.set_live_thumbnail(true);
                *page_slot.borrow_mut() = Some(page.clone());

                pages.borrow_mut().push((page.clone(), views.clone()));
                tab_view.set_selected_page(&page);
                if let Some(view) = views.first() {
                    view.focus();
                }
                Ok(page)
            },
        )
    };

    // Currently focused terminal of the selected page.
    let current_view = {
        let tab_view = tab_view.clone();
        let pages = pages.clone();
        let focused = focused.clone();
        Rc::new(move || -> Option<Rc<TerminalView>> {
            let page = tab_view.selected_page()?;
            let pgs = pages.borrow();
            let (_, views) = pgs.iter().find(|(p, _)| p == &page)?;
            if let Some(f) = focused.borrow().as_ref().and_then(Weak::upgrade)
                && views.iter().any(|v| Rc::ptr_eq(v, &f))
            {
                return Some(f);
            }
            views.first().cloned()
        })
    };

    // Directory a new tab or split should start in. `None` means "wherever the
    // shell would start anyway", which is also what the config key turns this
    // into when inheritance is off.
    let inherit_cwd: Rc<dyn Fn() -> Option<PathBuf>> = {
        let current_view = current_view.clone();
        let config = config.clone();
        Rc::new(move || {
            if !config.borrow().inherit_working_directory {
                return None;
            }
            current_view()?.pwd().map(PathBuf::from)
        })
    };

    let search_bar = Rc::new(SearchBar::new({
        let current_view = current_view.clone();
        Rc::new(move || current_view())
    }));
    content_box.prepend(&search_bar.widget);
    // Deliberately no `set_key_capture_widget`: in a terminal every keystroke
    // belongs to the shell, so the bar only opens via its explicit action.

    // Split the focused pane. `before` puts the new terminal on the
    // left/top side; splits nest arbitrarily (Paned inside Paned).
    let split = {
        let tab_view = tab_view.clone();
        let pages = pages.clone();
        let make_view = make_view.clone();
        let current_view = current_view.clone();
        let inherit_cwd = inherit_cwd.clone();
        let unzoom = unzoom.clone();
        Rc::new(move |orientation: gtk4::Orientation, before: bool| {
            unzoom();
            let Some(page) = tab_view.selected_page() else {
                return;
            };
            if is_browser_tab(&page) {
                return;
            }
            let Some(target) = current_view() else { return };
            let page_slot = Rc::new(RefCell::new(Some(page.clone())));
            let cwd = inherit_cwd();
            let Ok(new_view) = make_view(page_slot, cwd, None) else {
                return;
            };

            let old = target.widget().clone().upcast::<gtk4::Widget>();
            // Start with an even 50/50 split for a predictable layout.
            let half = match orientation {
                gtk4::Orientation::Horizontal => old.width(),
                _ => old.height(),
            } / 2;
            let paned = gtk4::Paned::new(orientation);
            paned.set_hexpand(true);
            paned.set_vexpand(true);
            paned.set_wide_handle(true);
            paned.set_resize_start_child(true);
            paned.set_resize_end_child(true);
            paned.set_shrink_start_child(false);
            paned.set_shrink_end_child(false);
            replace_in_parent(&old, paned.upcast_ref());
            if before {
                paned.set_start_child(Some(new_view.widget()));
                paned.set_end_child(Some(&old));
            } else {
                paned.set_start_child(Some(&old));
                paned.set_end_child(Some(new_view.widget()));
            }
            if half > 0 {
                paned.set_position(half);
            }

            if let Some((_, views)) = pages.borrow_mut().iter_mut().find(|(p, _)| p == &page) {
                views.push(new_view.clone());
            }
            new_view.focus();
        })
    };

    // Directional focus between splits , based on the
    // on-screen geometry of each pane.
    let goto_split = {
        let tab_view = tab_view.clone();
        let pages = pages.clone();
        let current_view = current_view.clone();
        let unzoom = unzoom.clone();
        Rc::new(move |dir: &str| {
            unzoom();
            let Some(page) = tab_view.selected_page() else {
                return;
            };
            let Some(cur) = current_view() else { return };
            let target: Option<Rc<TerminalView>> = {
                let pgs = pages.borrow();
                let Some((_, views)) = pgs.iter().find(|(p, _)| p == &page) else {
                    return;
                };
                if views.len() < 2 {
                    return;
                }
                match dir {
                    "previous" | "next" => {
                        let idx = views.iter().position(|v| Rc::ptr_eq(v, &cur)).unwrap_or(0);
                        let n = views.len();
                        let t = if dir == "next" {
                            (idx + 1) % n
                        } else {
                            (idx + n - 1) % n
                        };
                        views.get(t).cloned()
                    }
                    _ => {
                        let root = page.child();
                        let Some(cb) = cur.widget().compute_bounds(&root) else {
                            return;
                        };
                        let (cx, cy) = (cb.x() + cb.width() / 2.0, cb.y() + cb.height() / 2.0);
                        let mut best: Option<(f32, Rc<TerminalView>)> = None;
                        for v in views {
                            if Rc::ptr_eq(v, &cur) {
                                continue;
                            }
                            let Some(b) = v.widget().compute_bounds(&root) else {
                                continue;
                            };
                            let (dx, dy) =
                                (b.x() + b.width() / 2.0 - cx, b.y() + b.height() / 2.0 - cy);
                            let matches_dir = match dir {
                                "left" => dx < -1.0,
                                "right" => dx > 1.0,
                                "up" => dy < -1.0,
                                _ => dy > 1.0,
                            };
                            if !matches_dir {
                                continue;
                            }
                            let dist = dx * dx + dy * dy;
                            if best.as_ref().is_none_or(|(d, _)| dist < *d) {
                                best = Some((dist, v.clone()));
                            }
                        }
                        best.map(|(_, v)| v)
                    }
                }
            };
            if let Some(v) = target {
                v.focus();
            }
        })
    };

    // Move the nearest matching divider by 10px .
    let resize_split = {
        let current_view = current_view.clone();
        Rc::new(move |dir: &str| {
            let Some(cur) = current_view() else { return };
            let horizontal = matches!(dir, "left" | "right");
            let mut widget: gtk4::Widget = cur.widget().clone().upcast();
            while let Some(parent) = widget.parent() {
                if let Some(paned) = parent.downcast_ref::<gtk4::Paned>() {
                    let is_horizontal = paned.orientation() == gtk4::Orientation::Horizontal;
                    if is_horizontal == horizontal {
                        let delta = if matches!(dir, "left" | "up") {
                            -10
                        } else {
                            10
                        };
                        paned.set_position(paned.position() + delta);
                        return;
                    }
                }
                widget = parent;
            }
        })
    };

    let toggle_split_zoom = {
        let zoom_hidden = zoom_hidden.clone();
        let unzoom = unzoom.clone();
        let current_view = current_view.clone();
        Rc::new(move || {
            if !zoom_hidden.borrow().is_empty() {
                unzoom();
                return;
            }
            let Some(cur) = current_view() else { return };
            let mut widget: gtk4::Widget = cur.widget().clone().upcast();
            let mut hidden = Vec::new();
            while let Some(parent) = widget.parent() {
                if let Some(paned) = parent.downcast_ref::<gtk4::Paned>() {
                    let sibling = if paned.start_child().as_ref() == Some(&widget) {
                        paned.end_child()
                    } else {
                        paned.start_child()
                    };
                    if let Some(sibling) = sibling {
                        sibling.set_visible(false);
                        let weak = glib::WeakRef::new();
                        weak.set(Some(&sibling));
                        hidden.push(weak);
                    }
                }
                widget = parent;
                if widget.downcast_ref::<gtk4::Box>().is_some() {
                    break;
                }
            }
            if !hidden.is_empty() {
                *zoom_hidden.borrow_mut() = hidden;
                cur.focus();
            }
        })
    };

    // --- Actions / shortcuts ---
    let add_simple = |name: &str, f: Box<dyn Fn()>| {
        let action = gio::SimpleAction::new(name, None);
        action.connect_activate(move |_, _| f());
        action
    };

    // Right-click on a tab: rename / split / close.
    tab_view.set_menu_model(Some(&tab_menu()));

    // Middle click on a tab, per config.
    {
        let config = config.clone();
        let tab_view = tab_view.clone();
        let tab_bar_c = tab_bar.clone();
        let add_tab = add_tab.clone();
        let inherit_cwd = inherit_cwd.clone();
        let middle = gtk4::GestureClick::new();
        middle.set_button(gdk::BUTTON_MIDDLE);
        // AdwTabBar closes the tab on middle click by itself. Without running
        // ahead of it and claiming the sequence, "New Tab" opened a tab *and*
        // let libadwaita close the one under the pointer, and "Nothing" still
        // closed it. Capturing makes the setting authoritative for all three.
        middle.set_propagation_phase(gtk4::PropagationPhase::Capture);
        middle.connect_pressed(move |gesture, _, x, y| {
            gesture.set_state(gtk4::EventSequenceState::Claimed);
            let action = config.borrow().middle_click_tab;
            match action {
                MiddleClickTab::NewTab => {
                    let launch = LaunchRequest {
                        cwd: inherit_cwd(),
                        command: None,
                    };
                    if let Err(err) = add_tab(launch) {
                        tracing::error!("middle-click new tab failed: {err:#}");
                    }
                }
                MiddleClickTab::CloseTab => {
                    // Only close the tab actually under the pointer; acting on
                    // the selected one instead would close the wrong tab.
                    if let Some(page) = tab_page_at(&tab_bar_c, x, y) {
                        tab_view.close_page(&page);
                    }
                }
                MiddleClickTab::Ignore => {}
            }
        });
        tab_bar.add_controller(middle);
    }

    {
        // The overview has its own `+`; without this it would do nothing.
        let add_tab = add_tab.clone();
        let inherit_cwd = inherit_cwd.clone();
        let tab_view = tab_view.clone();
        tab_overview.connect_create_tab(move |_| {
            let launch = LaunchRequest {
                cwd: inherit_cwd(),
                command: None,
            };
            match add_tab(launch) {
                Ok(page) => page,
                Err(err) => {
                    // The signal has to return a page, and a TabPage cannot be
                    // built standalone, so hand back an empty one.
                    tracing::error!("new tab from overview failed: {err:#}");
                    tab_view.append(&gtk4::Box::new(gtk4::Orientation::Vertical, 0))
                }
            }
        });
    }

    {
        let tab_overview = tab_overview.clone();
        window.add_action(&add_simple(
            "tab-overview",
            Box::new(move || tab_overview.set_open(!tab_overview.is_open())),
        ));
    }

    {
        let add_tab = add_tab.clone();
        let inherit_cwd = inherit_cwd.clone();
        window.add_action(&add_simple(
            "new-tab",
            Box::new(move || {
                let launch = LaunchRequest {
                    cwd: inherit_cwd(),
                    command: None,
                };
                if let Err(err) = add_tab(launch) {
                    tracing::error!("new tab failed: {err:#}");
                }
            }),
        ));
    }

    {
        let tab_view = tab_view.clone();
        let pages = pages.clone();
        let window_c = window.clone();
        window.add_action(&add_simple(
            "close-tab",
            Box::new(move || {
                if let Some(page) = tab_view.selected_page() {
                    tab_view.close_page(&page);
                    pages.borrow_mut().retain(|(p, _)| p != &page);
                    if tab_view.n_pages() == 0 {
                        window_c.close();
                    }
                }
            }),
        ));
    }

    {
        let tab_view = tab_view.clone();
        window.add_action(&add_simple(
            "next-tab",
            Box::new(move || cycle_tab(&tab_view, 1)),
        ));
    }
    {
        let tab_view = tab_view.clone();
        window.add_action(&add_simple(
            "prev-tab",
            Box::new(move || cycle_tab(&tab_view, -1)),
        ));
    }

    {
        let split = split.clone();
        window.add_action(&add_simple(
            "split-right",
            Box::new(move || split(gtk4::Orientation::Horizontal, false)),
        ));
    }
    {
        let split = split.clone();
        window.add_action(&add_simple(
            "split-down",
            Box::new(move || split(gtk4::Orientation::Vertical, false)),
        ));
    }
    {
        let split = split.clone();
        window.add_action(&add_simple(
            "split-left",
            Box::new(move || split(gtk4::Orientation::Horizontal, true)),
        ));
    }
    {
        let split = split.clone();
        window.add_action(&add_simple(
            "split-up",
            Box::new(move || split(gtk4::Orientation::Vertical, true)),
        ));
    }

    for dir in ["left", "right", "up", "down", "previous", "next"] {
        let goto_split = goto_split.clone();
        window.add_action(&add_simple(
            &format!("focus-split-{dir}"),
            Box::new(move || goto_split(dir)),
        ));
    }
    for dir in ["left", "right", "up", "down"] {
        let resize_split = resize_split.clone();
        window.add_action(&add_simple(
            &format!("resize-split-{dir}"),
            Box::new(move || resize_split(dir)),
        ));
    }
    {
        let toggle_split_zoom = toggle_split_zoom.clone();
        window.add_action(&add_simple(
            "toggle-split-zoom",
            Box::new(move || toggle_split_zoom()),
        ));
    }
    {
        let tab_view = tab_view.clone();
        window.add_action(&add_simple(
            "equalize-splits",
            Box::new(move || {
                if let Some(page) = tab_view.selected_page()
                    && let Some(root) = page.child().downcast_ref::<gtk4::Box>()
                    && let Some(first) = root.first_child()
                {
                    equalize_splits(&first);
                }
            }),
        ));
    }

    {
        let window_c = window.clone();
        window.add_action(&add_simple("quit", Box::new(move || window_c.close())));
    }

    {
        let current_view = current_view.clone();
        let toast = toast.clone();
        window.add_action(&add_simple(
            "copy",
            Box::new(move || {
                let Some(view) = current_view() else { return };
                match view.selection_text() {
                    Some(text) => {
                        if let Some(display) = gdk::Display::default() {
                            display.clipboard().set_text(&text);
                            toast("Copied");
                        }
                    }
                    None => toast("Nothing selected"),
                }
            }),
        ));
    }

    {
        let current_view = current_view.clone();
        window.add_action(&add_simple(
            "paste",
            Box::new(move || {
                let Some(view) = current_view() else { return };
                let Some(display) = gdk::Display::default() else {
                    return;
                };
                display
                    .clipboard()
                    .read_text_async(gio::Cancellable::NONE, move |res| {
                        if let Ok(Some(text)) = res {
                            view.paste(&text);
                        }
                    });
            }),
        ));
    }

    {
        let current_view = current_view.clone();
        window.add_action(&add_simple(
            "select-all",
            Box::new(move || {
                if let Some(view) = current_view() {
                    view.select_all();
                }
            }),
        ));
    }

    {
        let current_view = current_view.clone();
        let toast = toast.clone();
        window.add_action(&add_simple(
            "clear-tab",
            Box::new(move || {
                if let Some(view) = current_view() {
                    view.clear_screen();
                    toast("Screen cleared");
                }
            }),
        ));
    }

    {
        let current_view = current_view.clone();
        let toast = toast.clone();
        window.add_action(&add_simple(
            "restart-tab",
            Box::new(move || {
                if let Some(view) = current_view() {
                    view.restart();
                    view.focus();
                    toast("Terminal restarted");
                }
            }),
        ));
    }

    let apply_zoom = {
        let pages = pages.clone();
        let config = config.clone();
        let toast = toast.clone();
        let save_config = save_config.clone();
        let sync_quick = sync_quick.clone();
        Rc::new(move |size: f32| {
            let mut applied = size;
            for (_, views) in pages.borrow().iter() {
                for view in views {
                    applied = view.set_font_size(size);
                }
            }
            config.borrow_mut().font_size = applied;
            save_config();
            sync_quick();
            toast(&format!("Font: {applied:.0} pt"));
        })
    };

    {
        let config = config.clone();
        let apply_zoom = apply_zoom.clone();
        window.add_action(&add_simple(
            "zoom-in",
            Box::new(move || apply_zoom(config.borrow().font_size + 1.0)),
        ));
    }
    {
        let config = config.clone();
        let apply_zoom = apply_zoom.clone();
        window.add_action(&add_simple(
            "zoom-out",
            Box::new(move || apply_zoom(config.borrow().font_size - 1.0)),
        ));
    }
    {
        let base_font_size = base_font_size.clone();
        let apply_zoom = apply_zoom.clone();
        window.add_action(&add_simple(
            "zoom-reset",
            Box::new(move || apply_zoom(base_font_size.get())),
        ));
    }

    // Re-read config.toml and push it into every live pane. Shared by the
    // explicit action and the file-monitor auto-reload.
    let reload_config: Rc<dyn Fn(bool)> = {
        let config = config.clone();
        let pages = pages.clone();
        let toast = toast.clone();
        let base_font_size = base_font_size.clone();
        let set_tabs_location = set_tabs_location.clone();
        let window_c = window.clone();
        Rc::new(move |notify: bool| match Config::load() {
            Ok(new_cfg) => {
                base_font_size.set(new_cfg.font_size);
                *config.borrow_mut() = new_cfg.clone();
                for (_, views) in pages.borrow().iter() {
                    for view in views {
                        view.apply_config(&new_cfg);
                    }
                }
                apply_theme(new_cfg.theme);
                apply_window_opacity(&window_c, new_cfg.background_opacity);
                set_tabs_location(new_cfg.tabs_location);
                tracing::info!(
                    "configuration reloaded (font={} size={})",
                    new_cfg.font_family,
                    new_cfg.font_size
                );
                if notify {
                    toast("Configuration reloaded");
                }
            }
            Err(err) => {
                tracing::error!("config reload failed: {err:#}");
                if notify {
                    toast("Failed to reload configuration");
                }
            }
        })
    };

    {
        let reload_config = reload_config.clone();
        window.add_action(&add_simple(
            "reload-config",
            Box::new(move || reload_config(true)),
        ));
    }

    // Watch config.toml and pick up external edits automatically, like
    // Editors write via rename/replace, so CHANGED alone is not
    // enough — react to created/renamed too, and debounce the burst.
    {
        let path = config.borrow().source.clone();
        let file = gio::File::for_path(&path);
        match file.monitor_file(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE) {
            Ok(monitor) => {
                let reload_config = reload_config.clone();
                let toast = toast.clone();
                let self_write = self_write.clone();
                let pending = Rc::new(Cell::new(false));
                monitor.connect_changed(move |_, _, _, event| {
                    use gio::FileMonitorEvent as Ev;
                    if !matches!(
                        event,
                        Ev::ChangesDoneHint | Ev::Created | Ev::MovedIn | Ev::Renamed
                    ) {
                        return;
                    }
                    // Ignore the write we just made from the UI.
                    if self_write.get().elapsed() < SELF_WRITE_GRACE {
                        return;
                    }
                    if pending.replace(true) {
                        return;
                    }
                    let reload_config = reload_config.clone();
                    let toast = toast.clone();
                    let pending = pending.clone();
                    glib::timeout_add_local_once(
                        std::time::Duration::from_millis(150),
                        move || {
                            pending.set(false);
                            reload_config(false);
                            toast("Configuration reloaded");
                        },
                    );
                });
                // The monitor must outlive this scope to keep firing.
                std::mem::forget(monitor);
                tracing::info!("watching {} for changes", path.display());
            }
            Err(err) => tracing::warn!("could not watch {}: {err}", path.display()),
        }
    }

    // The header and the sidebar each have their own palette button.
    let set_search_visible: Rc<dyn Fn(bool)> = {
        let palette_btn = palette_btn.clone();
        let sidebar_palette_btn = sidebar_palette_btn.clone();
        Rc::new(move |visible| {
            palette_btn.set_visible(visible);
            sidebar_palette_btn.set_visible(visible);
        })
    };

    {
        let window_c = window.clone();
        let config = config.clone();
        let pages = pages.clone();
        let apply_zoom = apply_zoom.clone();
        let set_tabs_location = set_tabs_location.clone();
        let apply_tab_shape = apply_tab_shape.clone();
        let set_search_visible = set_search_visible.clone();
        let save_config = save_config.clone();
        let bindings = bindings.clone();
        let apply_bindings = apply_bindings.clone();
        window.add_action(&add_simple(
            "preferences",
            Box::new(move || {
                show_preferences(
                    &window_c,
                    &config,
                    &pages,
                    PrefsHooks {
                        apply_zoom: apply_zoom.clone(),
                        set_tabs_location: set_tabs_location.clone(),
                        apply_tab_shape: apply_tab_shape.clone(),
                        set_search_visible: set_search_visible.clone(),
                        save_config: save_config.clone(),
                        bindings: bindings.clone(),
                        apply_bindings: apply_bindings.clone(),
                    },
                );
            }),
        ));
    }

    {
        let window_c = window.clone();
        let config_c = config.clone();
        let add_tab_c = add_tab.clone();
        window.add_action(&add_simple(
            "command-palette",
            Box::new(move || {
                let open_launch: Rc<dyn Fn(LaunchRequest)> = {
                    let add_tab = add_tab_c.clone();
                    Rc::new(move |req| {
                        if let Err(err) = add_tab(req) {
                            tracing::error!("palette launch failed: {err:#}");
                        }
                    })
                };
                show_command_palette(&window_c, &config_c, open_launch);
            }),
        ));
    }

    {
        let search_bar = search_bar.clone();
        window.add_action(&add_simple("find", Box::new(move || search_bar.open())));
    }

    {
        let add_browser_tab = add_browser_tab.clone();
        window.add_action(&add_simple(
            "open-browser",
            Box::new(move || add_browser_tab()),
        ));
    }

    {
        let tab_view = tab_view.clone();
        let window_c = window.clone();
        window.add_action(&add_simple(
            "rename-tab",
            Box::new(move || {
                if let Some(page) = tab_view.selected_page() {
                    rename_tab_dialog(&window_c, &page);
                }
            }),
        ));
    }

    {
        let window_c = window.clone();
        window.add_action(&add_simple(
            "about",
            Box::new(move || show_about(&window_c)),
        ));
    }

    {
        let window_c = window.clone();
        window.add_action(&add_simple(
            "shortcuts",
            Box::new(move || show_shortcuts(&window_c)),
        ));
    }

    // Apply a config mutation to every live terminal (used by menu radios).
    let update_all_cfg = {
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

    // --- Stateful menu actions (radios/toggles) ---
    let theme_action = gio::SimpleAction::new_stateful(
        "theme",
        Some(glib::VariantTy::STRING),
        &config.borrow().theme.as_str().to_variant(),
    );
    {
        let config = config.clone();
        let save_config = save_config.clone();
        let sync_quick = sync_quick.clone();
        theme_action.connect_activate(move |action, param| {
            let value = param.and_then(|p| p.str()).unwrap_or("system");
            let theme = Theme::parse(value);
            apply_theme(theme);
            config.borrow_mut().theme = theme;
            save_config();
            sync_quick();
            action.set_state(&value.to_variant());
        });
    }
    window.add_action(&theme_action);

    let tabs_pos_initial = match config.borrow().tabs_location {
        TabsLocation::Top => "top",
        TabsLocation::Bottom => "bottom",
        TabsLocation::Left => "left",
        TabsLocation::Right => "right",
        TabsLocation::Hidden => "hidden",
    };
    let tabs_pos_action = gio::SimpleAction::new_stateful(
        "tabs-pos",
        Some(glib::VariantTy::STRING),
        &tabs_pos_initial.to_variant(),
    );
    {
        let config = config.clone();
        let set_tabs_location = set_tabs_location.clone();
        let save_config = save_config.clone();
        tabs_pos_action.connect_activate(move |action, param| {
            let value = param.and_then(|p| p.str()).unwrap_or("top");
            let location = match value {
                "bottom" => TabsLocation::Bottom,
                "left" => TabsLocation::Left,
                "right" => TabsLocation::Right,
                "hidden" => TabsLocation::Hidden,
                _ => TabsLocation::Top,
            };
            config.borrow_mut().tabs_location = location;
            set_tabs_location(location);
            save_config();
            action.set_state(&value.to_variant());
        });
    }
    window.add_action(&tabs_pos_action);

    let cursor_initial = match config.borrow().cursor_style {
        CursorStyle::Bar => "bar",
        CursorStyle::Underline => "underline",
        _ => "block",
    };
    let cursor_action = gio::SimpleAction::new_stateful(
        "cursor-shape",
        Some(glib::VariantTy::STRING),
        &cursor_initial.to_variant(),
    );
    {
        let update_all_cfg = update_all_cfg.clone();
        cursor_action.connect_activate(move |action, param| {
            let value = param.and_then(|p| p.str()).unwrap_or("block");
            let style = match value {
                "bar" => CursorStyle::Bar,
                "underline" => CursorStyle::Underline,
                _ => CursorStyle::Block,
            };
            update_all_cfg(Rc::new(move |cfg| cfg.cursor_style = style));
            action.set_state(&value.to_variant());
        });
    }
    window.add_action(&cursor_action);

    let blink_action = gio::SimpleAction::new_stateful(
        "cursor-blink",
        None,
        &config.borrow().cursor_blink.to_variant(),
    );
    {
        let update_all_cfg = update_all_cfg.clone();
        blink_action.connect_activate(move |action, _| {
            let value = !action.state().and_then(|s| s.get::<bool>()).unwrap_or(true);
            update_all_cfg(Rc::new(move |cfg| cfg.cursor_blink = value));
            action.set_state(&value.to_variant());
        });
    }
    window.add_action(&blink_action);

    let sidebar_always_action = gio::SimpleAction::new_stateful(
        "sidebar-always",
        None,
        &config.borrow().sidebar_always.to_variant(),
    );
    {
        let config = config.clone();
        let refresh_tabs = refresh_tabs.clone();
        let save_config = save_config.clone();
        sidebar_always_action.connect_activate(move |action, _| {
            let value = !action
                .state()
                .and_then(|s| s.get::<bool>())
                .unwrap_or(false);
            config.borrow_mut().sidebar_always = value;
            refresh_tabs();
            save_config();
            action.set_state(&value.to_variant());
        });
    }
    window.add_action(&sidebar_always_action);

    app.set_accels_for_action("win.tab-overview", &["F1", "<Super>Tab"]);
    app.set_accels_for_action("win.new-tab", &["<Control><Shift>t"]);
    app.set_accels_for_action("win.close-tab", &["<Control><Shift>w"]);
    app.set_accels_for_action("win.next-tab", &["<Control>Page_Down", "<Control>Tab"]);
    app.set_accels_for_action("win.prev-tab", &["<Control>Page_Up", "<Control><Shift>Tab"]);
    app.set_accels_for_action("win.quit", &["<Control><Shift>q"]);
    app.set_accels_for_action("win.copy", &["<Control><Shift>c"]);
    app.set_accels_for_action("win.paste", &["<Control><Shift>v"]);
    app.set_accels_for_action("win.select-all", &["<Control><Shift>a"]);
    app.set_accels_for_action("win.clear-tab", &["<Control><Shift>k"]);
    app.set_accels_for_action("win.restart-tab", &["<Control><Shift>r"]);
    // Split bindings for tiling panes.
    app.set_accels_for_action("win.split-right", &["<Control><Shift>o"]);
    app.set_accels_for_action("win.split-down", &["<Control><Shift>e"]);
    app.set_accels_for_action("win.split-left", &["<Control><Shift>l"]);
    app.set_accels_for_action("win.split-up", &["<Control><Shift>u"]);
    app.set_accels_for_action("win.focus-split-up", &["<Control><Alt>Up"]);
    app.set_accels_for_action("win.focus-split-down", &["<Control><Alt>Down"]);
    app.set_accels_for_action("win.focus-split-left", &["<Control><Alt>Left"]);
    app.set_accels_for_action("win.focus-split-right", &["<Control><Alt>Right"]);
    app.set_accels_for_action("win.focus-split-previous", &["<Control><Super>bracketleft"]);
    app.set_accels_for_action("win.focus-split-next", &["<Control><Super>bracketright"]);
    app.set_accels_for_action("win.resize-split-up", &["<Control><Shift><Super>Up"]);
    app.set_accels_for_action("win.resize-split-down", &["<Control><Shift><Super>Down"]);
    app.set_accels_for_action("win.resize-split-left", &["<Control><Shift><Super>Left"]);
    app.set_accels_for_action("win.resize-split-right", &["<Control><Shift><Super>Right"]);
    app.set_accels_for_action("win.toggle-split-zoom", &["<Control><Shift>Return"]);
    app.set_accels_for_action("win.command-palette", &["<Control><Shift>p"]);
    app.set_accels_for_action("win.find", &["<Control><Shift>f"]);
    app.set_accels_for_action("win.open-browser", &["<Control><Shift>b"]);
    app.set_accels_for_action("win.rename-tab", &["F2"]);
    app.set_accels_for_action(
        "win.zoom-in",
        &["<Control>plus", "<Control>equal", "<Control>KP_Add"],
    );
    app.set_accels_for_action("win.zoom-out", &["<Control>minus", "<Control>KP_Subtract"]);
    app.set_accels_for_action("win.zoom-reset", &["<Control>0", "<Control>KP_0"]);
    app.set_accels_for_action("win.preferences", &["<Control>comma"]);

    // keys.toml wins over everything set above.
    apply_bindings();

    // Browser tabs keep split and tiling actions unavailable.
    let sync_split_actions: Rc<dyn Fn(Option<adw::TabPage>)> = {
        let window = window.clone();
        Rc::new(move |page| {
            let enabled = page.map(|page| !is_browser_tab(&page)).unwrap_or(true);
            for name in [
                "split-right",
                "split-down",
                "split-left",
                "split-up",
                "toggle-split-zoom",
                "equalize-splits",
                "focus-split-left",
                "focus-split-right",
                "focus-split-up",
                "focus-split-down",
                "focus-split-previous",
                "focus-split-next",
                "resize-split-left",
                "resize-split-right",
                "resize-split-up",
                "resize-split-down",
            ] {
                if let Some(action) = window
                    .lookup_action(name)
                    .and_then(|action| action.downcast::<gio::SimpleAction>().ok())
                {
                    action.set_enabled(enabled);
                }
            }
        })
    };
    sync_split_actions(tab_view.selected_page());

    // Focus terminal when switching tabs; sync window title + sidebar row.
    {
        let current_view = current_view.clone();
        let window = window.clone();
        let sidebar_list = sidebar_list.clone();
        let sidebar_syncing = sidebar_syncing.clone();
        let sync_split_actions = sync_split_actions.clone();
        tab_view.connect_notify_local(Some("selected-page"), move |tv, _| {
            sync_split_actions(tv.selected_page());
            if let Some(page) = tv.selected_page() {
                if let Some(view) = current_view() {
                    window.set_title(Some(&view.title()));
                    view.focus();
                } else {
                    window.set_title(Some(&page.title()));
                }
                sidebar_syncing.set(true);
                let idx = tv.page_position(&page);
                sidebar_list.select_row(sidebar_list.row_at_index(idx).as_ref());
                sidebar_syncing.set(false);
            }
        });
    }

    // Confirm close; drop our page refs.
    {
        let pages = pages.clone();
        let window = window.clone();
        let config = config.clone();
        tab_view.connect_close_page(move |tv, page| {
            // AdwTabView allows an async answer: hold the close, then finish it
            // once the user has decided.
            if config.borrow().confirm_close_tab {
                let dialog = adw::AlertDialog::new(
                    Some("Close this tab?"),
                    Some("Anything still running in it will be terminated."),
                );
                dialog.add_responses(&[("cancel", "Cancel"), ("close", "Close")]);
                dialog.set_response_appearance("close", adw::ResponseAppearance::Destructive);
                dialog.set_default_response(Some("cancel"));
                dialog.set_close_response("cancel");

                let tv = tv.clone();
                let page = page.clone();
                let pages = pages.clone();
                let parent = window.clone();
                let window = window.clone();
                dialog.choose(&parent, gio::Cancellable::NONE, move |response| {
                    let closing = response == "close";
                    if closing {
                        pages.borrow_mut().retain(|(p, _)| p != &page);
                    }
                    tv.close_page_finish(&page, closing);
                    if closing && tv.n_pages() == 0 {
                        window.close();
                    }
                });
                return glib::Propagation::Stop;
            }

            pages.borrow_mut().retain(|(p, _)| p != page);
            // Must call close_page_finish for AdwTabView.
            tv.close_page_finish(page, true);
            if tv.n_pages() == 0 {
                window.close();
            }
            glib::Propagation::Stop
        });
    }

    // Apply font family to chrome labels where useful.
    {
        let css = gtk4::CssProvider::new();
        let cfg = config.borrow();
        let family = cfg.font_family.replace(['\'', '"'], "");
        css.load_from_string(&format!(
            ".terminal {{ font-family: \"{family}\"; font-size: {}pt; }}",
            cfg.font_size
        ));
        if let Some(display) = gdk::Display::default() {
            gtk4::style_context_add_provider_for_display(
                &display,
                &css,
                gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
    }

    // Keep the session awake while any pane has a foreground job, so a long
    // build does not get interrupted by the screen locking.
    {
        let config = config.clone();
        let pages = pages.clone();
        let app = app.clone();
        let window = window.clone();
        // The GTK inhibit cookie; 0 means "not inhibiting".
        let cookie = Rc::new(Cell::new(0u32));
        // Was anything running last time we looked? A busy-to-idle transition
        // is what "the command finished" means without shell integration.
        let was_busy = Rc::new(Cell::new(false));
        glib::timeout_add_seconds_local(4, move || {
            let busy = pages
                .borrow()
                .iter()
                .any(|(_, views)| views.iter().any(|v| v.is_busy()));

            let (keep_awake, notify) = {
                let cfg = config.borrow();
                (cfg.keep_awake, cfg.command_finished_sound)
            };
            // Only when unfocused: otherwise it beeps at someone who is already
            // watching the command finish.
            if notify
                && was_busy.get()
                && !busy
                && !window.is_active()
                && let Some(display) = gdk::Display::default()
            {
                display.beep();
            }
            was_busy.set(busy);

            let wanted = keep_awake && busy;
            let held = cookie.get();
            if wanted && held == 0 {
                cookie.set(app.inhibit(
                    Some(&window),
                    gtk4::ApplicationInhibitFlags::IDLE,
                    Some("a terminal command is running"),
                ));
            } else if !wanted && held != 0 {
                app.uninhibit(held);
                cookie.set(0);
            }
            glib::ControlFlow::Continue
        });
    }

    // Persist the workspace so the next start can restore it.
    let save_session: Rc<dyn Fn()> = {
        let config = config.clone();
        let tab_view = tab_view.clone();
        let pages = pages.clone();
        let window = window.clone();
        // SIGTERM saves and then closes the window, which would otherwise run
        // the same work again through `close-request`.
        let saved = Cell::new(false);
        Rc::new(move || {
            if saved.replace(true) {
                return;
            }
            if !config.borrow().session_restore {
                SessionState::clear();
                return;
            }
            let mut session = capture_session(&tab_view, &pages);
            session.width = Some(window.width().max(1));
            session.height = Some(window.height().max(1));
            session.maximized = window.is_maximized();
            match session.save() {
                Ok(()) => {
                    tracing::info!("saved {} tab(s) for the next session", session.tabs.len())
                }
                Err(err) => tracing::warn!("could not save session: {err:#}"),
            }
            // Drop leftover VT dumps from ≤0.1.x installs.
            SessionState::clear_legacy_scrollback();
        })
    };

    {
        let save_session = save_session.clone();
        let config = config.clone();
        let tab_view = tab_view.clone();
        let confirmed_quit = Rc::new(Cell::new(false));
        window.connect_close_request(move |window| {
            // Confirming a single tab would just be in the way.
            let ask = config.borrow().confirm_quit && tab_view.n_pages() > 1;
            if ask && !confirmed_quit.get() {
                let dialog = adw::AlertDialog::new(
                    Some("Close this window?"),
                    Some("It has more than one tab open."),
                );
                dialog.add_responses(&[("cancel", "Cancel"), ("close", "Close")]);
                dialog.set_response_appearance("close", adw::ResponseAppearance::Destructive);
                dialog.set_default_response(Some("cancel"));
                dialog.set_close_response("cancel");

                let confirmed_quit = confirmed_quit.clone();
                let parent = window.clone();
                let window = window.clone();
                dialog.choose(&parent, gio::Cancellable::NONE, move |response| {
                    if response == "close" {
                        // Remember the answer so the retry is not asked again.
                        confirmed_quit.set(true);
                        window.close();
                    }
                });
                return glib::Propagation::Stop;
            }
            save_session();
            glib::Propagation::Proceed
        });
    }

    // A logout or `systemctl --user stop` sends SIGTERM, which bypasses
    // `close-request` and would silently lose the session; shut down cleanly.
    for signal in [nix::libc::SIGTERM, nix::libc::SIGINT] {
        let save_session = save_session.clone();
        let window_c = window.clone();
        glib::unix_signal_add_local(signal, move || {
            save_session();
            window_c.close();
            glib::ControlFlow::Break
        });
    }

    // Apply the configured tab layout, then open the tabs.
    set_tabs_location(config.borrow().tabs_location);

    // A non-default CLI launch (`-e` / cwd) skips session restore so the
    // requested command/directory is what the user sees.
    let skip_restore = !initial.is_default();
    let restored = if skip_restore {
        None
    } else {
        config
            .borrow()
            .session_restore
            .then(SessionState::load)
            .flatten()
    };

    match restored {
        Some(session) => {
            // Drop leftover VT dumps from ≤0.1.x installs.
            SessionState::clear_legacy_scrollback();
            if let (Some(w), Some(h)) = (session.width, session.height) {
                window.set_default_size(w, h);
            }
            for tab in &session.tabs {
                add_tab_layout(tab.title.clone(), &tab.layout)?;
            }
            if tab_view.n_pages() == 0 {
                add_tab(LaunchRequest::default())?;
            } else {
                let index = session.active.min(tab_view.n_pages().max(1) as usize - 1);
                let page = tab_view.nth_page(index as i32);
                tab_view.set_selected_page(&page);
            }
            if session.maximized {
                window.maximize();
            }
            tracing::info!(
                "restored {} tab(s) from the last session",
                session.tabs.len()
            );
        }
        None => {
            add_tab(initial)?;
        }
    }

    // Second instances hand their argv to this callback.
    {
        let add_tab = add_tab.clone();
        *shared.open_in_window.borrow_mut() = Some(Rc::new(move |req| {
            if let Err(err) = add_tab(req) {
                tracing::error!("launch from second instance failed: {err:#}");
            }
        }));
    }

    window.present();
    Ok(())
}

/// Snapshot the open tabs, nested split trees and each leaf's directory.
fn capture_session(tab_view: &adw::TabView, pages: &Pages) -> SessionState {
    let mut tabs = Vec::new();
    for i in 0..tab_view.n_pages() {
        let page = tab_view.nth_page(i);
        let Some((_, views)) = pages.borrow().iter().find(|(p, _)| p == &page).cloned() else {
            continue;
        };
        let layout = capture_pane_layout(&page.child(), &views);
        tabs.push(TabState {
            title: tab_is_renamed(&page).then(|| page.title().to_string()),
            layout,
        });
    }
    let active = tab_view
        .selected_page()
        .map(|p| tab_view.page_position(&p).max(0) as usize)
        .unwrap_or(0);
    SessionState {
        tabs,
        active,
        width: None,
        height: None,
        maximized: false,
    }
}

/// Walk a tab's widget tree into a `PaneLayout`.
fn capture_pane_layout(widget: &gtk4::Widget, views: &[Rc<TerminalView>]) -> PaneLayout {
    if let Ok(paned) = widget.clone().downcast::<gtk4::Paned>() {
        let orientation = match paned.orientation() {
            gtk4::Orientation::Horizontal => SplitOrientation::Horizontal,
            _ => SplitOrientation::Vertical,
        };
        let size = match paned.orientation() {
            gtk4::Orientation::Horizontal => paned.width(),
            _ => paned.height(),
        };
        let ratio = if size > 0 {
            (paned.position() as f64 / size as f64).clamp(0.05, 0.95)
        } else {
            0.5
        };
        let start = paned
            .start_child()
            .map(|c| capture_pane_layout(&c, views))
            .unwrap_or_default();
        let end = paned
            .end_child()
            .map(|c| capture_pane_layout(&c, views))
            .unwrap_or_default();
        PaneLayout::Split {
            orientation,
            ratio,
            start: Box::new(start),
            end: Box::new(end),
        }
    } else if let Ok(overlay) = widget.clone().downcast::<gtk4::Overlay>() {
        let cwd = views
            .iter()
            .find(|v| v.widget() == &overlay)
            .and_then(|v| v.pwd());
        PaneLayout::Leaf { cwd }
    } else if let Ok(bx) = widget.clone().downcast::<gtk4::Box>() {
        if let Some(child) = bx.first_child() {
            capture_pane_layout(&child, views)
        } else {
            PaneLayout::Leaf { cwd: None }
        }
    } else {
        PaneLayout::Leaf { cwd: None }
    }
}

/// Recreate a nested split tree as widgets + TerminalViews.
fn build_layout_widget(
    layout: &PaneLayout,
    make_view: &MakeViewFn,
    page_slot: &Rc<RefCell<Option<adw::TabPage>>>,
) -> anyhow::Result<(gtk4::Widget, Vec<Rc<TerminalView>>)> {
    match layout {
        PaneLayout::Leaf { cwd } => {
            let view = make_view(page_slot.clone(), cwd.clone().map(PathBuf::from), None)?;
            Ok((view.widget().clone().upcast(), vec![view]))
        }
        PaneLayout::Split {
            orientation,
            ratio,
            start,
            end,
        } => {
            let (start_w, mut start_views) = build_layout_widget(start, make_view, page_slot)?;
            let (end_w, end_views) = build_layout_widget(end, make_view, page_slot)?;
            let orient = match orientation {
                SplitOrientation::Horizontal => gtk4::Orientation::Horizontal,
                SplitOrientation::Vertical => gtk4::Orientation::Vertical,
            };
            let paned = gtk4::Paned::new(orient);
            paned.set_wide_handle(true);
            paned.set_resize_start_child(true);
            paned.set_resize_end_child(true);
            paned.set_shrink_start_child(false);
            paned.set_shrink_end_child(false);
            paned.set_start_child(Some(&start_w));
            paned.set_end_child(Some(&end_w));
            let ratio = (*ratio).clamp(0.05, 0.95);
            // Apply the saved divider once the paned has a real size.
            paned.connect_map(move |p| {
                let size = match p.orientation() {
                    gtk4::Orientation::Horizontal => p.width(),
                    _ => p.height(),
                };
                if size > 0 {
                    p.set_position((size as f64 * ratio) as i32);
                }
            });
            start_views.extend(end_views);
            Ok((paned.upcast(), start_views))
        }
    }
}

/// Honor `gtk-decoration-layout`, chrome font and text scaling.
fn apply_desktop_chrome(header: &adw::HeaderBar, window: &adw::ApplicationWindow) {
    let Some(settings) = gtk4::Settings::default() else {
        return;
    };

    let apply_decoration = {
        let header = header.clone();
        let settings = settings.clone();
        Rc::new(move || {
            let layout = settings.gtk_decoration_layout().unwrap_or_default();
            let mut parts = layout.splitn(2, ':');
            let start = parts.next().unwrap_or("");
            let end = parts.next().unwrap_or("");
            header.set_show_start_title_buttons(!start.is_empty());
            header.set_show_end_title_buttons(!end.is_empty());
        })
    };
    apply_decoration();
    {
        let apply_decoration = apply_decoration.clone();
        settings.connect_gtk_decoration_layout_notify(move |_| apply_decoration());
    }

    let provider = gtk4::CssProvider::new();
    if let Some(display) = gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION + 2,
        );
    }
    let apply_font = {
        let settings = settings.clone();
        let provider = provider.clone();
        let window = window.clone();
        Rc::new(move || {
            let font = settings.gtk_font_name().unwrap_or_else(|| "Sans 10".into());
            let scale = settings.gtk_xft_dpi() as f64 / 1024.0 / 96.0;
            let scale = if scale <= 0.0 { 1.0 } else { scale };
            // Prefer the desktop text-scaling-factor when available via settings.
            let text_scale = settings
                .property_value("gtk-xft-dpi")
                .get::<i32>()
                .ok()
                .map(|dpi| dpi as f64 / 1024.0 / 96.0)
                .unwrap_or(scale);
            let css = format!(
                ".option-chrome, .option-chrome * {{ font: {font}; }}\n\
                 window {{ font-size: {}pt; }}",
                10.0 * text_scale.max(0.5)
            );
            provider.load_from_string(&css);
            let _ = &window;
        })
    };
    apply_font();
    {
        let apply_font = apply_font.clone();
        settings.connect_gtk_font_name_notify(move |_| apply_font());
    }
    {
        let apply_font = apply_font.clone();
        settings.connect_gtk_xft_dpi_notify(move |_| apply_font());
    }
    // Animations: leave GtkSettings alone so gtk-enable-animations is honored.
}

/// Leaf-weighted split equalization, :
/// each divider is placed proportionally to the terminal count on each side.
fn equalize_splits(widget: &gtk4::Widget) -> i32 {
    let Some(paned) = widget.downcast_ref::<gtk4::Paned>() else {
        return 1;
    };
    let start = paned
        .start_child()
        .map(|c| equalize_splits(&c))
        .unwrap_or(0);
    let end = paned.end_child().map(|c| equalize_splits(&c)).unwrap_or(0);
    let total = (start + end).max(1);
    let size = match paned.orientation() {
        gtk4::Orientation::Horizontal => paned.width(),
        _ => paned.height(),
    };
    if size > 0 && start > 0 && end > 0 {
        paned.set_position(size * start / total);
    }
    start + end
}

fn cycle_tab(tab_view: &adw::TabView, dir: i32) {
    let n = tab_view.n_pages();
    if n == 0 {
        return;
    }
    let cur = tab_view
        .selected_page()
        .map(|p| tab_view.page_position(&p))
        .unwrap_or(0);
    let next = (cur + dir).rem_euclid(n);
    let page = tab_view.nth_page(next);
    tab_view.set_selected_page(&page);
}
