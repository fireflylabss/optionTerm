//! Terminal surface: thin wrapper around `vte4::Terminal`.

use std::{
    cell::{Cell, RefCell},
    os::fd::AsRawFd,
    path::PathBuf,
    rc::Rc,
};

use anyhow::Result;
use gtk4::{
    EventControllerKey, GestureClick, Overlay, ScrolledWindow,
    gdk::{self, RGBA},
    gio, glib,
    pango::FontDescription,
    prelude::*,
};
use vte4::{
    CursorBlinkMode, CursorShape, Format, PtyFlags, Regex, Terminal as VteTerminal, prelude::*,
};

// FFI for the kitty graphics protocol support added by the patched VTE
// (vte-fork/patches/kitty-graphics.patch). Not exposed by the stock vte4 crate.
unsafe extern "C" {
    fn vte_terminal_set_enable_inline_images(
        terminal: *mut vte4::ffi::VteTerminal,
        enabled: glib::ffi::gboolean,
    );
}
#[allow(unsafe_op_in_unsafe_fn)]
unsafe fn enable_inline_images(terminal: &VteTerminal) {
    unsafe {
        vte_terminal_set_enable_inline_images(terminal.as_ptr(), glib::ffi::GTRUE);
    }
}

use crate::{
    config::{Config, CursorStyle, RgbColor},
    pty,
};

/// Pane root: Overlay so split collapse/bounds in `app.rs` keep working.
pub struct TerminalView {
    overlay: Overlay,
    terminal: VteTerminal,
    config: Rc<RefCell<Config>>,
    child_pid: Rc<Cell<i32>>,
    cwd: Rc<RefCell<Option<PathBuf>>>,
    /// Optional one-shot command argv (instead of the login shell).
    command: Option<Vec<String>>,
    title: Rc<RefCell<String>>,
    on_title: Rc<RefCell<Option<Box<dyn Fn(String)>>>>,
    on_exit: Rc<RefCell<Option<Box<dyn Fn()>>>>,
    on_focus: Rc<RefCell<Option<Box<dyn Fn()>>>>,
    on_resize: Rc<RefCell<Option<Box<dyn Fn(u16, u16)>>>>,
    on_link: Rc<RefCell<Option<Box<dyn Fn(String)>>>>,
    scroll_btn: gtk4::Button,
}

