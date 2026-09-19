//! The libghostty-vt terminal behind one pane, without a PTY.
//!
//! `Emulator` owns the `!Send` terminal, its render state, encoders and
//! selection/search state. Tests drive it directly; `pane.rs` wraps one in a
//! thread with a real PTY.

use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    rc::Rc,
};

use anyhow::{Result, anyhow};
use libghostty_vt::{
    RenderState, Terminal, key,
    kitty::graphics::PlacementIterator,
    mouse,
    render::{CellIterator, CursorVisualStyle, Dirty, RowIterator},
    screen::{CellWide, Screen, TrackedGridRef},
    selection::{SelectLineOptions, SelectWordBetweenOptions, SelectWordOptions, Selection},
    style::{PaletteIndex, RgbColor, StyleColor, Underline},
    terminal::{
        ConformanceLevel, DeviceAttributeFeature, DeviceAttributes, DeviceType, Mode, Point,
        PointCoordinate, PointSpace, PrimaryDeviceAttributes, ScrollViewport,
        SecondaryDeviceAttributes, SizeReportSize,
    },
    unicode::codepoint_width,
};

use crate::{
    frame::{
        Cursor, CursorShape, Frame, ImageData, Kitty, Line, Match, Modes, Placement, Rgb, Run,
        Scroll, Style,
    },
    graphics,
    input::{
        ClipboardTarget, Event, Input, Key, KeyAction, KeyEvent, MouseButton, MouseEvent,
        MouseKind, NamedKey, ScrollTarget, SelectKind,
    },
    links,
    palette::Palette,
    search,
};

fn anyhow_err(e: libghostty_vt::Error) -> anyhow::Error {
    anyhow!("{e:?}")
}

/// Configuration for a new emulator. `Palette` and the cursor shape come from
/// `option_term_core::config` via `palette.rs`.
pub struct EmulatorOptions {
    pub cols: u16,
    pub rows: u16,
    pub cell_w_px: u16,
    pub cell_h_px: u16,
    /// Scrollback limit in lines.
    pub scrollback: u32,
    pub palette: Palette,
    pub cursor_shape: CursorShape,
    pub cursor_blink: bool,
    /// Kitty storage cap in bytes; `graphics::STORAGE_LIMIT` is the default,
    /// `0` disables the protocol.
    pub kitty_storage_limit: u64,
    pub xtversion: String,
}

/// Only whitespace ends a word for link detection: the default boundary set
/// breaks on `/`, `:` and `.`, which would chop every URL into pieces.
const WORD_BOUNDARIES: &[char] = &[' ', '\t', '\n', '\r', '"', '\'', '`', '<', '>', '|'];

pub struct Emulator {
    terminal: Terminal<'static, 'static>,
    render_state: RenderState<'static>,
    row_it: RowIterator<'static>,
    cell_it: CellIterator<'static>,
    kitty_iter: Option<PlacementIterator<'static>>,
    key_enc: key::Encoder<'static>,
    key_ev: key::Event<'static>,
    mouse_enc: mouse::Encoder<'static>,
    mouse_ev: mouse::Event<'static>,

    /// Bytes the terminal wants on the PTY (DA/kitty/OSC replies) plus what
    /// our key/mouse/paste encoders produce.
    output: Rc<RefCell<Vec<u8>>>,
    /// Title/Bell/ClipboardWrite accumulated by callbacks.
    events: Rc<RefCell<Vec<Event>>>,
    /// Grid + cell size the `on_size` callback reports on XTWINOPS.
    size: Rc<Cell<(u16, u16, u32, u32)>>,

    opts: EmulatorOptions,

    sel_start: Option<TrackedGridRef>,
    sel_end: Option<TrackedGridRef>,
    sel_rectangle: bool,
    sel_kind: SelectKind,

    needle: Option<String>,
    matches: Vec<Match>,
    active_match: Option<usize>,

    /// Kitty storage bookkeeping for `Kitty::images`/`dropped` rules.
    kitty_generation: u64,
    kitty_sent: HashSet<u32>,
    kitty_known: HashSet<u32>,

    /// Non-VT changes (selection, search, scroll) that must still emit a frame.
    forced_dirty: bool,
    seq: u64,
}

