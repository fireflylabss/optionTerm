//! Custom `Element` that paints a terminal `Frame` produced by
//! `option-term-vt`. Kitty image placements are intentionally ignored in
//! phase 2a — the `Pane` still retains `Frame.kitty` for a later phase.

use std::sync::Arc;

use gpui::{
    App, Bounds, CursorStyle, Element, ElementInputHandler, Entity, FontStyle, FontWeight,
    GlobalElementId, Hitbox, HitboxBehavior, Hsla, InspectorElementId, IntoElement, LayoutId,
    MouseMoveEvent, PaintQuad, Pixels, Style, TextAlign, TextRun, UnderlineStyle, Window, fill,
    point, px, size,
};
use option_term_vt::frame::{CursorShape, Frame, Style as CellStyle};

use crate::metrics;
use crate::pane::Pane;
use crate::theme;

pub struct TerminalElement {
    pane: Entity<Pane>,
}

impl TerminalElement {
    pub fn new(pane: Entity<Pane>) -> Self {
        Self { pane }
    }
}

impl IntoElement for TerminalElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

pub struct TerminalPrepaint {
    hitbox: Hitbox,
}

impl Element for TerminalElement {
    type RequestLayoutState = ();
    type PrepaintState = TerminalPrepaint;

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let style = Style {
            size: gpui::Size {
                width: gpui::relative(1.).into(),
                height: gpui::relative(1.).into(),
            },
            ..Default::default()
        };
        (window.request_layout(style, None, cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let text_system = window.text_system().clone();
        let scale_factor = window.scale_factor();
        let (font, font_size) = self
            .pane
            .read_with(cx, |pane, _| (pane.font.clone(), pane.font_size));
        let cell = metrics::cell_size(&text_system, &font, font_size, scale_factor);
        self.pane.update(cx, |pane, _| {
            pane.update_layout(bounds, cell, scale_factor);
        });
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        TerminalPrepaint { hitbox }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        // Route IME/input-method events to the pane entity.
        let focus_handle = self.pane.read(cx).focus_handle();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.pane.clone()),
            cx,
        );

        // Selection drags extend even when the pointer leaves the element.
        // A WeakEntity is required: listeners live as long as the window and a
        // strong Entity<Pane> here would leak the pane on exit.
        window.on_mouse_event({
            let pane = self.pane.downgrade();
            move |ev: &MouseMoveEvent, phase, _window, cx| {
                if phase.bubble() {
                    let _ = pane.update(cx, |pane, _| pane.mouse_move(ev));
                }
            }
        });

        window.set_cursor_style(CursorStyle::IBeam, &prepaint.hitbox);

        let (frame, cell, origin, font, font_size, theme, preedit, blink_on, focused, bg_opacity) =
            self.pane.read_with(cx, |pane, _| {
                (
                    pane.frame.clone(),
                    pane.cell,
                    pane.content_origin,
                    pane.font.clone(),
                    pane.font_size,
                    pane.theme,
                    pane.preedit.clone(),
                    pane.blink_on,
                    pane.focused,
                    pane.background_opacity,
                )
            });
        let window_active = window.is_window_active();
        let text_system = window.text_system().clone();

        // Panel background: `background_opacity` applies here only; explicit
        // per-cell `run.bg` colors below stay opaque (kitty behaviour).
        let mut panel_bg = theme::to_hsla(frame.default_bg);
        panel_bg.a *= bg_opacity;
        window.paint_quad(fill(bounds, panel_bg));

        // Search-match highlights (a later phase wires the search UI, but the
        // emulator already reports matches — paint them now).
        for (index, m) in frame.matches.iter().enumerate() {
            let color = if frame.active_match == Some(index) {
                theme.search_match_active
            } else {
                theme.search_match
            };
            let m_bounds = Bounds::new(
                origin
                    + point(
                        cell.width * f32::from(m.col),
                        cell.height * f32::from(m.row),
                    ),
                size(cell.width * f32::from(m.width), cell.height),
            );
            window.paint_quad(fill(m_bounds, color));
        }

