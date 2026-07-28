//! GDK → libghostty key encoding.

use std::collections::HashSet;

use gtk4::gdk;
use libghostty_vt::{
    Terminal,
    key::{self, Key},
};

pub struct Input {
    encoder: key::Encoder<'static>,
    event: key::Event<'static>,
    pressed: HashSet<u32>,
    pub buf: Vec<u8>,
}

impl Input {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            encoder: key::Encoder::new().map_err(|e| anyhow::anyhow!("{e:?}"))?,
            event: key::Event::new().map_err(|e| anyhow::anyhow!("{e:?}"))?,
            pressed: HashSet::new(),
            buf: Vec::with_capacity(64),
        })
    }

    pub fn encode_press(
        &mut self,
        terminal: &Terminal<'_, '_>,
        keyval: gdk::Key,
        keycode: u32,
        state: gdk::ModifierType,
        text: Option<&str>,
    ) -> anyhow::Result<()> {
        let is_repeat = self.pressed.contains(&keycode);
        if !is_repeat {
            self.pressed.insert(keycode);
        }
        let action = if is_repeat {
            key::Action::Repeat
        } else {
            key::Action::Press
        };
        self.encode_inner(terminal, keyval, state, text, action)
    }

    pub fn encode_release(
        &mut self,
        terminal: &Terminal<'_, '_>,
        keyval: gdk::Key,
        keycode: u32,
        state: gdk::ModifierType,
    ) -> anyhow::Result<()> {
        self.pressed.remove(&keycode);
        self.encode_inner(terminal, keyval, state, None, key::Action::Release)
    }

    fn encode_inner(
        &mut self,
        terminal: &Terminal<'_, '_>,
        keyval: gdk::Key,
        state: gdk::ModifierType,
        text: Option<&str>,
        action: key::Action,
    ) -> anyhow::Result<()> {
        let key = gdk_to_key(keyval);
        if key == Key::Unidentified && text.is_none() && !is_modifier(keyval) {
            if let Some(t) = text {
                self.buf.extend_from_slice(t.as_bytes());
            }
            return Ok(());
        }

        let mods = gdk_mods(state);
        let mut consumed = key::Mods::empty();
        if text.is_some() && mods.contains(key::Mods::SHIFT) {
            consumed |= key::Mods::SHIFT;
        }

        let unshifted = unshifted_codepoint(keyval, text);

        self.event
            .set_action(action)
            .set_key(key)
            .set_mods(mods)
            .set_consumed_mods(consumed)
            .set_unshifted_codepoint(unshifted)
            .set_utf8(text);

        self.encoder
            .set_options_from_terminal(terminal)
            .encode_to_vec(&self.event, &mut self.buf)
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;

        if self.buf.is_empty() {
            if let Some(t) = text {
                if !mods.intersects(key::Mods::CTRL | key::Mods::ALT | key::Mods::SUPER) {
                    self.buf.extend_from_slice(t.as_bytes());
                }
            }
        }
        Ok(())
    }
}

fn is_modifier(keyval: gdk::Key) -> bool {
    matches!(
        keyval,
        gdk::Key::Shift_L
            | gdk::Key::Shift_R
            | gdk::Key::Control_L
            | gdk::Key::Control_R
            | gdk::Key::Alt_L
            | gdk::Key::Alt_R
            | gdk::Key::Super_L
            | gdk::Key::Super_R
            | gdk::Key::Meta_L
            | gdk::Key::Meta_R
            | gdk::Key::Caps_Lock
            | gdk::Key::Num_Lock
    )
}

fn gdk_mods(state: gdk::ModifierType) -> key::Mods {
    let mut mods = key::Mods::empty();
    if state.contains(gdk::ModifierType::SHIFT_MASK) {
        mods |= key::Mods::SHIFT;
    }
    if state.contains(gdk::ModifierType::CONTROL_MASK) {
        mods |= key::Mods::CTRL;
    }
    if state.contains(gdk::ModifierType::ALT_MASK) {
        mods |= key::Mods::ALT;
    }
    if state.contains(gdk::ModifierType::SUPER_MASK) {
        mods |= key::Mods::SUPER;
    }
    if state.contains(gdk::ModifierType::LOCK_MASK) {
        mods |= key::Mods::CAPS_LOCK;
    }
    mods
}

fn unshifted_codepoint(keyval: gdk::Key, text: Option<&str>) -> char {
    if let Some(t) = text {
        if let Some(c) = t.chars().next() {
            return c.to_ascii_lowercase();
        }
    }
    keyval
        .to_unicode()
        .map(|c| c.to_ascii_lowercase())
        .unwrap_or('\0')
}

