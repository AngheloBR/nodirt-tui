//! Parser y formateo de duraciones. Puro: sin E/S ni reloj.
//!
//! Formatos válidos: `30s`, `1m`, `2m30s`, `1m 30s`, `1h`. Unidades `h`, `m`, `s`,
//! cada una como máximo una vez y en ese orden. Solo enteros. La unidad es obligatoria:
//! `90` a secas es un error (¿segundos o minutos?).

use std::time::Duration;

/// Tope máximo de un bloqueo.
pub const MAX_LOCK: Duration = Duration::from_secs(60 * 60);
/// Mínimo de un bloqueo.
pub const MIN_LOCK: Duration = Duration::from_secs(1);
/// Rango permitido para `--unlock-hold`.
pub const MIN_HOLD: Duration = Duration::from_secs(1);
pub const MAX_HOLD: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    #[error("duración vacía · usa 30s, 1m, 2m30s")]
    Empty,
    #[error("falta la unidad en {0:?} · usa 90s o 1m30s")]
    MissingUnit(String),
    #[error("no entiendo {0:?} · usa 30s, 1m, 2m30s")]
    Invalid(String),
    #[error("la duración debe ser mayor que 0")]
    Zero,
    #[error("{text:?} es demasiado corto · mínimo {}", fmt_short(*.min))]
    TooShort { text: String, min: Duration },
    #[error("{text:?} es demasiado largo · máximo {}", fmt_short(*.max))]
    TooLong { text: String, max: Duration },
}

/// Convierte el texto en una duración, sin aplicar rango (salvo > 0).
pub fn parse(text: &str) -> Result<Duration, ParseError> {
    let t = text.trim().to_ascii_lowercase();
    if t.is_empty() {
        return Err(ParseError::Empty);
    }
    let invalid = || ParseError::Invalid(text.to_string());

    // Rango de cada unidad: h=0, m=1, s=2. Exige orden estricto y sin repetir.
    let mut last_rank: Option<u8> = None;
    let mut total: u64 = 0;
    let mut num = String::new();
    for c in t.chars() {
        match c {
            '0'..='9' => {
                // 6 dígitos sobran (999999s ≈ 11 días) y evitan desbordes.
                if num.len() >= 6 {
                    return Err(ParseError::TooLong {
                        text: text.to_string(),
                        max: MAX_LOCK,
                    });
                }
                num.push(c);
            }
            'h' | 'm' | 's' => {
                if num.is_empty() {
                    return Err(invalid());
                }
                let (rank, mult) = match c {
                    'h' => (0, 3600),
                    'm' => (1, 60),
                    _ => (2, 1),
                };
                if last_rank.is_some_and(|r| rank <= r) {
                    return Err(invalid());
                }
                last_rank = Some(rank);
                let v: u64 = num.parse().map_err(|_| invalid())?;
                total += v * mult;
                num.clear();
            }
            ' ' => {
                // Espacio solo entre componentes completos ("1m 30s"), no "1 m".
                if !num.is_empty() {
                    return Err(invalid());
                }
            }
            _ => return Err(invalid()),
        }
    }
    if !num.is_empty() {
        return Err(if last_rank.is_none() {
            ParseError::MissingUnit(text.to_string())
        } else {
            invalid()
        });
    }
    if total == 0 {
        return Err(ParseError::Zero);
    }
    Ok(Duration::from_secs(total))
}

/// `parse` + comprobación de rango `[min, max]`.
pub fn parse_in(text: &str, min: Duration, max: Duration) -> Result<Duration, ParseError> {
    let d = parse(text)?;
    if d < min {
        return Err(ParseError::TooShort {
            text: text.to_string(),
            min,
        });
    }
    if d > max {
        return Err(ParseError::TooLong {
            text: text.to_string(),
            max,
        });
    }
    Ok(d)
}

/// Duración de un bloqueo (1s..=60m). Usado por clap en `--time`.
pub fn parse_lock(text: &str) -> Result<Duration, ParseError> {
    parse_in(text, MIN_LOCK, MAX_LOCK)
}

/// Tiempo a mantener la tecla (1s..=10s). Usado por clap en `--unlock-hold`.
pub fn parse_hold(text: &str) -> Result<Duration, ParseError> {
    parse_in(text, MIN_HOLD, MAX_HOLD)
}

