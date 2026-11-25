use std::fmt;
use std::str::FromStr;

use anyhow::{Context, bail};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub super_: bool,
}

/// A global keyboard shortcut such as `Ctrl+Alt+Left`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Shortcut {
    pub modifiers: Modifiers,
    pub key: String,
    pub keysym: u32,
}

impl FromStr for Shortcut {
    type Err = anyhow::Error;

    fn from_str(text: &str) -> anyhow::Result<Self> {
        let mut parts: Vec<&str> = text.split('+').map(str::trim).collect();
        let key = parts
            .pop()
            .filter(|k| !k.is_empty())
            .context("shortcut has no key")?;
        let mut modifiers = Modifiers::default();
        for part in parts {
            let flag = match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => &mut modifiers.ctrl,
                "alt" | "option" => &mut modifiers.alt,
                "shift" => &mut modifiers.shift,
                "super" | "win" | "cmd" | "command" => &mut modifiers.super_,
                other => bail!("unknown modifier `{other}` in shortcut `{text}`"),
            };
            if std::mem::replace(flag, true) {
                bail!("modifier `{part}` repeated in shortcut `{text}`");
            }
        }
        let (key, keysym) =
            keysym(key).with_context(|| format!("unknown key `{key}` in shortcut `{text}`"))?;
        Ok(Self {
            modifiers,
            key,
            keysym,
        })
    }
}

impl Shortcut {
    /// The shortcut in the `<Control><Alt>Left` form Mutter accepts.
    #[cfg(not(target_os = "macos"))]
    pub fn gnome_accelerator(&self) -> String {
        let m = self.modifiers;
        let mut out = String::new();
        for (on, name) in [
            (m.ctrl, "<Control>"),
            (m.alt, "<Alt>"),
            (m.shift, "<Shift>"),
            (m.super_, "<Super>"),
        ] {
            if on {
                out.push_str(name);
            }
        }
        if self.key.len() == 1 {
            out.push_str(&self.key.to_ascii_lowercase());
        } else {
            out.push_str(&self.key);
        }
        out
    }

    /// macOS virtual key code (`kVK_*`) for this shortcut's key.
    #[cfg(target_os = "macos")]
    pub fn macos_virtual_key(&self) -> Option<u16> {
        if self.key.len() == 1 {
            let byte = self.key.as_bytes()[0].to_ascii_lowercase();
            return ANSI_KEY
                .iter()
                .find(|(name, _)| *name == byte)
                .map(|(_, code)| *code);
        }
        Some(match self.keysym {
            0xff51 => 0x7b, // Left
            0xff52 => 0x7e, // Up
            0xff53 => 0x7c, // Right
            0xff54 => 0x7d, // Down
            0xff0d => 0x24, // Return
            0xff08 => 0x33, // Backspace
            0xffff => 0x75, // Forward delete
            0xff09 => 0x30, // Tab
            0xff1b => 0x35, // Escape
            0x20 => 0x31,   // Space
            0xff50 => 0x73, // Home
            0xff57 => 0x77, // End
            0xff55 => 0x74, // Page up
            0xff56 => 0x79, // Page down
            _ => return None,
        })
    }
}

impl Shortcut {
    /// The shortcut for a macOS virtual key code, for keys that
    /// `macos_virtual_key` can turn back into the same code.
    #[cfg(target_os = "macos")]
    pub fn from_macos_key(modifiers: Modifiers, keycode: u16) -> Option<Self> {
        let name = match ANSI_KEY.iter().find(|(_, code)| *code == keycode) {
            Some((letter, _)) => char::from(*letter).to_string(),
            None => match keycode {
                0x7b => "Left",
                0x7e => "Up",
                0x7c => "Right",
                0x7d => "Down",
                0x24 => "Return",
                0x33 => "BackSpace",
                0x75 => "Delete",
                0x30 => "Tab",
                0x31 => "space",
                0x73 => "Home",
                0x77 => "End",
                0x74 => "Page_Up",
                0x79 => "Page_Down",
                _ => return None,
            }
            .to_string(),
        };
        let (key, keysym) = keysym(&name)?;
        Some(Self {
            modifiers,
            key,
            keysym,
        })
    }
}

/// ANSI virtual key codes from HIToolbox/Events.h. The keyboard layout does not change them.
#[cfg(target_os = "macos")]
const ANSI_KEY: &[(u8, u16)] = &[
    (b'a', 0x00),
    (b's', 0x01),
    (b'd', 0x02),
    (b'f', 0x03),
    (b'h', 0x04),
    (b'g', 0x05),
    (b'z', 0x06),
    (b'x', 0x07),
    (b'c', 0x08),
    (b'v', 0x09),
    (b'b', 0x0b),
    (b'q', 0x0c),
    (b'w', 0x0d),
    (b'e', 0x0e),
    (b'r', 0x0f),
    (b'y', 0x10),
    (b't', 0x11),
    (b'1', 0x12),
    (b'2', 0x13),
    (b'3', 0x14),
    (b'4', 0x15),
    (b'6', 0x16),
    (b'5', 0x17),
    (b'9', 0x19),
    (b'7', 0x1a),
    (b'8', 0x1c),
    (b'0', 0x1d),
    (b'o', 0x1f),
    (b'u', 0x20),
    (b'i', 0x22),
    (b'p', 0x23),
    (b'l', 0x25),
    (b'j', 0x26),
    (b'k', 0x28),
    (b'n', 0x2d),
    (b'm', 0x2e),
];