impl TerminalView {
    pub fn new(config: Config, cwd: Option<PathBuf>, command: Option<Vec<String>>) -> Result<Self> {
        let terminal = VteTerminal::new();
        terminal.set_hexpand(true);
        terminal.set_vexpand(true);
        terminal.add_css_class("terminal");
        terminal.set_allow_hyperlink(true);
        terminal.set_enable_fallback_scrolling(false);
        terminal.set_scroll_unit_is_pixels(true);
        terminal.set_scrollback_lines(config.scroll_lines);
        terminal.search_set_wrap_around(true);
        // Patched VTE: kitty graphics protocol (inline images).
        unsafe {
            enable_inline_images(&terminal);
        }

        let url_regexes = install_url_matches(&terminal);
        apply_visuals(&terminal, &config);

        let scroll = ScrolledWindow::builder()
            .child(&terminal)
            .hexpand(true)
            .vexpand(true)
            .propagate_natural_width(false)
            .propagate_natural_height(false)
            .build();
        scroll.add_css_class("terminal-surface");
        // The terminal is the pane surface, not a framed document. Removing the
        // ScrolledWindow frame prevents a one-pixel inset around every pane.
        scroll.set_has_frame(false);
        // VTE is already a Scrollable; wrapping still gives us a themed bar
        // when the config asks for one.
        scroll.set_policy(
            gtk4::PolicyType::Never,
            if config.scroll_bar {
                gtk4::PolicyType::Automatic
            } else {
                gtk4::PolicyType::Never
            },
        );

        let scroll_btn = gtk4::Button::from_icon_name("go-bottom-symbolic");
        scroll_btn.set_valign(gtk4::Align::End);
        scroll_btn.set_halign(gtk4::Align::End);
        scroll_btn.set_margin_end(12);
        scroll_btn.set_margin_bottom(12);
        scroll_btn.add_css_class("circular");
        scroll_btn.add_css_class("osd");
        scroll_btn.set_tooltip_text(Some("Scroll to bottom"));
        scroll_btn.set_visible(false);

        let overlay = Overlay::new();
        overlay.add_css_class("terminal-surface");
        overlay.set_hexpand(true);
        overlay.set_vexpand(true);
        overlay.set_child(Some(&scroll));
        overlay.add_overlay(&scroll_btn);

        let config = Rc::new(RefCell::new(config));
        let child_pid = Rc::new(Cell::new(-1));
        let cwd = Rc::new(RefCell::new(cwd));
        let title = Rc::new(RefCell::new(String::from("Terminal")));
        let on_title: Rc<RefCell<Option<Box<dyn Fn(String)>>>> = Rc::new(RefCell::new(None));
        let on_exit: Rc<RefCell<Option<Box<dyn Fn()>>>> = Rc::new(RefCell::new(None));
        let on_focus: Rc<RefCell<Option<Box<dyn Fn()>>>> = Rc::new(RefCell::new(None));
        let on_resize: Rc<RefCell<Option<Box<dyn Fn(u16, u16)>>>> = Rc::new(RefCell::new(None));
        let on_link: Rc<RefCell<Option<Box<dyn Fn(String)>>>> = Rc::new(RefCell::new(None));
        let last_cols = Rc::new(Cell::new(0u16));
        let last_rows = Rc::new(Cell::new(0u16));

        {
            let title = title.clone();
            let on_title = on_title.clone();
            terminal.connect_window_title_changed(move |t| {
                let next = sanitize_title(t.window_title().as_deref().unwrap_or("Terminal"));
                *title.borrow_mut() = next.clone();
                if let Some(cb) = on_title.borrow().as_ref() {
                    cb(next);
                }
            });
        }

        {
            let on_exit = on_exit.clone();
            let child_pid = child_pid.clone();
            terminal.connect_child_exited(move |_, _status| {
                child_pid.set(-1);
                if let Some(cb) = on_exit.borrow().as_ref() {
                    cb();
                }
            });
        }

        {
            let config = config.clone();
            terminal.connect_bell(move |t| {
                if config.borrow().bell_sound {
                    t.set_audible_bell(true);
                }
            });
        }

        {
            let on_focus = on_focus.clone();
            let focus = gtk4::EventControllerFocus::new();
            focus.connect_enter(move |_| {
                if let Some(cb) = on_focus.borrow().as_ref() {
                    cb();
                }
            });
            terminal.add_controller(focus);
        }

        // Ctrl/Shift+click opens OSC-8 hyperlinks or URL regex matches.
        {
            let terminal_c = terminal.clone();
            let on_link = on_link.clone();
            let url_regexes = url_regexes.clone();
            let click = GestureClick::new();
            click.set_button(1);
            click.connect_pressed(move |gesture, _n, x, y| {
                let Some(event) = gesture.current_event() else {
                    return;
                };
                let mods = event.modifier_state();
                if !mods.contains(gdk::ModifierType::CONTROL_MASK)
                    && !mods.contains(gdk::ModifierType::SHIFT_MASK)
                {
                    return;
                }
                let uri = terminal_c
                    .hyperlink_hover_uri()
                    .map(|s| s.to_string())
                    .or_else(|| {
                        let refs: Vec<&Regex> = url_regexes.iter().collect();
                        terminal_c
                            .check_regex_simple_at(x, y, &refs, 0)
                            .into_iter()
                            .next()
                            .map(|s| s.to_string())
                    });
                if let Some(uri) = uri
                    && let Some(cb) = on_link.borrow().as_ref()
                {
                    cb(uri);
                }
            });
            terminal.add_controller(click);
        }

        // Swallow Accel keys that belong to the window (copy/paste etc. still
        // reach VTE; app actions handle the rest via application accelerators).
        {
            let key = EventControllerKey::new();
            key.set_propagation_phase(gtk4::PropagationPhase::Capture);
            key.connect_key_pressed(move |_, keyval, _, modifier| {
                if is_window_shortcut(keyval, modifier) {
                    return glib::Propagation::Proceed;
                }
                glib::Propagation::Proceed
            });
            terminal.add_controller(key);
        }

        {
            let on_resize = on_resize.clone();
            let last_cols = last_cols.clone();
            let last_rows = last_rows.clone();
            let notify = {
                let terminal = terminal.clone();
                Rc::new(move || {
                    let cols = terminal.column_count().clamp(0, u16::MAX as i64) as u16;
                    let rows = terminal.row_count().clamp(0, u16::MAX as i64) as u16;
                    if cols == 0 || rows == 0 {
                        return;
                    }
                    if last_cols.get() == cols && last_rows.get() == rows {
                        return;
                    }
                    last_cols.set(cols);
                    last_rows.set(rows);
                    if let Some(cb) = on_resize.borrow().as_ref() {
                        cb(cols, rows);
                    }
                })
            };
            {
                let notify = notify.clone();
                terminal.connect_char_size_changed(move |_, _, _| notify());
            }
            {
                // VTE does not expose a grid-size signal; poll while mapped.
                let terminal = terminal.clone();
                let notify = notify.clone();
                glib::timeout_add_local(std::time::Duration::from_millis(250), move || {
                    if terminal.is_mapped() {
                        notify();
                    }
                    glib::ControlFlow::Continue
                });
            }
        }

        // Jump-to-bottom button: visible while scrolled up when enabled.
        {
            let btn = scroll_btn.clone();
            let config = config.clone();
            let term = terminal.clone();
            if let Some(adj) = gtk4::prelude::ScrollableExt::vadjustment(&terminal) {
                let sync = {
                    let btn = btn.clone();
                    let config = config.clone();
                    let adj = adj.clone();
                    Rc::new(move || {
                        let show = config.borrow().scroll_button
                            && adj.upper() - adj.page_size() - adj.value() > 1.0;
                        btn.set_visible(show);
                    })
                };
                {
                    let sync = sync.clone();
                    adj.connect_value_changed(move |_| sync());
                }
                {
                    let sync = sync.clone();
                    adj.connect_upper_notify(move |_| sync());
                }
                btn.connect_clicked(move |_| {
                    if let Some(adj) = gtk4::prelude::ScrollableExt::vadjustment(&term) {
                        adj.set_value(adj.upper() - adj.page_size());
                    }
                });
            }
        }

        let view = Self {
            overlay,
            terminal,
            config,
            child_pid,
            cwd,
            command,
            title,
            on_title,
            on_exit,
            on_focus,
            on_resize,
            on_link,
            scroll_btn,
        };
        view.spawn_process();
        Ok(view)
    }

