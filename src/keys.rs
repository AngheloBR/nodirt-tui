//! Nombres de tecla → keysym de X11. Puro: tabla fija, sin libxkbcommon.
//!
//! El backend traduce cada keycode a su keysym (columna 0, sin modificadores)
//! y la máquina de `unlock` compara keysyms. Por eso las letras se guardan en
//! minúscula: es lo que da la columna 0 del mapa de teclado.

use std::fmt;

/// Keysym de X11 (`/usr/include/X11/keysymdef.h`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Key(pub u32);

impl Key {
    pub const ESCAPE: Key = Key(0xff1b);
}

/// Teclas con nombre (no letras ni dígitos). Comparación sin mayúsculas.
const NAMED: &[(&str, u32)] = &[
    ("escape", 0xff1b),
    ("esc", 0xff1b),
    ("return", 0xff0d),
    ("enter", 0xff0d),
    ("space", 0x0020),
    ("tab", 0xff09),
    ("backspace", 0xff08),
    ("delete", 0xffff),
    ("insert", 0xff63),
    ("home", 0xff50),
    ("end", 0xff57),
    ("prior", 0xff55),
    ("pageup", 0xff55),
    ("next", 0xff56),
    ("pagedown", 0xff56),
    ("pause", 0xff13),
    ("scroll_lock", 0xff14),
    ("print", 0xff61),
    ("left", 0xff51),
    ("up", 0xff52),
    ("right", 0xff53),
    ("down", 0xff54),
];

const F1: u32 = 0xffbe;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("tecla desconocida {0:?} · usa Escape, F1…F12, space, Return, a…z, 0…9")]
pub struct UnknownKey(pub String);

/// Usado por clap en `--unlock-key`.
pub fn parse_key(name: &str) -> Result<Key, UnknownKey> {
    let n = name.trim();
    let lower = n.to_ascii_lowercase();
    if let Some(&(_, sym)) = NAMED.iter().find(|(k, _)| *k == lower) {
        return Ok(Key(sym));
    }
    // F1..F12
    if let Some(num) = lower.strip_prefix('f')
        && let Ok(i) = num.parse::<u32>()
        && (1..=12).contains(&i)
        && !num.starts_with('0')
    {
        return Ok(Key(F1 + i - 1));
    }
    // Una sola letra o dígito ASCII: su keysym es el código ASCII (letra en minúscula).
    let mut chars = lower.chars();
    if let (Some(c), None) = (chars.next(), chars.next())
        && (c.is_ascii_lowercase() || c.is_ascii_digit())
    {
        return Ok(Key(u32::from(c)));
    }
    Err(UnknownKey(name.to_string()))
}

/// Nombre canónico para mostrar en la TUI/CLI.
pub fn name(k: Key) -> String {
    match k.0 {
        0xff1b => "Escape".into(),
        0xff0d => "Return".into(),
        0x0020 => "space".into(),
        s if (F1..F1 + 12).contains(&s) => format!("F{}", s - F1 + 1),
        s if (0x30..=0x39).contains(&s) || (0x61..=0x7a).contains(&s) => {
            char::from_u32(s).map_or_else(|| format!("{s:#x}"), |c| c.to_string())
        }
        s => NAMED
            .iter()
            .find(|(_, v)| *v == s)
            .map_or_else(|| format!("{s:#x}"), |(n, _)| (*n).to_string()),
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&name(*self))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nombres_validos() {
        assert_eq!(parse_key("Escape"), Ok(Key::ESCAPE));
        assert_eq!(parse_key("esc"), Ok(Key::ESCAPE));
        assert_eq!(parse_key("ESCAPE"), Ok(Key::ESCAPE));
        assert_eq!(parse_key("F1"), Ok(Key(0xffbe)));
        assert_eq!(parse_key("f12"), Ok(Key(0xffc9)));
        assert_eq!(parse_key("space"), Ok(Key(0x20)));
        assert_eq!(parse_key("Return"), Ok(Key(0xff0d)));
        assert_eq!(parse_key("a"), Ok(Key(0x61)));
        assert_eq!(parse_key("A"), Ok(Key(0x61))); // columna 0 = minúscula
        assert_eq!(parse_key("7"), Ok(Key(0x37)));
    }

    #[test]
    fn nombres_invalidos() {
        for n in ["", "F0", "F13", "F01", "ab", "ñ", "ctrl", "Escape2", "é"] {
            assert!(parse_key(n).is_err(), "debió fallar con {n:?}");
        }
    }

    #[test]
    fn ida_y_vuelta() {
        for n in ["Escape", "F5", "space", "Return", "q", "3", "pause"] {
            let k = parse_key(n).unwrap();
            assert_eq!(parse_key(&name(k)), Ok(k), "{n:?}");
        }
        assert_eq!(Key::ESCAPE.to_string(), "Escape");
    }
}
