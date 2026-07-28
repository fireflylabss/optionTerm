//! Terminal surface: libghostty-vt + Cairo/Pango DrawingArea.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use anyhow::{Context, Result, anyhow};
use gtk4::{
    cairo, gdk, glib, pango,
    prelude::*,
    DrawingArea, EventControllerFocus, EventControllerKey, EventControllerScroll,
    EventControllerScrollFlags, GestureClick, IMMulticontext,
};
use libghostty_vt::{
    Terminal, TerminalOptions,
    fmt::{Formatter, FormatterOptions},
    paste,
    render::{CellIterator, CursorVisualStyle, RenderState, RowIterator},
    screen::TrackedGridRef,
    selection::{
        Selection,
        gesture::{DragEvent, Geometry, Gesture, PressEvent, ReleaseEvent},
    },
    style::{RgbColor, Underline},
    terminal::{
        ConformanceLevel, DeviceAttributeFeature, DeviceAttributes, DeviceType, Mode, ModeKind,
        Point, PointCoordinate, PointSpace, PrimaryDeviceAttributes, ScrollViewport,
        SecondaryDeviceAttributes, SizeReportSize,
    },
};

use crate::{
    config::{Config, CursorStyle},
    input::Input,
    pty::{self, Child, Pty, PtyError},
};

type TitleCb = Rc<dyn Fn(String)>;
type ExitCb = Rc<dyn Fn()>;
type FocusCb = Rc<dyn Fn()>;
type ResizeCb = Rc<dyn Fn(u16, u16)>;

pub struct TerminalView {
    area: DrawingArea,
    state: Rc<RefCell<Option<Session>>>,
    title_cb: Rc<RefCell<Option<TitleCb>>>,
    exit_cb: Rc<RefCell<Option<ExitCb>>>,
    focus_cb: Rc<RefCell<Option<FocusCb>>>,
    resize_cb: Rc<RefCell<Option<ResizeCb>>>,
    /// Set once the PTY fd source is installed; cleared on restart.
    watcher_attached: Rc<Cell<bool>>,
    /// Live PTY fd source, so a restart can drop it before closing the fd.
    watcher: Rc<RefCell<Option<glib::SourceId>>>,
}

struct Session {
    config: Config,
    terminal: Terminal<'static, 'static>,
    render_state: RenderState<'static>,
    row_it: RowIterator<'static>,
    cell_it: CellIterator<'static>,
    input: Input,
    pty: Pty,
    child: Child,
    cols: u16,
    rows: u16,
    cell_width: f64,
    cell_height: f64,
    surface_height: f64,
    title: String,
    blink_on: bool,
    blink_enabled: bool,
    fonts: Option<FontSet>,
    gesture: Gesture<'static>,
    sel_start: Option<TrackedGridRef>,
    sel_end: Option<TrackedGridRef>,
    sel_rectangle: bool,
}