        // Cell backgrounds and text runs.
        // NOTE: `shape_line` per run per frame is acceptable for phase 2a; a
        // cache keyed on (line seq, run index, font, zoom) belongs here.
        for (row, line) in frame.lines.iter().enumerate() {
            let row_y = cell.height * row as f32;
            for run in &line.runs {
                let run_origin = origin + point(cell.width * f32::from(run.col), row_y);
                let run_width = cell.width * f32::from(run.cells);
                let run_bounds = Bounds::new(run_origin, size(run_width, cell.height));

                if run.selected {
                    window.paint_quad(fill(run_bounds, theme.selection_background));
                } else if let Some(bg) = run.bg {
                    window.paint_quad(fill(run_bounds, theme::to_hsla(bg)));
                }

                if run.style.contains(CellStyle::HIDDEN) {
                    continue;
                }
                if run.text.is_empty() {
                    self.paint_decorations(
                        window,
                        run_bounds,
                        run.style,
                        run.underline_color,
                        run.fg,
                    );
                    continue;
                }

                let mut color = if run.selected {
                    theme.selection_foreground
                } else {
                    theme::to_hsla(run.fg)
                };
                if run.style.contains(CellStyle::FAINT) {
                    color.a *= 0.6;
                }
                let mut font = font.clone();
                if run.style.contains(CellStyle::BOLD) {
                    font.weight = FontWeight::BOLD;
                }
                if run.style.contains(CellStyle::ITALIC) {
                    font.style = FontStyle::Italic;
                }
                self.paint_run_text(
                    window,
                    &text_system,
                    run,
                    run_origin,
                    cell,
                    &font,
                    font_size,
                    color,
                    cx,
                );
                self.paint_decorations(window, run_bounds, run.style, run.underline_color, run.fg);
            }
        }

        // IME preedit: painted in the cursor cell with an underline, never sent
        // to the PTY until committed.
        if let (Some(text), Some(cursor)) = (preedit.as_ref(), frame.cursor.as_ref()) {
            let cursor_origin = origin
                + point(
                    cell.width * f32::from(cursor.col),
                    cell.height * f32::from(cursor.row),
                );
            let width = cell.width * text.chars().count().max(1) as f32;
            let preedit_bounds = Bounds::new(cursor_origin, size(width, cell.height));
            window.paint_quad(fill(preedit_bounds, theme.selection_background));
            let shaped = text_system.shape_line(
                text.clone().into(),
                font_size,
                &[TextRun {
                    len: text.len(),
                    font: font.clone(),
                    color: theme.foreground,
                    background_color: None,
                    underline: Some(UnderlineStyle {
                        color: Some(theme.foreground),
                        thickness: px(1.0),
                        wavy: false,
                    }),
                    strikethrough: None,
                }],
                Some(cell.width),
            );
            if let Err(err) = shaped.paint(
                cursor_origin,
                cell.height,
                TextAlign::Left,
                Some(width),
                window,
                cx,
            ) {
                tracing::warn!("failed to paint IME preedit: {err}");
            }
        }

        // Cursor. Hollow whenever the window is inactive or the cursor is
        // blinking off.
        if let Some(cursor) = frame.cursor.as_ref() {
            let cursor_bounds = Bounds::new(
                origin
                    + point(
                        cell.width * f32::from(cursor.col),
                        cell.height * f32::from(cursor.row),
                    ),
                size(cell.width, cell.height),
            );
            let visible = !cursor.blinking || !focused || blink_on || !window_active;
            if visible {
                self.paint_cursor(
                    window,
                    cursor_bounds,
                    cursor.shape,
                    theme::to_hsla(cursor.color),
                    !window_active,
                );
                // Block cursor: re-paint the cell's glyph in the cursor text
                // color so the character stays legible under the block.
                if matches!(cursor.shape, CursorShape::Block) && window_active {
                    self.paint_cursor_text(
                        window,
                        &text_system,
                        &frame,
                        cursor.row,
                        cursor.col,
                        cursor_bounds,
                        cell,
                        &font,
                        font_size,
                        theme::to_hsla(cursor.text_color),
                        cx,
                    );
                }
            }
        }
    }
}