impl Emulator {
    pub fn new(opts: EmulatorOptions) -> Result<Self> {
        let mut terminal = Terminal::new(opts.cols, opts.rows).map_err(anyhow_err)?;
        terminal
            .set_scrollback_max_lines(Some(opts.scrollback as usize))
            .map_err(anyhow_err)?;
        terminal
            .resize(
                opts.cols,
                opts.rows,
                u32::from(opts.cell_w_px),
                u32::from(opts.cell_h_px),
            )
            .map_err(anyhow_err)?;
        apply_palette(&mut terminal, &opts)?;

        // Kitty graphics: a non-zero storage limit turns it on; the PNG
        // decoder (thread-local) is what makes `f=100` payloads work.
        graphics::install_png_decoder();
        if opts.kitty_storage_limit > 0 {
            terminal
                .set_kitty_image_storage_limit(opts.kitty_storage_limit)
                .map_err(anyhow_err)?;
            terminal
                .set_kitty_image_from_file_allowed(true)
                .map_err(anyhow_err)?;
            terminal
                .set_kitty_image_from_shared_mem_allowed(true)
                .map_err(anyhow_err)?;
        }

        let output = Rc::new(RefCell::new(Vec::new()));
        let events = Rc::new(RefCell::new(Vec::new()));
        let size = Rc::new(Cell::new((
            opts.cols,
            opts.rows,
            u32::from(opts.cell_w_px),
            u32::from(opts.cell_h_px),
        )));

        {
            let output = output.clone();
            terminal
                .on_pty_write(move |_t, data| output.borrow_mut().extend_from_slice(data))
                .map_err(anyhow_err)?;
        }
        {
            let size = size.clone();
            terminal
                .on_size(move |_t| {
                    let (columns, rows, cell_width, cell_height) = size.get();
                    Some(SizeReportSize {
                        rows,
                        columns,
                        cell_width,
                        cell_height,
                    })
                })
                .map_err(anyhow_err)?;
        }
        terminal
            .on_device_attributes(|_t| {
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
            .map_err(anyhow_err)?;
        // The callback must return a &'t str, so the configured version is
        // leaked once per emulator — a bounded, one-time allocation.
        let xtversion: &'static str = Box::leak(opts.xtversion.clone().into_boxed_str());
        terminal
            .on_xtversion(move |_t| Some(xtversion))
            .map_err(anyhow_err)?;
        {
            let events = events.clone();
            terminal
                .on_title_changed(move |t| {
                    let title = sanitize_title(t.title().unwrap_or_default());
                    if !title.is_empty() {
                        events.borrow_mut().push(Event::Title(title));
                    }
                })
                .map_err(anyhow_err)?;
        }
        // OSC 52 / iTerm2 copy: libghostty normalises the protocol and hands
        // us decoded MIME parts. Without this callback, CLIs that copy via
        // OSC 52 (Grok, OpenCode, …) silently fail.
        {
            let events = events.clone();
            terminal
                .on_clipboard_write(move |_t, write| {
                    let location = write.location();
                    let mut chosen: Option<String> = None;
                    for content in write.contents() {
                        if content.mime == "text/plain" || content.mime.starts_with("text/") {
                            chosen = Some(String::from_utf8_lossy(content.data).into_owned());
                            break;
                        }
                        if chosen.is_none() {
                            chosen = Some(String::from_utf8_lossy(content.data).into_owned());
                        }
                    }
                    let Some(text) = chosen else {
                        return Err(libghostty_vt::terminal::ClipboardWriteError::InvalidData);
                    };
                    // OSC 52 `c` → CLIPBOARD; `p`/`s` → PRIMARY selection.
                    let target = match location {
                        libghostty_vt::terminal::ClipboardLocation::Standard => {
                            ClipboardTarget::Clipboard
                        }
                        _ => ClipboardTarget::Primary,
                    };
                    events
                        .borrow_mut()
                        .push(Event::ClipboardWrite { target, text });
                    Ok(())
                })
                .map_err(anyhow_err)?;
        }
        {
            let events = events.clone();
            terminal
                .on_bell(move |_t| events.borrow_mut().push(Event::Bell))
                .map_err(anyhow_err)?;
        }
        terminal.on_color_scheme(|_t| None).map_err(anyhow_err)?;

        Ok(Self {
            terminal,
            render_state: RenderState::new().map_err(anyhow_err)?,
            row_it: RowIterator::new().map_err(anyhow_err)?,
            cell_it: CellIterator::new().map_err(anyhow_err)?,
            kitty_iter: PlacementIterator::new()
                .inspect_err(|err| tracing::warn!("kitty placement iterator unavailable: {err:?}"))
                .ok(),
            key_enc: key::Encoder::new().map_err(anyhow_err)?,
            key_ev: key::Event::new().map_err(anyhow_err)?,
            mouse_enc: mouse::Encoder::new().map_err(anyhow_err)?,
            mouse_ev: mouse::Event::new().map_err(anyhow_err)?,
            output,
            events,
            size,
            opts,
            sel_start: None,
            sel_end: None,
            sel_rectangle: false,
            sel_kind: SelectKind::Char,
            needle: None,
            matches: Vec::new(),
            active_match: None,
            kitty_generation: 0,
            kitty_sent: HashSet::new(),
            kitty_known: HashSet::new(),
            forced_dirty: true, // first snapshot must always emit
            seq: 0,
        })
    }

    /// Feed raw PTY output through the VT parser.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.terminal.vt_write(bytes);
    }

    /// Bytes the terminal wants written to the PTY.
    pub fn take_output(&mut self) -> Vec<u8> {
        std::mem::take(&mut *self.output.borrow_mut())
    }

    /// Title/Bell/ClipboardWrite events accumulated since the last call.
    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut *self.events.borrow_mut())
    }

    /// Whether a new frame would differ from the last one emitted.
    ///
    /// `RenderState::update` consumes the terminal's dirty state, so a
    /// positive answer is latched into `forced_dirty`: repeat calls between
    /// snapshots keep returning true until `snapshot()` clears it.
    pub fn is_dirty(&mut self) -> bool {
        if self.forced_dirty {
            return true;
        }
        let vt_dirty = self
            .render_state
            .update(&self.terminal)
            .and_then(|s| s.dirty())
            .is_ok_and(|d| d != Dirty::Clean);
        let kitty_dirty = self
            .terminal
            .kitty_graphics()
            .and_then(|g| g.generation())
            .is_ok_and(|g| g != self.kitty_generation);
        let dirty = vt_dirty || kitty_dirty;
        self.forced_dirty |= dirty;
        dirty
    }

    pub fn handle(&mut self, input: Input) -> Result<()> {
        match input {
            Input::Key(ev) => self.encode_key(&ev),
            Input::Mouse(ev) => self.encode_mouse(&ev),
            Input::Paste(text) => self.paste(&text),
            Input::Bytes(bytes) => {
                self.output.borrow_mut().extend_from_slice(&bytes);
                Ok(())
            }
            Input::Resize {
                cols,
                rows,
                cell_w_px,
                cell_h_px,
            } => {
                self.size
                    .set((cols, rows, u32::from(cell_w_px), u32::from(cell_h_px)));
                self.terminal
                    .resize(cols, rows, u32::from(cell_w_px), u32::from(cell_h_px))
                    .map_err(anyhow_err)?;
                self.forced_dirty = true;
                Ok(())
            }
            Input::ScrollLines(n) => {
                if self.on_alternate_screen() {
                    self.send_cursor_keys(n as isize);
                } else {
                    self.terminal
                        .scroll_viewport(ScrollViewport::Delta(n as isize));
                }
                self.forced_dirty = true;
                Ok(())
            }
            Input::ScrollTo(target) => {
                let target = match target {
                    ScrollTarget::Top => ScrollViewport::Top,
                    ScrollTarget::Bottom => ScrollViewport::Bottom,
                    ScrollTarget::Rows(n) => ScrollViewport::Row(n),
                };
                self.terminal.scroll_viewport(target);
                self.forced_dirty = true;
                Ok(())
            }
            Input::SelectionBegin {
                col,
                row,
                kind,
                rectangle,
            } => {
                self.selection_begin(col, row, kind, rectangle);
                Ok(())
            }
            Input::SelectionExtend { col, row } => {
                self.selection_extend(col, row);
                Ok(())
            }
            Input::SelectionClear => {
                self.clear_selection();
                Ok(())
            }
            Input::SelectAll => {
                let sel = self.terminal.select_all().ok().flatten();
                let points = self.selection_points(sel.as_ref());
                let _ = self.terminal.set_selection(sel.as_ref());
                self.set_selection_points(points);
                self.sel_kind = SelectKind::Char;
                self.forced_dirty = true;
                Ok(())
            }
            Input::CopySelection => {
                let text = self.selection_text();
                self.events.borrow_mut().push(Event::SelectionText(text));
                Ok(())
            }
            Input::LinkAt { col, row } => {
                let link = self.link_at(col, row);
                self.events.borrow_mut().push(Event::Link(link));
                Ok(())
            }
            Input::Search(needle) => {
                self.set_search(needle);
                Ok(())
            }
            Input::SearchNext => {
                self.step_match(1);
                Ok(())
            }
            Input::SearchPrev => {
                self.step_match(-1);
                Ok(())
            }
            Input::Focus(gained) => {
                if self.terminal.mode(Mode::FOCUS_EVENT).unwrap_or(false) {
                    let event = if gained {
                        libghostty_vt::focus::Event::Gained
                    } else {
                        libghostty_vt::focus::Event::Lost
                    };
                    let mut buf = [0u8; 8];
                    if let Ok(len) = event.encode(&mut buf) {
                        self.output.borrow_mut().extend_from_slice(&buf[..len]);
                    }
                }
                Ok(())
            }
            Input::ClearScreenAndScrollback => {
                // ED2 clears the screen, ED3 the scrollback, CUP homes.
                self.terminal.vt_write(b"\x1b[2J\x1b[3J\x1b[H");
                Ok(())
            }
            Input::RequestFrame => {
                self.forced_dirty = true;
                Ok(())
            }
            // The pane thread intercepts Shutdown before it reaches here.
            Input::Shutdown => Ok(()),
        }
    }

    /// Build the immutable frame for the UI.
    pub fn snapshot(&mut self) -> Frame {
        self.seq += 1;
        self.forced_dirty = false;

        // The snapshot holds `&mut render_state`, so everything read from it
        // happens in this block using only disjoint fields of `self`.
        let (cols, rows, lines, cursor, default_bg, default_fg) = {
            let Ok(snapshot) = self.render_state.update(&self.terminal) else {
                tracing::warn!("render state update failed");
                return self.empty_frame();
            };
            // The caller owns clearing the render state dirty flag — update()
            // sets it but never resets it to Clean on its own.
            let _ = snapshot.set_dirty(Dirty::Clean);
            let colors = snapshot
                .colors()
                .unwrap_or_else(|_| libghostty_vt::render::Colors {
                    background: RgbColor::default(),
                    foreground: RgbColor::default(),
                    cursor: None,
                    palette: [RgbColor::default(); 256],
                });
            let cursor = Self::cursor(&snapshot, &self.opts);
            let lines = Self::collect_lines(
                &snapshot,
                &mut self.row_it,
                &mut self.cell_it,
                &colors,
                &self.opts.palette,
            );
            (
                snapshot.cols().unwrap_or(self.opts.cols),
                snapshot.rows().unwrap_or(self.opts.rows),
                lines,
                cursor,
                rgb(colors.background),
                rgb(colors.foreground),
            )
        };

        let kitty = self.collect_kitty();
        let title = sanitize_title(self.terminal.title().unwrap_or_default());
        let pwd = self
            .terminal
            .pwd()
            .ok()
            .filter(|p| !p.is_empty())
            .map(links::pwd_to_path);

        Frame {
            cols,
            rows,
            lines,
            cursor,
            scroll: Self::scroll_state(&self.terminal),
            modes: Self::modes(&self.terminal),
            title,
            pwd,
            kitty,
            matches: self.matches.clone(),
            active_match: self.active_match,
            default_bg,
            default_fg,
            seq: self.seq,
        }
    }

    fn empty_frame(&self) -> Frame {
        Frame {
            cols: self.opts.cols,
            rows: self.opts.rows,
            lines: Vec::new(),
            cursor: None,
            scroll: Scroll::default(),
            modes: Self::modes(&self.terminal),
            title: String::new(),
            pwd: None,
            kitty: Kitty::default(),
            matches: Vec::new(),
            active_match: None,
            default_bg: self.opts.palette.bg,
            default_fg: self.opts.palette.fg,
            seq: self.seq,
        }
    }

    // --- snapshot helpers -------------------------------------------------

    fn cursor(
        snapshot: &libghostty_vt::render::Snapshot<'static, '_>,
        opts: &EmulatorOptions,
    ) -> Option<Cursor> {
        if !snapshot.cursor_visible().unwrap_or(false) {
            return None;
        }
        let pos = snapshot.cursor_viewport().ok()??;
        let shape = match snapshot.cursor_visual_style() {
            // DECSCUSR default: honor the configured shape.
            Ok(CursorVisualStyle::Block) => opts.cursor_shape,
            Ok(CursorVisualStyle::Bar) => CursorShape::Bar,
            Ok(CursorVisualStyle::Underline) => CursorShape::Underline,
            Ok(CursorVisualStyle::BlockHollow) => CursorShape::BlockHollow,
            _ => opts.cursor_shape,
        };
        Some(Cursor {
            col: pos.x,
            row: pos.y,
            shape,
            color: snapshot
                .cursor_color()
                .ok()
                .flatten()
                .map(rgb)
                .unwrap_or(opts.palette.cursor),
            text_color: opts.palette.cursor_text,
            blinking: opts.cursor_blink && snapshot.cursor_blinking().unwrap_or(false),
        })
    }

    fn collect_lines(
        snapshot: &libghostty_vt::render::Snapshot<'static, '_>,
        row_it: &mut RowIterator<'static>,
        cell_it: &mut CellIterator<'static>,
        colors: &libghostty_vt::render::Colors,
        palette: &Palette,
    ) -> Vec<Line> {
        let default_fg = colors.foreground;
        let mut lines = Vec::new();
        let Ok(mut rows) = row_it.update(snapshot) else {
            return lines;
        };
        let mut text = String::with_capacity(16);
        while let Some(row) = rows.next() {
            let mut line = Line::default();
            let Ok(mut cells) = cell_it.update(row) else {
                lines.push(line);
                continue;
            };
            let mut col = 0u16;
            let mut run = RunBuilder::default();
            while let Some(cell) = cells.next() {
                let raw = cell.raw_cell().ok();
                let wide = raw.and_then(|c| c.wide().ok());
                if matches!(wide, Some(CellWide::SpacerTail)) {
                    // Tail of a wide char: covered by its run.
                    col = col.saturating_add(1);
                    continue;
                }
                let spacer_head = matches!(wide, Some(CellWide::SpacerHead));
                text.clear();
                if !spacer_head {
                    let _ = cell.graphemes_utf8(&mut text);
                }
                let blank = text.is_empty() || text == " ";
                let selected = cell.is_selected().unwrap_or(false);
                let bg = cell
                    .bg_color()
                    .ok()
                    .flatten()
                    .map(rgb)
                    .or(selected.then_some(palette.selection_bg));

                if blank && bg.is_none() && !selected {
                    // Empty cell on the default background: absorbed into the
                    // run only if a later same-pen cell continues it.
                    run.gap += 1;
                    col = col.saturating_add(1);
                    continue;
                }

                let style = cell.style().unwrap_or_default();
                let fg = if selected {
                    palette.selection_fg
                } else {
                    cell.fg_color()
                        .ok()
                        .flatten()
                        .map(rgb)
                        .unwrap_or_else(|| rgb(default_fg))
                };
                let cell_style = map_style(&style);
                let underline_color = resolve_color(style.underline_color, colors).map(rgb);
                let hyperlink = raw.and_then(|c| c.has_hyperlink().ok()).unwrap_or(false);
                let width = match wide {
                    Some(CellWide::Wide) => 2,
                    _ => 1,
                };
                let batchable = blank || is_batchable(&text, &cell_style, width);

                if batchable {
                    run.push(
                        &mut line.runs,
                        col,
                        Pen {
                            fg,
                            bg,
                            underline_color,
                            style: cell_style,
                            selected,
                            hyperlink,
                        },
                        if blank { " " } else { &text },
                        width,
                    );
                } else {
                    // Decorated, wide or composed cells get their own run so
                    // the UI can keep them pixel-identical per cell.
                    run.flush(&mut line.runs);
                    line.runs.push(Run {
                        col,
                        cells: width,
                        text: if blank { " ".to_string() } else { text.clone() },
                        fg,
                        bg,
                        underline_color,
                        style: cell_style,
                        selected,
                        hyperlink,
                    });
                }
                col = col.saturating_add(width);
            }
            run.flush(&mut line.runs);
            lines.push(line);
        }
        lines
    }

    fn collect_kitty(&mut self) -> Kitty {
        let mut out = Kitty::default();
        let Ok(graphics) = self.terminal.kitty_graphics() else {
            return out;
        };
        let generation = graphics.generation().unwrap_or(0);
        // Track the generation even when the placement iterator is missing:
        // `is_dirty` compares against this and would otherwise stay latched.
        if generation != self.kitty_generation {
            // New storage generation: nothing counts as sent anymore.
            self.kitty_generation = generation;
            self.kitty_sent.clear();
        }
        out.generation = generation;

        let Some(iter) = self.kitty_iter.as_mut() else {
            return out;
        };
        let Ok(mut placements) = iter.update(&graphics) else {
            return out;
        };
        let _ = placements.set_layer(libghostty_vt::kitty::graphics::Layer::All);

        let mut current: HashSet<u32> = HashSet::new();
        let mut frame_images: HashSet<u32> = HashSet::new();
        while let Some(p) = placements.next() {
            let Ok(image_id) = p.image_id() else {
                continue;
            };
            current.insert(image_id);
            let Some(image) = graphics.image(image_id) else {
                continue;
            };
            let (Ok(Some(pos)), Ok(size), Ok(src)) = (
                p.viewport_pos(&image, &self.terminal),
                p.pixel_size(&image, &self.terminal),
                p.source_rect(&image),
            ) else {
                continue;
            };
            if size.width == 0 || size.height == 0 || src.width == 0 || src.height == 0 {
                continue;
            }
            out.placements.push(Placement {
                image_id,
                placement_id: p.placement_id().unwrap_or(0),
                z: p.z().unwrap_or(0),
                col: pos.col,
                row: pos.row,
                offset_px: (
                    p.x_offset().unwrap_or(0).min(u16::MAX as u32) as u16,
                    p.y_offset().unwrap_or(0).min(u16::MAX as u32) as u16,
                ),
                dest_px: (size.width, size.height),
                src_px: (src.x, src.y, src.width, src.height),
            });

            // Ship the pixels only the first time this generation sees them.
            if !self.kitty_sent.contains(&image_id)
                && frame_images.insert(image_id)
                && let Ok(Some(data)) = image.data()
                && let (Ok(w), Ok(h)) = (image.width(), image.height())
                && let Some(rgba) = graphics::to_rgba8(data, w, h)
            {
                out.images.push(ImageData {
                    id: image_id,
                    width: w,
                    height: h,
                    rgba,
                });
                self.kitty_sent.insert(image_id);
            }
        }

        // Ids we reported before that are no longer in storage.
        self.kitty_known.extend(&current);
        let mut dropped = Vec::new();
        self.kitty_known.retain(|id| {
            let alive = graphics.image(*id).is_some();
            if !alive {
                dropped.push(*id);
                self.kitty_sent.remove(id);
            }
            alive
        });
        dropped.sort_unstable();
        out.dropped = dropped;
        out
    }

    /// Viewport position as lines-above-bottom + scrollback size, derived from
    /// the scrollbar report rather than a counter of our own.
    fn scroll_state(terminal: &Terminal<'_, '_>) -> Scroll {
        let Ok(sb) = terminal.scrollbar() else {
            return Scroll::default();
        };
        let scrollback = (sb.total as usize).saturating_sub(sb.len as usize);
        let offset = scrollback.saturating_sub(sb.offset as usize);
        Scroll {
            offset,
            total: scrollback,
        }
    }

    fn modes(t: &Terminal<'_, '_>) -> Modes {
        Modes {
            mouse_reporting: t.is_mouse_tracking().unwrap_or(false),
            bracketed_paste: t.mode(Mode::BRACKETED_PASTE).unwrap_or(false),
            alt_screen: matches!(t.active_screen(), Ok(Screen::Alternate)),
            focus_events: t.mode(Mode::FOCUS_EVENT).unwrap_or(false),
            kitty_keyboard: t
                .kitty_keyboard_flags()
                .map(|f| !f.is_empty())
                .unwrap_or(false),
        }
    }

    // --- input ------------------------------------------------------------

    fn encode_key(&mut self, ev: &KeyEvent) -> Result<()> {
        let action = match ev.action {
            KeyAction::Press => key::Action::Press,
            KeyAction::Release => key::Action::Release,
            KeyAction::Repeat => key::Action::Repeat,
        };
        let mods = to_key_mods(ev.mods);
        // Shift that produced the text is consumed, matching the 0.1.x rule.
        let mut consumed = key::Mods::empty();
        if ev.text.is_some() && mods.contains(key::Mods::SHIFT) {
            consumed |= key::Mods::SHIFT;
        }
        let unshifted = ev
            .unshifted
            .or_else(|| {
                ev.text
                    .as_deref()
                    .and_then(|t| t.chars().next())
                    .map(|c| c.to_ascii_lowercase())
            })
            .unwrap_or('\0');

        self.key_ev
            .set_action(action)
            .set_key(to_vt_key(ev.key))
            .set_mods(mods)
            .set_consumed_mods(consumed)
            .set_unshifted_codepoint(unshifted)
            .set_utf8(ev.text.clone());

        let before = self.output.borrow().len();
        self.key_enc
            .set_options_from_terminal(&self.terminal)
            .encode_to_vec(&self.key_ev, &mut self.output.borrow_mut())
            .map_err(anyhow_err)?;

        // Encoder produced nothing: for plain typing the text itself is the
        // input (no Kitty flags, no application cursor keys).
        let out = self.output.borrow();
        if out.len() == before
            && let Some(text) = &ev.text
            && !mods.intersects(key::Mods::CTRL | key::Mods::ALT | key::Mods::SUPER)
            && matches!(ev.action, KeyAction::Press | KeyAction::Repeat)
        {
            drop(out);
            self.output.borrow_mut().extend_from_slice(text.as_bytes());
        }
        Ok(())
    }

    fn encode_mouse(&mut self, ev: &MouseEvent) -> Result<()> {
        let (cols, rows, cell_w, cell_h) = self.size.get();
        self.mouse_enc.set_options_from_terminal(&self.terminal);
        self.mouse_enc.set_size(mouse::EncoderSize {
            screen_width: u32::from(cols) * cell_w,
            screen_height: u32::from(rows) * cell_h,
            cell_width: cell_w.max(1),
            cell_height: cell_h.max(1),
            padding_top: 0,
            padding_bottom: 0,
            padding_left: 0,
            padding_right: 0,
        });

        let pos = mouse::Position {
            x: f32::from(ev.col) * cell_w as f32 + ev.px_in_cell.0 as f32,
            y: f32::from(ev.row) * cell_h as f32 + ev.px_in_cell.1 as f32,
        };
        let mods = to_key_mods(ev.mods);

        let (action, button, repeat) = match ev.kind {
            MouseKind::Press(b) => (mouse::Action::Press, Some(to_mouse_button(b)), 1),
            MouseKind::Release(b) => (mouse::Action::Release, Some(to_mouse_button(b)), 1),
            MouseKind::Motion => (mouse::Action::Motion, None, 1),
            // A wheel line is one discrete button-4/5 click.
            MouseKind::Scroll { dy_lines, .. } => (
                mouse::Action::Press,
                Some(if dy_lines < 0 {
                    mouse::Button::Four
                } else {
                    mouse::Button::Five
                }),
                dy_lines.unsigned_abs(),
            ),
        };

        self.mouse_ev
            .set_action(action)
            .set_button(button)
            .set_mods(mods)
            .set_position(pos);
        let mut out = self.output.borrow_mut();
        for _ in 0..repeat {
            self.mouse_enc
                .encode_to_vec(&self.mouse_ev, &mut out)
                .map_err(anyhow_err)?;
        }
        Ok(())
    }

    fn paste(&mut self, text: &str) -> Result<()> {
        let bracketed = self.terminal.mode(Mode::BRACKETED_PASTE).unwrap_or(false);
        let mut data = text.as_bytes().to_vec();
        let mut buf = vec![0u8; data.len() + 16];
        loop {
            match libghostty_vt::paste::encode(&mut data, bracketed, &mut buf) {
                Ok(len) => {
                    self.output.borrow_mut().extend_from_slice(&buf[..len]);
                    return Ok(());
                }
                Err(libghostty_vt::Error::OutOfSpace { required }) => buf.resize(required, 0),
                Err(err) => return Err(anyhow_err(err)),
            }
        }
    }

    /// xterm's alternate scroll: on the alternate screen the wheel becomes
    /// cursor keys, which is what `less`, `man` and friends actually read.
    fn send_cursor_keys(&mut self, lines: isize) {
        // DECCKM switches the cursor keys from CSI to SS3.
        let application = self.terminal.mode(Mode::DECCKM).unwrap_or(false);
        let seq: &[u8] = match (lines < 0, application) {
            (true, false) => b"\x1b[A",
            (true, true) => b"\x1bOA",
            (false, false) => b"\x1b[B",
            (false, true) => b"\x1bOB",
        };
        let mut out = self.output.borrow_mut();
        for _ in 0..lines.unsigned_abs() {
            out.extend_from_slice(seq);
        }
    }

    fn on_alternate_screen(&self) -> bool {
        matches!(self.terminal.active_screen(), Ok(Screen::Alternate))
    }

    // --- selection --------------------------------------------------------

    fn selection_begin(&mut self, col: u16, row: u16, kind: SelectKind, rectangle: bool) {
        let Ok(grid_ref) = self.terminal.grid_ref(Point::Viewport(PointCoordinate {
            x: col,
            y: row as u32,
        })) else {
            return;
        };
        let sel = match kind {
            SelectKind::Char => Some(Selection::new(grid_ref.clone(), grid_ref, rectangle)),
            SelectKind::Word => self
                .terminal
                .select_word(
                    SelectWordOptions::new(grid_ref).with_boundary_codepoints(WORD_BOUNDARIES),
                )
                .ok()
                .flatten(),
            SelectKind::Line => self
                .terminal
                .select_line(SelectLineOptions::new(grid_ref))
                .ok()
                .flatten(),
        };
        let points = self.selection_points(sel.as_ref());
        let _ = self.terminal.set_selection(sel.as_ref());
        self.sel_kind = kind;
        self.sel_rectangle = rectangle;
        self.set_selection_points(points);
        self.forced_dirty = true;
    }

    fn selection_extend(&mut self, col: u16, row: u16) {
        let Ok(grid_ref) = self.terminal.grid_ref(Point::Viewport(PointCoordinate {
            x: col,
            y: row as u32,
        })) else {
            return;
        };
        let Some(start) = self
            .sel_start
            .as_ref()
            .and_then(|t| t.snapshot(&self.terminal).ok().flatten())
        else {
            return;
        };
        let sel = match self.sel_kind {
            SelectKind::Word => self
                .terminal
                .select_word_between(
                    SelectWordBetweenOptions::new(start.clone(), grid_ref.clone())
                        .with_boundary_codepoints(WORD_BOUNDARIES),
                )
                .ok()
                .flatten(),
            SelectKind::Line => {
                // Expand the anchor's line to the dragged line.
                let anchor_line = self.terminal.select_line(SelectLineOptions::new(start));
                let drag_line = self.terminal.select_line(SelectLineOptions::new(grid_ref));
                match (anchor_line.ok().flatten(), drag_line.ok().flatten()) {
                    (Some(a), Some(b)) => Some(Selection::new(a.start(), b.end(), false)),
                    _ => None,
                }
            }
            SelectKind::Char => Some(Selection::new(start, grid_ref, self.sel_rectangle)),
        };
        let points = self.selection_points(sel.as_ref());
        let _ = self.terminal.set_selection(sel.as_ref());
        self.set_selection_points(points);
        self.forced_dirty = true;
    }

    fn clear_selection(&mut self) {
        let _ = self.terminal.set_selection(None);
        self.set_selection_points(None);
        self.forced_dirty = true;
    }

    /// Screen-space endpoints of a selection, for tracked refs that survive
    /// terminal mutations.
    fn selection_points(
        &self,
        sel: Option<&Selection<'_>>,
    ) -> Option<(PointCoordinate, PointCoordinate, bool)> {
        let sel = sel?;
        let start = self
            .terminal
            .point_from_grid_ref(&sel.start(), PointSpace::Screen)
            .ok()??;
        let end = self
            .terminal
            .point_from_grid_ref(&sel.end(), PointSpace::Screen)
            .ok()??;
        Some((start, end, sel.is_rectangle()))
    }

    fn set_selection_points(&mut self, points: Option<(PointCoordinate, PointCoordinate, bool)>) {
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

    fn selection_text(&self) -> Option<String> {
        let start = self.sel_start.as_ref()?.snapshot(&self.terminal).ok()??;
        let end = self.sel_end.as_ref()?.snapshot(&self.terminal).ok()??;
        let sel = Selection::new(start, end, self.sel_rectangle);
        search::selection_text(&self.terminal, &sel)
    }

    // --- links ------------------------------------------------------------

    /// URI under the cell: an explicit OSC 8 hyperlink if present, otherwise
    /// a URL or existing path detected in the word there.
    fn link_at(&self, col: u16, row: u16) -> Option<String> {
        if let Some(uri) = self.hyperlink_at(col, row) {
            return Some(uri);
        }
        let word = self.word_at(col, row)?;
        let pwd = self.terminal.pwd().ok().map(links::pwd_to_path);
        links::detect_link(&word, pwd.as_deref())
    }

    fn hyperlink_at(&self, col: u16, row: u16) -> Option<String> {
        let grid_ref = self
            .terminal
            .grid_ref(Point::Viewport(PointCoordinate {
                x: col,
                y: row as u32,
            }))
            .ok()?;
        // An empty buffer just reports the length; 0 means "no hyperlink".
        let len = grid_ref.hyperlink_uri(&mut []).ok()?;
        if len == 0 {
            return None;
        }
        let mut buf = vec![0u8; len];
        let written = grid_ref.hyperlink_uri(&mut buf).ok()?;
        buf.truncate(written);
        String::from_utf8(buf).ok().filter(|s| !s.is_empty())
    }

    /// Whitespace-delimited word under the cell, for bare-URL detection.
    fn word_at(&self, col: u16, row: u16) -> Option<String> {
        let grid_ref = self
            .terminal
            .grid_ref(Point::Viewport(PointCoordinate {
                x: col,
                y: row as u32,
            }))
            .ok()?;
        let sel = self
            .terminal
            .select_word(SelectWordOptions::new(grid_ref).with_boundary_codepoints(WORD_BOUNDARIES))
            .ok()??;
        search::selection_text(&self.terminal, &sel)
    }

    // --- search -----------------------------------------------------------

    fn set_search(&mut self, needle: Option<String>) {
        self.needle = needle;
        self.matches = match &self.needle {
            Some(n) => search::search_terminal(&self.terminal, self.opts.cols, n),
            None => Vec::new(),
        };
        self.active_match = (!self.matches.is_empty()).then_some(0);
        if let Some(idx) = self.active_match {
            self.reveal_match(self.matches[idx]);
        }
        self.forced_dirty = true;
    }

    fn step_match(&mut self, delta: isize) {
        if self.matches.is_empty() {
            return;
        }
        let len = self.matches.len() as isize;
        let cur = self.active_match.unwrap_or(0) as isize;
        let next = (cur + delta).rem_euclid(len) as usize;
        self.active_match = Some(next);
        self.reveal_match(self.matches[next]);
        self.forced_dirty = true;
    }

    /// Scroll so the hit sits near the middle of the viewport.
    fn reveal_match(&mut self, m: Match) {
        let target = (m.row as usize).saturating_sub(self.opts.rows as usize / 2);
        self.terminal.scroll_viewport(ScrollViewport::Row(target));
    }
}