impl TerminalView {
    pub fn new(config: Config) -> Result<Self> {
        let area = DrawingArea::new();
        area.set_hexpand(true);
        area.set_vexpand(true);
        area.set_focusable(true);
        area.set_can_focus(true);
        area.set_focus_on_click(true);
        area.add_css_class("terminal");

        let state: Rc<RefCell<Option<Session>>> = Rc::new(RefCell::new(None));
        let title_cb: Rc<RefCell<Option<TitleCb>>> = Rc::new(RefCell::new(None));
        let exit_cb: Rc<RefCell<Option<ExitCb>>> = Rc::new(RefCell::new(None));
        let focus_cb: Rc<RefCell<Option<FocusCb>>> = Rc::new(RefCell::new(None));
        let grid = Rc::new(Cell::new((80u16, 24u16)));
        let watcher_attached = Rc::new(Cell::new(false));
        let watcher: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));

        let resize_cb: Rc<RefCell<Option<ResizeCb>>> = Rc::new(RefCell::new(None));

        {
            let state = state.clone();
            let config = config.clone();
            let grid = grid.clone();
            let title_cb = title_cb.clone();
            let exit_cb = exit_cb.clone();
            let resize_cb = resize_cb.clone();
            let watcher_attached = watcher_attached.clone();
            let watcher = watcher.clone();
            area.set_draw_func(move |da, cr, width, height| {
                let mut just_bootstrapped = false;
                if state.borrow().is_none() {
                    match bootstrap_session(&config, &grid, width, height, da) {
                        Ok(session) => {
                            *state.borrow_mut() = Some(session);
                            just_bootstrapped = true;
                        }
                        Err(err) => {
                            tracing::error!("failed to start terminal session: {err:#}");
                            paint_error(cr, width, height, &err);
                            return;
                        }
                    }
                }
                if !watcher_attached.get() && state.borrow().is_some() {
                    *watcher.borrow_mut() =
                        attach_pty_watcher(da, &state, &title_cb, &exit_cb, &watcher);
                    watcher_attached.set(true);
                }
                let mut resized = None;
                if let Ok(mut borrow) = state.try_borrow_mut() {
                    if let Some(session) = borrow.as_mut() {
                        match session.ensure_size(da, width, height, &grid) {
                            Ok(changed) => resized = changed,
                            Err(err) => tracing::warn!("resize failed: {err:#}"),
                        }
                        if let Err(err) = session.paint(da, cr, width, height) {
                            tracing::warn!("paint failed: {err:#}");
                        }
                    }
                }
                // A freshly split/opened pane starts at its final size, so
                // report the initial grid too (Ghostty shows it as well).
                if just_bootstrapped && resized.is_none() {
                    resized = Some(grid.get());
                }
                if let Some((cols, rows)) = resized {
                    // Defer out of the draw cycle: adding widgets (toasts)
                    // while snapshotting triggers GTK allocation warnings.
                    let resize_cb = resize_cb.clone();
                    glib::idle_add_local_once(move || {
                        if let Some(cb) = resize_cb.borrow().as_ref() {
                            cb(cols, rows);
                        }
                    });
                }
            });
        }

        attach_input(&area, &state, &focus_cb);

        // Cursor blink timer. Stops on its own once the widget is destroyed.
        {
            let weak_area = glib::object::WeakRef::<DrawingArea>::new();
            weak_area.set(Some(&area));
            let state = state.clone();
            glib::timeout_add_local(std::time::Duration::from_millis(550), move || {
                let Some(area) = weak_area.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                if let Ok(mut borrow) = state.try_borrow_mut() {
                    if let Some(session) = borrow.as_mut() {
                        if session.blink_enabled {
                            session.blink_on = !session.blink_on;
                            area.queue_draw();
                        } else if !session.blink_on {
                            session.blink_on = true;
                            area.queue_draw();
                        }
                    }
                }
                glib::ControlFlow::Continue
            });
        }

        Ok(Self {
            area,
            state,
            title_cb,
            exit_cb,
            focus_cb,
            resize_cb,
            watcher_attached,
            watcher,
        })
    }

    pub fn widget(&self) -> &DrawingArea {
        &self.area
    }

    pub fn focus(&self) {
        self.area.grab_focus();
    }

    pub fn title(&self) -> String {
        self.state
            .borrow()
            .as_ref()
            .map(|s| s.title.clone())
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| "Terminal".into())
    }

    pub fn set_on_title_changed(&self, cb: impl Fn(String) + 'static) {
        *self.title_cb.borrow_mut() = Some(Rc::new(cb));
    }

    pub fn set_on_exit(&self, cb: impl Fn() + 'static) {
        *self.exit_cb.borrow_mut() = Some(Rc::new(cb));
    }

    /// Called whenever this terminal gains keyboard focus.
    pub fn set_on_focus(&self, cb: impl Fn() + 'static) {
        *self.focus_cb.borrow_mut() = Some(Rc::new(cb));
    }

    /// Called with the new grid size (cols, rows) after a resize.
    pub fn set_on_resize(&self, cb: impl Fn(u16, u16) + 'static) {
        *self.resize_cb.borrow_mut() = Some(Rc::new(cb));
    }

    /// Mutate the live session config (cursor style, blink, padding, ...).
    pub fn update_config(&self, f: impl FnOnce(&mut Config)) {
        if let Some(session) = self.state.borrow_mut().as_mut() {
            f(&mut session.config);
        }
        self.area.queue_draw();
    }

    /// Text of the current selection, if any.
    pub fn selection_text(&self) -> Option<String> {
        self.state
            .borrow_mut()
            .as_mut()
            .and_then(|s| s.selection_text())
    }

    /// Paste text into the terminal (honors bracketed paste mode).
    pub fn paste(&self, text: &str) {
        if let Some(session) = self.state.borrow_mut().as_mut() {
            session.paste_text(text);
        }
        self.area.queue_draw();
    }

    /// Select all terminal content.
    pub fn select_all(&self) {
        if let Some(session) = self.state.borrow_mut().as_mut() {
            session.select_all();
        }
        self.area.queue_draw();
    }

    /// Set the font size in points (zoom). Returns the clamped value.
    pub fn set_font_size(&self, size: f32) -> f32 {
        let size = size.clamp(6.0, 40.0);
        if let Some(session) = self.state.borrow_mut().as_mut() {
            session.config.font_size = size;
        }
        self.area.queue_draw();
        size
    }

    /// Clear the screen and the scrollback (Ghostty `clear_screen`).
    pub fn clear_screen(&self) {
        if let Some(session) = self.state.borrow_mut().as_mut() {
            // Home the cursor, erase the display, then drop the scrollback.
            session.terminal.vt_write(b"\x1b[H\x1b[2J\x1b[3J");
            session.clear_selection();
        }
        self.area.queue_draw();
    }

    /// Kill the child process and start a fresh shell in this pane
    /// (Ghostty `reset` + respawn). The split layout is preserved.
    pub fn restart(&self) {
        // Drop the fd source before the PTY closes, or the stale watcher
        // would see HUP on a dead fd and report the pane as exited.
        if let Some(source) = self.watcher.borrow_mut().take() {
            source.remove();
        }
        self.watcher_attached.set(false);
        // Dropping the session SIGHUPs the child; the draw handler then
        // bootstraps a brand new session at the current size.
        *self.state.borrow_mut() = None;
        self.area.queue_draw();
    }

    /// Re-apply a freshly loaded config (colors, font, padding) at runtime.
    pub fn apply_config(&self, config: &Config) {
        if let Some(session) = self.state.borrow_mut().as_mut() {
            let mut config = config.clone();
            config.font_family =
                resolve_font_family(&self.area.pango_context(), &config.font_family);
            if let Err(err) = config.apply_to_terminal(&mut session.terminal) {
                tracing::warn!("failed to re-apply config: {err:#}");
            }
            session.config = config;
        }
        self.area.queue_draw();
    }
}

