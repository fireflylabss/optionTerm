//! Terminal grid geometry: cell metrics and grid sizing.

use std::sync::Arc;

use gpui::{Font, Pixels, Size, WindowTextSystem, px};

/// Cell size for `font` at `font_size` (px, not points).
///
/// Kitty-style: dimensions are whole *device* pixels, converted back to
/// logical px by dividing by `scale_factor` — at scale 1.25 a 10.4px cell
/// becomes 9.6px (12 device px). Height is `ascent + descent` like kitty; no
/// synthetic line-gap fudge. Painting and hit-testing both derive from this
/// function so they stay consistent.
pub fn cell_size(
    text_system: &Arc<WindowTextSystem>,
    font: &Font,
    font_size: Pixels,
    scale_factor: f32,
) -> Size<Pixels> {
    let font_id = text_system.resolve_font(font);
    let size = f32::from(font_size);

    let width = text_system
        .advance(font_id, font_size, 'M')
        .map(|a| f32::from(a.width))
        .unwrap_or(size * 0.6);

    // GPUI reports `descent` as a negative distance (see
    // `cosmic_text_system.rs`: `descent: -metrics.descent`), hence `abs`.
    let raw_height = f32::from(text_system.ascent(font_id, font_size))
        + f32::from(text_system.descent(font_id, font_size)).abs();
    let height = if raw_height > 0.0 {
        raw_height
    } else {
        size * 1.2
    };

    Size {
        width: snap_to_device_px(width, scale_factor),
        height: snap_to_device_px(height, scale_factor),
    }
}

/// Round a logical-px length up to a whole device pixel at `scale_factor`.
fn snap_to_device_px(value: f32, scale_factor: f32) -> Pixels {
    let scale = scale_factor.max(0.001);
    px((value * scale).ceil() / scale)
}

/// Snap a logical-px point down to the device-pixel grid.
pub fn snap_point_to_device_px(
    value: gpui::Point<Pixels>,
    scale_factor: f32,
) -> gpui::Point<Pixels> {
    let scale = scale_factor.max(0.001);
    gpui::point(
        px((f32::from(value.x) * scale).floor() / scale),
        px((f32::from(value.y) * scale).floor() / scale),
    )
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

    #[test]
    fn snap_ceil_at_scale_one() {
        // 10.4 logical px at scale 1 -> 11 device px -> 11.0 logical px.
        assert_eq!(snap_to_device_px(10.4, 1.0), px(11.0));
        assert_eq!(snap_to_device_px(9.0, 1.0), px(9.0));
    }

    #[test]
    fn snap_ceil_at_fractional_scale() {
        // 9.4 logical px at scale 1.25 -> 11.75 -> ceil 12 device px -> 9.6.
        let snapped = snap_to_device_px(9.4, 1.25);
        assert_eq!(snapped, px(9.6));
        assert_eq!(f32::from(snapped) * 1.25, 12.0);
        // Points snap down: 10.04 * 1.25 = 12.55 -> floor 12 -> 9.6.
        let snapped = snap_point_to_device_px(gpui::point(px(10.04), px(4.9)), 1.25);
        assert_eq!(snapped, gpui::point(px(9.6), px(4.8)));
    }
}
