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
        let cell = metrics::cell_size(&text_system, &font, font_size);
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
        window.on_mouse_event({
            let pane = self.pane.clone();
            move |ev: &MouseMoveEvent, phase, _window, cx| {
                if phase.bubble() {
                    pane.update(cx, |pane, _| pane.mouse_move(ev));
                }
            }
        });

        window.set_cursor_style(CursorStyle::IBeam, &prepaint.hitbox);

        let (frame, cell, origin, font, font_size, theme, preedit, blink_on, focused) =
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
                )
            });
        let window_active = window.is_window_active();
        let text_system = window.text_system().clone();

        // Panel background.
        window.paint_quad(fill(bounds, theme::to_hsla(frame.default_bg)));

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
                let shaped = text_system.shape_line(
                    run.text.clone().into(),
                    font_size,
                    &[TextRun {
                        len: run.text.len(),
                        font,
                        color,
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    }],
                    Some(cell.width),
                );
                if let Err(err) = shaped.paint(
                    run_origin,
                    cell.height,
                    TextAlign::Left,
                    Some(run_width),
                    window,
                    cx,
                ) {
                    tracing::warn!("failed to paint text run: {err}");
                }
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