fn bootstrap_session(
    config: &Config,
    grid: &Rc<Cell<(u16, u16)>>,
    width: i32,
    height: i32,
    da: &DrawingArea,
) -> Result<Session> {
    let mut config = config.clone();
    config.font_family = resolve_font_family(&da.pango_context(), &config.font_family);
    let config = &config;
    let (cell_w, cell_h) = measure_cell(da, config);
    let pad_x = config.padding_x;
    let pad_y = config.padding_y;
    let cols = (((width as f64 - pad_x * 2.0) / cell_w).floor() as u16).max(1);
    let rows = (((height as f64 - pad_y * 2.0) / cell_h).floor() as u16).max(1);
    grid.set((cols, rows));

    let (pty, child) = Pty::spawn(
        cols,
        rows,
        cell_w.round() as u16,
        cell_h.round() as u16,
    )
    .context("spawn pty")?;

    let pty_fd = pty.as_raw_fd();

    let mut terminal = Terminal::new(TerminalOptions {
        cols,
        rows,
        max_scrollback: 10_000,
    })
    .map_err(|e| anyhow!("{e:?}"))?;

    terminal
        .resize(cols, rows, cell_w.round() as u32, cell_h.round() as u32)
        .map_err(|e| anyhow!("{e:?}"))?;

    config.apply_to_terminal(&mut terminal)?;

    terminal
        .on_pty_write(move |_t, data| {
            pty::write_fd(pty_fd, data);
        })
        .map_err(|e| anyhow!("{e:?}"))?;

    {
        let grid = grid.clone();
        let cw = cell_w.round() as u32;
        let ch = cell_h.round() as u32;
        terminal
            .on_size(move |_term| {
                let (columns, rows) = grid.get();
                Some(SizeReportSize {
                    rows,
                    columns,
                    cell_width: cw,
                    cell_height: ch,
                })
            })
            .map_err(|e| anyhow!("{e:?}"))?;
    }

    terminal
        .on_device_attributes(|_term| {
            Some(DeviceAttributes {
                primary: PrimaryDeviceAttributes::new(
                    ConformanceLevel::VT220,
                    &[
                        DeviceAttributeFeature::COLUMNS_132,
                        DeviceAttributeFeature::SELECTIVE_ERASE,
                        DeviceAttributeFeature::ANSI_COLOR,
                    ],
                ),
                secondary: SecondaryDeviceAttributes {
                    device_type: DeviceType::VT220,
                    firmware_version: 1,
                    rom_cartridge: 0,
                },
                tertiary: Default::default(),
            })
        })
        .map_err(|e| anyhow!("{e:?}"))?;

    terminal
        .on_xtversion(|_term| Some("optionTerm"))
        .map_err(|e| anyhow!("{e:?}"))?;

    terminal
        .on_color_scheme(|_term| None)
        .map_err(|e| anyhow!("{e:?}"))?;

    Ok(Session {
        config: config.clone(),
        terminal,
        render_state: RenderState::new().map_err(|e| anyhow!("{e:?}"))?,
        row_it: RowIterator::new().map_err(|e| anyhow!("{e:?}"))?,
        cell_it: CellIterator::new().map_err(|e| anyhow!("{e:?}"))?,
        input: Input::new()?,
        pty,
        child,
        cols,
        rows,
        cell_width: cell_w,
        cell_height: cell_h,
        surface_height: height as f64,
        title: "Terminal".into(),
        blink_on: true,
        blink_enabled: false,
        fonts: None,
        gesture: Gesture::new().map_err(|e| anyhow!("{e:?}"))?,
        sel_start: None,
        sel_end: None,
        sel_rectangle: false,
    })
}

fn attach_pty_watcher(
    da: &DrawingArea,
    state: &Rc<RefCell<Option<Session>>>,
    title_cb: &Rc<RefCell<Option<TitleCb>>>,
    exit_cb: &Rc<RefCell<Option<ExitCb>>>,
    watcher: &Rc<RefCell<Option<glib::SourceId>>>,
) -> Option<glib::SourceId> {
    let fd = state.borrow().as_ref().map(|s| s.pty.as_raw_fd())?;
    let state = state.clone();
    let da = da.clone();
    let title_cb = title_cb.clone();
    let exit_cb = exit_cb.clone();
    // When the source removes itself, forget the id so a later restart does
    // not try to remove an already-finished source.
    let watcher = watcher.clone();
    let finish = move || {
        let _ = watcher.borrow_mut().take();
        glib::ControlFlow::Break
    };

    Some(glib::unix_fd_add_local(fd, glib::IOCondition::IN | glib::IOCondition::HUP, move |_fd, cond| {
        let mut close = cond.contains(glib::IOCondition::HUP);
        if !close {
            if let Ok(mut borrow) = state.try_borrow_mut() {
                if let Some(session) = borrow.as_mut() {
                    match session.pty.read_into(&mut session.terminal) {
                        Ok(()) => {}
                        Err(PtyError::EndOfStream) => {
                            if let Child::Active(pid) = session.child {
                                session.child = Child::Exited(pid);
                            }
                            close = true;
                        }
                        Err(PtyError::Other(err)) => {
                            tracing::error!("pty read error: {err}");
                            close = true;
                        }
                    }
                    if let Ok(title) = session.terminal.title() {
                        let title = sanitize_title(title);
                        if !title.is_empty() && title != session.title {
                            session.title = title;
                            let t = session.title.clone();
                            drop(borrow);
                            if let Some(cb) = title_cb.borrow().as_ref() {
                                cb(t);
                            }
                            da.queue_draw();
                            return if close {
                                if let Some(cb) = exit_cb.borrow().as_ref() {
                                    cb();
                                }
                                finish()
                            } else {
                                glib::ControlFlow::Continue
                            };
                        }
                    }
                }
            }
        }
        da.queue_draw();
        if close {
            if let Some(cb) = exit_cb.borrow().as_ref() {
                cb();
            }
            finish()
        } else {
            glib::ControlFlow::Continue
        }
    }))
}