fn gdk_to_key(keyval: gdk::Key) -> Key {
    if keyval == gdk::Key::space {
        return Key::Space;
    }
    if keyval == gdk::Key::Return {
        return Key::Enter;
    }
    if keyval == gdk::Key::Tab || keyval == gdk::Key::ISO_Left_Tab {
        return Key::Tab;
    }
    if keyval == gdk::Key::BackSpace {
        return Key::Backspace;
    }
    if keyval == gdk::Key::Delete {
        return Key::Delete;
    }
    if keyval == gdk::Key::Escape {
        return Key::Escape;
    }
    if keyval == gdk::Key::Up {
        return Key::ArrowUp;
    }
    if keyval == gdk::Key::Down {
        return Key::ArrowDown;
    }
    if keyval == gdk::Key::Left {
        return Key::ArrowLeft;
    }
    if keyval == gdk::Key::Right {
        return Key::ArrowRight;
    }
    if keyval == gdk::Key::Home {
        return Key::Home;
    }
    if keyval == gdk::Key::End {
        return Key::End;
    }
    if keyval == gdk::Key::Page_Up {
        return Key::PageUp;
    }
    if keyval == gdk::Key::Page_Down {
        return Key::PageDown;
    }
    if keyval == gdk::Key::Insert {
        return Key::Insert;
    }

    if keyval == gdk::Key::F1 {
        return Key::F1;
    }
    if keyval == gdk::Key::F2 {
        return Key::F2;
    }
    if keyval == gdk::Key::F3 {
        return Key::F3;
    }
    if keyval == gdk::Key::F4 {
        return Key::F4;
    }
    if keyval == gdk::Key::F5 {
        return Key::F5;
    }
    if keyval == gdk::Key::F6 {
        return Key::F6;
    }
    if keyval == gdk::Key::F7 {
        return Key::F7;
    }
    if keyval == gdk::Key::F8 {
        return Key::F8;
    }
    if keyval == gdk::Key::F9 {
        return Key::F9;
    }
    if keyval == gdk::Key::F10 {
        return Key::F10;
    }
    if keyval == gdk::Key::F11 {
        return Key::F11;
    }
    if keyval == gdk::Key::F12 {
        return Key::F12;
    }

    if keyval == gdk::Key::Shift_L {
        return Key::ShiftLeft;
    }
    if keyval == gdk::Key::Shift_R {
        return Key::ShiftRight;
    }
    if keyval == gdk::Key::Control_L {
        return Key::ControlLeft;
    }
    if keyval == gdk::Key::Control_R {
        return Key::ControlRight;
    }
    if keyval == gdk::Key::Alt_L {
        return Key::AltLeft;
    }
    if keyval == gdk::Key::Alt_R {
        return Key::AltRight;
    }
    if keyval == gdk::Key::Super_L || keyval == gdk::Key::Meta_L {
        return Key::MetaLeft;
    }
    if keyval == gdk::Key::Super_R || keyval == gdk::Key::Meta_R {
        return Key::MetaRight;
    }

    if keyval == gdk::Key::KP_Enter {
        return Key::NumpadEnter;
    }
    if keyval == gdk::Key::KP_Add {
        return Key::NumpadAdd;
    }
    if keyval == gdk::Key::KP_Subtract {
        return Key::NumpadSubtract;
    }
    if keyval == gdk::Key::KP_Multiply {
        return Key::NumpadMultiply;
    }
    if keyval == gdk::Key::KP_Divide {
        return Key::NumpadDivide;
    }
    if keyval == gdk::Key::KP_Decimal {
        return Key::NumpadDecimal;
    }
    if keyval == gdk::Key::KP_0 {
        return Key::Numpad0;
    }
    if keyval == gdk::Key::KP_1 {
        return Key::Numpad1;
    }
    if keyval == gdk::Key::KP_2 {
        return Key::Numpad2;
    }
    if keyval == gdk::Key::KP_3 {
        return Key::Numpad3;
    }
    if keyval == gdk::Key::KP_4 {
        return Key::Numpad4;
    }
    if keyval == gdk::Key::KP_5 {
        return Key::Numpad5;
    }
    if keyval == gdk::Key::KP_6 {
        return Key::Numpad6;
    }
    if keyval == gdk::Key::KP_7 {
        return Key::Numpad7;
    }
    if keyval == gdk::Key::KP_8 {
        return Key::Numpad8;
    }
    if keyval == gdk::Key::KP_9 {
        return Key::Numpad9;
    }

    let name = keyval.name().map(|s| s.to_ascii_lowercase()).unwrap_or_default();
    match name.as_str() {
        "a" => Key::A,
        "b" => Key::B,
        "c" => Key::C,
        "d" => Key::D,
        "e" => Key::E,
        "f" => Key::F,
        "g" => Key::G,
        "h" => Key::H,
        "i" => Key::I,
        "j" => Key::J,
        "k" => Key::K,
        "l" => Key::L,
        "m" => Key::M,
        "n" => Key::N,
        "o" => Key::O,
        "p" => Key::P,
        "q" => Key::Q,
        "r" => Key::R,
        "s" => Key::S,
        "t" => Key::T,
        "u" => Key::U,
        "v" => Key::V,
        "w" => Key::W,
        "x" => Key::X,
        "y" => Key::Y,
        "z" => Key::Z,
        "0" | "parenright" => Key::Digit0,
        "1" | "exclam" => Key::Digit1,
        "2" | "at" => Key::Digit2,
        "3" | "numbersign" => Key::Digit3,
        "4" | "dollar" => Key::Digit4,
        "5" | "percent" => Key::Digit5,
        "6" | "asciicircum" => Key::Digit6,
        "7" | "ampersand" => Key::Digit7,
        "8" | "asterisk" => Key::Digit8,
        "9" | "parenleft" => Key::Digit9,
        "minus" | "underscore" => Key::Minus,
        "equal" | "plus" => Key::Equal,
        "bracketleft" | "braceleft" => Key::BracketLeft,
        "bracketright" | "braceright" => Key::BracketRight,
        "backslash" | "bar" => Key::Backslash,
        "semicolon" | "colon" => Key::Semicolon,
        "apostrophe" | "quotedbl" => Key::Quote,
        "comma" | "less" => Key::Comma,
        "period" | "greater" => Key::Period,
        "slash" | "question" => Key::Slash,
        "grave" | "asciitilde" => Key::Backquote,
        _ => Key::Unidentified,
    }
}
