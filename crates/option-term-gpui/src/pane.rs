//! GPUI entity wrapping a `option-term-vt` pane: owns the latest `Frame`,
//! routes input to the emulator thread and renders via `TerminalElement`.

use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result};
use futures::StreamExt;
use futures::channel::mpsc::{self, UnboundedReceiver};
use gpui::{
    AnyWindowHandle, App, Bounds, Context, EntityInputHandler, FocusHandle, KeyDownEvent,
    KeyUpEvent, MouseButton as GpuiMouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    Pixels, Point, Render, ScrollDelta, ScrollWheelEvent, Size, Subscription, Task, UTF16Selection,
    Window, div, point, prelude::*, px, size,
};
use option_term_core::config::Config;
use option_term_vt::emulator::EmulatorOptions;
use option_term_vt::frame::{CursorShape, Frame, Kitty, Modes, Scroll};
use option_term_vt::input::{
    ClipboardTarget, Event, Input, Mods, MouseButton, MouseEvent, MouseKind, ScrollTarget,
    SelectKind,
};
use option_term_vt::palette::Palette;
use option_term_vt::pane::{self, PaneHandle, PaneOptions};

use crate::actions::{
    Copy, Paste, Quit, ScrollPageDown, ScrollPageUp, ScrollToBottom, ScrollToTop, SelectAll,
    ZoomIn, ZoomOut, ZoomReset,
};
use crate::keymap;
use crate::terminal_element::TerminalElement;
use crate::theme::{self, Theme};

const BLINK_INTERVAL: Duration = Duration::from_millis(500);
const ZOOM_STEP: f32 = 1.1;

/// `Config::font_size` is in *points* (GTK/Pango semantics); GPUI works in px.
fn font_size_px(config: &Config) -> f32 {
    config.font_size * 96.0 / 72.0
}

fn empty_frame(palette: &Palette) -> Frame {
    Frame {
        cols: 80,
        rows: 24,
        lines: Vec::new(),
        cursor: None,
        scroll: Scroll {
            offset: 0,
            total: 0,
        },
        modes: Modes {
            mouse_reporting: false,
            bracketed_paste: false,
            alt_screen: false,
            focus_events: false,
            kitty_keyboard: false,
        },
        title: String::new(),
        pwd: None,
        kitty: Kitty {
            generation: 0,
            placements: Vec::new(),
            images: Vec::new(),
            dropped: Vec::new(),
        },
        matches: Vec::new(),
        active_match: None,
        default_bg: palette.bg,
        default_fg: palette.fg,
        seq: 0,
    }
}

pub struct Pane {
    handle: Option<PaneHandle>,
    focus: FocusHandle,
    /// Latest frame from the emulator thread. `frame.kitty` is retained so a
    /// later phase can paint images; `TerminalElement` ignores it for now.
    pub(crate) frame: Arc<Frame>,
    pub(crate) palette: Palette,
    pub(crate) theme: Theme,
    pub(crate) title: String,

    pub(crate) font: gpui::Font,
    /// Effective font size in px (zoom applied).
    pub(crate) font_size: Pixels,
    /// Configured font size in px (no zoom).
    base_font_size: f32,
    zoom_steps: i32,
    pub(crate) background_opacity: f32,
    pub(crate) padding: Size<Pixels>,
    /// Cell metrics and content origin, refreshed by `TerminalElement` prepaint.
    pub(crate) cell: Size<Pixels>,
    pub(crate) content_origin: Point<Pixels>,
    pub(crate) bounds: Bounds<Pixels>,
    last_grid: Option<(u16, u16, u16, u16)>,

    pub(crate) preedit: Option<String>,
    pending_copy: Option<ClipboardTarget>,
    selecting: bool,
    /// Set when a ctrl-click goes down without dragging; cleared on drag.
    link_candidate: Option<(u16, u16)>,
    wheel_remainder: Point<f32>,

    pub(crate) focused: bool,
    pub(crate) blink_on: bool,
    cursor_blink: bool,
    scroll_on_keystroke: bool,
    _blink_task: Option<Task<()>>,
    _events_task: Task<()>,
    _subscriptions: Vec<Subscription>,
    window_handle: Option<AnyWindowHandle>,
}