fn attach_input(
    area: &DrawingArea,
    state: &Rc<RefCell<Option<Session>>>,
    focus_cb: &Rc<RefCell<Option<FocusCb>>>,
) {
    // Input method: makes dead keys / compose sequences work, so `´` + `a`
    // produces `á` instead of two separate keystrokes. Also covers CJK IMEs.
    let im = IMMulticontext::new();
    im.set_client_widget(Some(area));

    // While `filter_keypress` runs, commits land here so they can be fed to
    // the libghostty encoder as the event text (preserving key protocols).
    // Commits that arrive asynchronously (IBus engines) are written directly.
    let im_pending: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let im_filtering = Rc::new(Cell::new(false));
    {
        let state = state.clone();
        let area = area.clone();
        let im_pending = im_pending.clone();
        let im_filtering = im_filtering.clone();
        im.connect_commit(move |_, text| {
            if text.is_empty() {
                return;
            }
            if im_filtering.get() {
                im_pending.borrow_mut().replace(text.to_string());
                return;
            }
            if let Ok(mut borrow) = state.try_borrow_mut() {
                if let Some(session) = borrow.as_mut() {
                    session.blink_on = true;
                    session.pty.write_all(text.as_bytes());
                }
            }
            area.queue_draw();
        });
    }

    let key = EventControllerKey::new();
    {
        let state = state.clone();
        let area = area.clone();
        let im = im.clone();
        let im_pending = im_pending.clone();
        let im_filtering = im_filtering.clone();
        key.connect_key_pressed(move |ctrl, keyval, keycode, modifier| {
            // Let window-level actions handle their accelerators.
            if is_window_shortcut(keyval, modifier) {
                return glib::Propagation::Proceed;
            }

            // Only route plain typing through the IM. Ctrl/Alt/Super combos
            // must reach the encoder untouched (Ctrl+C, Alt+B, ...).
            let mut im_text = None;
            if !modifier.intersects(
                gdk::ModifierType::CONTROL_MASK
                    | gdk::ModifierType::ALT_MASK
                    | gdk::ModifierType::SUPER_MASK,
            ) {
                if let Some(event) = ctrl.current_event() {
                    im_pending.borrow_mut().take();
                    im_filtering.set(true);
                    let handled = im.filter_keypress(&event);
                    im_filtering.set(false);
                    let committed = im_pending.borrow_mut().take();
                    match (handled, committed) {
                        // Composed text (accents, IME): encode it as the event text.
                        (_, Some(text)) => im_text = Some(text),
                        // Dead key / preedit in progress: swallow, nothing to send yet.
                        (true, None) => {
                            area.queue_draw();
                            return glib::Propagation::Stop;
                        }
                        (false, None) => {}
                    }
                }
            }
            let im_text = im_text.or_else(|| keyval.to_unicode().map(|c| c.to_string()));

            if let Ok(mut borrow) = state.try_borrow_mut() {
                if let Some(session) = borrow.as_mut() {
                    session.blink_on = true;
                    session.input.buf.clear();
                    let _ = session.input.encode_press(
                        &session.terminal,
                        keyval,
                        keycode,
                        modifier,
                        im_text.as_deref(),
                    );
                    if !session.input.buf.is_empty() {
                        session.pty.write_all(&session.input.buf);
                        session.input.buf.clear();
                    }
                }
            }
            area.queue_draw();
            glib::Propagation::Stop
        });
    }
    {
        let state = state.clone();
        let im = im.clone();
        key.connect_key_released(move |ctrl, keyval, keycode, modifier| {
            if is_window_shortcut(keyval, modifier) {
                return;
            }
            // Keep the IM in sync with releases (some engines need them).
            if let Some(event) = ctrl.current_event() {
                if im.filter_keypress(&event) {
                    return;
                }
            }
            if let Ok(mut borrow) = state.try_borrow_mut() {
                if let Some(session) = borrow.as_mut() {
                    session.input.buf.clear();
                    let _ = session.input.encode_release(
                        &session.terminal,
                        keyval,
                        keycode,
                        modifier,
                    );
                    if !session.input.buf.is_empty() {
                        session.pty.write_all(&session.input.buf);
                        session.input.buf.clear();
                    }
                }
            }
        });
    }
    area.add_controller(key);

    let focus = EventControllerFocus::new();
    {
        let area = area.clone();
        let focus_cb = focus_cb.clone();
        let im = im.clone();
        focus.connect_enter(move |_| {
            im.focus_in();
            if let Some(cb) = focus_cb.borrow().as_ref() {
                cb();
            }
            area.queue_draw();
        });
    }
    {
        let area = area.clone();
        let im = im.clone();
        focus.connect_leave(move |_| {
            im.focus_out();
            area.queue_draw();
        });
    }
    area.add_controller(focus);

    let scroll = EventControllerScroll::new(EventControllerScrollFlags::VERTICAL);
    {
        let state = state.clone();
        let area = area.clone();
        scroll.connect_scroll(move |_, _dx, dy| {
            if let Ok(mut borrow) = state.try_borrow_mut() {
                if let Some(session) = borrow.as_mut() {
                    let delta = (dy * 3.0).round() as isize;
                    if delta != 0 {
                        session
                            .terminal
                            .scroll_viewport(ScrollViewport::Delta(delta));
                    }
                }
            }
            area.queue_draw();
            glib::Propagation::Stop
        });
    }
    area.add_controller(scroll);

    // Left button: focus + text selection (click, double/triple click, drag).
    let click = GestureClick::new();
    click.set_button(gdk::BUTTON_PRIMARY);
    {
        let state = state.clone();
        let area = area.clone();
        click.connect_pressed(move |gesture, _n, x, y| {
            area.grab_focus();
            let time = gesture.current_event_time();
            if let Ok(mut borrow) = state.try_borrow_mut() {
                if let Some(session) = borrow.as_mut() {
                    session.selection_press(x, y, time);
                }
            }
            area.queue_draw();
        });
    }
    {
        let state = state.clone();
        let area = area.clone();
        click.connect_released(move |_, _n, x, y| {
            if let Ok(mut borrow) = state.try_borrow_mut() {
                if let Some(session) = borrow.as_mut() {
                    session.selection_release(x, y);
                }
            }
            area.queue_draw();
        });
    }
    area.add_controller(click);

    let motion = gtk4::EventControllerMotion::new();
    {
        let state = state.clone();
        let area = area.clone();
        // Track drag via motion while primary button is held.
        motion.connect_motion(move |controller, x, y| {
            let held = controller
                .current_event_state()
                .contains(gdk::ModifierType::BUTTON1_MASK);
            if !held {
                return;
            }
            if let Ok(mut borrow) = state.try_borrow_mut() {
                if let Some(session) = borrow.as_mut() {
                    session.selection_drag(x, y);
                }
            }
            area.queue_draw();
        });
    }
    area.add_controller(motion);
}

