//! Keyboard shortcuts through a uinput virtual keyboard (works on Wayland and X11).

use std::time::Duration;

use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, EventType, InputEvent, KeyCode};

use super::ActionError;

/// Parse an xdotool-style combo such as `ctrl+alt+t` or `super+F12` (names are case-insensitive).
pub fn parse_combo(s: &str) -> Result<Vec<KeyCode>, ActionError> {
    let parts: Vec<&str> = s.split('+').map(str::trim).filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return Err(ActionError::Failed("the shortcut is empty".into()));
    }
    parts
        .iter()
        .map(|p| {
            key_named(p)
                .ok_or_else(|| ActionError::Failed(format!("unknown key \"{p}\" in shortcut \"{s}\"")))
        })
        .collect()
}

/// The key code for a name [`ulanzi_core::keys::is_sendable`] accepts (the GUI's recorder stores
/// only those); `None` for any other name.
pub fn key_named(name: &str) -> Option<KeyCode> {
    use KeyCode as K;
    if !ulanzi_core::keys::is_sendable(name) {
        return None;
    }
    let n = name.to_ascii_lowercase();
    let letter = |c: char| {
        const LETTERS: [K; 26] = [
            K::KEY_A,
            K::KEY_B,
            K::KEY_C,
            K::KEY_D,
            K::KEY_E,
            K::KEY_F,
            K::KEY_G,
            K::KEY_H,
            K::KEY_I,
            K::KEY_J,
            K::KEY_K,
            K::KEY_L,
            K::KEY_M,
            K::KEY_N,
            K::KEY_O,
            K::KEY_P,
            K::KEY_Q,
            K::KEY_R,
            K::KEY_S,
            K::KEY_T,
            K::KEY_U,
            K::KEY_V,
            K::KEY_W,
            K::KEY_X,
            K::KEY_Y,
            K::KEY_Z,
        ];
        LETTERS[(c as u8 - b'a') as usize]
    };
    let mut chars = n.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        match c {
            'a'..='z' => return Some(letter(c)),
            '0' => return Some(K::KEY_0),
            '1'..='9' => return Some(KeyCode(K::KEY_1.0 + (c as u16 - '1' as u16))),
            _ => {}
        }
    }
    if let Some(f) = n.strip_prefix('f').and_then(|d| d.parse::<u16>().ok()) {
        return match f {
            1..=10 => Some(KeyCode(K::KEY_F1.0 + f - 1)),
            11 => Some(K::KEY_F11),
            12 => Some(K::KEY_F12),
            13..=24 => Some(KeyCode(K::KEY_F13.0 + f - 13)),
            _ => None,
        };
    }
    Some(match n.as_str() {
        "ctrl" | "control" | "control_l" | "ctrl_l" => K::KEY_LEFTCTRL,
        "control_r" | "ctrl_r" => K::KEY_RIGHTCTRL,
        "alt" | "alt_l" => K::KEY_LEFTALT,
        "alt_r" | "altgr" | "iso_level3_shift" => K::KEY_RIGHTALT,
        "shift" | "shift_l" => K::KEY_LEFTSHIFT,
        "shift_r" => K::KEY_RIGHTSHIFT,
        "super" | "meta" | "win" | "hyper" | "super_l" | "meta_l" | "hyper_l" => K::KEY_LEFTMETA,
        "super_r" | "meta_r" | "hyper_r" => K::KEY_RIGHTMETA,
        "return" | "enter" => K::KEY_ENTER,
        "space" => K::KEY_SPACE,
        "tab" => K::KEY_TAB,
        "escape" | "esc" => K::KEY_ESC,
        "backspace" => K::KEY_BACKSPACE,
        "delete" | "del" => K::KEY_DELETE,
        "insert" => K::KEY_INSERT,
        "home" => K::KEY_HOME,
        "end" => K::KEY_END,
        "prior" | "page_up" | "pageup" => K::KEY_PAGEUP,
        "next" | "page_down" | "pagedown" => K::KEY_PAGEDOWN,
        "up" => K::KEY_UP,
        "down" => K::KEY_DOWN,
        "left" => K::KEY_LEFT,
        "right" => K::KEY_RIGHT,
        "print" | "sysrq" => K::KEY_SYSRQ,
        "minus" => K::KEY_MINUS,
        "equal" => K::KEY_EQUAL,
        "comma" => K::KEY_COMMA,
        "period" => K::KEY_DOT,
        "slash" => K::KEY_SLASH,
        "semicolon" => K::KEY_SEMICOLON,
        "apostrophe" => K::KEY_APOSTROPHE,
        "bracketleft" => K::KEY_LEFTBRACE,
        "bracketright" => K::KEY_RIGHTBRACE,
        "backslash" => K::KEY_BACKSLASH,
        "grave" => K::KEY_GRAVE,
        "menu" => K::KEY_COMPOSE,
        "caps_lock" => K::KEY_CAPSLOCK,
        "num_lock" => K::KEY_NUMLOCK,
        "scroll_lock" => K::KEY_SCROLLLOCK,
        "pause" => K::KEY_PAUSE,
        "kp_0" | "kp_insert" => K::KEY_KP0,
        "kp_1" | "kp_end" => K::KEY_KP1,
        "kp_2" | "kp_down" => K::KEY_KP2,
        "kp_3" | "kp_next" | "kp_page_down" => K::KEY_KP3,
        "kp_4" | "kp_left" => K::KEY_KP4,
        "kp_5" | "kp_begin" => K::KEY_KP5,
        "kp_6" | "kp_right" => K::KEY_KP6,
        "kp_7" | "kp_home" => K::KEY_KP7,
        "kp_8" | "kp_up" => K::KEY_KP8,
        "kp_9" | "kp_prior" | "kp_page_up" => K::KEY_KP9,
        "kp_decimal" | "kp_delete" | "kp_separator" => K::KEY_KPDOT,
        "kp_enter" => K::KEY_KPENTER,
        "kp_add" => K::KEY_KPPLUS,
        "kp_subtract" => K::KEY_KPMINUS,
        "kp_multiply" => K::KEY_KPASTERISK,
        "kp_divide" => K::KEY_KPSLASH,
        "xf86audioplay" | "xf86audiopause" => K::KEY_PLAYPAUSE,
        "xf86audionext" => K::KEY_NEXTSONG,
        "xf86audioprev" => K::KEY_PREVIOUSSONG,
        "xf86audiostop" => K::KEY_STOPCD,
        "xf86audiomute" => K::KEY_MUTE,
        "xf86audioraisevolume" => K::KEY_VOLUMEUP,
        "xf86audiolowervolume" => K::KEY_VOLUMEDOWN,
        "xf86audiomicmute" => K::KEY_MICMUTE,
        _ => return None,
    })
}