impl Pane {
    /// Spawn a real pane thread (PTY + emulator) and wrap it in an entity state.
    pub fn new(config: &Config, window: &mut Window, cx: &mut Context<Self>) -> Result<Self> {
        let palette = Palette::from(config);
        // `resolve_font` falls back to GPUI's default font stack if the
        // configured family is missing, so a bad `font_family` cannot panic.
        let font = base_font(config);
        let font_size = font_size_px(config);
        let scale = window.scale_factor();
        let cell = crate::metrics::cell_size(window.text_system(), &font, px(font_size), scale);
        let options = PaneOptions {
            emulator: EmulatorOptions {
                cols: 80,
                rows: 24,
                cell_w_px: (f32::from(cell.width) * scale).round().max(1.0) as u16,
                cell_h_px: (f32::from(cell.height) * scale).round().max(1.0) as u16,
                scrollback: config.scroll_lines.max(0) as u32,
                palette,
                cursor_shape: CursorShape::from(config.cursor_style),
                cursor_blink: config.cursor_blink,
                kitty_storage_limit: option_term_vt::graphics::STORAGE_LIMIT,
                xtversion: concat!("optionterm(", env!("CARGO_PKG_VERSION"), ")").to_string(),
            },
            cwd: std::env::current_dir().ok(),
            argv: None,
            env: Vec::new(),
            min_frame_interval: Duration::from_millis(8),
        };

        let (tx, rx) = mpsc::unbounded();
        let handle = pane::spawn(
            options,
            Box::new(move |ev| {
                if tx.unbounded_send(ev).is_err() {
                    // Entity gone; the pane thread will exit on its own.
                }
            }),
        )
        .context("spawn terminal pane")?;

        let mut pane = Self::with_channel_inner(config, Some(handle), rx, cx);
        pane.window_handle = Some(window.window_handle());
        let weak = cx.weak_entity();
        pane._subscriptions = vec![
            window.on_focus_in(&pane.focus, cx, {
                let weak = weak.clone();
                move |_window, cx| {
                    let _ = weak.update(cx, |pane, cx| pane.set_focused(true, cx));
                }
            }),
            window.on_focus_out(&pane.focus, cx, move |_event, _window, cx| {
                let _ = weak.update(cx, |pane, cx| pane.set_focused(false, cx));
            }),
        ];

        if config.cursor_blink {
            pane._blink_task = Some(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(BLINK_INTERVAL).await;
                    if this
                        .update(cx, |pane, cx| {
                            if pane.focused && pane.cursor_blink {
                                pane.blink_on = !pane.blink_on;
                                cx.notify();
                            }
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            }));
        }

        Ok(pane)
    }

    /// Pane with no backing thread (spawn failure): paints an empty frame.
    pub fn without_thread(config: &Config, cx: &mut Context<Self>) -> Self {
        let (_tx, rx) = mpsc::unbounded();
        Self::with_channel_inner(config, None, rx, cx)
    }

    /// Test constructor: no PTY, events are injected through `rx`.
    pub fn with_channel(
        config: &Config,
        handle: Option<PaneHandle>,
        rx: UnboundedReceiver<Event>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::with_channel_inner(config, handle, rx, cx)
    }

