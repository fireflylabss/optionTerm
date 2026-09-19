//! Terminal grid geometry: cell metrics and grid sizing.

use std::sync::Arc;

use gpui::{Font, Pixels, Size, WindowTextSystem, px};

/// Cell size for `font` at `font_size`.
///
/// The width is the *fractional* advance of `M` — do not round it: rounding
/// broke alignment with Fira Code (see AGENTS). The height is
/// `ascent + descent + 0.2em` (the usual monospace line pitch); the
/// `WindowTextSystem` used during paint does not expose `line_gap`, so the gap
/// term is approximated. Everything that matters for grid alignment comes from
/// this function, so painting and hit-testing stay consistent.
pub fn cell_size(
    text_system: &Arc<WindowTextSystem>,
    font: &Font,
    font_size: Pixels,
) -> Size<Pixels> {
    let font_id = text_system.resolve_font(font);

    let width = text_system
        .advance(font_id, font_size, 'M')
        .map(|s| s.width)
        .unwrap_or(px(0.0))
        .max(px(1.0));

    let mut height = text_system.ascent(font_id, font_size)
        + text_system.descent(font_id, font_size)
        + font_size * 0.2;
    if height <= px(0.0) {
        height = font_size * 1.2;
    }

    Size { width, height }
}

/// `(cols, rows)` that fit inside `bounds` with `padding` on all sides,
/// clamped to at least 1x1.
pub fn grid_for(bounds: Size<Pixels>, cell: Size<Pixels>, padding: Size<Pixels>) -> (u16, u16) {
    let usable_w = (bounds.width - padding.width * 2.0).max(px(0.0));
    let usable_h = (bounds.height - padding.height * 2.0).max(px(0.0));
    let cols = (usable_w / cell.width).floor().max(1.0) as u16;
    let rows = (usable_h / cell.height).floor().max(1.0) as u16;
    (cols, rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_for_reference_size() {
        // Size(1000, 640), cell(9.6, 20), pad 4 -> (103, 31)
        let (cols, rows) = grid_for(
            Size {
                width: px(1000.0),
                height: px(640.0),
            },
            Size {
                width: px(9.6),
                height: px(20.0),
            },
            Size {
                width: px(4.0),
                height: px(4.0),
            },
        );
        assert_eq!(cols, 103);
        assert_eq!(rows, 31);
    }

    #[test]
    fn grid_for_clamps_to_one() {
        let (cols, rows) = grid_for(
            Size {
                width: px(1.0),
                height: px(1.0),
            },
            Size {
                width: px(9.6),
                height: px(20.0),
            },
            Size {
                width: px(4.0),
                height: px(4.0),
            },
        );
        assert_eq!((cols, rows), (1, 1));
    }
}
