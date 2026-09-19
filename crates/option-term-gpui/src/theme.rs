//! Conversion between the VT palette and GPUI colors.

use gpui::Hsla;
use option_term_vt::frame::Rgb;
use option_term_vt::palette::Palette;

pub fn to_hsla(color: Rgb) -> Hsla {
    gpui::rgb((u32::from(color.r) << 16) | (u32::from(color.g) << 8) | u32::from(color.b)).into()
}

/// GPUI-facing terminal colors, derived from the shared palette (which in turn
/// comes from `option_term_core::config::Config`).
#[derive(Clone, Copy)]
pub struct Theme {
    pub background: Hsla,
    pub foreground: Hsla,
    pub cursor: Hsla,
    pub cursor_text: Hsla,
    pub selection_background: Hsla,
    pub selection_foreground: Hsla,
    /// Translucent yellow used for search-match highlights.
    pub search_match: Hsla,
    /// Stronger variant for the active search match.
    pub search_match_active: Hsla,
}

impl From<&Palette> for Theme {
    fn from(palette: &Palette) -> Self {
        Self {
            background: to_hsla(palette.bg),
            foreground: to_hsla(palette.fg),
            cursor: to_hsla(palette.cursor),
            cursor_text: to_hsla(palette.cursor_text),
            selection_background: to_hsla(palette.selection_bg),
            selection_foreground: to_hsla(palette.selection_fg),
            search_match: gpui::rgba(0xffff0040).into(),
            search_match_active: gpui::rgba(0xffff0080).into(),
        }
    }
}