fn is_window_shortcut(keyval: gdk::Key, modifier: gdk::ModifierType) -> bool {
    let ctrl = modifier.contains(gdk::ModifierType::CONTROL_MASK);
    let shift = modifier.contains(gdk::ModifierType::SHIFT_MASK);
    let alt = modifier.contains(gdk::ModifierType::ALT_MASK);
    let super_ = modifier.contains(gdk::ModifierType::SUPER_MASK);

    let arrow = matches!(
        keyval,
        gdk::Key::Up | gdk::Key::Down | gdk::Key::Left | gdk::Key::Right
    );
    // Ghostty-style split bindings:
    // ctrl+alt+arrows = goto_split, ctrl+shift+super+arrows = resize_split,
    // ctrl+super+[ / ] = previous/next split.
    if ctrl && alt && !shift && !super_ {
        return arrow;
    }
    if ctrl && super_ && !alt {
        if shift {
            return arrow;
        }
        return matches!(keyval, gdk::Key::bracketleft | gdk::Key::bracketright);
    }

    if !ctrl || alt || super_ {
        return false;
    }
    if shift {
        return matches!(
            keyval,
            gdk::Key::T
                | gdk::Key::t
                | gdk::Key::W
                | gdk::Key::w
                | gdk::Key::Q
                | gdk::Key::q
                | gdk::Key::N
                | gdk::Key::n
                | gdk::Key::C
                | gdk::Key::c
                | gdk::Key::V
                | gdk::Key::v
                | gdk::Key::A
                | gdk::Key::a
                | gdk::Key::P
                | gdk::Key::p
                | gdk::Key::E
                | gdk::Key::e
                | gdk::Key::O
                | gdk::Key::o
                | gdk::Key::U
                | gdk::Key::u
                | gdk::Key::L
                | gdk::Key::l
                | gdk::Key::plus
                | gdk::Key::Return
                | gdk::Key::Tab
                | gdk::Key::ISO_Left_Tab
        );
    }
    matches!(
        keyval,
        gdk::Key::Page_Down
            | gdk::Key::Page_Up
            | gdk::Key::Tab
            | gdk::Key::ISO_Left_Tab
            | gdk::Key::plus
            | gdk::Key::equal
            | gdk::Key::minus
            | gdk::Key::_0
            | gdk::Key::KP_Add
            | gdk::Key::KP_Subtract
            | gdk::Key::KP_0
            | gdk::Key::comma
    )
}

/// Cached Pango state, rebuilt only when the font family/size changes.
struct FontSet {
    key: (String, u32),
    regular: pango::FontDescription,
    bold: pango::FontDescription,
    italic: pango::FontDescription,
    bold_italic: pango::FontDescription,
    cell: (f64, f64),
}

impl FontSet {
    fn build(da: &DrawingArea, config: &Config) -> Self {
        let regular = font_description(config);
        let mut bold = regular.clone();
        bold.set_weight(pango::Weight::Bold);
        let mut italic = regular.clone();
        italic.set_style(pango::Style::Italic);
        let mut bold_italic = bold.clone();
        bold_italic.set_style(pango::Style::Italic);
        Self {
            key: font_key(config),
            regular,
            bold,
            italic,
            bold_italic,
            cell: measure_cell(da, config),
        }
    }
}

fn font_key(config: &Config) -> (String, u32) {
    (config.font_family.clone(), (config.font_size * 64.0) as u32)
}

/// OSC titles come straight from the child process: strip control characters
/// and keep them short so they cannot mangle the UI.
fn sanitize_title(title: &str) -> String {
    title.chars().filter(|c| !c.is_control()).take(256).collect()
}

/// Screen-space endpoints of a selection, used to persist it across borrows.
type SelectionPoints = (PointCoordinate, PointCoordinate, bool);

fn selection_points(
    terminal: &Terminal<'_, '_>,
    sel: Option<&Selection<'_>>,
) -> Option<SelectionPoints> {
    let sel = sel?;
    let start = terminal
        .point_from_grid_ref(&sel.start(), PointSpace::Screen)
        .ok()??;
    let end = terminal
        .point_from_grid_ref(&sel.end(), PointSpace::Screen)
        .ok()??;
    Some((start, end, sel.is_rectangle()))
}

impl Session {
    fn cell_coord(&self, x: f64, y: f64) -> PointCoordinate {
        let col = ((x - self.config.padding_x) / self.cell_width)
            .floor()
            .clamp(0.0, (self.cols - 1) as f64) as u16;
        let row = ((y - self.config.padding_y) / self.cell_height)
            .floor()
            .clamp(0.0, (self.rows - 1) as f64) as u32;
        PointCoordinate { x: col, y: row }
    }

