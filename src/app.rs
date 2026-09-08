//! Adwaita application shell with TabView, splits and command palette.

use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::{Rc, Weak},
};

use gtk4::gdk;
use gtk4::gio::{self, prelude::*};
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;

use crate::{
    agents, browser, codex,
    config::{
        Config, CursorStyle, MiddleClickTab, NewTabPosition, TabOverflow, TabWidth, TabsLocation,
        Theme,
    },
    launch::LaunchRequest,
    session::{PaneLayout, Session as SessionState, SplitOrientation, TabKind, TabState},
    terminal::TerminalView,
    tree::FileTree,
    ui::{
        PrefsHooks, SearchBar, attach_context_menu, main_popover, show_about, show_command_palette,
        show_preferences, show_shortcuts, tab_menu, tabs_menu,
    },
};

const APP_ID: &str = "io.option.terminal";

/// How long after our own `config.toml` write the file monitor stays quiet.
pub(crate) const SELF_WRITE_GRACE: std::time::Duration = std::time::Duration::from_millis(150);

pub type Pages = Rc<RefCell<Vec<(adw::TabPage, Vec<Rc<TerminalView>>)>>>;
type Toast = Rc<dyn Fn(&str)>;
type CallbackSlot = Rc<RefCell<Option<Rc<dyn Fn()>>>>;
type Focused = Rc<RefCell<Option<Weak<TerminalView>>>>;
type LaunchHandler = Rc<dyn Fn(LaunchRequest)>;
/// Splits the focused pane in a direction (`orientation`, `before`).
type SplitFn = Rc<dyn Fn(gtk4::Orientation, bool)>;
/// Records the direction, then runs a `SplitFn`.
type SplitDone = Rc<dyn Fn(SplitFn, gtk4::Orientation, bool)>;
/// The file tree panel attached to one tab: the tree plus the `Paned` that
/// hosts it beside the terminal.
#[derive(Clone)]
struct FileTreeSlot {
    tree: Rc<FileTree>,
    paned: gtk4::Paned,
    /// Last pane cwd the tree was synced to; skips re-rooting when the user
    /// has navigated the tree elsewhere and just clicks the terminal again.
    last_pwd: Rc<RefCell<Option<PathBuf>>>,
}
type FileTrees = Rc<RefCell<std::collections::HashMap<adw::TabPage, FileTreeSlot>>>;
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
    open_in_window: RefCell<Option<LaunchHandler>>,
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
            let cwd = cmdline.cwd().unwrap_or_else(|| PathBuf::from("/"));
            let mut req = match crate::launch::parse_command_line(&args, &cwd) {
                Ok(crate::launch::CommandLine::Launch(request)) => request,
                Ok(crate::launch::CommandLine::Help) => {
                    command_line_message(cmdline, crate::launch::HELP, false);
                    return 0;
                }
                Ok(crate::launch::CommandLine::Version) => {
                    command_line_message(
                        cmdline,
                        &format!("optionterm {}\n", env!("CARGO_PKG_VERSION")),
                        false,
                    );
                    return 0;
                }
                Ok(crate::launch::CommandLine::SelfTest) => {
                    command_line_message(cmdline, "Run --self-test in a separate process\n", true);
                    return 2;
                }
                Err(err) => {
                    command_line_message(cmdline, &format!("{err}\n"), true);
                    return 2;
                }
            };
            if cmdline.is_remote() && req.cwd.is_none() {
                req.cwd = Some(cwd);
            }
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