impl fmt::Display for Shortcut {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let m = self.modifiers;
        for (on, name) in [
            (m.ctrl, "Ctrl"),
            (m.alt, "Alt"),
            (m.shift, "Shift"),
            (m.super_, "Super"),
        ] {
            if on {
                write!(f, "{name}+")?;
            }
        }
        f.write_str(&self.key)
    }
}

/// Resolves a key name to its canonical spelling and X11 keysym.
fn keysym(name: &str) -> Option<(String, u32)> {
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        let lower = c.to_ascii_lowercase();
        if lower.is_ascii_alphanumeric() {
            return Some((lower.to_ascii_uppercase().to_string(), u32::from(lower)));
        }
    }
    if let Some(n) = name
        .strip_prefix(['F', 'f'])
        .and_then(|n| n.parse::<u32>().ok())
        && (1..=24).contains(&n)
    {
        return Some((format!("F{n}"), 0xffbe + n - 1));
    }
    const NAMED: &[(&[&str], u32)] = &[
        (&["Left"], 0xff51),
        (&["Up"], 0xff52),
        (&["Right"], 0xff53),
        (&["Down"], 0xff54),
        (&["Return", "Enter"], 0xff0d),
        (&["BackSpace", "Backspace"], 0xff08),
        (&["Delete"], 0xffff),
        (&["Tab"], 0xff09),
        (&["Escape", "Esc"], 0xff1b),
        (&["space", "Space"], 0x20),
        (&["Home"], 0xff50),
        (&["End"], 0xff57),
        (&["Page_Up", "PageUp", "Prior"], 0xff55),
        (&["Page_Down", "PageDown", "Next"], 0xff56),
        (&["Insert"], 0xff63),
        (&["minus"], 0x2d),
        (&["equal"], 0x3d),
        (&["comma"], 0x2c),
        (&["period"], 0x2e),
        (&["slash"], 0x2f),
        (&["backslash"], 0x5c),
        (&["semicolon"], 0x3b),
        (&["apostrophe"], 0x27),
        (&["grave"], 0x60),
        (&["bracketleft"], 0x5b),
        (&["bracketright"], 0x5d),
    ];
    NAMED
        .iter()
        .find(|(names, _)| names.iter().any(|n| n.eq_ignore_ascii_case(name)))
        .map(|(names, sym)| (names[0].to_string(), *sym))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_canonicalises() {
        let s: Shortcut = "alt + ctrl + left".parse().unwrap();
        assert_eq!(s.to_string(), "Ctrl+Alt+Left");
        assert_eq!(s.keysym, 0xff51);

        let s: Shortcut = "Ctrl+Option+Cmd+u".parse().unwrap();
        assert_eq!(s.to_string(), "Ctrl+Alt+Super+U");
        assert_eq!(s.keysym, u32::from(b'u'));

        assert_eq!("Super+F12".parse::<Shortcut>().unwrap().keysym, 0xffc9);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_virtual_keys() {
        assert_eq!(
            "Ctrl+Alt+Left"
                .parse::<Shortcut>()
                .unwrap()
                .macos_virtual_key(),
            Some(0x7b)
        );
        assert_eq!(
            "Ctrl+Alt+U"
                .parse::<Shortcut>()
                .unwrap()
                .macos_virtual_key(),
            Some(0x20)
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_key_codes_round_trip() {
        let modifiers = Modifiers {
            ctrl: true,
            ..Modifiers::default()
        };
        let mut found = 0;
        for code in 0..0x80u16 {
            if let Some(shortcut) = Shortcut::from_macos_key(modifiers, code) {
                assert_eq!(shortcut.macos_virtual_key(), Some(code), "{shortcut}");
                found += 1;
            }
        }
        assert_eq!(found, 36 + 13);
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn gnome_accelerators() {
        let accel = |text: &str| text.parse::<Shortcut>().unwrap().gnome_accelerator();
        assert_eq!(accel("Ctrl+Alt+Left"), "<Control><Alt>Left");
        assert_eq!(accel("Ctrl+Alt+U"), "<Control><Alt>u");
        assert_eq!(accel("Ctrl+Alt+Super+Up"), "<Control><Alt><Super>Up");
        assert_eq!(accel("Ctrl+Alt+Backspace"), "<Control><Alt>BackSpace");
    }

    #[test]
    fn rejects_invalid_shortcuts() {
        assert!("Ctrl+Hyper+Left".parse::<Shortcut>().is_err());
        assert!("Ctrl+Ctrl+Left".parse::<Shortcut>().is_err());
        assert!("Ctrl+".parse::<Shortcut>().is_err());
        assert!("Ctrl+Nope".parse::<Shortcut>().is_err());
    }
}
