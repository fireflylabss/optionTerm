//! UI → emulator input events and emulator → UI events.
//!
//! These types are framework-agnostic: the GTK/GPUI frontend maps its native
//! events onto `Input` and consumes `Event`. Everything crosses the pane
//! thread boundary, so all of it is `Send`.

use std::sync::Arc;

use crate::frame::Frame;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyAction {
    Press,
    Release,
    Repeat,
}

/// Modifier state. `super_` is Command on macOS / the Windows key elsewhere.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub super_: bool,
    pub caps_lock: bool,
    pub num_lock: bool,
}

/// A physical/logical key, mapped to libghostty's `key::Key`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// A printable key identified by its base character (lowercase, no shift):
    /// `'a'`, `'1'`, `'-'`, `'['`, `';'`, etc.
    Char(char),
    Named(NamedKey),
    /// Modifier keys and anything unmapped.
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NamedKey {
    Enter,
    Tab,
    Backspace,
    Escape,
    Space,
    Insert,
    Delete,
    Home,
    End,
    PageUp,
    PageDown,
    Left,
    Right,
    Up,
    Down,
    /// F1..=F24 (values outside the range are clamped).
    F(u8),
    KpEnter,
    /// Numpad character: `'0'..'9'`, `'+'`, `'-'`, `'*'`, `'/'`, `'.'`, `'='`.
    Kp(char),
    /// Modifier keys, for Kitty keyboard release reporting.
    ShiftLeft,
    ShiftRight,
    CtrlLeft,
    CtrlRight,
    AltLeft,
    AltRight,
    SuperLeft,
    SuperRight,
    CapsLock,
    NumLock,
}

#[derive(Clone, Debug, PartialEq)]
pub struct KeyEvent {
    pub action: KeyAction,
    pub key: Key,
    pub mods: Mods,
    /// Already-composed text (IME/xkb), if any.
    pub text: Option<String>,
    /// Codepoint without shift applied, when known.
    pub unshifted: Option<char>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    Other(u8),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MouseKind {
    Press(MouseButton),
    Release(MouseButton),
    Motion,
    /// Wheel travel in whole lines; `dy` < 0 scrolls up.
    Scroll {
        dy_lines: i32,
        dx_lines: i32,
    },
}

/// Mouse event in cell coordinates. `px_in_cell` refines the position for
/// pixel-precise protocols (SGR-Pixels).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MouseEvent {
    pub kind: MouseKind,
    pub col: u16,
    pub row: u16,
    pub px_in_cell: (u16, u16),
    pub mods: Mods,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectKind {
    Char,
    Word,
    Line,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollTarget {
    Top,
    Bottom,
    /// Absolute row offset from the top of the scrollback.
    Rows(usize),
}

/// Everything the UI can ask the emulator to do.
#[derive(Clone, Debug)]
pub enum Input {
    Key(KeyEvent),
    /// Only sent when `Modes::mouse_reporting`; the UI decides.
    Mouse(MouseEvent),
    Paste(String),
    /// Raw escape bytes straight to the PTY.
    Bytes(Vec<u8>),
    Resize {
        cols: u16,
        rows: u16,
        cell_w_px: u16,
        cell_h_px: u16,
    },
    /// `n` < 0 scrolls up.
    ScrollLines(i32),
    ScrollTo(ScrollTarget),
    SelectionBegin {
        col: u16,
        row: u16,
        kind: SelectKind,
        rectangle: bool,
    },
    SelectionExtend {
        col: u16,
        row: u16,
    },
    SelectionClear,
    SelectAll,
    /// Answers with `Event::SelectionText`.
    CopySelection,
    /// Answers with `Event::Link`.
    LinkAt {
        col: u16,
        row: u16,
    },
    /// `None` clears the search.
    Search(Option<String>),
    SearchNext,
    SearchPrev,
    /// Focus gained/lost; reported to the application under mode 1004.
    Focus(bool),
    ClearScreenAndScrollback,
    /// Emit a `Frame` even when nothing looks dirty.
    RequestFrame,
    /// Terminate the child and stop the pane thread.
    Shutdown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipboardTarget {
    Clipboard,
    Primary,
}

/// Everything the emulator/pane reports back to the UI.
#[derive(Clone, Debug)]
pub enum Event {
    Frame(Arc<Frame>),
    Title(String),
    Bell,
    /// OSC 52 / iTerm2 copy from the application.
    ClipboardWrite {
        target: ClipboardTarget,
        text: String,
    },
    SelectionText(Option<String>),
    Link(Option<String>),
    Exited {
        status: Option<i32>,
    },
    Error(String),
}