/// Push the configured defaults into the terminal: colors, palette and the
/// DECSCUSR defaults so `cursor.blink` and the configured shape take effect.
fn apply_palette(terminal: &mut Terminal<'_, '_>, opts: &EmulatorOptions) -> Result<()> {
    let vt_style = match opts.cursor_shape {
        CursorShape::Block => libghostty_vt::terminal::CursorStyle::Block,
        CursorShape::Bar => libghostty_vt::terminal::CursorStyle::Bar,
        CursorShape::Underline => libghostty_vt::terminal::CursorStyle::Underline,
        CursorShape::BlockHollow => libghostty_vt::terminal::CursorStyle::BlockHollow,
    };
    terminal
        .set_default_cursor_style(Some(vt_style))
        .map_err(anyhow_err)?;
    terminal
        .set_default_cursor_blink(Some(opts.cursor_blink))
        .map_err(anyhow_err)?;

    let p = &opts.palette;
    terminal
        .set_default_fg_color(Some(vt_rgb(p.fg)))
        .map_err(anyhow_err)?;
    terminal
        .set_default_bg_color(Some(vt_rgb(p.bg)))
        .map_err(anyhow_err)?;
    terminal
        .set_default_cursor_color(Some(vt_rgb(p.cursor)))
        .map_err(anyhow_err)?;

    let mut palette = terminal.default_color_palette().map_err(anyhow_err)?;
    for (i, color) in p.colors.iter().enumerate() {
        palette.set(PaletteIndex(i as u8), vt_rgb(*color));
    }
    terminal
        .set_default_color_palette(Some(palette))
        .map_err(anyhow_err)?;
    Ok(())
}