    fn geometry(&self) -> Geometry {
        Geometry {
            columns: self.cols.max(1) as u32,
            cell_width: (self.cell_width.round() as u32).max(1),
            padding_left: self.config.padding_x.max(0.0).round() as u32,
            screen_height: (self.surface_height.round() as u32).max(1),
        }
    }

    fn set_selection_points(&mut self, points: Option<SelectionPoints>) {
        match points {
            Some((start, end, rectangle)) => {
                self.sel_start = self.terminal.track_grid_ref(Point::Screen(start)).ok();
                self.sel_end = self.terminal.track_grid_ref(Point::Screen(end)).ok();
                self.sel_rectangle = rectangle;
            }
            None => {
                self.sel_start = None;
                self.sel_end = None;
                self.sel_rectangle = false;
            }
        }
    }

    /// Drop any active selection (tracked refs die with the erased rows).
    fn clear_selection(&mut self) {
        let _ = self.terminal.set_selection(None);
        self.set_selection_points(None);
        self.gesture.reset(&self.terminal);
    }

    fn selection_press(&mut self, x: f64, y: f64, time_ms: u32) {
        let coord = self.cell_coord(x, y);
        let Ok(grid_ref) = self.terminal.grid_ref(Point::Viewport(coord)) else {
            return;
        };
        let Ok(mut event) = PressEvent::new() else {
            return;
        };
        let _ = event.set_position(x, y);
        let _ = event.set_time(std::time::Duration::from_millis(time_ms as u64));
        let sel = event
            .apply(&mut self.gesture, &self.terminal, grid_ref)
            .ok()
            .flatten();
        let points = selection_points(&self.terminal, sel.as_ref());
        let _ = self.terminal.set_selection(sel.as_ref());
        drop(sel);
        self.set_selection_points(points);
    }

    fn selection_drag(&mut self, x: f64, y: f64) {
        let coord = self.cell_coord(x, y);
        let Ok(grid_ref) = self.terminal.grid_ref(Point::Viewport(coord)) else {
            return;
        };
        let Ok(mut event) = DragEvent::new() else {
            return;
        };
        let _ = event.set_position(x, y);
        let geometry = self.geometry();
        let sel = event
            .apply(&mut self.gesture, &self.terminal, grid_ref, geometry)
            .ok()
            .flatten();
        if let Some(sel) = sel {
            let points = selection_points(&self.terminal, Some(&sel));
            let _ = self.terminal.set_selection(Some(&sel));
            drop(sel);
            self.set_selection_points(points);
        }
    }

    fn selection_release(&mut self, x: f64, y: f64) {
        let coord = self.cell_coord(x, y);
        let grid_ref = self.terminal.grid_ref(Point::Viewport(coord)).ok();
        if let Ok(mut event) = ReleaseEvent::new() {
            let _ = event.apply(&mut self.gesture, &self.terminal, grid_ref);
        }
    }

    fn select_all(&mut self) {
        let sel = self.terminal.select_all().ok().flatten();
        let points = selection_points(&self.terminal, sel.as_ref());
        let _ = self.terminal.set_selection(sel.as_ref());
        drop(sel);
        self.set_selection_points(points);
    }

    fn selection_text(&mut self) -> Option<String> {
        let start = self.sel_start.as_ref()?.snapshot(&self.terminal).ok()??;
        let end = self.sel_end.as_ref()?.snapshot(&self.terminal).ok()??;
        let sel = Selection::new(start, end, self.sel_rectangle);
        let opts = FormatterOptions::new().with_selection(&sel).with_trim(true);
        let mut formatter = Formatter::new(&self.terminal, opts).ok()?;
        let bytes = formatter.format_alloc(None).ok()?;
        let text = String::from_utf8_lossy(&bytes).into_owned();
        (!text.is_empty()).then_some(text)
    }

    fn paste_text(&mut self, text: &str) {
        let bracketed = self
            .terminal
            .mode(Mode::new(2004, ModeKind::Dec))
            .unwrap_or(false);
        let mut data = text.as_bytes().to_vec();
        let mut buf = vec![0u8; data.len() + 16];
        loop {
            match paste::encode(&mut data, bracketed, &mut buf) {
                Ok(len) => {
                    self.pty.write_all(&buf[..len]);
                    return;
                }
                Err(libghostty_vt::Error::OutOfSpace { required }) => buf.resize(required, 0),
                Err(err) => {
                    tracing::warn!("paste encode failed: {err:?}");
                    return;
                }
            }
        }
    }

    /// Cached fonts + cell metrics; rebuilt only when family/size changes.
    fn ensure_fonts(&mut self, da: &DrawingArea) {
        let stale = self
            .fonts
            .as_ref()
            .is_none_or(|f| f.key != font_key(&self.config));
        if stale {
            self.fonts = Some(FontSet::build(da, &self.config));
        }
    }

    /// Returns `Some((cols, rows))` when the grid size actually changed.
    fn ensure_size(
        &mut self,
        da: &DrawingArea,
        width: i32,
        height: i32,
        grid: &Rc<Cell<(u16, u16)>>,
    ) -> Result<Option<(u16, u16)>> {
        self.ensure_fonts(da);
        let (cell_w, cell_h) = self.fonts.as_ref().map(|f| f.cell).unwrap_or((1.0, 1.0));
        self.cell_width = cell_w;
        self.cell_height = cell_h;
        self.surface_height = height as f64;
        let cols = (((width as f64 - self.config.padding_x * 2.0) / cell_w).floor() as u16).max(1);
        let rows = (((height as f64 - self.config.padding_y * 2.0) / cell_h).floor() as u16).max(1);
        if cols == self.cols && rows == self.rows {
            return Ok(None);
        }
        self.cols = cols;
        self.rows = rows;
        grid.set((cols, rows));
        self.terminal
            .resize(cols, rows, cell_w.round() as u32, cell_h.round() as u32)
            .map_err(|e| anyhow!("{e:?}"))?;
        self.pty
            .resize(cols, rows, cell_w.round() as u16, cell_h.round() as u16);
        Ok(Some((cols, rows)))
    }

