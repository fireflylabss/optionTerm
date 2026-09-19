//! Pure translation from GPUI keystrokes to VT `KeyEvent`s.

use gpui::Keystroke;
use option_term_vt::input::{Key, KeyAction, KeyEvent, Mods, NamedKey};

/// Convert a GPUI `Keystroke` into a VT `KeyEvent`.
///
/// `is_held` marks repeats. Composed text (shift, IME, dead keys) arrives in
/// `keystroke.key_char` and is passed through as `text`; the encoder only
/// falls back to the base key when no text is present.
pub fn keystroke_to_key_event(keystroke: &Keystroke, is_held: bool) -> Option<KeyEvent> {
    let key = named_key(&keystroke.key).map(Key::Named).or_else(|| {
        let mut chars = keystroke.key.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) => Some(Key::Char(c)),
            _ => Some(Key::Unknown),
        }
    })?;

    let unshifted = match key {
        Key::Char(c) => Some(c),
        _ => None,
    };

    Some(KeyEvent {
        action: if is_held {
            KeyAction::Repeat
        } else {
            KeyAction::Press
        },
        key,
        mods: mods(&keystroke.modifiers),
        text: keystroke.key_char.clone(),
        unshifted,
    })
}

fn mods(m: &gpui::Modifiers) -> Mods {
    Mods {
        shift: m.shift,
        ctrl: m.control,
        alt: m.alt,
        super_: m.platform,
        // GPUI's `Modifiers` does not report caps/num lock at this rev.
        caps_lock: false,
        num_lock: false,
    }
}

fn named_key(name: &str) -> Option<NamedKey> {
    let key = match name {
        "enter" => NamedKey::Enter,
        "tab" => NamedKey::Tab,
        "backspace" => NamedKey::Backspace,
        "escape" => NamedKey::Escape,
        "space" => NamedKey::Space,
        "insert" => NamedKey::Insert,
        "delete" => NamedKey::Delete,
        "home" => NamedKey::Home,
        "end" => NamedKey::End,
        "pageup" => NamedKey::PageUp,
        "pagedown" => NamedKey::PageDown,
        "left" => NamedKey::Left,
        "right" => NamedKey::Right,
        "up" => NamedKey::Up,
        "down" => NamedKey::Down,
        "shift" => NamedKey::ShiftLeft,
        "control" => NamedKey::CtrlLeft,
        "alt" | "option" => NamedKey::AltLeft,
        "super" | "cmd" | "win" => NamedKey::SuperLeft,
        "capslock" => NamedKey::CapsLock,
        "numlock" => NamedKey::NumLock,
        _ => {
            if let Some(digits) = name.strip_prefix('f')
                && let Ok(n) = digits.parse::<u8>()
            {
                return Some(NamedKey::F(n));
            }
            return None;
        }
    };
    Some(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::Modifiers;

    fn keystroke(key: &str, key_char: Option<&str>, modifiers: Modifiers) -> Keystroke {
        Keystroke {
            modifiers,
            key: key.into(),
            key_char: key_char.map(str::to_string),
        }
    }

    #[test]
    fn ctrl_a() {
        let ev = keystroke_to_key_event(
            &keystroke(
                "a",
                None,
                Modifiers {
                    control: true,
                    ..Default::default()
                },
            ),
            false,
        )
        .unwrap();
        assert_eq!(ev.action, KeyAction::Press);
        assert_eq!(ev.key, Key::Char('a'));
        assert!(ev.mods.ctrl);
        assert_eq!(ev.unshifted, Some('a'));
        assert_eq!(ev.text, None);
    }

    #[test]
    fn shifted_uppercase_uses_composed_text() {
        let ev = keystroke_to_key_event(
            &keystroke(
                "a",
                Some("A"),
                Modifiers {
                    shift: true,
                    ..Default::default()
                },
            ),
            false,
        )
        .unwrap();
        assert_eq!(ev.key, Key::Char('a'));
        assert!(ev.mods.shift);
        assert_eq!(ev.text.as_deref(), Some("A"));
        assert_eq!(ev.unshifted, Some('a'));
    }

    #[test]
    fn named_keys() {
        let enter = keystroke_to_key_event(&keystroke("enter", None, Modifiers::default()), false);
        assert_eq!(enter.unwrap().key, Key::Named(NamedKey::Enter));

        let pageup =
            keystroke_to_key_event(&keystroke("pageup", None, Modifiers::default()), false);
        assert_eq!(pageup.unwrap().key, Key::Named(NamedKey::PageUp));

        let f5 = keystroke_to_key_event(&keystroke("f5", None, Modifiers::default()), false);
        assert_eq!(f5.unwrap().key, Key::Named(NamedKey::F(5)));
    }

    #[test]
    fn alt_left() {
        let ev = keystroke_to_key_event(
            &keystroke(
                "left",
                None,
                Modifiers {
                    alt: true,
                    ..Default::default()
                },
            ),
            false,
        )
        .unwrap();
        assert_eq!(ev.key, Key::Named(NamedKey::Left));
        assert!(ev.mods.alt);
    }

    #[test]
    fn repeat_is_held() {
        let ev = keystroke_to_key_event(&keystroke("a", None, Modifiers::default()), true).unwrap();
        assert_eq!(ev.action, KeyAction::Repeat);
    }
}