    pub fn widget(&self) -> &Overlay {
        &self.overlay
    }

    pub fn focus(&self) {
        self.terminal.grab_focus();
    }

    pub fn title(&self) -> String {
        self.title.borrow().clone()
    }

    pub fn set_on_title_changed(&self, cb: impl Fn(String) + 'static) {
        *self.on_title.borrow_mut() = Some(Box::new(cb));
    }

    pub fn set_on_exit(&self, cb: impl Fn() + 'static) {
        *self.on_exit.borrow_mut() = Some(Box::new(cb));
    }

    pub fn set_on_focus(&self, cb: impl Fn() + 'static) {
        *self.on_focus.borrow_mut() = Some(Box::new(cb));
    }

    pub fn set_on_resize(&self, cb: impl Fn(u16, u16) + 'static) {
        *self.on_resize.borrow_mut() = Some(Box::new(cb));
    }

    pub fn set_on_link(&self, cb: impl Fn(String) + 'static) {
        *self.on_link.borrow_mut() = Some(Box::new(cb));
    }

    pub fn update_config(&self, f: impl FnOnce(&mut Config)) {
        f(&mut self.config.borrow_mut());
        apply_visuals(&self.terminal, &self.config.borrow());
        self.sync_scroll_chrome();
    }

    pub fn apply_config(&self, config: &Config) {
        *self.config.borrow_mut() = config.clone();
        apply_visuals(&self.terminal, config);
        self.sync_scroll_chrome();
    }