    fn paint(&mut self, da: &DrawingArea, cr: &cairo::Context, width: i32, height: i32) -> Result<()> {
        let bg = self.config.background;
        cr.set_source_rgb(bg.r as f64 / 255.0, bg.g as f64 / 255.0, bg.b as f64 / 255.0);
        cr.paint().ok();
        let _ = (width, height);

        // Grab cached font descriptions before `render_state` borrows self.
        self.ensure_fonts(da);
        let (font, font_bold, font_italic, font_bi) = {
            let fonts = self.fonts.as_ref().expect("fonts built above");
            (
                fonts.regular.clone(),
                fonts.bold.clone(),
                fonts.italic.clone(),
                fonts.bold_italic.clone(),
            )
        };
        let (font, font_bold, font_italic, font_bi) = (&font, &font_bold, &font_italic, &font_bi);

        let snapshot = self
            .render_state
            .update(&self.terminal)
            .map_err(|e| anyhow!("{e:?}"))?;
        let colors = snapshot.colors().map_err(|e| anyhow!("{e:?}"))?;

        let pango_ctx = da.pango_context();
        // Do NOT call pangocairo::update_context here — on HiDPI it bakes the
        // Cairo scale matrix into the widget PangoContext, so the next
        // measure_cell() returns ~2× cell width while glyphs stay logical-sized.

        let layout = pango::Layout::new(&pango_ctx);
        layout.set_font_description(Some(font));
        disable_ligatures(&layout);

        let pad_x = self.config.padding_x;
        let pad_y = self.config.padding_y;
        let cell_w = self.cell_width;
        let cell_h = self.cell_height;

        // --- Cursor state ---
        let focused = da.has_focus();
        let cursor_color = snapshot
            .cursor_color()
            .ok()
            .flatten()
            .unwrap_or(self.config.cursor);
        let cursor_pos = if snapshot.cursor_visible().unwrap_or(false) {
            snapshot.cursor_viewport().ok().flatten()
        } else {
            None
        };
        let cursor_style = if !focused {
            CursorVisualStyle::BlockHollow
        } else {
            match snapshot.cursor_visual_style() {
                // DECSCUSR default: honor the configured shape.
                Ok(CursorVisualStyle::Block) => match self.config.cursor_style {
                    CursorStyle::Bar => CursorVisualStyle::Bar,
                    CursorStyle::Underline => CursorVisualStyle::Underline,
                    CursorStyle::BlockHollow => CursorVisualStyle::BlockHollow,
                    CursorStyle::Block => CursorVisualStyle::Block,
                },
                Ok(style) => style,
                Err(_) => CursorVisualStyle::Block,
            }
        };
        let blinking = self.config.cursor_blink && snapshot.cursor_blinking().unwrap_or(false);
        self.blink_enabled = focused && blinking && cursor_pos.is_some();
        let cursor_shown = cursor_pos.is_some() && (!self.blink_enabled || self.blink_on);
        // Filled block cursor: painted inside the cell loop so the glyph on top
        // is redrawn using the cursor-text color.
        let block_cursor_cell = cursor_pos
            .as_ref()
            .filter(|_| cursor_shown && matches!(cursor_style, CursorVisualStyle::Block))
            .map(|c| (c.x, c.y));

        let mut row_it = self.row_it.update(&snapshot).map_err(|e| anyhow!("{e:?}"))?;
        let mut row_idx = 0u16;
        let mut text = String::with_capacity(16);

        while let Some(row) = row_it.next() {
            let y = pad_y + row_idx as f64 * cell_h;
            let mut cell_it = self.cell_it.update(row).map_err(|e| anyhow!("{e:?}"))?;
            let mut col_idx = 0u16;

            while let Some(cell) = cell_it.next() {
                let x = pad_x + col_idx as f64 * cell_w;
                let selected = cell.is_selected().unwrap_or(false);
                let graphemes = cell.graphemes_len().unwrap_or(0);
                let style = cell.style().unwrap_or_default();
                let at_block_cursor =
                    block_cursor_cell.map(|(cx, cy)| (cx as u32, cy as u32))
                        == Some((col_idx as u32, row_idx as u32));
                let (mut fg, bg_cell) = if selected {
                    (
                        self.config.selection_foreground,
                        Some(self.config.selection_background),
                    )
                } else {
                    (
                        cell.fg_color().ok().flatten().unwrap_or(colors.foreground),
                        cell.bg_color().ok().flatten(),
                    )
                };

                if let Some(bgc) = bg_cell {
                    cr.set_source_rgb(
                        bgc.r as f64 / 255.0,
                        bgc.g as f64 / 255.0,
                        bgc.b as f64 / 255.0,
                    );
                    cr.rectangle(x, y, cell_w, cell_h);
                    cr.fill().ok();
                }

                if at_block_cursor {
                    cr.set_source_rgb(
                        cursor_color.r as f64 / 255.0,
                        cursor_color.g as f64 / 255.0,
                        cursor_color.b as f64 / 255.0,
                    );
                    cr.rectangle(x, y, cell_w, cell_h);
                    cr.fill().ok();
                    fg = self.config.cursor_text;
                }

                if graphemes > 0 {
                    text.clear();
                    let _ = cell.graphemes_utf8(&mut text);
                    if !text.is_empty() && text != " " {
                        let mut draw_fg = fg;
                        if style.faint {
                            draw_fg = RgbColor {
                                r: draw_fg.r / 2,
                                g: draw_fg.g / 2,
                                b: draw_fg.b / 2,
                            };
                        }
                        cr.set_source_rgb(
                            draw_fg.r as f64 / 255.0,
                            draw_fg.g as f64 / 255.0,
                            draw_fg.b as f64 / 255.0,
                        );

                        let desc = match (style.bold, style.italic) {
                            (true, true) => font_bi,
                            (true, false) => font_bold,
                            (false, true) => font_italic,
                            (false, false) => font,
                        };
                        layout.set_font_description(Some(desc));
                        layout.set_text(&text);
                        // Monospace terminals left-align glyphs in the cell.
                        cr.move_to(x, y);
                        pangocairo::functions::show_layout(cr, &layout);

                        if style.underline != Underline::None {
                            cr.set_line_width(1.0);
                            cr.move_to(x, y + cell_h - 1.0);
                            cr.line_to(x + cell_w, y + cell_h - 1.0);
                            cr.stroke().ok();
                        }
                        if style.strikethrough {
                            cr.set_line_width(1.0);
                            cr.move_to(x, y + cell_h / 2.0);
                            cr.line_to(x + cell_w, y + cell_h / 2.0);
                            cr.stroke().ok();
                        }
                    }
                }

                col_idx = col_idx.saturating_add(1);
            }
            row_idx = row_idx.saturating_add(1);
        }

        // Thin cursor shapes drawn on top (they don't obscure the glyph).
        if cursor_shown && !matches!(cursor_style, CursorVisualStyle::Block) {
            if let Some(cursor) = cursor_pos {
                let x = pad_x + cursor.x as f64 * cell_w;
                let y = pad_y + cursor.y as f64 * cell_h;
                cr.set_source_rgb(
                    cursor_color.r as f64 / 255.0,
                    cursor_color.g as f64 / 255.0,
                    cursor_color.b as f64 / 255.0,
                );
                match cursor_style {
                    CursorVisualStyle::Bar => {
                        cr.rectangle(x, y, (cell_w * 0.15).clamp(1.0, 2.0), cell_h);
                        cr.fill().ok();
                    }
                    CursorVisualStyle::Underline => {
                        let thickness = (cell_h * 0.08).clamp(1.0, 2.0);
                        cr.rectangle(x, y + cell_h - thickness, cell_w, thickness);
                        cr.fill().ok();
                    }
                    _ => {
                        // Hollow block (also used when unfocused).
                        cr.set_line_width(1.0);
                        cr.rectangle(x + 0.5, y + 0.5, cell_w - 1.0, cell_h - 1.0);
                        cr.stroke().ok();
                    }
                }
            }
        }

        Ok(())
    }
}