/// Longest text typed key by key; longer text is pasted.
pub const MAX_TYPED: usize = 200;

/// The keys for one character on a US layout (Shift first when needed); `None` if it has none.
pub fn char_keys(c: char) -> Option<Vec<KeyCode>> {
    use KeyCode as K;
    let plain = |k: KeyCode| Some(vec![k]);
    let shift = |k: KeyCode| Some(vec![K::KEY_LEFTSHIFT, k]);
    if c.is_ascii_lowercase() {
        return plain(letter(c));
    }
    if c.is_ascii_uppercase() {
        return shift(letter(c.to_ascii_lowercase()));
    }
    let digit = |d: char| match d {
        '0' => K::KEY_0,
        _ => KeyCode(K::KEY_1.0 + (d as u16 - '1' as u16)),
    };
    match c {
        '0'..='9' => plain(digit(c)),
        ' ' => plain(K::KEY_SPACE),
        '\n' => plain(K::KEY_ENTER),
        '\t' => plain(K::KEY_TAB),
        '-' => plain(K::KEY_MINUS),
        '=' => plain(K::KEY_EQUAL),
        '[' => plain(K::KEY_LEFTBRACE),
        ']' => plain(K::KEY_RIGHTBRACE),
        '\\' => plain(K::KEY_BACKSLASH),
        ';' => plain(K::KEY_SEMICOLON),
        '\'' => plain(K::KEY_APOSTROPHE),
        '`' => plain(K::KEY_GRAVE),
        ',' => plain(K::KEY_COMMA),
        '.' => plain(K::KEY_DOT),
        '/' => plain(K::KEY_SLASH),
        '!' => shift(K::KEY_1),
        '@' => shift(K::KEY_2),
        '#' => shift(K::KEY_3),
        '$' => shift(K::KEY_4),
        '%' => shift(K::KEY_5),
        '^' => shift(K::KEY_6),
        '&' => shift(K::KEY_7),
        '*' => shift(K::KEY_8),
        '(' => shift(K::KEY_9),
        ')' => shift(K::KEY_0),
        '_' => shift(K::KEY_MINUS),
        '+' => shift(K::KEY_EQUAL),
        '{' => shift(K::KEY_LEFTBRACE),
        '}' => shift(K::KEY_RIGHTBRACE),
        '|' => shift(K::KEY_BACKSLASH),
        ':' => shift(K::KEY_SEMICOLON),
        '"' => shift(K::KEY_APOSTROPHE),
        '~' => shift(K::KEY_GRAVE),
        '<' => shift(K::KEY_COMMA),
        '>' => shift(K::KEY_DOT),
        '?' => shift(K::KEY_SLASH),
        _ => None,
    }
}