    pub fn selection_text(&self) -> Option<String> {
        if !self.terminal.has_selection() {
            return None;
        }
        self.terminal
            .text_selected(Format::Text)
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty())
    }

    pub fn paste(&self, text: &str) {
        self.terminal.paste_text(text);
    }

    pub fn select_all(&self) {
        self.terminal.select_all();
    }

    pub fn set_font_size(&self, size: f32) -> f32 {
        let size = size.clamp(6.0, 40.0);
        self.config.borrow_mut().font_size = size;
        apply_font(&self.terminal, &self.config.borrow());
        size
    }

    pub fn pwd(&self) -> Option<String> {
        if let Some(uri) = self.terminal.current_directory_uri() {
            let path = pwd_to_path(&uri);
            if !path.is_empty() {
                return Some(path);
            }
        }
        let fd = self.pty_fd()?;
        pty::foreground_cwd(fd).map(|p| p.to_string_lossy().into_owned())
    }

    pub fn is_busy(&self) -> bool {
        let pid = self.child_pid.get();
        if pid <= 0 {
            return false;
        }
        let Some(fd) = self.pty_fd() else {
            return false;
        };
        pty::is_busy(fd, pid)
    }

    /// Install a case-insensitive search regex (empty clears).
    pub fn search_set_query(&self, query: &str) {
        let q = query.trim();
        if q.is_empty() {
            self.terminal.search_set_regex(None::<&Regex>, 0);
            return;
        }
        let pattern = regex_escape(q);
        // PCRE2_CASELESS | PCRE2_MULTILINE — VTE asserts multiline on search regexes.
        const PCRE2_CASELESS: u32 = 0x0000_0008;
        const PCRE2_MULTILINE: u32 = 0x0000_0400;
        match Regex::for_search(&pattern, PCRE2_CASELESS | PCRE2_MULTILINE) {
            Ok(re) => self.terminal.search_set_regex(Some(&re), 0),
            Err(err) => tracing::warn!("search regex: {err}"),
        }
    }

    pub fn search_find_next(&self) -> bool {
        self.terminal.search_find_next()
    }

    pub fn search_find_previous(&self) -> bool {
        self.terminal.search_find_previous()
    }

    pub fn clear_screen(&self) {
        self.terminal.reset(true, true);
    }

    fn sync_scroll_chrome(&self) {
        let cfg = self.config.borrow();
        if let Some(scroll) = self
            .overlay
            .child()
            .and_then(|w| w.downcast::<ScrolledWindow>().ok())
        {
            scroll.set_policy(
                gtk4::PolicyType::Never,
                if cfg.scroll_bar {
                    gtk4::PolicyType::Automatic
                } else {
                    gtk4::PolicyType::Never
                },
            );
        }
        if !cfg.scroll_button {
            self.scroll_btn.set_visible(false);
        }
        self.terminal
            .set_scroll_on_keystroke(cfg.scroll_on_keystroke);
        self.terminal.set_audible_bell(cfg.bell_sound);
        self.terminal.set_scrollback_lines(cfg.scroll_lines);
    }

    fn pty_fd(&self) -> Option<i32> {
        let pty = self.terminal.pty()?;
        Some(pty.fd().as_raw_fd())
    }

    pub fn restart(&self) {
        let pid = self.child_pid.get();
        if pid > 0 {
            let _ = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(pid),
                nix::sys::signal::Signal::SIGHUP,
            );
            self.child_pid.set(-1);
        }
        // Remember cwd before the child exits clears it.
        if let Some(pwd) = self.pwd() {
            *self.cwd.borrow_mut() = Some(PathBuf::from(pwd));
        }
        self.terminal.reset(true, true);
        self.spawn_process();
    }

    fn spawn_process(&self) {
        let shell = match std::env::var_os("SHELL") {
            Some(s) if !s.is_empty() => PathBuf::from(s),
            _ => match nix::unistd::User::from_uid(nix::unistd::getuid()) {
                Ok(Some(user)) => user.shell,
                _ => PathBuf::from("/bin/sh"),
            },
        };
        let shell_s = shell.to_string_lossy().into_owned();

        // Own the argv strings for the duration of spawn_async.
        let argv_owned: Vec<String> = if let Some(cmd) = &self.command {
            cmd.clone()
        } else {
            vec![shell_s]
        };
        let argv_refs: Vec<&str> = argv_owned.iter().map(String::as_str).collect();

        let env_term = "TERM=xterm-256color".to_string();
        let env_color = "COLORTERM=truecolor".to_string();
        let env_prog = "TERM_PROGRAM=optionTerm".to_string();
        let env_ver = format!("TERM_PROGRAM_VERSION={}", env!("CARGO_PKG_VERSION"));
        let envv = [
            env_term.as_str(),
            env_color.as_str(),
            env_prog.as_str(),
            env_ver.as_str(),
        ];

        let cwd_owned = self
            .cwd
            .borrow()
            .as_ref()
            .filter(|p| p.is_dir())
            .map(|p| p.to_string_lossy().into_owned());
        let cwd = cwd_owned.as_deref();

        let child_pid = self.child_pid.clone();
        self.terminal.spawn_async(
            PtyFlags::DEFAULT,
            cwd,
            &argv_refs,
            &envv,
            glib::SpawnFlags::DEFAULT,
            || {},
            -1,
            None::<&gio::Cancellable>,
            move |result| match result {
                Ok(pid) => child_pid.set(pid.0),
                Err(err) => {
                    tracing::error!("failed to spawn process: {err}");
                    child_pid.set(-1);
                }
            },
        );
    }
}