/// Resolve the configured font family, falling back to `monospace` when the
/// family is missing or not monospace — a proportional fallback (e.g. Noto
/// Sans) would break the cell grid and render with huge glyph gaps.
fn resolve_font_family(pango_ctx: &pango::Context, requested: &str) -> String {
    let requested_trimmed = requested.trim();
    if requested_trimmed.eq_ignore_ascii_case("monospace") {
        return "monospace".into();
    }
    let found = pango_ctx.list_families().into_iter().find(|f| {
        f.name().eq_ignore_ascii_case(requested_trimmed)
    });
    match found {
        Some(f) if f.is_monospace() => requested_trimmed.to_string(),
        Some(_) => {
            tracing::warn!("font family {requested_trimmed:?} is not monospace, using `monospace`");
            "monospace".into()
        }
        None => {
            tracing::warn!("font family {requested_trimmed:?} not found, using `monospace`");
            "monospace".into()
        }
    }
}

fn font_description(config: &Config) -> pango::FontDescription {
    let mut desc = pango::FontDescription::new();
    // Ghostty `font-family` verbatim — same family string Fontconfig resolves.
    desc.set_family(&config.font_family);
    desc.set_size((config.font_size * pango::SCALE as f32).round() as i32);
    desc
}

/// Cell advance in DrawingArea logical pixels (same space as draw `width`/`height`).
fn measure_cell(da: &DrawingArea, config: &Config) -> (f64, f64) {
    let pango_ctx = da.pango_context();
    let desc = font_description(config);
    let layout = pango::Layout::new(&pango_ctx);
    layout.set_font_description(Some(&desc));
    disable_ligatures(&layout);
    layout.set_text("M");

    // Widget PangoContext is already set up for the widget's coordinate space
    // (logical pixels). Do not divide by scale_factor — that would under-size
    // cells on HiDPI after we stopped calling update_context.
    let (w, h) = layout.pixel_size();
    ((w as f64).max(1.0), (h as f64).max(1.0))
}

fn disable_ligatures(layout: &pango::Layout) {
    let attrs = pango::AttrList::new();
    attrs.insert(pango::AttrFontFeatures::new("liga=0,clig=0,calt=0,dlig=0"));
    layout.set_attributes(Some(&attrs));
}

fn paint_error(cr: &cairo::Context, width: i32, height: i32, err: &anyhow::Error) {
    cr.set_source_rgb(0.1, 0.05, 0.05);
    cr.paint().ok();
    cr.set_source_rgb(1.0, 0.4, 0.4);
    cr.move_to(16.0, 32.0);
    let layout = pangocairo::functions::create_layout(cr);
    layout.set_text(&format!("optionTerm failed to start:\n{err:#}"));
    layout.set_width(width * pango::SCALE);
    pangocairo::functions::show_layout(cr, &layout);
    let _ = height;
}