/// `MM:SS` (los bloqueos nunca pasan de 60 min).
pub fn fmt_mmss(d: Duration) -> String {
    let s = d.as_secs();
    format!("{:02}:{:02}", s / 60, s % 60)
}

/// `1h`, `2m 30s`, `45s`.
pub fn fmt_short(d: Duration) -> String {
    let s = d.as_secs();
    let (h, m, sec) = (s / 3600, (s % 3600) / 60, s % 60);
    let mut out = Vec::new();
    if h > 0 {
        out.push(format!("{h}h"));
    }
    if m > 0 {
        out.push(format!("{m}m"));
    }
    if sec > 0 || out.is_empty() {
        out.push(format!("{sec}s"));
    }
    out.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(t: &str) -> u64 {
        parse(t).unwrap().as_secs()
    }

    #[test]
    fn formatos_validos() {
        assert_eq!(secs("30s"), 30);
        assert_eq!(secs("1m"), 60);
        assert_eq!(secs("2m30s"), 150);
        assert_eq!(secs("90s"), 90);
        assert_eq!(secs("1m 30s"), 90);
        assert_eq!(secs("  2M30S  "), 150);
        assert_eq!(secs("1h"), 3600);
        assert_eq!(secs("0m5s"), 5);
    }

    #[test]
    fn numero_sin_unidad_es_error() {
        assert_eq!(parse("90"), Err(ParseError::MissingUnit("90".into())));
        assert_eq!(parse("2m30"), Err(ParseError::Invalid("2m30".into())));
    }

    #[test]
    fn invalidos() {
        for t in [
            "abc", "1.5m", "-5s", "+5s", "m", "s30", "1d", "30s1m", "1m1m", "1 m", "1e3s", "nan",
            "inf", "5ms", "½m",
        ] {
            assert!(parse(t).is_err(), "debió fallar con {t:?}");
        }
        assert_eq!(parse(""), Err(ParseError::Empty));
        assert_eq!(parse("   "), Err(ParseError::Empty));
        assert_eq!(parse("0s"), Err(ParseError::Zero));
        assert_eq!(parse("0m0s"), Err(ParseError::Zero));
    }

    #[test]
    fn sin_desbordes() {
        for t in ["99999999999999999999s", "9999999h", "1234567m"] {
            assert!(matches!(parse(t), Err(ParseError::TooLong { .. })), "{t:?}");
        }
    }

    #[test]
    fn tope_de_bloqueo() {
        assert_eq!(parse_lock("60m").unwrap(), MAX_LOCK);
        assert_eq!(parse_lock("1h").unwrap(), MAX_LOCK);
        assert_eq!(parse_lock("59m59s").unwrap().as_secs(), 3599);
        assert!(matches!(
            parse_lock("60m1s"),
            Err(ParseError::TooLong { .. })
        ));
        assert!(matches!(parse_lock("2h"), Err(ParseError::TooLong { .. })));
        assert_eq!(parse_lock("1s").unwrap().as_secs(), 1);
    }

    #[test]
    fn rango_de_hold() {
        assert_eq!(parse_hold("3s").unwrap().as_secs(), 3);
        assert_eq!(parse_hold("10s").unwrap().as_secs(), 10);
        assert!(matches!(parse_hold("11s"), Err(ParseError::TooLong { .. })));
        assert!(matches!(parse_hold("1m"), Err(ParseError::TooLong { .. })));
        assert_eq!(parse_hold("0s"), Err(ParseError::Zero));
    }

    #[test]
    fn mensajes_de_error() {
        assert_eq!(
            parse_lock("2h").unwrap_err().to_string(),
            "\"2h\" es demasiado largo · máximo 1h"
        );
        assert_eq!(
            parse("90").unwrap_err().to_string(),
            "falta la unidad en \"90\" · usa 90s o 1m30s"
        );
    }

    #[test]
    fn formateo() {
        assert_eq!(fmt_mmss(Duration::from_secs(150)), "02:30");
        assert_eq!(fmt_mmss(Duration::from_secs(3600)), "60:00");
        assert_eq!(fmt_mmss(Duration::ZERO), "00:00");
        assert_eq!(fmt_short(Duration::from_secs(150)), "2m 30s");
        assert_eq!(fmt_short(Duration::from_secs(45)), "45s");
        assert_eq!(fmt_short(Duration::from_secs(3600)), "1h");
        assert_eq!(fmt_short(Duration::ZERO), "0s");
    }
}