fn apply_visuals(terminal: &VteTerminal, config: &Config) {
    apply_font(terminal, config);
    apply_colors(terminal, config);
    apply_cursor(terminal, config);
    terminal.set_scroll_on_keystroke(config.scroll_on_keystroke);
    terminal.set_audible_bell(config.bell_sound);
    terminal.set_scrollback_lines(config.scroll_lines);
    terminal.set_enable_shaping(config.font_ligatures);

    let pad_x = config.padding_x.max(0.0);
    let pad_y = config.padding_y.max(0.0);
    terminal.set_margin_start(pad_x as i32);
    terminal.set_margin_end(pad_x as i32);
    terminal.set_margin_top(pad_y as i32);
    terminal.set_margin_bottom(pad_y as i32);
}

fn apply_font(terminal: &VteTerminal, config: &Config) {
    let family = if config.use_system_font {
        system_monospace()
    } else {
        config.font_family.clone()
    };
    let desc = FontDescription::from_string(&format!("{family} {}", config.font_size));
    terminal.set_font(Some(&desc));
}

fn apply_colors(terminal: &VteTerminal, config: &Config) {
    let fg = rgba(config.foreground, 1.0);
    let mut bg = rgba(config.background, config.background_opacity as f32);
    if config.background_opacity < 1.0 {
        bg.set_alpha(config.background_opacity as f32);
    }
    let palette: Vec<RGBA> = config.palette.iter().map(|c| rgba(*c, 1.0)).collect();
    let palette_refs: Vec<&RGBA> = palette.iter().collect();
    terminal.set_colors(Some(&fg), Some(&bg), &palette_refs);
    terminal.set_color_cursor(Some(&rgba(config.cursor, 1.0)));
    terminal.set_color_cursor_foreground(Some(&rgba(config.cursor_text, 1.0)));
    terminal.set_color_highlight(Some(&rgba(config.selection_background, 1.0)));
    terminal.set_color_highlight_foreground(Some(&rgba(config.selection_foreground, 1.0)));
}