impl TerminalElement {
    /// Paint a run's text, splitting out block-element characters which are
    /// drawn geometrically (fonts rasterize them with gaps and wrong metrics).
    #[allow(clippy::too_many_arguments)]
    fn paint_run_text(
        &self,
        window: &mut Window,
        text_system: &Arc<gpui::WindowTextSystem>,
        run: &option_term_vt::frame::Run,
        run_origin: gpui::Point<Pixels>,
        cell: gpui::Size<Pixels>,
        font: &gpui::Font,
        font_size: Pixels,
        color: Hsla,
        cx: &mut App,
    ) {
        let mut text_seg = String::new();
        let mut text_seg_col = run.col;
        let mut block_seg: Vec<char> = Vec::new();
        let mut block_seg_col = run.col;
        let mut col = run.col;

        let mut flush_text =
            |this: &Self, window: &mut Window, text: &mut String, start_col: u16, end_col: u16| {
                if !text.is_empty() {
                    this.shape_and_paint(
                        window,
                        text_system,
                        text,
                        run_origin + point(cell.width * f32::from(start_col - run.col), px(0.0)),
                        cell.width * f32::from(end_col - start_col),
                        cell,
                        font,
                        font_size,
                        color,
                        cx,
                    );
                    text.clear();
                }
            };

        for ch in run.text.chars() {
            if is_block_element(ch) {
                flush_text(self, window, &mut text_seg, text_seg_col, col);
                if block_seg.is_empty() {
                    block_seg_col = col;
                }
                block_seg.push(ch);
                col += 1;
            } else {
                if !block_seg.is_empty() {
                    self.paint_block_segment(
                        window,
                        &block_seg,
                        block_seg_col,
                        run_origin,
                        run.col,
                        cell,
                        color,
                    );
                    block_seg.clear();
                }
                if text_seg.is_empty() {
                    text_seg_col = col;
                }
                text_seg.push(ch);
                col += char_cells(ch);
            }
        }
        flush_text(self, window, &mut text_seg, text_seg_col, col);
        if !block_seg.is_empty() {
            self.paint_block_segment(
                window,
                &block_seg,
                block_seg_col,
                run_origin,
                run.col,
                cell,
                color,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn shape_and_paint(
        &self,
        window: &mut Window,
        text_system: &Arc<gpui::WindowTextSystem>,
        text: &str,
        origin: gpui::Point<Pixels>,
        width: Pixels,
        cell: gpui::Size<Pixels>,
        font: &gpui::Font,
        font_size: Pixels,
        color: Hsla,
        cx: &mut App,
    ) {
        let shaped = text_system.shape_line(
            text.to_string().into(),
            font_size,
            &[TextRun {
                len: text.len(),
                font: font.clone(),
                color,
                background_color: None,
                underline: None,
                strikethrough: None,
            }],
            Some(cell.width),
        );
        if let Err(err) = shaped.paint(
            origin,
            cell.height,
            TextAlign::Left,
            Some(width),
            window,
            cx,
        ) {
            tracing::warn!("failed to paint text run: {err}");
        }
    }

    /// Paint consecutive block-element chars as geometric quads, one cell each.
    #[allow(clippy::too_many_arguments)]
    fn paint_block_segment(
        &self,
        window: &mut Window,
        chars: &[char],
        start_col: u16,
        run_origin: gpui::Point<Pixels>,
        run_col: u16,
        cell: gpui::Size<Pixels>,
        color: Hsla,
    ) {
        for (index, ch) in chars.iter().enumerate() {
            let cell_bounds = Bounds::new(
                run_origin
                    + point(
                        cell.width * (f32::from(start_col - run_col) + index as f32),
                        px(0.0),
                    ),
                cell,
            );
            if let Some(rects) = block_element_rects(*ch, cell_bounds) {
                for (rect, alpha) in rects {
                    let mut color = color;
                    color.a *= alpha;
                    window.paint_quad(fill(rect, color));
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_cursor_text(
        &self,
        window: &mut Window,
        text_system: &Arc<gpui::WindowTextSystem>,
        frame: &Frame,
        row: u16,
        col: u16,
        cursor_bounds: Bounds<Pixels>,
        cell: gpui::Size<Pixels>,
        font: &gpui::Font,
        font_size: Pixels,
        color: Hsla,
        cx: &mut App,
    ) {
        let Some(line) = frame.lines.get(usize::from(row)) else {
            return;
        };
        let text: String = line
            .runs
            .iter()
            .filter(|run| run.col <= col && col < run.col + run.cells)
            .map(|run| run.text.as_str())
            .collect();
        if text.is_empty() {
            return;
        }
        let shaped = text_system.shape_line(
            text.clone().into(),
            font_size,
            &[TextRun {
                len: text.len(),
                font: font.clone(),
                color,
                background_color: None,
                underline: None,
                strikethrough: None,
            }],
            Some(cell.width),
        );
        // The run may start before the cursor cell; offset origin accordingly.
        let run_col = line
            .runs
            .iter()
            .find(|run| run.col <= col && col < run.col + run.cells)
            .map(|run| run.col)
            .unwrap_or(col);
        let run_origin = point(
            cursor_bounds.origin.x - cell.width * f32::from(col - run_col),
            cursor_bounds.origin.y,
        );
        let _ = shaped.paint(
            run_origin,
            cell.height,
            TextAlign::Left,
            Some(cell.width * f32::from(col - run_col + 1)),
            window,
            cx,
        );
    }

    fn paint_cursor(
        &self,
        window: &mut Window,
        bounds: Bounds<Pixels>,
        shape: CursorShape,
        color: Hsla,
        force_hollow: bool,
    ) {
        match (shape, force_hollow) {
            (CursorShape::Block, false) => {
                window.paint_quad(fill(bounds, color));
            }
            (CursorShape::Block, true) | (CursorShape::BlockHollow, _) => {
                window.paint_quad(PaintQuad {
                    bounds,
                    corner_radii: Default::default(),
                    background: gpui::transparent_black().into(),
                    border_widths: gpui::Edges::all(px(1.0)),
                    border_color: color,
                    border_style: gpui::BorderStyle::Solid,
                });
            }
            (CursorShape::Bar, _) => {
                window.paint_quad(fill(
                    Bounds::new(bounds.origin, size(px(2.0), bounds.size.height)),
                    color,
                ));
            }
            (CursorShape::Underline, _) => {
                window.paint_quad(fill(
                    Bounds::new(
                        bounds.origin + point(px(0.0), bounds.size.height - px(2.0)),
                        size(bounds.size.width, px(2.0)),
                    ),
                    color,
                ));
            }
        }
    }

    fn paint_decorations(
        &self,
        window: &mut Window,
        bounds: Bounds<Pixels>,
        style: CellStyle,
        underline_color: Option<option_term_vt::frame::Rgb>,
        fg: option_term_vt::frame::Rgb,
    ) {
        let color = underline_color
            .map(theme::to_hsla)
            .unwrap_or(theme::to_hsla(fg));
        let baseline = bounds.origin.y + bounds.size.height - px(1.0);
        let origin = point(bounds.origin.x, baseline);
        if style.contains(CellStyle::UNDERLINE_SINGLE)
            || style.contains(CellStyle::UNDERLINE_DOTTED)
            || style.contains(CellStyle::UNDERLINE_DASHED)
        {
            window.paint_underline(
                origin,
                bounds.size.width,
                &UnderlineStyle {
                    color: Some(color),
                    thickness: px(1.0),
                    wavy: false,
                },
            );
        }
        if style.contains(CellStyle::UNDERLINE_DOUBLE) {
            window.paint_underline(
                point(origin.x, origin.y - px(1.0)),
                bounds.size.width,
                &UnderlineStyle {
                    color: Some(color),
                    thickness: px(1.0),
                    wavy: false,
                },
            );
            window.paint_underline(
                point(origin.x, origin.y + px(1.0)),
                bounds.size.width,
                &UnderlineStyle {
                    color: Some(color),
                    thickness: px(1.0),
                    wavy: false,
                },
            );
        }
        if style.contains(CellStyle::UNDERLINE_CURLY) {
            window.paint_underline(
                origin,
                bounds.size.width,
                &UnderlineStyle {
                    color: Some(color),
                    thickness: px(1.0),
                    wavy: true,
                },
            );
        }
        if style.contains(CellStyle::STRIKE) {
            window.paint_strikethrough(
                point(bounds.origin.x, bounds.center().y - px(0.5)),
                bounds.size.width,
                &gpui::StrikethroughStyle {
                    color: Some(color),
                    thickness: px(1.0),
                },
            );
        }
        if style.contains(CellStyle::OVERLINE) {
            window.paint_underline(
                bounds.origin,
                bounds.size.width,
                &UnderlineStyle {
                    color: Some(color),
                    thickness: px(1.0),
                    wavy: false,
                },
            );
        }
    }
}

fn is_block_element(ch: char) -> bool {
    (0x2580..=0x259F).contains(&(ch as u32))
}

/// Geometry for block-element chars (U+2580..=U+259F) inside `cell`, as
/// `(rect, alpha)` pairs; `alpha` multiplies the run's fg color. Quadrant
/// letters below follow the Unicode chart: TL/TR/BL/BR half-cell quadrants.
fn block_element_rects(ch: char, cell: Bounds<Pixels>) -> Option<Vec<(Bounds<Pixels>, f32)>> {
    let code = ch as u32;
    if !is_block_element(ch) {
        return None;
    }
    let x = f32::from(cell.origin.x);
    let y = f32::from(cell.origin.y);
    let w = f32::from(cell.size.width);
    let h = f32::from(cell.size.height);
    let rect =
        |x: f32, y: f32, w: f32, h: f32| Bounds::new(point(px(x), px(y)), size(px(w), px(h)));
    // n/8 fractions measured from each edge.
    let lower = |n: f32| rect(x, y + h * (8.0 - n) / 8.0, w, h * n / 8.0);
    let upper = |n: f32| rect(x, y, w, h * n / 8.0);
    let left = |n: f32| rect(x, y, w * n / 8.0, h);
    let right = |n: f32| rect(x + w * (8.0 - n) / 8.0, y, w * n / 8.0, h);
    let tl = rect(x, y, w / 2.0, h / 2.0);
    let tr = rect(x + w / 2.0, y, w / 2.0, h / 2.0);
    let bl = rect(x, y + h / 2.0, w / 2.0, h / 2.0);
    let br = rect(x + w / 2.0, y + h / 2.0, w / 2.0, h / 2.0);

    let rects = match code {
        0x2580 => vec![(upper(4.0), 1.0)],
        0x2581..=0x2588 => vec![(lower((code - 0x2580) as f32), 1.0)],
        0x2589..=0x258F => vec![(left((8 - (code - 0x2588)) as f32), 1.0)],
        0x2590 => vec![(right(4.0), 1.0)],
        0x2591 => vec![(rect(x, y, w, h), 0.25)],
        0x2592 => vec![(rect(x, y, w, h), 0.5)],
        0x2593 => vec![(rect(x, y, w, h), 0.75)],
        0x2594 => vec![(upper(1.0), 1.0)],
        0x2595 => vec![(right(1.0), 1.0)],
        0x2596 => vec![(bl, 1.0)],
        0x2597 => vec![(br, 1.0)],
        0x2598 => vec![(tl, 1.0)],
        0x2599 => vec![(tl, 1.0), (bl, 1.0), (br, 1.0)],
        0x259A => vec![(tl, 1.0), (br, 1.0)],
        0x259B => vec![(tl, 1.0), (tr, 1.0), (bl, 1.0)],
        0x259C => vec![(tl, 1.0), (tr, 1.0), (br, 1.0)],
        0x259D => vec![(tr, 1.0)],
        0x259E => vec![(tr, 1.0), (bl, 1.0)],
        0x259F => vec![(tr, 1.0), (bl, 1.0), (br, 1.0)],
        _ => return None,
    };
    Some(rects)
}

/// Approximate terminal cell width of a char without pulling in unicode-width:
/// combining marks take 0 cells, East-Asian wide ranges take 2, the rest 1.
/// Only used to position block-element quads inside mixed runs.
fn char_cells(ch: char) -> u16 {
    match ch as u32 {
        0x300..=0x36F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF | 0xFE20..=0xFE2F => 0,
        0x1100..=0x115F
        | 0x2329..=0x232A
        | 0x2E80..=0x303E
        | 0x3040..=0xA4CF
        | 0xAC00..=0xD7A3
        | 0xF900..=0xFAFF
        | 0xFE30..=0xFE6F
        | 0xFF00..=0xFF60
        | 0xFFE0..=0xFFE6
        | 0x20000..=0x2FFFD
        | 0x30000..=0x3FFFD => 2,
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell() -> Bounds<Pixels> {
        Bounds::new(point(px(4.0), px(8.0)), size(px(10.0), px(20.0)))
    }

    #[test]
    fn block_upper_half() {
        // U+2580 ▀: top half.
        let rects = block_element_rects('\u{2580}', cell()).unwrap();
        assert_eq!(rects.len(), 1);
        let (r, alpha) = rects[0];
        assert_eq!(alpha, 1.0);
        assert_eq!(r.origin, point(px(4.0), px(8.0)));
        assert_eq!(r.size, size(px(10.0), px(10.0)));
    }

    #[test]
    fn block_lower_half() {
        // U+2584 ▄: bottom half.
        let rects = block_element_rects('\u{2584}', cell()).unwrap();
        assert_eq!(rects.len(), 1);
        let (r, _) = rects[0];
        assert_eq!(r.origin, point(px(4.0), px(18.0)));
        assert_eq!(r.size, size(px(10.0), px(10.0)));
    }

    #[test]
    fn block_full_and_shade() {
        // U+2588 █: full cell.
        let (r, alpha) = block_element_rects('\u{2588}', cell()).unwrap()[0];
        assert_eq!(r, cell());
        assert_eq!(alpha, 1.0);
        // U+2591 ░: full cell at 25% alpha.
        let (r, alpha) = block_element_rects('\u{2591}', cell()).unwrap()[0];
        assert_eq!(r, cell());
        assert_eq!(alpha, 0.25);
    }

    #[test]
    fn block_quadrants() {
        // U+259F ▟ = TR + BL + BR.
        let rects = block_element_rects('\u{259F}', cell()).unwrap();
        assert_eq!(rects.len(), 3);
        let origins: Vec<_> = rects.iter().map(|(r, _)| r.origin).collect();
        assert!(origins.contains(&point(px(9.0), px(8.0)))); // TR
        assert!(origins.contains(&point(px(4.0), px(18.0)))); // BL
        assert!(origins.contains(&point(px(9.0), px(18.0)))); // BR
    }

    #[test]
    fn non_block_returns_none() {
        assert!(block_element_rects('a', cell()).is_none());
        assert!(block_element_rects('╱', cell()).is_none());
    }
}
