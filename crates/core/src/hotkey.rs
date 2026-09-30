//! Keyboard shortcut for "Swap ports 1 and 2", stored as text in config.json
//! ("Ctrl+Alt+S"). There is no default: the user picks one from the menu.

use std::fmt;

pub const MOD_ALT: u32 = 0x1;
pub const MOD_CONTROL: u32 = 0x2;
pub const MOD_SHIFT: u32 = 0x4;
pub const MOD_WIN: u32 = 0x8;

const MODIFIERS: [(u32, &str); 4] = [
    (MOD_CONTROL, "Ctrl"),
    (MOD_ALT, "Alt"),
    (MOD_SHIFT, "Shift"),
    (MOD_WIN, "Win"),
];

/// Named keys besides letters, digits, F1-F24 and Numpad0-9.
const KEYS: [(u32, &str); 34] = [
    (0x08, "Backspace"),
    (0x09, "Tab"),
    (0x0D, "Enter"),
    (0x13, "Pause"),
    (0x20, "Space"),
    (0x21, "PageUp"),
    (0x22, "PageDown"),
    (0x23, "End"),
    (0x24, "Home"),
    (0x25, "Left"),
    (0x26, "Up"),
    (0x27, "Right"),
    (0x28, "Down"),
    (0x2D, "Insert"),
    (0x2E, "Delete"),
    (0x6A, "NumpadMultiply"),
    (0x6B, "NumpadAdd"),
    (0x6D, "NumpadSubtract"),
    (0x6E, "NumpadDecimal"),
    (0x6F, "NumpadDivide"),
    (0x91, "ScrollLock"),
    (0xBA, ";"),
    (0xBB, "="),
    (0xBC, ","),
    (0xBD, "-"),
    (0xBE, "."),
    (0xBF, "/"),
    (0xC0, "`"),
    (0xDB, "["),
    (0xDC, "\\"),
    (0xDD, "]"),
    (0xDE, "'"),
    (0x1B, "Esc"),
    (0x2C, "PrintScreen"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hotkey {
    /// MOD_* flags as used by RegisterHotKey.
    pub modifiers: u32,
    /// Virtual-key code.
    pub vk: u32,
}

fn key_name(vk: u32) -> String {
    match vk {
        0x30..=0x39 | 0x41..=0x5A => char::from(vk as u8).to_string(),
        0x60..=0x69 => format!("Numpad{}", vk - 0x60),
        0x70..=0x87 => format!("F{}", vk - 0x6F),
        _ => KEYS
            .iter()
            .find(|(k, _)| *k == vk)
            .map(|(_, n)| n.to_string())
            .unwrap_or_else(|| format!("0x{vk:02X}")),
    }
}

fn key_code(name: &str) -> Option<u32> {
    let up = name.to_ascii_uppercase();
    if let [c] = up.as_bytes()
        && c.is_ascii_alphanumeric()
    {
        return Some(*c as u32);
    }
    if let Some(n) = up
        .strip_prefix("NUMPAD")
        .and_then(|n| n.parse::<u32>().ok())
        .filter(|&n| n <= 9)
    {
        return Some(0x60 + n);
    }
    if let Some(n) = up
        .strip_prefix('F')
        .and_then(|n| n.parse::<u32>().ok())
        .filter(|&n| (1..=24).contains(&n))
    {
        return Some(0x6F + n);
    }
    if let Some(hex) = up.strip_prefix("0X") {
        return u32::from_str_radix(hex, 16).ok().filter(|&v| v > 0 && v < 0xFF);
    }
    KEYS.iter().find(|(_, n)| n.eq_ignore_ascii_case(name)).map(|(k, _)| *k)
}

/// Keys that may be used without a modifier: they do not get in the way of typing.
fn standalone(vk: u32) -> bool {
    matches!(vk, 0x70..=0x87 | 0x13 | 0x91)
}

/// True for Shift, Ctrl, Alt and Win keys themselves (left, right or generic).
pub fn is_modifier_key(vk: u32) -> bool {
    matches!(vk, 0x10..=0x12 | 0xA0..=0xA5 | 0x5B | 0x5C)
}

impl Hotkey {
    /// Checks that the shortcut is usable; the error says what to change.
    pub fn new(modifiers: u32, vk: u32) -> Result<Self, String> {
        let hk = Hotkey {
            modifiers: modifiers & (MOD_ALT | MOD_CONTROL | MOD_SHIFT | MOD_WIN),
            vk,
        };
        if vk == 0 || is_modifier_key(vk) {
            return Err("A shortcut needs a key besides Ctrl, Alt, Shift or Win.".into());
        }
        if hk.modifiers == 0 && !standalone(vk) {
            return Err(format!(
                "{} alone would get in the way of typing. Hold Ctrl, Alt, Shift or Win with it, or use F1-F24.",
                key_name(vk)
            ));
        }
        Ok(hk)
    }

    /// Parses "Ctrl+Alt+S" (case-insensitive).
    pub fn parse(text: &str) -> Result<Self, String> {
        let bad = || format!("\"{text}\" is not a valid shortcut; use a form like \"Ctrl+Alt+S\".");
        let parts: Vec<&str> = text.split('+').map(str::trim).collect();
        let (key, mods) = parts.split_last().ok_or_else(bad)?;
        let mut modifiers = 0;
        for m in mods {
            let (flag, _) = MODIFIERS
                .iter()
                .find(|(_, n)| n.eq_ignore_ascii_case(m))
                .ok_or_else(bad)?;
            modifiers |= flag;
        }
        Self::new(modifiers, key_code(key).ok_or_else(bad)?)
    }
}

impl fmt::Display for Hotkey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (flag, name) in MODIFIERS {
            if self.modifiers & flag != 0 {
                write!(f, "{name}+")?;
            }
        }
        f.write_str(&key_name(self.vk))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        for text in [
            "Ctrl+Alt+S",
            "Shift+F9",
            "F10",
            "Ctrl+Numpad5",
            "Shift+Win+.",
            "Alt+PageDown",
            "Ctrl+0xA7",
        ] {
            let hk = Hotkey::parse(text).unwrap();
            assert_eq!(hk.to_string(), text);
        }
    }

    #[test]
    fn parse_is_case_insensitive_and_orders_modifiers() {
        let hk = Hotkey::parse("alt + ctrl + s").unwrap();
        assert_eq!(
            hk,
            Hotkey {
                modifiers: MOD_ALT | MOD_CONTROL,
                vk: 0x53
            }
        );
        assert_eq!(hk.to_string(), "Ctrl+Alt+S");
    }

    #[test]
    fn rejects_unusable_shortcuts() {
        assert!(Hotkey::parse("S").is_err());
        assert!(Hotkey::parse("Space").is_err());
        assert!(Hotkey::parse("Ctrl+").is_err());
        assert!(Hotkey::parse("Hyper+S").is_err());
        assert!(Hotkey::parse("Ctrl+F25").is_err());
        assert!(Hotkey::new(MOD_CONTROL, 0x11).is_err());
        assert!(Hotkey::new(0, 0x78).is_ok());
    }
}