fn apply_cursor(terminal: &VteTerminal, config: &Config) {
    let shape = match config.cursor_style {
        CursorStyle::Block | CursorStyle::BlockHollow => CursorShape::Block,
        CursorStyle::Bar => CursorShape::Ibeam,
        CursorStyle::Underline => CursorShape::Underline,
    };
    terminal.set_cursor_shape(shape);
    terminal.set_cursor_blink_mode(if config.cursor_blink {
        CursorBlinkMode::On
    } else {
        CursorBlinkMode::Off
    });
}

fn rgba(c: RgbColor, alpha: f32) -> RGBA {
    RGBA::new(
        c.r as f32 / 255.0,
        c.g as f32 / 255.0,
        c.b as f32 / 255.0,
        alpha.clamp(0.0, 1.0),
    )
}

fn install_url_matches(terminal: &VteTerminal) -> Rc<Vec<Regex>> {
    // VTE requires PCRE2_MULTILINE on match regexes (runtime assert).
    const PCRE2_MULTILINE: u32 = 0x0000_0400;
    const PATTERNS: &[&str] = &[
        r"https?://[[:alnum:][:punct:]]+",
        r"www\.[[:alnum:][:punct:]]+",
        r"mailto:[[:alnum:][:punct:]]+",
    ];
    let mut out = Vec::new();
    for pat in PATTERNS {
        match Regex::for_match(pat, PCRE2_MULTILINE) {
            Ok(re) => {
                let id = terminal.match_add_regex(&re, 0);
                terminal.match_set_cursor_name(id, "pointer");
                out.push(re);
            }
            Err(err) => tracing::debug!("url match regex failed: {err}"),
        }
    }
    Rc::new(out)
}

fn sanitize_title(title: &str) -> String {
    let t = title.trim();
    if t.is_empty() {
        "Terminal".into()
    } else {
        t.chars().take(120).collect()
    }
}

fn system_monospace() -> String {
    "monospace".into()
}