/// OSC titles come straight from the child process: strip control characters
/// and keep them short so they cannot mangle the UI.
fn sanitize_title(title: &str) -> String {
    title
        .chars()
        .filter(|c| !c.is_control())
        .take(256)
        .collect()
}

fn rgb(c: RgbColor) -> Rgb {
    Rgb {
        r: c.r,
        g: c.g,
        b: c.b,
    }
}

fn vt_rgb(c: Rgb) -> RgbColor {
    RgbColor {
        r: c.r,
        g: c.g,
        b: c.b,
    }
}

/// Resolve a style color that may be a palette index to RGB.
fn resolve_color(color: StyleColor, colors: &libghostty_vt::render::Colors) -> Option<RgbColor> {
    match color {
        StyleColor::None => None,
        StyleColor::Rgb(c) => Some(c),
        StyleColor::Palette(idx) => Some(colors.palette[idx.0 as usize]),
    }
}

fn map_style(s: &libghostty_vt::style::Style) -> Style {
    let mut out = Style::empty();
    out.set(Style::BOLD, s.bold);
    out.set(Style::ITALIC, s.italic);
    out.set(Style::FAINT, s.faint);
    out.set(Style::INVERSE, s.inverse);
    out.set(Style::BLINK, s.blink);
    out.set(Style::HIDDEN, s.invisible);
    out.set(Style::STRIKE, s.strikethrough);
    out.set(Style::OVERLINE, s.overline);
    out.set(
        match s.underline {
            Underline::None => Style::empty(),
            Underline::Single => Style::UNDERLINE_SINGLE,
            Underline::Double => Style::UNDERLINE_DOUBLE,
            Underline::Curly => Style::UNDERLINE_CURLY,
            Underline::Dotted => Style::UNDERLINE_DOTTED,
            Underline::Dashed => Style::UNDERLINE_DASHED,
            _ => Style::empty(),
        },
        true,
    );
    out
}

