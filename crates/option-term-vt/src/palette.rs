//! Theme colors from the shared config, ready for the emulator.

use option_term_core::config::{Config, CursorStyle, RgbColor};

use crate::frame::{CursorShape, Rgb};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    pub fg: Rgb,
    pub bg: Rgb,
    pub cursor: Rgb,
    pub cursor_text: Rgb,
    pub selection_bg: Rgb,
    pub selection_fg: Rgb,
    pub colors: [Rgb; 16],
}

fn rgb(c: RgbColor) -> Rgb {
    Rgb {
        r: c.r,
        g: c.g,
        b: c.b,
    }
}

impl From<&Config> for Palette {
    fn from(cfg: &Config) -> Self {
        Self {
            fg: rgb(cfg.foreground),
            bg: rgb(cfg.background),
            cursor: rgb(cfg.cursor),
            cursor_text: rgb(cfg.cursor_text),
            selection_bg: rgb(cfg.selection_background),
            selection_fg: rgb(cfg.selection_foreground),
            colors: cfg.palette.map(rgb),
        }
    }
}

impl From<CursorStyle> for CursorShape {
    fn from(style: CursorStyle) -> Self {
        match style {
            CursorStyle::Block => CursorShape::Block,
            CursorStyle::Bar => CursorShape::Bar,
            CursorStyle::Underline => CursorShape::Underline,
            CursorStyle::BlockHollow => CursorShape::BlockHollow,
        }
    }
}