    fn with_channel_inner(
        config: &Config,
        handle: Option<PaneHandle>,
        mut rx: UnboundedReceiver<Event>,
        cx: &mut Context<Self>,
    ) -> Self {
        let palette = Palette::from(config);
        let font = base_font(config);
        let events_task = cx.spawn(async move |this, cx| {
            while let Some(first) = rx.next().await {
                // Coalesce: several events may be queued before we get to run;
                // only the last Frame matters for painting.
                let mut batch = vec![first];
                while let Ok(ev) = rx.try_recv() {
                    batch.push(ev);
                }
                if this
                    .update(cx, |pane, cx| {
                        for ev in batch {
                            pane.on_event(ev, cx);
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        });

        Self {
            handle,
            focus: cx.focus_handle(),
            frame: Arc::new(empty_frame(&palette)),
            palette,
            theme: Theme::from(&palette),
            title: String::new(),
            font,
            font_size: px(font_size_px(config)),
            base_font_size: font_size_px(config),
            zoom_steps: 0,
            background_opacity: config.background_opacity as f32,
            padding: size(px(config.padding_x as f32), px(config.padding_y as f32)),
            cell: size(px(9.6), px(20.0)),
            content_origin: point(px(0.0), px(0.0)),
            bounds: Bounds::default(),
            last_grid: None,
            preedit: None,
            pending_copy: None,
            selecting: false,
            link_candidate: None,
            wheel_remainder: point(0.0, 0.0),
            focused: false,
            blink_on: true,
            cursor_blink: config.cursor_blink,
            scroll_on_keystroke: config.scroll_on_keystroke,
            _blink_task: None,
            _events_task: events_task,
            _subscriptions: Vec::new(),
            window_handle: None,
        }
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }

    /// Latest frame from the emulator thread.
    pub fn frame(&self) -> &Arc<Frame> {
        &self.frame
    }

    /// Last title reported by the terminal (OSC 0/2 or `Frame::title`).
    pub fn title(&self) -> &str {
        &self.title
    }

    fn send(&self, input: Input) {
        if let Some(handle) = &self.handle {
            handle.send(input);
        }
    }

    fn set_focused(&mut self, focused: bool, cx: &mut Context<Self>) {
        if self.focused == focused {
            return;
        }
        self.focused = focused;
        if focused {
            self.blink_on = true;
        }
        self.send(Input::Focus(focused));
        cx.notify();
    }

    fn on_event(&mut self, ev: Event, cx: &mut Context<Self>) {
        match ev {
            Event::Frame(frame) => {
                self.blink_on = true;
                if !frame.title.is_empty() && frame.title != self.title {
                    self.title = frame.title.clone();
                    self.update_window_title(cx);
                }
                self.frame = frame;
                cx.notify();
            }
            Event::Title(title) => {
                self.title = title;
                self.update_window_title(cx);
                cx.notify();
            }
            Event::Bell => tracing::debug!("bell"),
            Event::ClipboardWrite { target, text } => {
                self.write_clipboard(target, text, cx);
            }
            Event::SelectionText(text) => {
                if let (Some(target), Some(text)) = (self.pending_copy.take(), text) {
                    self.write_clipboard(target, text, cx);
                }
            }
            Event::Link(Some(url)) => {
                cx.open_url(&url);
            }
            Event::Link(None) => {}
            Event::Exited { status } => {
                tracing::info!(?status, "pane exited");
                // Drop the window before quitting so entity handles rooted in
                // it (including this Pane) are released instead of leaking.
                if let Some(handle) = self.window_handle
                    && let Err(err) = handle.update(cx, |_, window, _| window.remove_window())
                {
                    tracing::warn!("failed to remove window on exit: {err}");
                }
                cx.quit();
            }
            Event::Error(msg) => tracing::error!("pane: {msg}"),
        }
    }

    fn update_window_title(&self, cx: &mut App) {
        let title = if self.title.is_empty() {
            "optionTerm".to_string()
        } else {
            format!("{} — optionTerm", self.title)
        };
        if let Some(handle) = self.window_handle
            && let Err(err) = handle.update(cx, |_, window, _| window.set_window_title(&title))
        {
            tracing::warn!("failed to set window title: {err}");
        }
    }

    fn write_clipboard(&mut self, target: ClipboardTarget, text: String, cx: &mut App) {
        let item = gpui::ClipboardItem::new_string(text);
        match target {
            ClipboardTarget::Clipboard => cx.write_to_clipboard(item),
            ClipboardTarget::Primary => cx.write_to_primary(item),
        }
    }

    // ---------------------------------------------------------------- layout

    /// Called by `TerminalElement` prepaint with the element bounds.
    pub(crate) fn update_layout(
        &mut self,
        bounds: Bounds<Pixels>,
        cell: Size<Pixels>,
        scale_factor: f32,
    ) {
        self.bounds = bounds;
        self.cell = cell;
        // Snap the content origin to the device-pixel grid so every cell (and
        // thus every glyph) lands on whole device pixels.
        self.content_origin = crate::metrics::snap_point_to_device_px(
            bounds.origin + point(self.padding.width, self.padding.height),
            scale_factor,
        );
        let (cols, rows) = crate::metrics::grid_for(bounds.size, cell, self.padding);
        let cell_w = (f32::from(cell.width) * scale_factor).round().max(1.0) as u16;
        let cell_h = (f32::from(cell.height) * scale_factor).round().max(1.0) as u16;
        let grid = (cols, rows, cell_w, cell_h);
        if self.last_grid != Some(grid) {
            self.last_grid = Some(grid);
            self.send(Input::Resize {
                cols,
                rows,
                cell_w_px: cell_w,
                cell_h_px: cell_h,
            });
        }
    }

    /// `(col, row, px_in_cell)` for a window position, clamped to the grid.
    pub(crate) fn cell_at(&self, pos: Point<Pixels>) -> Option<(u16, u16, (u16, u16))> {
        let rel_x = f32::from(pos.x) - f32::from(self.content_origin.x);
        let rel_y = f32::from(pos.y) - f32::from(self.content_origin.y);
        let cell_w = f32::from(self.cell.width).max(1.0);
        let cell_h = f32::from(self.cell.height).max(1.0);
        let (cols, rows) = crate::metrics::grid_for(self.bounds.size, self.cell, self.padding);
        let col = (rel_x / cell_w)
            .floor()
            .clamp(0.0, f32::from(cols.saturating_sub(1))) as u16;
        let row = (rel_y / cell_h)
            .floor()
            .clamp(0.0, f32::from(rows.saturating_sub(1))) as u16;
        let px_x = (rel_x - f32::from(col) * cell_w).clamp(0.0, cell_w) as u16;
        let px_y = (rel_y - f32::from(row) * cell_h).clamp(0.0, cell_h) as u16;
        Some((col, row, (px_x, px_y)))
    }

    fn mods(m: &gpui::Modifiers) -> Mods {
        Mods {
            shift: m.shift,
            ctrl: m.control,
            alt: m.alt,
            super_: m.platform,
            caps_lock: false,
            num_lock: false,
        }
    }

    // ---------------------------------------------------------------- events

    fn key_down(&mut self, ev: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if self.scroll_on_keystroke && self.frame.scroll.offset > 0 {
            self.send(Input::ScrollTo(ScrollTarget::Bottom));
        }
        if let Some(key) = keymap::keystroke_to_key_event(&ev.keystroke, ev.is_held) {
            self.send(Input::Key(key));
        }
        cx.stop_propagation();
    }

    fn key_up(&mut self, ev: &KeyUpEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(mut key) = keymap::keystroke_to_key_event(&ev.keystroke, false) {
            key.action = option_term_vt::input::KeyAction::Release;
            self.send(Input::Key(key));
        }
        cx.stop_propagation();
    }

    fn mouse_down_left(
        &mut self,
        ev: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus.focus(window, cx);
        let Some((col, row, px_in_cell)) = self.cell_at(ev.position) else {
            return;
        };
        if self.frame.modes.mouse_reporting && !ev.modifiers.shift && !ev.modifiers.control {
            self.send(Input::Mouse(MouseEvent {
                kind: MouseKind::Press(MouseButton::Left),
                col,
                row,
                px_in_cell,
                mods: Self::mods(&ev.modifiers),
            }));
            return;
        }
        if ev.modifiers.control {
            self.link_candidate = Some((col, row));
            return;
        }
        let kind = match ev.click_count {
            2 => SelectKind::Word,
            3.. => SelectKind::Line,
            _ => SelectKind::Char,
        };
        self.send(Input::SelectionBegin {
            col,
            row,
            kind,
            rectangle: ev.modifiers.alt,
        });
        self.selecting = true;
    }

    fn mouse_down_middle(
        &mut self,
        ev: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.frame.modes.mouse_reporting && !ev.modifiers.shift {
            if let Some((col, row, px_in_cell)) = self.cell_at(ev.position) {
                for kind in [
                    MouseKind::Press(MouseButton::Middle),
                    MouseKind::Release(MouseButton::Middle),
                ] {
                    self.send(Input::Mouse(MouseEvent {
                        kind,
                        col,
                        row,
                        px_in_cell,
                        mods: Self::mods(&ev.modifiers),
                    }));
                }
            }
            return;
        }
        if let Some(text) = cx.read_from_primary().and_then(|item| item.text()) {
            self.send(Input::Paste(text));
        }
    }

    /// Mouse move, registered at window level so drags outside the element
    /// still extend the selection.
    pub(crate) fn mouse_move(&mut self, ev: &MouseMoveEvent) {
        let Some((col, row, px_in_cell)) = self.cell_at(ev.position) else {
            return;
        };
        if ev.dragging() && self.selecting {
            self.link_candidate = None;
            self.send(Input::SelectionExtend { col, row });
        } else if self.frame.modes.mouse_reporting {
            self.send(Input::Mouse(MouseEvent {
                kind: MouseKind::Motion,
                col,
                row,
                px_in_cell,
                mods: Self::mods(&ev.modifiers),
            }));
        }
    }

    fn mouse_up_left(&mut self, ev: &MouseUpEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let pos = self.cell_at(ev.position);
        if self.frame.modes.mouse_reporting && !ev.modifiers.shift && !self.selecting {
            if let Some((col, row, px_in_cell)) = pos {
                self.send(Input::Mouse(MouseEvent {
                    kind: MouseKind::Release(MouseButton::Left),
                    col,
                    row,
                    px_in_cell,
                    mods: Self::mods(&ev.modifiers),
                }));
            }
            return;
        }
        if let Some((col, row)) = self.link_candidate.take() {
            self.send(Input::LinkAt { col, row });
            return;
        }
        if self.selecting {
            self.selecting = false;
            // Auto-copy to the primary selection (Linux convention).
            self.pending_copy = Some(ClipboardTarget::Primary);
            self.send(Input::CopySelection);
            cx.notify();
        }
    }

    fn scroll_wheel(
        &mut self,
        ev: &ScrollWheelEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let delta = match ev.delta {
            ScrollDelta::Lines(p) => p,
            ScrollDelta::Pixels(p) => point(
                f32::from(p.x) / f32::from(self.cell.width).max(1.0),
                f32::from(p.y) / f32::from(self.cell.height).max(1.0),
            ),
        };
        self.wheel_remainder += delta;
        let lines = self.wheel_remainder.y.trunc() as i32;
        self.wheel_remainder.y -= lines as f32;
        let dx = self.wheel_remainder.x.trunc() as i32;
        self.wheel_remainder.x -= dx as f32;
        if lines == 0 && dx == 0 {
            return;
        }
        if self.frame.modes.mouse_reporting {
            let (col, row, px_in_cell) = self.cell_at(ev.position).unwrap_or((0, 0, (0, 0)));
            self.send(Input::Mouse(MouseEvent {
                kind: MouseKind::Scroll {
                    dy_lines: -lines,
                    dx_lines: dx,
                },
                col,
                row,
                px_in_cell,
                mods: Self::mods(&ev.modifiers),
            }));
        } else {
            self.send(Input::ScrollLines(-lines));
        }
        cx.notify();
    }

    // -------------------------------------------------------------- actions

    fn on_copy(&mut self, _: &Copy, _window: &mut Window, cx: &mut Context<Self>) {
        self.pending_copy = Some(ClipboardTarget::Clipboard);
        self.send(Input::CopySelection);
        cx.stop_propagation();
    }

    fn on_paste(&mut self, _: &Paste, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.send(Input::Paste(text));
        }
        cx.stop_propagation();
    }

    fn on_select_all(&mut self, _: &SelectAll, _window: &mut Window, cx: &mut Context<Self>) {
        self.send(Input::SelectAll);
        cx.stop_propagation();
    }

    fn on_scroll_page_up(&mut self, _: &ScrollPageUp, _: &mut Window, cx: &mut Context<Self>) {
        self.send(Input::ScrollLines(-i32::from(self.frame.rows.max(1))));
        cx.stop_propagation();
    }

    fn on_scroll_page_down(&mut self, _: &ScrollPageDown, _: &mut Window, cx: &mut Context<Self>) {
        self.send(Input::ScrollLines(i32::from(self.frame.rows.max(1))));
        cx.stop_propagation();
    }

    fn on_scroll_top(&mut self, _: &ScrollToTop, _: &mut Window, cx: &mut Context<Self>) {
        self.send(Input::ScrollTo(ScrollTarget::Top));
        cx.stop_propagation();
    }

    fn on_scroll_bottom(&mut self, _: &ScrollToBottom, _: &mut Window, cx: &mut Context<Self>) {
        self.send(Input::ScrollTo(ScrollTarget::Bottom));
        cx.stop_propagation();
    }

    fn zoom(&mut self, delta: i32, cx: &mut Context<Self>) {
        self.zoom_steps = (self.zoom_steps + delta).clamp(-10, 10);
        self.font_size = px(self.base_font_size * ZOOM_STEP.powi(self.zoom_steps));
        self.last_grid = None;
        cx.notify();
    }

    fn on_zoom_in(&mut self, _: &ZoomIn, _: &mut Window, cx: &mut Context<Self>) {
        self.zoom(1, cx);
        cx.stop_propagation();
    }

    fn on_zoom_out(&mut self, _: &ZoomOut, _: &mut Window, cx: &mut Context<Self>) {
        self.zoom(-1, cx);
        cx.stop_propagation();
    }

    fn on_zoom_reset(&mut self, _: &ZoomReset, _: &mut Window, cx: &mut Context<Self>) {
        self.zoom_steps = 0;
        self.font_size = px(self.base_font_size);
        self.last_grid = None;
        cx.notify();
        cx.stop_propagation();
    }

    fn on_quit(&mut self, _: &Quit, window: &mut Window, cx: &mut Context<Self>) {
        self.send(Input::Shutdown);
        window.remove_window();
        cx.quit();
    }
}

// --------------------------------------------------------------------- IME

impl EntityInputHandler for Pane {
    fn text_for_range(
        &mut self,
        _range: Range<usize>,
        _adjusted: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        // A terminal has no editable document; committed text goes straight to
        // the PTY via `replace_text_in_range`.
        None
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        // Always report an empty selection so IME candidate windows position
        // themselves at the cursor even in ALT_SCREEN apps.
        Some(UTF16Selection {
            range: 0..0,
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.preedit
            .as_ref()
            .map(|text| 0..text.encode_utf16().count())
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.preedit = None;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.preedit = None;
        if !text.is_empty() {
            // Committed text (typing, IME commit, dead-key composition) goes in
            // raw — never bracketed-paste wrapped.
            self.send(Input::Bytes(text.as_bytes().to_vec()));
        }
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        new_text: &str,
        _new_selected_range: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.preedit = if new_text.is_empty() {
            None
        } else {
            Some(new_text.to_string())
        };
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        _element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let cursor = self.frame.cursor.as_ref()?;
        let col = u32::from(cursor.col) + range.start.min(u32::MAX as usize) as u32;
        let origin = self.content_origin
            + point(
                self.cell.width * col as f32,
                self.cell.height * f32::from(cursor.row),
            );
        Some(Bounds::new(origin, size(self.cell.width, self.cell.height)))
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

// ------------------------------------------------------------------ render

fn base_font(config: &Config) -> gpui::Font {
    let mut font = gpui::font(config.font_family.clone());
    if !config.font_ligatures {
        font.features = gpui::FontFeatures::disable_ligatures();
    }
    // Glyph-level fallback: prefer the configured family, then the generic
    // monospace stack.
    font.fallbacks = Some(gpui::FontFallbacks::from_fonts(vec![
        "monospace".to_string(),
        "DejaVu Sans Mono".to_string(),
    ]));
    font
}

impl Render for Pane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("pane")
            .key_context("Pane")
            .track_focus(&self.focus)
            .size_full()
            .bg(theme::to_hsla(self.palette.bg))
            .on_action(cx.listener(Self::on_copy))
            .on_action(cx.listener(Self::on_paste))
            .on_action(cx.listener(Self::on_select_all))
            .on_action(cx.listener(Self::on_scroll_page_up))
            .on_action(cx.listener(Self::on_scroll_page_down))
            .on_action(cx.listener(Self::on_scroll_top))
            .on_action(cx.listener(Self::on_scroll_bottom))
            .on_action(cx.listener(Self::on_zoom_in))
            .on_action(cx.listener(Self::on_zoom_out))
            .on_action(cx.listener(Self::on_zoom_reset))
            .on_action(cx.listener(Self::on_quit))
            .on_key_down(cx.listener(Self::key_down))
            .on_key_up(cx.listener(Self::key_up))
            .on_mouse_down(GpuiMouseButton::Left, cx.listener(Self::mouse_down_left))
            .on_mouse_down(
                GpuiMouseButton::Middle,
                cx.listener(Self::mouse_down_middle),
            )
            .on_mouse_up(GpuiMouseButton::Left, cx.listener(Self::mouse_up_left))
            .on_scroll_wheel(cx.listener(Self::scroll_wheel))
            .child(TerminalElement::new(cx.entity()))
    }
}