/// Whether a cell's text can share a run with its neighbors.
///
/// A run is painted as one string, so every cell in it must advance by
/// exactly one cell width. That holds only for a lone narrow codepoint: wide
/// (CJK) and zero-width cells break the grid, and multi-codepoint clusters
/// may shape to any width.
fn is_batchable(text: &str, style: &Style, width: u16) -> bool {
    if width != 1
        || style.intersects(
            Style::UNDERLINE_SINGLE
                | Style::UNDERLINE_DOUBLE
                | Style::UNDERLINE_CURLY
                | Style::UNDERLINE_DOTTED
                | Style::UNDERLINE_DASHED
                | Style::STRIKE,
        )
    {
        return false;
    }
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => codepoint_width(c) == 1,
        _ => false,
    }
}

/// Everything that has to match for two cells to share a run.
#[derive(Clone, Copy, PartialEq)]
struct Pen {
    fg: Rgb,
    bg: Option<Rgb>,
    underline_color: Option<Rgb>,
    style: Style,
    selected: bool,
    hyperlink: bool,
}

/// Accumulates adjacent same-pen cells into one `Run`.
///
/// Blank cells are kept as a pending gap: they become spaces only if a later
/// same-pen cell continues the run, so trailing blanks cost nothing.
#[derive(Default)]
struct RunBuilder {
    col: u16,
    cells: u16,
    text: String,
    pen: Option<Pen>,
    gap: u16,
}