fn regex_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(
            c,
            '\\' | '.' | '+' | '*' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '^' | '$' | '|'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Keys handled by window actions — let them bubble.
fn is_window_shortcut(keyval: gdk::Key, modifier: gdk::ModifierType) -> bool {
    let ctrl = modifier.contains(gdk::ModifierType::CONTROL_MASK);
    let shift = modifier.contains(gdk::ModifierType::SHIFT_MASK);
    if ctrl && shift {
        matches!(
            keyval,
            gdk::Key::C
                | gdk::Key::V
                | gdk::Key::T
                | gdk::Key::W
                | gdk::Key::N
                | gdk::Key::F
                | gdk::Key::plus
                | gdk::Key::equal
                | gdk::Key::minus
                | gdk::Key::O
                | gdk::Key::E
                | gdk::Key::Return
        )
    } else {
        false
    }
}

/// Decode OSC 7 `file://host/path` into a plain filesystem path.
pub fn pwd_to_path(raw: &str) -> String {
    let s = raw.trim();
    if let Some(rest) = s.strip_prefix("file://") {
        let path = if let Some(idx) = rest.find('/') {
            // Skip authority (host).
            &rest[idx..]
        } else {
            rest
        };
        match urlencoding_decode(path) {
            Some(decoded) => decoded,
            None => path.to_string(),
        }
    } else {
        s.to_string()
    }
}

fn urlencoding_decode(path: &str) -> Option<String> {
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let h = std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?;
            let b = u8::from_str_radix(h, 16).ok()?;
            out.push(b);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Turn a screen word into an openable URI (http, mailto, existing paths).
#[allow(dead_code)]
pub fn detect_link(word: &str, pwd: Option<&str>) -> Option<String> {
    let w = word.trim_matches(|c: char| matches!(c, '.' | ',' | ';' | ':' | ')' | '(' | '[' | ']'));
    if w.is_empty() {
        return None;
    }
    if w.starts_with("https://") || w.starts_with("http://") || w.starts_with("mailto:") {
        return Some(w.to_string());
    }
    if w.starts_with("www.") {
        return Some(format!("https://{w}"));
    }
    let candidate = if w.starts_with('/') {
        PathBuf::from(w)
    } else if let Some(pwd) = pwd {
        PathBuf::from(pwd).join(w)
    } else {
        return None;
    };
    candidate.exists().then(|| {
        let path = candidate.clone();
        glib::filename_to_uri(candidate, None)
            .map(|u| u.to_string())
            .unwrap_or_else(|_| format!("file://{}", path.display()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pwd_to_path_strips_file_uri() {
        assert_eq!(pwd_to_path("file://host/home/me"), "/home/me");
        assert_eq!(pwd_to_path("file:///tmp/x"), "/tmp/x");
        assert_eq!(pwd_to_path("/plain"), "/plain");
    }

    #[test]
    fn detect_link_recognises_urls() {
        assert_eq!(
            detect_link("https://example.com/docs.", None).as_deref(),
            Some("https://example.com/docs")
        );
        assert_eq!(
            detect_link("www.example.com", None).as_deref(),
            Some("https://www.example.com")
        );
    }

    #[test]
    fn regex_escape_quotes_metachars() {
        assert_eq!(regex_escape("a+b"), r"a\+b");
        assert_eq!(regex_escape("plain"), "plain");
    }

    /// The kitty graphics protocol must be enabled by default on the patched
    /// VTE, and the APC handler must accept (not crash on) a query sequence.
    #[gtk4::test]
    fn kitty_graphics_handler_accepts_query() {
        let terminal = VteTerminal::new();
        unsafe {
            enable_inline_images(&terminal);
            // APC query: `ESC _ G i=1,q=1 ESC \`
            let seq = b"\x1b_Gi=1,q=1\x1b\\";
            terminal.feed(seq);
            terminal.feed(b"\x1b_Ga=T,f=100;AAAA\x1b\\");
            terminal.feed(b"\x1b_Ga=c\x1b\\");
        }
    }

    /// End-to-end: the patched VTE answers the kitty query (a=q) with an
    /// "a=OK" response written to the PTY master.
    #[gtk4::test]
    fn kitty_graphics_answers_query_on_pty() {
        use std::{
            io::Read,
            os::fd::{AsRawFd, FromRawFd},
        };

        let winsize = nix::pty::Winsize {
            ws_row: 24,
            ws_col: 80,
            ws_xpixel: 640,
            ws_ypixel: 384,
        };
        let (master, _child) = match unsafe { nix::pty::forkpty(&winsize, None) } {
            Ok(nix::pty::ForkptyResult::Parent { master, child }) => (master, child),
            Ok(nix::pty::ForkptyResult::Child) => std::process::exit(0),
            Err(e) => panic!("forkpty failed: {e}"),
        };

        // Own the master fd: VTE will write the APC reply into it.
        let mut master = unsafe { std::fs::File::from_raw_fd(master.as_raw_fd()) };

        // Wrap our master fd in a VTE Pty (vte_pty_new_foreign_sync).
        unsafe extern "C" {
            fn vte_pty_new_foreign_sync(
                fd: i32,
                cancellable: *mut std::ffi::c_void,
                error: *mut *mut glib::ffi::GError,
            ) -> *mut vte4::ffi::VtePty;
        }
        let pty: vte4::Pty = unsafe {
            let mut error = std::ptr::null_mut();
            let ptr =
                vte_pty_new_foreign_sync(master.as_raw_fd(), std::ptr::null_mut(), &mut error);
            assert!(!ptr.is_null(), "vte_pty_new_foreign_sync failed");
            glib::translate::from_glib_full(ptr)
        };

        let terminal = VteTerminal::new();
        terminal.set_pty(Some(&pty));
        unsafe {
            enable_inline_images(&terminal);
        }
        terminal.feed(b"\x1b_Ga=q,i=7,q=42\x1b\\");

        // Pump the main context so VTE flushes m_outgoing into the PTY.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut buf = [0u8; 512];
        let mut got = Vec::new();
        while std::time::Instant::now() < deadline {
            while glib::MainContext::default().pending() {
                glib::MainContext::default().iteration(false);
            }
            match master.read(&mut buf) {
                Ok(0) | Err(_) => {}
                Ok(n) => {
                    got.extend_from_slice(&buf[..n]);
                    if got.windows(4).any(|w| w == b"a=OK") {
                        break;
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        let s = String::from_utf8_lossy(&got);
        assert!(
            s.contains("a=OK"),
            "kitty query went unanswered; PTY output was: {s:?}"
        );
        // The patched VTE echoes the image id and hardcodes q=1 (protocol says
        // the reply must include the query id; the patch keeps it simple).
        assert!(s.contains("i=7"), "reply must echo the image id: {s:?}");
        assert!(s.contains("OK=1"), "reply must advertise OK=1: {s:?}");
    }

    /// The patched VTE must accept t=f (transmit from a file path), the
    /// medium optionFiles uses for previews. The APC handler runs without a
    /// PTY here; the query-on-pty test above proves the wire path end to end.
    #[gtk4::test]
    fn kitty_graphics_transmits_from_file() {
        // A real PNG to transmit: a 1x1 pixel, fixed base64.
        let png_path = std::env::temp_dir().join("optionterm-kitty-tf-test.png");
        let one_px_png = base64_decode(
            "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==",
        );
        std::fs::write(&png_path, &one_px_png).expect("write test png");

        let terminal = VteTerminal::new();
        unsafe {
            enable_inline_images(&terminal);
        }

        // Transmit by file path (optionFiles style: I=, p=, C=1, q=1),
        // then display and delete. Any error is swallowed by the handler,
        // so the test passes if the feed does not abort.
        let path_b64 = base64_encode(png_path.to_str().unwrap().as_bytes());
        let seq = format!(
            "\x1b_Ga=T,f=100,t=f,I=1,p=1,c=12,r=6,C=1,q=1;{path_b64}\x1b\\\
             \x1b_Ga=T,f=100,t=d,i=2,c=12,r=6,C=1,q=1;iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==\x1b\\\
             \x1b_Ga=d,d=N,I=1,q=1\x1b\\"
        );
        terminal.feed(seq.as_bytes());
    }

    fn base64_decode(s: &str) -> Vec<u8> {
        let mut out = Vec::with_capacity(s.len() / 4 * 3);
        let mut buf = [0u8; 4];
        let mut n = 0usize;
        for c in s.bytes() {
            let v = match c {
                b'A'..=b'Z' => c - b'A',
                b'a'..=b'z' => c - b'a' + 26,
                b'0'..=b'9' => c - b'0' + 52,
                b'+' => 62,
                b'/' => 63,
                _ => continue,
            };
            buf[n] = v;
            n += 1;
            if n == 4 {
                out.push((buf[0] << 2) | (buf[1] >> 4));
                out.push((buf[1] << 4) | (buf[2] >> 2));
                out.push((buf[2] << 6) | buf[3]);
                n = 0;
            }
        }
        if n >= 2 {
            out.push((buf[0] << 2) | (buf[1] >> 4));
            if n == 3 {
                out.push((buf[1] << 4) | (buf[2] >> 2));
            }
        }
        out
    }

    fn base64_encode(bytes: &[u8]) -> String {
        const TABLE: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
        for chunk in bytes.chunks(3) {
            let a = chunk[0];
            let b = chunk.get(1).copied().unwrap_or(0);
            let c = chunk.get(2).copied().unwrap_or(0);
            out.push(TABLE[(a >> 2) as usize] as char);
            out.push(TABLE[(((a & 0x03) << 4) | (b >> 4)) as usize] as char);
            out.push(if chunk.len() > 1 {
                TABLE[(((b & 0x0f) << 2) | (c >> 6)) as usize] as char
            } else {
                '='
            });
            out.push(if chunk.len() > 2 {
                TABLE[(c & 0x3f) as usize] as char
            } else {
                '='
            });
        }
        out
    }
}