fn command_line_message(command: &gio::ApplicationCommandLine, text: &str, error: bool) {
    use glib::translate::ToGlibPtr;
    let text = std::ffi::CString::new(text.replace('\0', "")).expect("NUL removed");
    unsafe {
        if error {
            gio::ffi::g_application_command_line_printerr(
                command.to_glib_none().0,
                c"%s".as_ptr(),
                text.as_ptr(),
            );
        } else {
            gio::ffi::g_application_command_line_print(
                command.to_glib_none().0,
                c"%s".as_ptr(),
                text.as_ptr(),
            );
        }
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
    const INSTALLED: &str = "optionterm-css-installed";
    unsafe {
        if display.data::<bool>(INSTALLED).is_some() {
            return;
        }
        display.set_data(INSTALLED, true);
    }
    let provider = gtk4::CssProvider::new();
    provider.load_from_string(
        r#"
window.transparent-bg,
window.transparent-bg > * ,
window.transparent-bg .terminal { background-color: transparent; }

.terminal-surface,
.terminal-surface > scrolledwindow,
.terminal-surface > scrolledwindow > viewport {
  border: none;
  border-radius: 0px;
  box-shadow: none;
  margin: 0px;
  padding: 0px;
}

.quick-settings { padding: 8px 14px 4px 14px; }

/* Sidebar: flat dark surface with card-style tab rows (like the reference).
   Rows are rounded chips; the active one is a vivid orange card. */
.navigation-sidebar,
.navigation-sidebar-scroll {
  background-color: #1a1a1a;
}
.navigation-sidebar {
  padding: 6px;
}
.navigation-sidebar > row {
  min-height: 34px;
  padding: 4px 8px;
  border-radius: 10px;
  margin: 2px 0px;
  background-color: transparent;
  transition: background-color 120ms ease;
}
.navigation-sidebar > row:hover { background-color: #262626; }
.navigation-sidebar > row > box { margin: 0px; }
/* Agent logos and the close button render as a small white glyph. */
.navigation-sidebar > row > box > image {
  -gtk-icon-size: 14px;
  filter: grayscale(1) brightness(0) invert(1);
}
/* Active tab: vivid orange card, like an IDE active tab. */
.navigation-sidebar > row:selected {
  background-color: #f07826;
  color: #ffffff;
}
.navigation-sidebar > row:selected:hover { background-color: #f07826; }
.navigation-sidebar > row:selected > box > image {
  filter: grayscale(1) brightness(0) invert(1);
}
.navigation-sidebar > row:selected label { color: #ffffff; }
"#,
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
const SPLIT_RATIO_KEY: &str = "option-term-split-ratio";

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
    let (loaded, config_error) = match Config::load() {
        Ok(config) => (config, false),
        Err(_) => (
            Config {
                source: crate::config::option_config_path(),
                ..Config::default()
            },
            true,
        ),
    };
    let config = Rc::new(RefCell::new(loaded));
    let source_ids: Rc<RefCell<Vec<glib::SourceId>>> = Rc::new(RefCell::new(Vec::new()));
    let config_watch = crate::config_watch::ConfigWatch::new(&config.borrow());
    let applying_config = Rc::new(Cell::new(false));
    let sync_config: CallbackSlot = Rc::new(RefCell::new(None));
    let config_observers = crate::ui::ConfigObservers::default();
    let base_font_size = Rc::new(Cell::new(config.borrow().font_size));

    // Shortcut overrides live in their own file. Only overridden actions are
    // touched, so new built-in defaults still reach existing installs.
    let bindings = Rc::new(RefCell::new(crate::keys::Bindings::load()));
    let apply_bindings: Rc<dyn Fn()> = {
        let app = app.clone();
        let bindings = bindings.clone();
        Rc::new(move || {
            bindings.borrow().apply(&app);
            for window in app.windows() {
                bindings.borrow().update_tooltips(window.upcast_ref());
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
        let watcher = config_watch.clone();
        let applying = applying_config.clone();
        let sync_config = sync_config.clone();
        Rc::new(move || {
            if applying.get() {
                return;
            }
            tracing::trace!(elapsed = ?self_write.replace(std::time::Instant::now()).elapsed(), "configuration write queued");
            let sync = sync_config.borrow().clone();
            if let Some(sync) = sync {
                sync();
            }
            watcher.save(config.borrow().clone());
        })
    };

    // Honor the saved theme before the window is mapped to avoid a flash.
    apply_theme(config.borrow().theme);
    if let Some(display) = gdk::Display::default() {
        install_css(&display);
    }
    // Make the real agent logos resolvable as tab icons.
    agents::register_icons();
    let agent_menu = crate::ui::agent_menu();

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
        window_css(
            &window,
            &provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
        );
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

    // `+` opens tabs/docs (New Tab, Browser, File Tree, Command Palette),
    // same as the sidebar variant.
    let new_tab_btn = adw::SplitButton::builder()
        .icon_name("tab-new-symbolic")
        .tooltip_text("New Tab (Ctrl+Shift+T)")
        .menu_model(&tabs_menu(&agent_menu))
        .build();
    new_tab_btn.set_action_name(Some("win.new-tab"));
    header.pack_start(&new_tab_btn);

    // Splitting lives behind the `+` menu; there is no separate split button.
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
    content_box.set_hexpand(true);
    content_box.set_vexpand(true);
    content_box.append(&tab_view);

    let toast_overlay = adw::ToastOverlay::new();
    toast_overlay.set_hexpand(true);
    toast_overlay.set_vexpand(true);
    toast_overlay.set_child(Some(&content_box));

    // Tabs-as-sidebar support
    let sidebar_list = gtk4::ListBox::new();
    sidebar_list.add_css_class("navigation-sidebar");
    sidebar_list.set_selection_mode(gtk4::SelectionMode::Single);

    let sidebar_scroll = gtk4::ScrolledWindow::new();
    sidebar_scroll.set_vexpand(true);
    sidebar_scroll.set_child(Some(&sidebar_list));
    sidebar_scroll.set_has_frame(false);
    sidebar_scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    sidebar_scroll.add_css_class("navigation-sidebar-scroll");

    // `+` with the tabs/docs dropdown, same as the header variant.
    let sidebar_new_btn = adw::SplitButton::builder()
        .icon_name("tab-new-symbolic")
        .tooltip_text("New Tab (Ctrl+Shift+T)")
        .menu_model(&tabs_menu(&agent_menu))
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
    // bar is natively draggable. [+ ▾] [⇄▾] … [● ● ●] … [🔍] [☰]
    let sidebar_header = adw::HeaderBar::new();
    sidebar_header.add_css_class("flat");
    sidebar_header.add_css_class("option-chrome");
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
    split_view.set_hexpand(true);
    split_view.set_vexpand(true);
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
    tab_overview.set_hexpand(true);
    tab_overview.set_vexpand(true);
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
    let rebuild_sidebar = bind_sidebar(&sidebar_list, &tab_view, sidebar_syncing.clone());

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
        let applied = Cell::new(None::<(TabsLocation, bool)>);
        Rc::new(move |location: TabsLocation| {
            let sidebar_mode = matches!(location, TabsLocation::Left | TabsLocation::Right);
            let show_sidebar =
                sidebar_mode && (config.borrow().sidebar_always || tab_view.n_pages() > 1);
            if applied.replace(Some((location, show_sidebar))) == Some((location, show_sidebar)) {
                return;
            }
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
    let search_target_hook: CallbackSlot = Rc::new(RefCell::new(None));

    // Per-tab file-tree panels (shown/collapsed via `win.file-tree`).
    let file_trees: FileTrees = Rc::new(RefCell::new(std::collections::HashMap::new()));

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
        let search_target_hook = search_target_hook.clone();
        let unzoom = unzoom.clone();
        let toast = toast.clone();
        let show_resize = show_resize.clone();
        let tab_view = tab_view.clone();
        let config = config.clone();
        let pages = pages.clone();
        let window = window.clone();
        let focused = focused.clone();
        let file_trees = file_trees.clone();
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
                        let toast = toast.clone();
                        gio::AppInfo::launch_default_for_uri_async(
                            &uri,
                            gio::AppLaunchContext::NONE,
                            gio::Cancellable::NONE,
                            move |result| match result {
                                Ok(()) => toast("Link opened"),
                                Err(_) => toast("No application to open that link"),
                            },
                        );
                    });
                }

                {
                    let view_weak = Rc::downgrade(&view);
                    let focused = focused.clone();
                    let page_slot = page_slot.clone();
                    let file_trees = file_trees.clone();
                    let search_target_hook = search_target_hook.clone();
                    view.set_on_focus(move || {
                        *focused.borrow_mut() = Some(view_weak.clone());
                        let sync = search_target_hook.borrow().clone();
                        if let Some(sync) = sync {
                            sync();
                        }
                        // Sync this tab's file tree only when the pane moved to
                        // a different directory (cd or tab switch), so the user
                        // can navigate the tree without it snapping back.
                        let Some(view) = view_weak.upgrade() else {
                            return;
                        };
                        let Some(page) = page_slot.borrow().clone() else {
                            return;
                        };
                        let Some(slot) = file_trees.borrow().get(&page).cloned().filter(|slot| {
                            slot.paned
                                .end_child()
                                .is_some_and(|panel| panel.is_visible())
                        }) else {
                            return;
                        };
                        let Some(pwd) = view.pwd().map(PathBuf::from) else {
                            return;
                        };
                        if slot.last_pwd.borrow().as_ref() != Some(&pwd) {
                            slot.tree.set_root(pwd.clone());
                            *slot.last_pwd.borrow_mut() = Some(pwd);
                        }
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
                    let toast = toast.clone();
                    view.set_on_spawn_error(move |err| {
                        tracing::error!("terminal spawn failed: {err}");
                        toast(&format!("Failed to launch: {err}"));
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
                            if let Some((_, views)) = pgs.iter_mut().find(|(p, _)| p == &page)
                                && views.len() > 1
                            {
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
        let file_trees = file_trees.clone();
        Rc::new(
            move |launch: LaunchRequest| -> anyhow::Result<adw::TabPage> {
                let page_slot: Rc<RefCell<Option<adw::TabPage>>> = Rc::new(RefCell::new(None));
                let view = make_view(page_slot.clone(), launch.cwd, launch.command)?;

                // Every tab hosts a collapsible file-tree panel to the right,
                // toggled with `win.file-tree` (like an IDE explorer).
                let file_tree = Rc::new(FileTree::new());
                let paned = gtk4::Paned::new(gtk4::Orientation::Horizontal);
                paned.set_hexpand(true);
                paned.set_vexpand(true);
                paned.set_wide_handle(true);
                paned.set_start_child(Some(view.widget()));
                paned.set_end_child(Some(&file_tree.panel()));
                // Hidden until `win.file-tree` reveals it.
                if let Some(child) = paned.end_child() {
                    child.set_visible(false);
                }

                let root = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
                root.set_hexpand(true);
                root.set_vexpand(true);
                root.append(&paned);

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
                file_trees.borrow_mut().insert(
                    page.clone(),
                    FileTreeSlot {
                        tree: file_tree,
                        paned,
                        last_pwd: Rc::new(RefCell::new(None)),
                    },
                );
                tab_view.set_selected_page(&page);
                view.focus();
                Ok(page)
            },
        )
    };

    let add_browser_tab = {
        let tab_view = tab_view.clone();
        Rc::new(move |url: Option<&str>| -> adw::TabPage {
            let root = browser::new_tab(url);
            let page = tab_view.append(&root);
            set_browser_tab(&page);
            page.set_title("Browser");
            page.set_live_thumbnail(true);
            tab_view.set_selected_page(&page);
            page
        })
    };

    // Restore a tab from a nested split tree.
    let add_tab_layout = {
        let tab_view = tab_view.clone();
        let pages = pages.clone();
        let make_view = make_view.clone();
        let file_trees = file_trees.clone();
        Rc::new(
            move |title: Option<String>, layout: &PaneLayout| -> anyhow::Result<adw::TabPage> {
                let page_slot: Rc<RefCell<Option<adw::TabPage>>> = Rc::new(RefCell::new(None));
                let (child, views) = build_layout_widget(layout, &make_view, &page_slot)?;

                // Same per-tab file-tree panel as a fresh tab.
                let file_tree = Rc::new(FileTree::new());
                let paned = gtk4::Paned::new(gtk4::Orientation::Horizontal);
                paned.set_hexpand(true);
                paned.set_vexpand(true);
                paned.set_wide_handle(true);
                paned.set_start_child(Some(&child));
                paned.set_end_child(Some(&file_tree.panel()));
                if let Some(end) = paned.end_child() {
                    end.set_visible(false);
                }

                let root = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
                root.set_hexpand(true);
                root.set_vexpand(true);
                root.append(&paned);

                let page = tab_view.append(&root);
                if let Some(title) = title {
                    set_tab_renamed(&page, true);
                    page.set_title(&title);
                } else {
                    page.set_title("Terminal");
                }
                page.set_live_thumbnail(true);
                *page_slot.borrow_mut() = Some(page.clone());

                pages.borrow_mut().push((page.clone(), views.clone()));
                file_trees.borrow_mut().insert(
                    page.clone(),
                    FileTreeSlot {
                        tree: file_tree,
                        paned,
                        last_pwd: Rc::new(RefCell::new(None)),
                    },
                );
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
    {
        let search_bar = Rc::downgrade(&search_bar);
        *search_target_hook.borrow_mut() = Some(Rc::new(move || {
            if let Some(search_bar) = search_bar.upgrade() {
                search_bar.sync_target();
            }
        }));
    }
    // Deliberately no `set_key_capture_widget`: in a terminal every keystroke
    // belongs to the shell, so the bar only opens via its explicit action.

    // Split the focused pane. `before` puts the new terminal on the
    // left/top side; splits nest arbitrarily (Paned inside Paned).
    let split: SplitFn = {
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
            track_split_geometry(&paned, 0.5);
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
                if let Some(paned) = parent.downcast_ref::<gtk4::Paned>()
                    && paned.has_css_class("terminal-split")
                {
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
                if let Some(paned) = parent.downcast_ref::<gtk4::Paned>()
                    && paned.has_css_class("terminal-split")
                {
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
        window.add_action(&add_simple(
            "close-tab",
            Box::new(move || {
                if let Some(page) = tab_view.selected_page() {
                    request_tab_close(&tab_view, &page);
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

    // Remember the last split direction so the splits button's main click
    // repeats it (default: Split Right).
    let last_split: Rc<RefCell<(bool, gtk4::Orientation)>> =
        Rc::new(RefCell::new((false, gtk4::Orientation::Horizontal)));
    // Record the direction, then run the split.
    let split_done: SplitDone = {
        let last_split = last_split.clone();
        Rc::new(move |split, orientation, before| {
            *last_split.borrow_mut() = (before, orientation);
            split(orientation, before);
        })
    };

    {
        let split = split.clone();
        let split_done = split_done.clone();
        window.add_action(&add_simple(
            "split-right",
            Box::new(move || split_done(split.clone(), gtk4::Orientation::Horizontal, false)),
        ));
    }
    {
        let split = split.clone();
        let split_done = split_done.clone();
        window.add_action(&add_simple(
            "split-down",
            Box::new(move || split_done(split.clone(), gtk4::Orientation::Vertical, false)),
        ));
    }
    {
        let split = split.clone();
        let split_done = split_done.clone();
        window.add_action(&add_simple(
            "split-left",
            Box::new(move || split_done(split.clone(), gtk4::Orientation::Horizontal, true)),
        ));
    }
    {
        let split = split.clone();
        let split_done = split_done.clone();
        window.add_action(&add_simple(
            "split-up",
            Box::new(move || split_done(split.clone(), gtk4::Orientation::Vertical, true)),
        ));
    }
    {
        let split = split.clone();
        let split_done = split_done.clone();
        window.add_action(&add_simple(
            "split-last",
            Box::new(move || {
                let (before, orientation) = *last_split.borrow();
                split_done(split.clone(), orientation, before);
            }),
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
        let applying_config = applying_config.clone();
        Rc::new(move |size: f32| {
            if applying_config.get() {
                return;
            }
            let size = size.clamp(6.0, 40.0);
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
            Box::new(move || zoom_step(&config, &*apply_zoom, 1.0)),
        ));
    }
    {
        let config = config.clone();
        let apply_zoom = apply_zoom.clone();
        window.add_action(&add_simple(
            "zoom-out",
            Box::new(move || zoom_step(&config, &*apply_zoom, -1.0)),
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
    let apply_config_ui: Rc<dyn Fn()> = {
        let config = config.clone();
        let window = window.downgrade();
        let set_tabs_location = set_tabs_location.clone();
        let apply_tab_shape = apply_tab_shape.clone();
        let palette_btn = palette_btn.downgrade();
        let sidebar_palette_btn = sidebar_palette_btn.downgrade();
        let sync_quick = sync_quick.clone();
        Rc::new(move || {
            let Some(window) = window.upgrade() else {
                return;
            };
            let cfg = config.borrow().clone();
            apply_theme(cfg.theme);
            apply_window_opacity(&window, cfg.background_opacity);
            set_tabs_location(cfg.tabs_location);
            apply_tab_shape();
            for button in [&palette_btn, &sidebar_palette_btn] {
                if let Some(button) = button.upgrade() {
                    button.set_visible(cfg.show_search_button);
                }
            }
            sync_quick();
            let tabs = match cfg.tabs_location {
                TabsLocation::Top => "top",
                TabsLocation::Bottom => "bottom",
                TabsLocation::Left => "left",
                TabsLocation::Right => "right",
                TabsLocation::Hidden => "hidden",
            };
            let cursor = match cfg.cursor_style {
                CursorStyle::Bar => "bar",
                CursorStyle::Underline => "underline",
                _ => "block",
            };
            for (name, value) in [
                ("theme", cfg.theme.as_str().to_variant()),
                ("tabs-pos", tabs.to_variant()),
                ("cursor-shape", cursor.to_variant()),
                ("cursor-blink", cfg.cursor_blink.to_variant()),
                ("sidebar-always", cfg.sidebar_always.to_variant()),
            ] {
                if let Some(action) = window
                    .lookup_action(name)
                    .and_then(|a| a.downcast::<gio::SimpleAction>().ok())
                {
                    action.set_state(&value);
                }
            }
        })
    };
    *sync_config.borrow_mut() = Some(apply_config_ui.clone());
    let reload_config: Rc<dyn Fn(bool)> = {
        let watcher = config_watch.clone();
        Rc::new(move |notify| watcher.reload(notify))
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
        let config = config.clone();
        let pages = pages.clone();
        let base_font_size = base_font_size.clone();
        let applying = applying_config.clone();
        let observers = config_observers.clone();
        let apply_ui = apply_config_ui.clone();
        // Ignore the write we just made from the UI.
        // The monitor must outlive this scope to keep firing.
        config_watch.watch(
            Rc::new(move |new_cfg| {
                applying.set(true);
                base_font_size.set(new_cfg.font_size);
                *config.borrow_mut() = new_cfg.clone();
                let views: Vec<_> = pages
                    .borrow()
                    .iter()
                    .flat_map(|(_, views)| views.clone())
                    .collect();
                for view in views {
                    view.apply_config(&new_cfg);
                }
                apply_ui();
                let updates = observers.borrow().clone();
                for (owner, update) in updates {
                    if owner.upgrade().is_some() {
                        update(&new_cfg);
                    }
                }
                applying.set(false);
            }),
            toast.clone(),
        );
    }
    if config_error {
        toast("Invalid configuration; using defaults without replacing your file");
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
        let applying_config = applying_config.clone();
        let config_observers = config_observers.clone();
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
                        applying_config: applying_config.clone(),
                        config_observers: config_observers.clone(),
                    },
                );
            }),
        ));
    }

    {
        let window_c = window.clone();
        let config_c = config.clone();
        let bindings = bindings.clone();
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
                show_command_palette(&window_c, &config_c, &bindings.borrow(), open_launch);
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
            Box::new(move || {
                add_browser_tab(None);
            }),
        ));
    }

    // Toggle the file-tree panel of the selected tab (IDE-style explorer).
    {
        let tab_view = tab_view.clone();
        let file_trees = file_trees.clone();
        let current_view = current_view.clone();
        window.add_action(&add_simple(
            "file-tree",
            Box::new(move || {
                let Some(page) = tab_view.selected_page() else {
                    return;
                };
                let Some(slot) = file_trees.borrow().get(&page).cloned() else {
                    return;
                };
                let panel = slot.paned.end_child();
                let Some(panel) = panel else {
                    return;
                };
                if !panel.is_visible() {
                    // Re-root to the focused pane before revealing the panel.
                    if let Some(view) = current_view()
                        && let Some(pwd) = view.pwd().map(PathBuf::from)
                    {
                        slot.tree.set_root(pwd.clone());
                        *slot.last_pwd.borrow_mut() = Some(pwd);
                    }
                    let total = slot.paned.width().max(400);
                    slot.paned.set_position((total as f64 * 0.72) as i32);
                    panel.set_visible(true);
                } else {
                    panel.set_visible(false);
                }
            }),
        ));
    }

    // One dedicated tab per installed agent: run the CLI in the current pane's
    // directory, inserted to the right of the active tab, with its real logo
    // as the tab icon.
    {
        let add_tab = add_tab.clone();
        let inherit_cwd = inherit_cwd.clone();
        let toast = toast.clone();
        let tab_view = tab_view.clone();
        let rebuild_sidebar = rebuild_sidebar.clone();
        for kind in agents::AgentKind::ALL {
            window.add_action(&add_simple(&format!("agent-{}", kind.as_str()), {
                let add_tab = add_tab.clone();
                let inherit_cwd = inherit_cwd.clone();
                let toast = toast.clone();
                let tab_view = tab_view.clone();
                let rebuild_sidebar = rebuild_sidebar.clone();
                Box::new(move || {
                    // Always give the agent a real working directory — these
                    // CLIs tend to bail out (or open the wrong repo) without
                    // one, and `inherit_cwd` can be None on a fresh window.
                    let cwd = inherit_cwd().or_else(dirs::home_dir);
                    let cur = tab_view.selected_page();
                    // Copy the dir for the tab title before it is moved into
                    // the launch request.
                    let title_dir = cwd.clone();
                    match add_tab(LaunchRequest {
                        cwd,
                        command: Some(agents::new_command(kind)),
                    }) {
                        Ok(page) => {
                            // Insert to the right of the tab we opened from.
                            if let Some(cur) = cur {
                                let at = tab_view.page_position(&cur) + 1;
                                tab_view.reorder_page(&page, at);
                            }
                            // Real logo as the tab icon, resolved through the
                            // icon theme (registers the logos dir). Use the
                            // derived white glyph on dark themes so it stays
                            // legible; the original logo on light themes.
                            let dark = adw::StyleManager::default().is_dark();
                            let icon = gio::ThemedIcon::new(kind.theme_icon_name(dark));
                            page.set_icon(Some(&icon));
                            // Rebuild the sidebar so it mirrors the icon too
                            // (page_attached fired before set_icon).
                            rebuild_sidebar();
                            // Keep our "<Agent> (<dir>)" title (don't let the
                            // agent's OSC window-title overwrite it).
                            set_tab_renamed(&page, true);
                            // "<Agent> (<dir>)" — e.g. "Codex (~/projects/app)".
                            let dir = title_dir
                                .as_deref()
                                .and_then(|p| p.to_str())
                                .map(abbreviate_home)
                                .unwrap_or_else(|| "~".to_string());
                            page.set_title(&format!("{} ({})", kind.label(), dir));
                        }
                        Err(err) => {
                            tracing::error!("agent tab failed: {err:#}");
                            toast(&format!("Failed to open {}", kind.label()));
                        }
                    }
                })
            }));
        }
    }

    // Save the most recent Codex thread as a Markdown transcript to a chosen
    // directory (defaulting to the focused pane's working directory).
    {
        let window_c = window.downgrade();
        let toast = toast.clone();
        let current_view = current_view.clone();
        let busy = Rc::new(Cell::new(false));
        window.add_action(&add_simple(
            "save-codex-thread",
            Box::new(move || {
                if busy.replace(true) {
                    return;
                }
                let folder = current_view().and_then(|v| v.pwd()).map(PathBuf::from);
                let window_c = window_c.clone();
                let toast = toast.clone();
                let busy = busy.clone();
                let job = gio::spawn_blocking(move || {
                    (codex::list_threads().next(), folder.filter(|d| d.is_dir()))
                });
                glib::spawn_future_local(async move {
                    let result = job.await;
                    let Some(window) = window_c.upgrade().filter(|w| w.is_visible()) else {
                        busy.set(false);
                        return;
                    };
                    let (thread, folder) = match result {
                        Ok((Some(thread), folder)) => (thread, folder),
                        other => {
                            busy.set(false);
                            toast(if other.is_err() {
                                "Could not load Codex conversations"
                            } else {
                                "No Codex thread found"
                            });
                            return;
                        }
                    };
                    let dialog = gtk4::FileDialog::builder()
                        .title(format!("Export Codex: {}", thread.title))
                        .initial_name(format!("{}.md", codex::slugify(&thread.title)))
                        .build();
                    if let Some(folder) = folder {
                        dialog.set_initial_folder(Some(&gio::File::for_path(&folder)));
                    }
                    let thread_file = thread.file;
                    dialog.save(Some(&window), None::<&gio::Cancellable>, move |res| {
                        let Ok(file) = res else {
                            busy.set(false);
                            return;
                        }; // cancelled
                        let Some(path) = file.path() else {
                            busy.set(false);
                            toast("No local path for the chosen file");
                            return;
                        };
                        let job =
                            gio::spawn_blocking(move || codex::save_markdown(&thread_file, &path));
                        glib::spawn_future_local(async move {
                            let result = job.await;
                            busy.set(false);
                            if window_c.upgrade().is_some_and(|w| w.is_visible()) {
                                match result {
                                    Ok(Ok(())) => toast("Codex thread saved"),
                                    _ => toast("Failed to save thread"),
                                }
                            }
                        });
                    });
                });
            }),
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
        let bindings = bindings.clone();
        window.add_action(&add_simple(
            "shortcuts",
            Box::new(move || show_shortcuts(&window_c, &bindings.borrow())),
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

    // Split bindings for tiling panes.
    crate::keys::Bindings::default().apply(app);

    // keys.toml wins over everything set above.
    apply_bindings();
    apply_config_ui();

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
        let search_bar = Rc::downgrade(&search_bar);
        tab_view.connect_notify_local(Some("selected-page"), move |tv, _| {
            sync_split_actions(tv.selected_page());
            if let Some(search_bar) = search_bar.upgrade() {
                search_bar.sync_target();
            }
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
        let file_trees = file_trees.clone();
        let window = window.clone();
        let config = config.clone();
        tab_view.connect_close_page(move |tv, page| {
            // AdwTabView allows an async answer: hold the close, then finish it
            // once the user has decided.
            let has_child = pages
                .borrow()
                .iter()
                .find(|(p, _)| p == page)
                .is_none_or(|(_, views)| views.iter().any(|v| v.has_child()));
            if config.borrow().confirm_close_tab && has_child {
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
                let file_trees = file_trees.clone();
                let parent = window.clone();
                let window = window.clone();
                dialog.choose(&parent, gio::Cancellable::NONE, move |response| {
                    let closing = response == "close";
                    if closing {
                        release_tab(&pages, &file_trees, &page);
                    }
                    tv.close_page_finish(&page, closing);
                    if closing && tv.n_pages() == 0 {
                        window.close();
                    }
                });
                return glib::Propagation::Stop;
            }

            release_tab(&pages, &file_trees, page);
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
        window_css(&window, &css, gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION);
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
        {
            let app = app.clone();
            let cookie = cookie.clone();
            window.connect_destroy(move |_| {
                let held = cookie.replace(0);
                if held != 0 {
                    app.uninhibit(held);
                }
            });
        }
        // Was anything running last time we looked? A busy-to-idle transition
        // is what "the command finished" means without shell integration.
        let was_busy = Rc::new(Cell::new(false));
        let source = glib::timeout_add_seconds_local(4, move || {
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
        source_ids.borrow_mut().push(source);
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
                Err(err) => {
                    saved.set(false);
                    tracing::warn!("could not save session: {err:#}");
                }
            }
            // Drop leftover VT dumps from ≤0.1.x installs.
            SessionState::clear_legacy_scrollback();
        })
    };

    let shutdown: Rc<dyn Fn()> = {
        let pages = pages.clone();
        let file_trees = file_trees.clone();
        let source_ids = source_ids.clone();
        let config_watch = config_watch.clone();
        let sync_config = sync_config.clone();
        let observers = config_observers.clone();
        let shared = shared.clone();
        Rc::new(move || {
            for source in source_ids.borrow_mut().drain(..) {
                source.remove();
            }
            if let Err(err) = config_watch.stop() {
                tracing::warn!("settings flush failed: {err}");
            }
            sync_config.borrow_mut().take();
            observers.borrow_mut().clear();
            shared.open_in_window.borrow_mut().take();
            let tabs = std::mem::take(&mut *pages.borrow_mut());
            for (_, views) in tabs {
                for view in views {
                    view.close();
                }
            }
            file_trees.borrow_mut().clear();
        })
    };
    {
        let shutdown = shutdown.clone();
        window.connect_destroy(move |_| shutdown());
    }
    let confirmed_quit = Rc::new(Cell::new(false));
    {
        let save_session = save_session.clone();
        let config = config.clone();
        let tab_view = tab_view.clone();
        let confirmed_quit = confirmed_quit.clone();
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
            shutdown();
            glib::Propagation::Proceed
        });
    }

    // A logout or `systemctl --user stop` sends SIGTERM, which bypasses
    // `close-request` and would silently lose the session; shut down cleanly.
    for signal in [nix::libc::SIGTERM, nix::libc::SIGINT] {
        let save_session = save_session.clone();
        let window_c = window.clone();
        let confirmed_quit = confirmed_quit.clone();
        let source = glib::unix_signal_add_local(signal, move || {
            confirmed_quit.set(true);
            save_session();
            window_c.close();
            glib::ControlFlow::Break
        });
        source_ids.borrow_mut().push(source);
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
                match &tab.kind {
                    TabKind::Browser { url } => {
                        let page = add_browser_tab(url.as_deref());
                        if let Some(title) = &tab.title {
                            set_tab_renamed(&page, true);
                            page.set_title(title);
                        }
                    }
                    TabKind::Terminal => {
                        add_tab_layout(tab.title.clone(), &tab.layout)?;
                    }
                }
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
        let title = tab_is_renamed(&page).then(|| page.title().to_string());
        if is_browser_tab(&page) {
            let url = browser::current_uri(&page.child());
            tabs.push(TabState {
                title,
                layout: PaneLayout::default(),
                kind: TabKind::Browser { url },
            });
            continue;
        }
        let Some((_, views)) = pages.borrow().iter().find(|(p, _)| p == &page).cloned() else {
            continue;
        };
        let Some(layout) = capture_pane_layout(&page.child(), &views) else {
            continue;
        };
        tabs.push(TabState {
            title,
            layout,
            kind: TabKind::Terminal,
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
fn capture_pane_layout(widget: &gtk4::Widget, views: &[Rc<TerminalView>]) -> Option<PaneLayout> {
    if let Some(view) = views
        .iter()
        .find(|v| v.widget().upcast_ref::<gtk4::Widget>() == widget)
    {
        return Some(PaneLayout::Leaf { cwd: view.pwd() });
    }
    if !views.iter().any(|v| v.widget().is_ancestor(widget)) {
        return None;
    }
    if let Some(paned) = widget.downcast_ref::<gtk4::Paned>() {
        let start = paned
            .start_child()
            .and_then(|c| capture_pane_layout(&c, views));
        let end = paned
            .end_child()
            .and_then(|c| capture_pane_layout(&c, views));
        match (start, end) {
            (Some(start), Some(end)) => {
                let orientation = match paned.orientation() {
                    gtk4::Orientation::Horizontal => SplitOrientation::Horizontal,
                    _ => SplitOrientation::Vertical,
                };
                let ratio = visible_split_ratio(paned)
                    .or_else(|| unsafe {
                        paned
                            .data::<Rc<Cell<f64>>>(SPLIT_RATIO_KEY)
                            .map(|value| value.as_ref().get())
                    })
                    .unwrap_or(0.5);
                Some(PaneLayout::Split {
                    orientation,
                    ratio,
                    start: Box::new(start),
                    end: Box::new(end),
                })
            }
            (start, end) => start.or(end),
        }
    } else {
        widget
            .first_child()
            .and_then(|child| capture_pane_layout(&child, views))
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
            // A saved cwd may point at a directory that no longer exists (e.g.
            // an unmounted volume). Fall back to the shell's launch dir instead
            // of silently opening in `$HOME`, and surface the drop.
            let cwd = cwd.as_deref().filter(|p| Path::new(p).is_dir());
            if cwd.is_none() {
                tracing::debug!("restoring tab with a missing cwd; using launch cwd");
            }
            let view = make_view(page_slot.clone(), cwd.map(PathBuf::from), None)?;
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
            track_split_geometry(&paned, *ratio);
            paned.set_wide_handle(true);
            paned.set_resize_start_child(true);
            paned.set_resize_end_child(true);
            paned.set_shrink_start_child(false);
            paned.set_shrink_end_child(false);
            paned.set_start_child(Some(&start_w));
            paned.set_end_child(Some(&end_w));
            let ratio = (*ratio).clamp(0.05, 0.95);
            // Apply the saved divider once the paned has a real size.
            paned.add_tick_callback(move |p, _| {
                let size = match p.orientation() {
                    gtk4::Orientation::Horizontal => p.width(),
                    _ => p.height(),
                };
                if size <= 0 {
                    return glib::ControlFlow::Continue;
                }
                p.set_position((size as f64 * ratio) as i32);
                glib::ControlFlow::Break
            });
            start_views.extend(end_views);
            Ok((paned.upcast(), start_views))
        }
    }
}

fn window_css(window: &adw::ApplicationWindow, provider: &gtk4::CssProvider, priority: u32) {
    if let Some(display) = gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(&display, provider, priority);
        let provider = provider.clone();
        window.connect_destroy(move |_| {
            gtk4::style_context_remove_provider_for_display(&display, &provider)
        });
    }
}

fn chrome_font_css(font: &str, dpi_scale: f64, text_scale: f64) -> String {
    let font = gtk4::pango::FontDescription::from_string(font);
    let family = font
        .family()
        .unwrap_or_else(|| "Sans".into())
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    let size = (font.size() as f64 / gtk4::pango::SCALE as f64).max(1.0);
    let extra_scale = if (dpi_scale - text_scale).abs() < 0.01 {
        1.0
    } else {
        text_scale
    };
    format!(
        ".option-chrome {{ font-family: \"{family}\"; font-size: {}pt; }}",
        size * extra_scale
    )
}

/// Honor `gtk-decoration-layout`, chrome font and text scaling.
fn apply_desktop_chrome(header: &adw::HeaderBar, window: &adw::ApplicationWindow) {
    let Some(settings) = gtk4::Settings::default() else {
        return;
    };

    let mut handlers: Vec<(glib::Object, glib::SignalHandlerId)> = Vec::new();
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
        let handler = settings.connect_gtk_decoration_layout_notify(move |_| apply_decoration());
        handlers.push((settings.clone().upcast(), handler));
    }

    let provider = gtk4::CssProvider::new();
    if let Some(display) = gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION + 2,
        );
    }
    // GNOME's text-scaling-factor (org.gnome.desktop.interface) is what the
    // system uses to scale UI text; honoring it keeps our chrome in step.
    let gnome_interface = gio::SettingsSchemaSource::default()
        .and_then(|source| source.lookup("org.gnome.desktop.interface", true))
        .filter(|schema| schema.has_key("text-scaling-factor"))
        .map(|schema| gio::Settings::new_full(&schema, None::<&gio::SettingsBackend>, None));
    let apply_font = {
        let settings = settings.clone();
        let provider = provider.clone();
        let gnome_interface = gnome_interface.clone();
        Rc::new(move || {
            let font = settings.gtk_font_name().unwrap_or_else(|| "Sans 10".into());
            let dpi_scale = settings.gtk_xft_dpi() as f64 / 1024.0 / 96.0;
            let dpi_scale = if dpi_scale <= 0.0 { 1.0 } else { dpi_scale };
            // Multiply the DPI-derived scale by the GNOME text-scaling-factor.
            // `value()` returns a Variant without panicking; `get::<f64>()` is
            // fallible for a missing schema/key on non-GNOME desktops.
            let text_scale = gnome_interface
                .as_ref()
                .and_then(|settings| settings.value("text-scaling-factor").get::<f64>())
                .filter(|value| value.is_finite() && *value > 0.0)
                .unwrap_or(1.0);
            let css = chrome_font_css(&font, dpi_scale, text_scale);
            provider.load_from_string(&css);
        })
    };
    apply_font();
    {
        let apply_font = apply_font.clone();
        let handler = settings.connect_gtk_font_name_notify(move |_| apply_font());
        handlers.push((settings.clone().upcast(), handler));
    }
    {
        let apply_font = apply_font.clone();
        let handler = settings.connect_gtk_xft_dpi_notify(move |_| apply_font());
        handlers.push((settings.clone().upcast(), handler));
    }
    if let Some(gnome_interface) = gnome_interface {
        let apply_font = apply_font.clone();
        let handler = gnome_interface.connect_changed(Some("text-scaling-factor"), move |_, _| {
            apply_font();
        });
        handlers.push((gnome_interface.upcast(), handler));
    }
    let handlers = RefCell::new(handlers);
    window.connect_destroy(move |_| {
        for (object, handler) in handlers.borrow_mut().drain(..) {
            object.disconnect(handler);
        }
        if let Some(display) = gdk::Display::default() {
            gtk4::style_context_remove_provider_for_display(&display, &provider);
        }
    });
    // Animations: leave GtkSettings alone so gtk-enable-animations is honored.
}

/// Leaf-weighted split equalization, :
/// each divider is placed proportionally to the terminal count on each side.
fn equalize_splits(widget: &gtk4::Widget) -> i32 {
    let Some(paned) = widget.downcast_ref::<gtk4::Paned>() else {
        return 1;
    };
    if !paned.has_css_class("terminal-split") {
        return paned
            .start_child()
            .map(|child| equalize_splits(&child))
            .unwrap_or(0);
    }
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

/// Render a path for display, abbreviating the home dir to `~`.
fn abbreviate_home(path: &str) -> String {
    if let Some(home) = dirs::home_dir()
        && let Some(rest) = path.strip_prefix(&home.to_string_lossy().into_owned())
    {
        return if rest.is_empty() {
            "~".to_string()
        } else {
            format!("~{}", rest)
        };
    }
    path.to_string()
}

fn bind_sidebar(
    list: &gtk4::ListBox,
    view: &adw::TabView,
    syncing: Rc<Cell<bool>>,
) -> Rc<dyn Fn()> {
    let model = view.pages();
    let weak_view = view.downgrade();
    list.bind_model(Some(&model), move |item| {
        let page = item.downcast_ref::<adw::TabPage>().expect("TabView page");
        make_sidebar_row(&weak_view, page).upcast()
    });
    let list = list.downgrade();
    let view = view.downgrade();
    let sync: Rc<dyn Fn()> = Rc::new(move || {
        let (Some(list), Some(view)) = (list.upgrade(), view.upgrade()) else {
            return;
        };
        syncing.set(true);
        let row = view.selected_page().and_then(|page| {
            (0..view.n_pages())
                .find(|&i| view.nth_page(i) == page)
                .and_then(|i| list.row_at_index(i))
        });
        list.select_row(row.as_ref());
        syncing.set(false);
    });
    let update = sync.clone();
    model.connect_items_changed(move |_, _, _, _| update());
    sync();
    sync
}

fn make_sidebar_row(view: &glib::WeakRef<adw::TabView>, page: &adw::TabPage) -> adw::ActionRow {
    let row = adw::ActionRow::builder().activatable(true).build();
    row.set_use_markup(false);

    // Agent tabs carry a real logo icon (set on the TabPage); mirror
    // it in the sidebar row so the sidebar is as recognizable as the
    // tab bar.
    let image = gtk4::Image::new();
    image.set_icon_size(gtk4::IconSize::Normal);
    page.bind_property("icon", &image, "gicon")
        .sync_create()
        .build();
    page.bind_property("icon", &image, "visible")
        .transform_to(|_, icon: Option<gio::Icon>| Some(icon.is_some()))
        .sync_create()
        .build();
    row.add_prefix(&image);

    let close = gtk4::Button::from_icon_name("window-close-symbolic");
    close.add_css_class("flat");
    close.set_valign(gtk4::Align::Center);
    close.set_tooltip_text(Some("Close tab"));
    {
        let view = view.clone();
        let page = page.downgrade();
        close.connect_clicked(move |_| {
            if let (Some(view), Some(page)) = (view.upgrade(), page.upgrade()) {
                view.close_page(&page);
            }
        });
    }
    row.add_suffix(&close);

    // Double-click a row to rename the tab.
    {
        let page = page.downgrade();
        let row_weak = row.downgrade();
        let gesture = gtk4::GestureClick::new();
        gesture.set_button(gdk::BUTTON_PRIMARY);
        gesture.connect_pressed(move |_, n, _, _| {
            if n == 2
                && let (Some(row), Some(page)) = (row_weak.upgrade(), page.upgrade())
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
        let page = page.downgrade();
        source.connect_prepare(move |_, _, _| {
            Some(gdk::ContentProvider::for_value(&page.upgrade()?.to_value()))
        });
        row.add_controller(source);
    }
    {
        let target = gtk4::DropTarget::new(adw::TabPage::static_type(), gdk::DragAction::MOVE);
        let view = view.clone();
        let destination = page.downgrade();
        target.connect_drop(move |_, value, _, _| {
            let (Some(view), Some(destination), Ok(page)) = (
                view.upgrade(),
                destination.upgrade(),
                value.get::<adw::TabPage>(),
            ) else {
                return false;
            };
            reorder_sidebar_page(&view, &page, &destination)
        });
        row.add_controller(target);
    }

    // Keep the row title in sync without keeping the row alive.
    page.bind_property("title", &row, "title")
        .sync_create()
        .build();
    row
}

fn reorder_sidebar_page(
    view: &adw::TabView,
    page: &adw::TabPage,
    destination: &adw::TabPage,
) -> bool {
    if page == destination || !(0..view.n_pages()).any(|i| view.nth_page(i) == *page) {
        return false;
    }
    let Some(index) = (0..view.n_pages()).find(|&i| view.nth_page(i) == *destination) else {
        return false;
    };
    view.reorder_page(page, index);
    true
}

fn visible_split_ratio(paned: &gtk4::Paned) -> Option<f64> {
    if !paned.is_mapped() || !paned.start_child()?.is_visible() || !paned.end_child()?.is_visible()
    {
        return None;
    }
    let size = match paned.orientation() {
        gtk4::Orientation::Horizontal => paned.width(),
        _ => paned.height(),
    };
    (size > 0).then(|| (paned.position() as f64 / size as f64).clamp(0.05, 0.95))
}

fn track_split_geometry(paned: &gtk4::Paned, initial_ratio: f64) {
    paned.add_css_class("terminal-split");
    let ratio = Rc::new(Cell::new(initial_ratio.clamp(0.05, 0.95)));
    unsafe {
        paned.set_data(SPLIT_RATIO_KEY, ratio.clone());
    }
    paned.connect_position_notify(move |paned| {
        if let Some(value) = visible_split_ratio(paned) {
            ratio.set(value);
        }
    });
}

fn zoom_step(config: &RefCell<Config>, apply: &dyn Fn(f32), delta: f32) {
    let size = config.borrow().font_size;
    apply((size + delta).clamp(6.0, 40.0));
}

fn request_tab_close(tv: &adw::TabView, page: &adw::TabPage) {
    tv.close_page(page);
}

fn release_tab(pages: &Pages, trees: &FileTrees, page: &adw::TabPage) {
    let removed = {
        let mut pages = pages.borrow_mut();
        pages
            .iter()
            .position(|(p, _)| p == page)
            .map(|i| pages.remove(i))
    };
    if let Some((_, views)) = removed {
        for view in views {
            view.close();
        }
    }
    trees.borrow_mut().remove(page);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isolated_window_restores_order_and_saves_on_sigterm() {
        if std::env::var_os("OPTIONTERM_WINDOW_TEST").is_none() {
            let dir = crate::test_support::TestDir::new("window-session");
            let state_dir = dir.path().join(".option/terminal");
            let config = Config {
                source: state_dir.join("config.toml"),
                new_tab_position: NewTabPosition::Start,
                confirm_quit: true,
                ..Config::default()
            };
            config.save().unwrap();
            let session = SessionState {
                tabs: ["first", "second"]
                    .into_iter()
                    .map(|title| TabState {
                        title: Some(title.into()),
                        layout: PaneLayout::Leaf {
                            cwd: Some("/tmp".into()),
                        },
                        kind: TabKind::Terminal,
                    })
                    .collect(),
                active: 1,
                ..SessionState::default()
            };
            crate::storage::atomic_write(
                &state_dir.join("session.toml"),
                session.to_toml().as_bytes(),
            )
            .unwrap();
            let result = std::process::Command::new("/usr/bin/dbus-run-session")
                .arg("--dbus-daemon=/usr/bin/dbus-daemon")
                .arg("--")
                .arg(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "app::tests::isolated_window_restores_order_and_saves_on_sigterm",
                    "--nocapture",
                ])
                .env("OPTIONTERM_WINDOW_TEST", dir.path())
                .env("OPTION_HOME", dir.path().join(".option"))
                .env("HOME", dir.path())
                .env("XDG_CONFIG_HOME", dir.path().join("config"))
                .env("XDG_DATA_HOME", dir.path().join("data"))
                .env("XDG_CACHE_HOME", dir.path().join("cache"))
                .env("XDG_STATE_HOME", dir.path().join("state"))
                .env("PATH", dir.path())
                .env("SHELL", "/bin/cat")
                .env("GSETTINGS_BACKEND", "memory")
                .env("GTK_A11Y", "none")
                .env("GTK_USE_PORTAL", "0")
                .env("GIO_USE_VFS", "local")
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            );
            return;
        }
        let home = PathBuf::from(std::env::var_os("OPTIONTERM_WINDOW_TEST").unwrap());
        assert_eq!(crate::config::config_dir(), home.join(".option/terminal"));
        gtk4::init().unwrap();
        adw::init().unwrap();
        let app = adw::Application::builder()
            .application_id("io.option.terminal.test")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(gio::Cancellable::NONE).unwrap();
        let shared = Rc::new(SharedLaunch {
            pending: RefCell::new(Vec::new()),
            open_in_window: RefCell::new(None),
        });
        build_window(&app, shared, LaunchRequest::default()).unwrap();
        let window = app.windows().into_iter().next().unwrap();
        gtk4::prelude::WidgetExt::activate_action(&window, "win.zoom-in", None).unwrap();
        gtk4::prelude::WidgetExt::activate_action(&window, "win.zoom-out", None).unwrap();
        let context = glib::MainContext::default();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while window.width() == 0 && std::time::Instant::now() < deadline {
            context.iteration(false);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(window.width() > 0);
        nix::sys::signal::kill(nix::unistd::getpid(), nix::sys::signal::Signal::SIGTERM).unwrap();
        while !app.windows().is_empty() && std::time::Instant::now() < deadline {
            context.iteration(false);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(
            app.windows().is_empty(),
            "SIGTERM must not open a confirmation dialog"
        );
        let session = SessionState::load().unwrap();
        assert_eq!(session.active, 1);
        assert_eq!(
            session
                .tabs
                .iter()
                .map(|tab| tab.title.as_deref().unwrap())
                .collect::<Vec<_>>(),
            ["first", "second"]
        );
        assert!(
            session
                .tabs
                .iter()
                .all(|tab| tab.panes() == vec![Some("/tmp".into())])
        );
    }

    #[gtk4::test]
    fn sidebar_keeps_rows_when_tabs_are_added_and_selection_is_synced() {
        let view = adw::TabView::new();
        let list = gtk4::ListBox::new();
        let sync = bind_sidebar(&list, &view, Rc::new(Cell::new(false)));
        let first = view.append(&gtk4::Box::new(gtk4::Orientation::Vertical, 0));
        let row = list.row_at_index(0).unwrap();
        for _ in 0..30 {
            let page = view.append(&gtk4::Box::new(gtk4::Orientation::Vertical, 0));
            sync();
            assert_eq!(list.row_at_index(0).as_ref(), Some(&row));
            view.close_page(&page);
        }
        first.set_title("literal <b>title</b>");
        let row = row.downcast::<adw::ActionRow>().unwrap();
        assert_eq!(row.title(), "literal <b>title</b>");
        assert!(!row.uses_markup());
        assert_eq!(list.selected_row().as_ref(), Some(row.upcast_ref()));
    }

    #[gtk4::test]
    fn sidebar_drag_uses_page_identity_after_reordering() {
        let view = adw::TabView::new();
        let first = view.append(&gtk4::Box::new(gtk4::Orientation::Vertical, 0));
        let second = view.append(&gtk4::Box::new(gtk4::Orientation::Vertical, 0));
        let third = view.append(&gtk4::Box::new(gtk4::Orientation::Vertical, 0));
        view.reorder_page(&third, 0);
        assert!(reorder_sidebar_page(&view, &first, &second));
        assert_eq!(view.nth_page(2), first);
        assert!(!reorder_sidebar_page(&view, &first, &first));
        let other = adw::TabView::new();
        let foreign = other.append(&gtk4::Box::new(gtk4::Orientation::Vertical, 0));
        assert!(!reorder_sidebar_page(&view, &foreign, &second));
    }

    #[gtk4::test]
    fn sidebar_property_bindings_do_not_retain_rows() {
        let view = adw::TabView::new();
        let page = view.append(&gtk4::Box::new(gtk4::Orientation::Vertical, 0));
        for _ in 0..100 {
            let row = make_sidebar_row(&view.downgrade(), &page);
            page.set_icon(Some(&gio::ThemedIcon::new("utilities-terminal")));
            let weak = row.downgrade();
            drop(row);
            assert!(weak.upgrade().is_none());
        }
    }

    #[gtk4::test]
    fn desktop_css_is_valid_and_does_not_double_text_scaling() {
        let css = chrome_font_css("Sans 11", 1.25, 1.25);
        assert!(css.contains("11pt"));
        let provider = gtk4::CssProvider::new();
        let errors = Rc::new(Cell::new(0));
        let count = errors.clone();
        provider.connect_parsing_error(move |_, _, _| count.set(count.get() + 1));
        provider.load_from_string(&css);
        assert_eq!(errors.get(), 0);
        assert!(chrome_font_css("Sans 11", 1.0, 1.25).contains("13.75pt"));
    }

    #[test]
    fn zoom_releases_config_before_applying() {
        let config = RefCell::new(Config::default());
        zoom_step(&config, &|size| config.borrow_mut().font_size = size, 1.0);
        assert_eq!(config.borrow().font_size, 14.0);
    }

    #[gtk4::test]
    fn cancelled_close_preserves_page_state() {
        let tv = adw::TabView::new();
        let page = tv.append(&gtk4::Box::new(gtk4::Orientation::Vertical, 0));
        let pages: Pages = Rc::new(RefCell::new(vec![(page.clone(), Vec::new())]));
        tv.connect_close_page(|_, _| glib::Propagation::Stop);
        request_tab_close(&tv, &page);
        assert_eq!(pages.borrow().len(), 1);
        tv.close_page_finish(&page, false);
        assert_eq!(tv.n_pages(), 1);
        assert_eq!(pages.borrow()[0].0, page);
    }

    #[gtk4::test]
    fn releasing_a_tab_breaks_callback_ownership_cycles() {
        let tv = adw::TabView::new();
        let view = Rc::new(TerminalView::new(Config::default(), None, None).unwrap());
        let weak = Rc::downgrade(&view);
        let page = tv.append(view.widget());
        let pages: Pages = Rc::new(RefCell::new(vec![(page.clone(), vec![view.clone()])]));
        let owned_pages = pages.clone();
        view.set_on_exit(move || {
            let _ = owned_pages.borrow().len();
        });
        drop(view);
        let trees: FileTrees = Rc::new(RefCell::new(std::collections::HashMap::new()));
        release_tab(&pages, &trees, &page);
        release_tab(&pages, &trees, &page);
        assert!(pages.borrow().is_empty());
        assert!(weak.upgrade().is_none());
        tv.close_page(&page);
    }

    #[gtk4::test]
    fn repeated_restoration_preserves_only_terminal_leaves() {
        let initial = PaneLayout::Split {
            orientation: SplitOrientation::Vertical,
            ratio: 0.33,
            start: Box::new(PaneLayout::Leaf {
                cwd: Some("/tmp".into()),
            }),
            end: Box::new(PaneLayout::Leaf {
                cwd: Some("/".into()),
            }),
        };
        let make_view: MakeViewFn = Rc::new(|_, cwd, command| {
            Ok(Rc::new(TerminalView::new(Config::default(), cwd, command)?))
        });
        let mut layout = initial.clone();
        for _ in 0..5 {
            let slot = Rc::new(RefCell::new(None));
            let (terminals, views) = build_layout_widget(&layout, &make_view, &slot).unwrap();
            let container = gtk4::Paned::new(gtk4::Orientation::Horizontal);
            container.set_start_child(Some(&terminals));
            container.set_end_child(Some(&gtk4::Box::new(gtk4::Orientation::Vertical, 0)));
            layout = capture_pane_layout(container.upcast_ref(), &views).unwrap();
            assert_eq!(layout, initial);
            assert_eq!(equalize_splits(container.upcast_ref()), 2);
        }
    }

    #[gtk4::test]
    fn session_capture_does_not_turn_file_tree_into_terminal() {
        let view = Rc::new(TerminalView::new(Config::default(), None, None).unwrap());
        let root = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        let paned = gtk4::Paned::new(gtk4::Orientation::Horizontal);
        paned.set_start_child(Some(view.widget()));
        paned.set_end_child(Some(&gtk4::Box::new(gtk4::Orientation::Vertical, 0)));
        root.append(&paned);
        let layout = capture_pane_layout(root.upcast_ref(), &[view]).unwrap();
        assert_eq!(layout.leaf_cwds().len(), 1);
    }
}