impl RunBuilder {
    fn push(&mut self, runs: &mut Vec<Run>, col: u16, pen: Pen, text: &str, width: u16) {
        if self.pen != Some(pen) {
            self.flush(runs);
            self.col = col;
            self.pen = Some(pen);
        }
        let gap = std::mem::take(&mut self.gap);
        for _ in 0..gap {
            self.text.push(' ');
        }
        self.cells += gap + width;
        self.text.push_str(text);
    }

    fn flush(&mut self, runs: &mut Vec<Run>) {
        self.gap = 0;
        let Some(pen) = self.pen.take() else {
            return;
        };
        if self.text.is_empty() {
            return;
        }
        runs.push(Run {
            col: self.col,
            cells: self.cells,
            text: std::mem::take(&mut self.text),
            fg: pen.fg,
            bg: pen.bg,
            underline_color: pen.underline_color,
            style: pen.style,
            selected: pen.selected,
            hyperlink: pen.hyperlink,
        });
        self.cells = 0;
    }
}

fn to_key_mods(mods: crate::input::Mods) -> key::Mods {
    let mut out = key::Mods::empty();
    out.set(key::Mods::SHIFT, mods.shift);
    out.set(key::Mods::CTRL, mods.ctrl);
    out.set(key::Mods::ALT, mods.alt);
    out.set(key::Mods::SUPER, mods.super_);
    out.set(key::Mods::CAPS_LOCK, mods.caps_lock);
    out.set(key::Mods::NUM_LOCK, mods.num_lock);
    out
}