/// The key for a lowercase ASCII letter (evdev numbers keys by position, not alphabet).
fn letter(c: char) -> KeyCode {
    use KeyCode as K;
    const LETTERS: [KeyCode; 26] = [
        K::KEY_A,
        K::KEY_B,
        K::KEY_C,
        K::KEY_D,
        K::KEY_E,
        K::KEY_F,
        K::KEY_G,
        K::KEY_H,
        K::KEY_I,
        K::KEY_J,
        K::KEY_K,
        K::KEY_L,
        K::KEY_M,
        K::KEY_N,
        K::KEY_O,
        K::KEY_P,
        K::KEY_Q,
        K::KEY_R,
        K::KEY_S,
        K::KEY_T,
        K::KEY_U,
        K::KEY_V,
        K::KEY_W,
        K::KEY_X,
        K::KEY_Y,
        K::KEY_Z,
    ];
    LETTERS[(c as u8 - b'a') as usize]
}

/// How to type some text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Typing {
    /// Each character's keys.
    Keys(Vec<Vec<KeyCode>>),
    /// Through the clipboard (characters a US layout can't type, or long text).
    Paste,
}

pub fn plan_typing(text: &str) -> Typing {
    if text.chars().count() > MAX_TYPED {
        return Typing::Paste;
    }
    text.chars()
        .map(char_keys)
        .collect::<Option<Vec<_>>>()
        .map(Typing::Keys)
        .unwrap_or(Typing::Paste)
}

/// Key events for a combo: each key pressed in order, then released in reverse
/// (`super+x` presses Meta, then X, releases X, then Meta). 1 = press, 0 = release.
pub fn combo_events(keys: &[KeyCode]) -> Vec<(KeyCode, i32)> {
    let down = keys.iter().map(|&k| (k, 1));
    let up = keys.iter().rev().map(|&k| (k, 0));
    down.chain(up).collect()
}

/// A virtual keyboard that can press any key.
pub struct VirtualKeyboard {
    dev: VirtualDevice,
}

const GAP: Duration = Duration::from_millis(8);

impl VirtualKeyboard {
    pub fn new() -> Result<VirtualKeyboard, ActionError> {
        let unavailable = |e: std::io::Error| {
            ActionError::Unavailable(format!(
                "keyboard shortcuts need access to /dev/uinput ({e}); run the permission step in setup"
            ))
        };
        let mut keys = AttributeSet::<KeyCode>::new();
        for code in 1..=KeyCode::KEY_MICMUTE.0 {
            keys.insert(KeyCode(code));
        }
        let dev = VirtualDevice::builder()
            .map_err(unavailable)?
            .name("Ulanzi Deck virtual keyboard")
            .with_keys(&keys)
            .map_err(unavailable)?
            .build()
            .map_err(unavailable)?;
        Ok(VirtualKeyboard { dev })
    }

    /// Press or release one key, for macros.
    pub fn set_key(&mut self, key: KeyCode, down: bool) -> Result<(), ActionError> {
        self.dev
            .emit(&[InputEvent::new(EventType::KEY.0, key.0, down as i32)])
            .map_err(|e| ActionError::Failed(format!("could not send keys: {e}")))
    }

