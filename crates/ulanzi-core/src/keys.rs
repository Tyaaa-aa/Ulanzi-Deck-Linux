//! The key names a shortcut (`ctrl+alt+t`) may use: the ones the service can press. The GUI's
//! recorder checks a key against this list before storing it; the service maps the same names
//! to key codes (its tests check that the two agree).

/// Names besides the single letters `a`–`z`, the digits `0`–`9` and `f1`–`f24` (lowercase; a
/// name is matched case-insensitively). Mostly X keysym names, plus a few friendly aliases.
pub const NAMED_KEYS: &[&str] = &[
    "ctrl",
    "control",
    "control_l",
    "ctrl_l",
    "control_r",
    "ctrl_r",
    "alt",
    "alt_l",
    "alt_r",
    "altgr",
    "iso_level3_shift",
    "shift",
    "shift_l",
    "shift_r",
    "super",
    "meta",
    "win",
    "hyper",
    "super_l",
    "meta_l",
    "hyper_l",
    "super_r",
    "meta_r",
    "hyper_r",
    "return",
    "enter",
    "space",
    "tab",
    "escape",
    "esc",
    "backspace",
    "delete",
    "del",
    "insert",
    "home",
    "end",
    "prior",
    "page_up",
    "pageup",
    "next",
    "page_down",
    "pagedown",
    "up",
    "down",
    "left",
    "right",
    "print",
    "sysrq",
    "minus",
    "equal",
    "comma",
    "period",
    "slash",
    "semicolon",
    "apostrophe",
    "bracketleft",
    "bracketright",
    "backslash",
    "grave",
    "menu",
    "caps_lock",
    "num_lock",
    "scroll_lock",
    "pause",
    "kp_0",
    "kp_insert",
    "kp_1",
    "kp_end",
    "kp_2",
    "kp_down",
    "kp_3",
    "kp_next",
    "kp_page_down",
    "kp_4",
    "kp_left",
    "kp_5",
    "kp_begin",
    "kp_6",
    "kp_right",
    "kp_7",
    "kp_home",
    "kp_8",
    "kp_up",
    "kp_9",
    "kp_prior",
    "kp_page_up",
    "kp_decimal",
    "kp_delete",
    "kp_separator",
    "kp_enter",
    "kp_add",
    "kp_subtract",
    "kp_multiply",
    "kp_divide",
    "xf86audioplay",
    "xf86audiopause",
    "xf86audionext",
    "xf86audioprev",
    "xf86audiostop",
    "xf86audiomute",
    "xf86audioraisevolume",
    "xf86audiolowervolume",
    "xf86audiomicmute",
];

/// The highest function key the service can press (`f1` to `f24`).
pub const MAX_FUNCTION_KEY: u16 = 24;

/// Can the service press the key `name` (one part of a shortcut, case-insensitive)?
pub fn is_sendable(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    let mut chars = n.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return c.is_ascii_lowercase() || c.is_ascii_digit();
    }
    if let Some(f) = n.strip_prefix('f').and_then(|d| d.parse::<u16>().ok()) {
        return (1..=MAX_FUNCTION_KEY).contains(&f) && n == format!("f{f}");
    }
    NAMED_KEYS.contains(&n.as_str())
}

/// Every key name [`is_sendable`] accepts, in lowercase (for tests that compare lists).
pub fn all_sendable() -> Vec<String> {
    let mut out: Vec<String> = ('a'..='z').chain('0'..='9').map(String::from).collect();
    out.extend((1..=MAX_FUNCTION_KEY).map(|f| format!("f{f}")));
    out.extend(NAMED_KEYS.iter().map(|n| (*n).to_string()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_digits_function_keys_and_names() {
        for n in [
            "a",
            "Z",
            "0",
            "9",
            "F1",
            "f12",
            "f24",
            "Return",
            "page_down",
            "KP_Add",
            "XF86AudioPlay",
        ] {
            assert!(is_sendable(n), "{n}");
        }
    }

    #[test]
    fn keys_the_service_cannot_press() {
        for n in [
            "",
            "less",
            "udiaeresis",
            "plus",
            "exclam",
            "f0",
            "f25",
            "f01",
            "ä",
            "dead_acute",
            "ab",
        ] {
            assert!(!is_sendable(n), "{n}");
        }
    }

    #[test]
    fn the_list_is_lowercase_and_has_no_duplicates() {
        let all = all_sendable();
        for (i, n) in all.iter().enumerate() {
            assert_eq!(*n, n.to_ascii_lowercase());
            assert!(!all[..i].contains(n), "{n} twice");
            assert!(is_sendable(n), "{n}");
        }
    }
}