fn to_mouse_button(button: MouseButton) -> mouse::Button {
    match button {
        MouseButton::Left => mouse::Button::Left,
        MouseButton::Middle => mouse::Button::Middle,
        MouseButton::Right => mouse::Button::Right,
        MouseButton::Other(4) => mouse::Button::Four,
        MouseButton::Other(5) => mouse::Button::Five,
        MouseButton::Other(6) => mouse::Button::Six,
        MouseButton::Other(7) => mouse::Button::Seven,
        MouseButton::Other(8) => mouse::Button::Eight,
        MouseButton::Other(_) => mouse::Button::Unknown,
    }
}

fn to_vt_key(key: Key) -> key::Key {
    use key::Key as K;
    match key {
        Key::Char(c) => char_key(c),
        Key::Named(named) => match named {
            NamedKey::Enter => K::Enter,
            NamedKey::Tab => K::Tab,
            NamedKey::Backspace => K::Backspace,
            NamedKey::Escape => K::Escape,
            NamedKey::Space => K::Space,
            NamedKey::Insert => K::Insert,
            NamedKey::Delete => K::Delete,
            NamedKey::Home => K::Home,
            NamedKey::End => K::End,
            NamedKey::PageUp => K::PageUp,
            NamedKey::PageDown => K::PageDown,
            NamedKey::Left => K::ArrowLeft,
            NamedKey::Right => K::ArrowRight,
            NamedKey::Up => K::ArrowUp,
            NamedKey::Down => K::ArrowDown,
            NamedKey::F(n) => f_key(n),
            NamedKey::KpEnter => K::NumpadEnter,
            NamedKey::Kp(c) => kp_key(c),
            NamedKey::ShiftLeft => K::ShiftLeft,
            NamedKey::ShiftRight => K::ShiftRight,
            NamedKey::CtrlLeft => K::ControlLeft,
            NamedKey::CtrlRight => K::ControlRight,
            NamedKey::AltLeft => K::AltLeft,
            NamedKey::AltRight => K::AltRight,
            NamedKey::SuperLeft => K::MetaLeft,
            NamedKey::SuperRight => K::MetaRight,
            NamedKey::CapsLock => K::CapsLock,
            NamedKey::NumLock => K::NumLock,
        },
        Key::Unknown => K::Unidentified,
    }
}