    /// Press keys in order, then release them in reverse.
    pub fn press_combo(&mut self, keys: &[KeyCode]) -> Result<(), ActionError> {
        let err = |e: std::io::Error| ActionError::Failed(format!("could not send keys: {e}"));
        for (k, value) in combo_events(keys) {
            self.dev
                .emit(&[InputEvent::new(EventType::KEY.0, k.0, value)])
                .map_err(err)?;
            std::thread::sleep(GAP);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use KeyCode as K;

    #[test]
    fn super_names_are_left_meta() {
        for n in [
            "super", "Super", "meta", "win", "super_l", "meta_l", "hyper", "hyper_l",
        ] {
            assert_eq!(parse_combo(n).expect(n), vec![K::KEY_LEFTMETA], "{n}");
        }
        for n in ["super_r", "meta_r", "hyper_r"] {
            assert_eq!(parse_combo(n).expect(n), vec![K::KEY_RIGHTMETA], "{n}");
        }
    }

    #[test]
    fn a_lone_modifier_is_a_combo() {
        let keys = parse_combo("super").expect("super alone");
        assert_eq!(
            combo_events(&keys),
            vec![(K::KEY_LEFTMETA, 1), (K::KEY_LEFTMETA, 0)]
        );
        assert_eq!(parse_combo("ctrl").expect("ctrl"), vec![K::KEY_LEFTCTRL]);
    }

    #[test]
    fn super_x_presses_meta_first_and_releases_it_last() {
        let keys = parse_combo("super+x").expect("super+x");
        assert_eq!(keys, vec![K::KEY_LEFTMETA, K::KEY_X]);
        assert_eq!(
            combo_events(&keys),
            vec![
                (K::KEY_LEFTMETA, 1),
                (K::KEY_X, 1),
                (K::KEY_X, 0),
                (K::KEY_LEFTMETA, 0),
            ]
        );
    }

    #[test]
    fn every_recorded_modifier_set_parses() {
        let keys = parse_combo("ctrl+alt+shift+super+f5").expect("all modifiers");
        assert_eq!(
            keys,
            vec![
                K::KEY_LEFTCTRL,
                K::KEY_LEFTALT,
                K::KEY_LEFTSHIFT,
                K::KEY_LEFTMETA,
                K::KEY_F5
            ]
        );
        assert_eq!(
            parse_combo("ctrl+super").expect("modifiers only"),
            vec![K::KEY_LEFTCTRL, K::KEY_LEFTMETA]
        );
    }

    #[test]
    fn lock_and_keypad_keys_parse() {
        assert_eq!(parse_combo("caps_lock").expect("caps"), vec![K::KEY_CAPSLOCK]);
        assert_eq!(parse_combo("pause").expect("pause"), vec![K::KEY_PAUSE]);
        assert_eq!(
            parse_combo("super+kp_add").expect("kp"),
            vec![K::KEY_LEFTMETA, K::KEY_KPPLUS]
        );
        assert_eq!(parse_combo("kp_7").expect("kp7"), vec![K::KEY_KP7]);
    }

    #[test]
    fn every_name_the_gui_may_store_has_a_key_code() {
        for n in ulanzi_core::keys::all_sendable() {
            assert!(key_named(&n).is_some(), "{n} is sendable but has no key code");
            assert!(key_named(&n.to_ascii_uppercase()).is_some(), "{n} in capitals");
        }
        assert_eq!(key_named("less"), None);
        assert_eq!(key_named("f01"), None);
    }

    #[test]
    fn every_name_key_named_handles_is_shared_with_the_gui() {
        // The names spelled out in `key_named`'s table must all be in ulanzi-core's list, or the
        // GUI would refuse a key the service can press.
        let src = include_str!("keys.rs");
        let start = src.find("fn key_named").expect("key_named");
        let end = start + src[start..].find("\n}\n").expect("end of key_named");
        let names: Vec<&str> = src[start..end].split('"').skip(1).step_by(2).collect();
        assert!(names.len() > 50, "{names:?}");
        for n in names {
            assert!(ulanzi_core::keys::is_sendable(n), "{n} is handled but not shared");
            assert!(key_named(n).is_some(), "{n}");
        }
    }
}
