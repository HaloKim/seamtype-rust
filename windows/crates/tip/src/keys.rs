//! Virtual keys to `InputSession` keys (InputController.key(for:)). Seamtype reads the keyboard
//! as US QWERTY whatever the layout says, as the macOS version overrides the keyboard to US.

use seamtype_core::Key;
use windows::Win32::UI::Input::KeyboardAndMouse::*;

#[derive(Clone, Copy, Default, Debug)]
pub struct Modifiers {
    pub shift: bool,
    pub control: bool,
    pub alt: bool,
    pub win: bool,
    pub caps: bool,
}

fn down(vk: VIRTUAL_KEY) -> bool {
    unsafe { (GetKeyState(vk.0 as i32) as u16 & 0x8000) != 0 }
}

/// The modifier state of the key event being handled (GetKeyState follows the message queue).
pub fn modifiers() -> Modifiers {
    Modifiers {
        shift: down(VK_SHIFT),
        control: down(VK_CONTROL),
        alt: down(VK_MENU),
        win: down(VK_LWIN) || down(VK_RWIN),
        caps: unsafe { (GetKeyState(VK_CAPITAL.0 as i32) & 1) != 0 },
    }
}

/// Keys that are only modifiers or switches: never Seamtype's business.
pub fn is_modifier(vk: u32) -> bool {
    let vk = VIRTUAL_KEY(vk as u16);
    matches!(
        vk,
        VK_SHIFT | VK_LSHIFT | VK_RSHIFT | VK_CONTROL | VK_LCONTROL | VK_RCONTROL | VK_MENU | VK_LMENU | VK_RMENU
            | VK_LWIN | VK_RWIN | VK_CAPITAL | VK_NUMLOCK | VK_SCROLL | VK_HANGUL | VK_HANJA | VK_JUNJA | VK_FINAL
            | VK_CONVERT | VK_NONCONVERT | VK_ACCEPT | VK_MODECHANGE | VK_PROCESSKEY | VK_IME_ON | VK_IME_OFF
    ) || vk.0 == 0
}

/// The character a US keyboard types for `vk` (letters by `upper`, others by `shift`).
fn us_char(vk: u16, shift: bool, upper: bool) -> Option<char> {
    let c = match vk {
        0x41..=0x5A => {
            let lower = (b'a' + (vk - 0x41) as u8) as char;
            return Some(if upper { lower.to_ascii_uppercase() } else { lower });
        }
        0x30..=0x39 => {
            if shift { b")!@#$%^&*("[(vk - 0x30) as usize] as char } else { (b'0' + (vk - 0x30) as u8) as char }
        }
        0x20 => ' ',
        0xBA => if shift { ':' } else { ';' },
        0xBB => if shift { '+' } else { '=' },
        0xBC => if shift { '<' } else { ',' },
        0xBD => if shift { '_' } else { '-' },
        0xBE => if shift { '>' } else { '.' },
        0xBF => if shift { '?' } else { '/' },
        0xC0 => if shift { '~' } else { '`' },
        0xDB => if shift { '{' } else { '[' },
        0xDC => if shift { '|' } else { '\\' },
        0xDD => if shift { '}' } else { ']' },
        0xDE => if shift { '"' } else { '\'' },
        0x60..=0x69 => (b'0' + (vk - 0x60) as u8) as char, // numpad digits (Num Lock on)
        0x6A => '*',
        0x6B => '+',
        0x6D => '-',
        0x6E => '.',
        0x6F => '/',
        _ => return None,
    };
    Some(c)
}

/// The session key for a key press, or None for keys Seamtype leaves alone entirely.
pub fn key_for(vk: u32, m: &Modifiers) -> Option<Key> {
    if is_modifier(vk) {
        return None;
    }
    let v = VIRTUAL_KEY(vk as u16);
    if m.win {
        return Some(Key::Other);
    }
    if m.control && !m.alt {
        return Some(match v {
            VK_K => Key::Katakana,
            VK_J => Key::Hiragana,
            _ => Key::Other,
        });
    }
    if m.control || m.alt {
        return Some(Key::Other);
    }
    if v == VK_ESCAPE {
        return Some(if m.shift { Key::Cycle } else { Key::Escape });
    }
    Some(match v {
        VK_F7 => Key::Katakana,
        VK_F6 => Key::Hiragana,
        VK_BACK => Key::Delete,
        VK_RETURN => Key::Enter,
        VK_LEFT => Key::Left,
        VK_RIGHT => Key::Right,
        VK_DOWN => Key::Down,
        VK_UP => Key::Up,
        _ => match us_char(vk as u16, m.shift, m.shift != m.caps) {
            Some(c) if c.is_ascii_alphabetic() => Key::Letter(c),
            Some(c) => Key::Printable(c),
            None => Key::Other,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(shift: bool, caps: bool) -> Modifiers {
        Modifiers { shift, caps, ..Modifiers::default() }
    }

    #[test]
    fn letters_follow_shift_and_caps_lock() {
        assert_eq!(key_for(0x41, &m(false, false)), Some(Key::Letter('a')));
        assert_eq!(key_for(0x41, &m(true, false)), Some(Key::Letter('A')));
        assert_eq!(key_for(0x41, &m(false, true)), Some(Key::Letter('A')));
        assert_eq!(key_for(0x41, &m(true, true)), Some(Key::Letter('a')));
    }

    #[test]
    fn symbols_are_us_qwerty() {
        assert_eq!(key_for(0x31, &m(true, false)), Some(Key::Printable('!')));
        assert_eq!(key_for(0xDE, &m(false, false)), Some(Key::Printable('\'')));
        assert_eq!(key_for(0xBD, &m(true, false)), Some(Key::Printable('_')));
        assert_eq!(key_for(0x20, &m(false, false)), Some(Key::Printable(' ')));
    }

    #[test]
    fn special_keys() {
        assert_eq!(key_for(VK_ESCAPE.0 as u32, &m(true, false)), Some(Key::Cycle));
        assert_eq!(key_for(VK_ESCAPE.0 as u32, &m(false, false)), Some(Key::Escape));
        assert_eq!(key_for(VK_BACK.0 as u32, &m(false, false)), Some(Key::Delete));
        assert_eq!(key_for(VK_TAB.0 as u32, &m(false, false)), Some(Key::Other));
        assert_eq!(key_for(VK_SHIFT.0 as u32, &m(true, false)), None);
        assert_eq!(key_for(VK_HANGUL.0 as u32, &m(false, false)), None);
        let control = Modifiers { control: true, ..Modifiers::default() };
        assert_eq!(key_for(0x4B, &control), Some(Key::Katakana));
        assert_eq!(key_for(0x43, &control), Some(Key::Other));
    }
}