fn f_key(n: u8) -> key::Key {
    use key::Key as K;
    match n.clamp(1, 24) {
        1 => K::F1,
        2 => K::F2,
        3 => K::F3,
        4 => K::F4,
        5 => K::F5,
        6 => K::F6,
        7 => K::F7,
        8 => K::F8,
        9 => K::F9,
        10 => K::F10,
        11 => K::F11,
        12 => K::F12,
        13 => K::F13,
        14 => K::F14,
        15 => K::F15,
        16 => K::F16,
        17 => K::F17,
        18 => K::F18,
        19 => K::F19,
        20 => K::F20,
        21 => K::F21,
        22 => K::F22,
        23 => K::F23,
        _ => K::F24,
    }
}

fn kp_key(c: char) -> key::Key {
    use key::Key as K;
    match c {
        '0' => K::Numpad0,
        '1' => K::Numpad1,
        '2' => K::Numpad2,
        '3' => K::Numpad3,
        '4' => K::Numpad4,
        '5' => K::Numpad5,
        '6' => K::Numpad6,
        '7' => K::Numpad7,
        '8' => K::Numpad8,
        '9' => K::Numpad9,
        '+' => K::NumpadAdd,
        '-' => K::NumpadSubtract,
        '*' => K::NumpadMultiply,
        '/' => K::NumpadDivide,
        '.' => K::NumpadDecimal,
        '=' => K::NumpadEqual,
        ',' => K::NumpadComma,
        _ => K::Unidentified,
    }
}

fn char_key(c: char) -> key::Key {
    use key::Key as K;
    match c.to_ascii_lowercase() {
        'a' => K::A,
        'b' => K::B,
        'c' => K::C,
        'd' => K::D,
        'e' => K::E,
        'f' => K::F,
        'g' => K::G,
        'h' => K::H,
        'i' => K::I,
        'j' => K::J,
        'k' => K::K,
        'l' => K::L,
        'm' => K::M,
        'n' => K::N,
        'o' => K::O,
        'p' => K::P,
        'q' => K::Q,
        'r' => K::R,
        's' => K::S,
        't' => K::T,
        'u' => K::U,
        'v' => K::V,
        'w' => K::W,
        'x' => K::X,
        'y' => K::Y,
        'z' => K::Z,
        '0' => K::Digit0,
        '1' => K::Digit1,
        '2' => K::Digit2,
        '3' => K::Digit3,
        '4' => K::Digit4,
        '5' => K::Digit5,
        '6' => K::Digit6,
        '7' => K::Digit7,
        '8' => K::Digit8,
        '9' => K::Digit9,
        '-' => K::Minus,
        '=' => K::Equal,
        '[' => K::BracketLeft,
        ']' => K::BracketRight,
        '\\' => K::Backslash,
        ';' => K::Semicolon,
        '\'' => K::Quote,
        ',' => K::Comma,
        '.' => K::Period,
        '/' => K::Slash,
        '`' => K::Backquote,
        ' ' => K::Space,
        _ => K::Unidentified,
    }
}
