//! Immutable, `Send` snapshot of one rendered frame.
//!
//! The pane thread produces a `Frame` from the `!Send` libghostty terminal
//! state and hands it to the UI, which paints cells, cursor and Kitty
//! placements from this data alone.

use std::sync::Arc;

/// 8-bit RGB color.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

bitflags::bitflags! {
    /// Cell attributes, mirroring the SGR flags libghostty reports.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
    pub struct Style: u16 {
        const BOLD = 1 << 0;
        const ITALIC = 1 << 1;
        const FAINT = 1 << 2;
        const INVERSE = 1 << 3;
        const BLINK = 1 << 4;
        const HIDDEN = 1 << 5;
        const STRIKE = 1 << 6;
        const OVERLINE = 1 << 7;
        const UNDERLINE_SINGLE = 1 << 8;
        const UNDERLINE_DOUBLE = 1 << 9;
        const UNDERLINE_CURLY = 1 << 10;
        const UNDERLINE_DOTTED = 1 << 11;
        const UNDERLINE_DASHED = 1 << 12;
    }
}

/// A run of adjacent cells that share color and attributes.
///
/// `col`/`cells` are grid columns; a wide character reports `cells == 2` and
/// the spacer tail after it produces no run of its own. `text` covers the
/// run's cells; blank cells embedded between same-styled text are kept as
/// spaces so a single paint call can draw the whole run.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub col: u16,
    pub cells: u16,
    pub text: String,
    pub fg: Rgb,
    /// `None` means the terminal default background (`Frame::default_bg`).
    pub bg: Option<Rgb>,
    pub underline_color: Option<Rgb>,
    pub style: Style,
    pub selected: bool,
    /// Carries an OSC 8 hyperlink.
    pub hyperlink: bool,
}

/// One viewport row. `runs` covers only cells with content or an explicit
/// background; empty cells on the default background produce no run.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Line {
    pub runs: Vec<Run>,
}

/// DECSCUSR cursor shape.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CursorShape {
    #[default]
    Block,
    BlockHollow,
    Bar,
    Underline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor {
    pub col: u16,
    pub row: u16,
    pub shape: CursorShape,
    pub color: Rgb,
    /// Foreground to paint the glyph under a filled block cursor.
    pub text_color: Rgb,
    pub blinking: bool,
}

/// Viewport position within the scrollback.
///
/// `offset` is how many lines above the bottom the viewport sits (0 = at the
/// prompt); `total` is how many scrollback lines exist.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Scroll {
    pub offset: usize,
    pub total: usize,
}

/// Mode flags the UI needs to route input and paint chrome.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modes {
    /// Any of the mouse tracking modes (1000/1002/1003/9) is enabled.
    pub mouse_reporting: bool,
    pub bracketed_paste: bool,
    /// The alternate screen is active (any of 47/1047/1049).
    pub alt_screen: bool,
    /// Focus reporting (mode 1004) is enabled.
    pub focus_events: bool,
    /// The Kitty keyboard protocol stack is non-empty.
    pub kitty_keyboard: bool,
}

/// One Kitty image placement in viewport coordinates.
///
/// `row` may be negative when the placement's origin scrolled above the top
/// of the viewport; the UI clips.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    pub image_id: u32,
    pub placement_id: u32,
    pub z: i32,
    pub col: i32,
    pub row: i32,
    /// Pixel offset inside the origin cell.
    pub offset_px: (u16, u16),
    /// Destination size in pixels.
    pub dest_px: (u32, u32),
    /// Source rectangle `(x, y, w, h)` in image pixels.
    pub src_px: (u32, u32, u32, u32),
}

/// Decoded pixel data, always RGBA8 regardless of the transmission format.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageData {
    pub id: u32,
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<[u8]>,
}

/// Kitty graphics state for one frame.
///
/// `images` carries pixel data only for images not yet sent since the last
/// `generation` change; `dropped` lists ids that disappeared from storage.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Kitty {
    pub generation: u64,
    pub placements: Vec<Placement>,
    pub images: Vec<ImageData>,
    pub dropped: Vec<u32>,
}

/// One search hit. `row` is in screen space (scrollback + viewport), which is
/// what `Input::ScrollTo` needs to reveal it; the UI derives visibility from
/// `Frame::scroll`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Match {
    pub row: u16,
    pub col: u16,
    pub width: u16,
}

/// Everything the UI needs to paint one frame.
#[derive(Clone, Debug)]
pub struct Frame {
    pub cols: u16,
    pub rows: u16,
    /// `len == rows`.
    pub lines: Vec<Line>,
    /// `None` when the cursor is hidden or scrolled out of view.
    pub cursor: Option<Cursor>,
    pub scroll: Scroll,
    pub modes: Modes,
    pub title: String,
    /// OSC 7 working directory, decoded to a filesystem path.
    pub pwd: Option<String>,
    pub kitty: Kitty,
    pub matches: Vec<Match>,
    pub active_match: Option<usize>,
    pub default_bg: Rgb,
    pub default_fg: Rgb,
    /// Monotonically increasing sequence number.
    pub seq: u64,
}
