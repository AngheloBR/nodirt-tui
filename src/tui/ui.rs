//! Dibujo. Solo lee estado: `App` (configuración) y `View` (fase actual).

use std::time::Duration;

use ratatui::{
    Frame,
    layout::{Constraint, Flex, Layout, Rect},
    text::{Line, Span},
    widgets::{Block, BorderType, Paragraph, Wrap},
};

use super::{
    app::{App, Level},
    theme::{self, DIM, ERR, FAINT, KEY_BG, MUTED, OK, TEXT, WARN, bold, fg, pill},
};
use crate::{duration, lock::Phase, timer::PRESETS};

const CARD_W: u16 = 70;
const CARD_H: u16 = 15;
const BAR: usize = 40;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum View {
    /// Pantalla de configuración.
    Setup,
    /// Cuenta atrás o bloqueo activo, tal como lo reporta `lock::run`.
    Run(Phase),
}

pub fn draw(f: &mut Frame, app: &App, view: View) {
    let area = card(f.area());
    let title = Line::from(vec![
        Span::styled(" ", fg(TEXT)),
        Span::styled(env!("CARGO_BIN_NAME"), bold(theme::accent(0.0))),
        Span::styled(" · limpieza segura ", fg(MUTED)),
    ]);
    let hot = matches!(view, View::Run(Phase::Locked { .. }));
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(fg(if hot { WARN } else { FAINT }))
        .title(title);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let lines = match view {
        View::Setup => setup(app),
        View::Run(Phase::Countdown { secs_left }) => countdown(secs_left),
        View::Run(Phase::Locked {
            remaining,
            total,
            elapsed,
            hold,
        }) => locked(app, remaining, total, elapsed, hold),
    };
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

/// Tarjeta centrada; en terminales pequeñas ocupa lo que haya.
fn card(area: Rect) -> Rect {
    let [v] = Layout::vertical([Constraint::Length(CARD_H.min(area.height))])
        .flex(Flex::Center)
        .areas(area);
    let [h] = Layout::horizontal([Constraint::Length(CARD_W.min(area.width))])
        .flex(Flex::Center)
        .areas(v);
    h
}

fn key(k: &str) -> Span<'static> {
    Span::styled(format!(" {k} "), pill(TEXT, KEY_BG))
}

fn label(s: &str) -> Span<'static> {
    Span::styled(format!("  {s:<10}"), fg(DIM))
}

fn bar(ratio: f64, width: usize, color: ratatui::style::Color) -> Vec<Span<'static>> {
    // Redondeo hacia abajo: solo se llena del todo al 100 %.
    let filled = ((ratio.clamp(0.0, 1.0) * width as f64) as usize).min(width);
    vec![
        Span::styled("█".repeat(filled), fg(color)),
        Span::styled("░".repeat(width - filled), fg(FAINT)),
    ]
}

fn setup(app: &App) -> Vec<Line<'static>> {
    let mut lines = vec![Line::default()];

    if let Some(input) = &app.input {
        lines.push(Line::from(vec![
            label("Duración"),
            Span::styled(format!(" {}▏", input.buf), pill(TEXT, KEY_BG)),
            Span::styled("  ⏎ aplicar · Esc cancelar", fg(DIM)),
        ]));
        lines.push(match &input.error {
            Some(e) => Line::from(Span::styled(format!("  {:<10}✗ {e}", ""), fg(ERR))),
            None => Line::from(Span::styled(
                format!("  {:<10}30s, 1m, 2m30s… (máx. 60m)", ""),
                fg(DIM),
            )),
        });
    } else {
        let sel = app.selected();
        let mut row = vec![label("Duración")];
        for (i, p) in PRESETS.iter().enumerate() {
            let txt = duration::fmt_short(*p);
            row.push(if i == sel {
                Span::styled(format!(" {txt} "), pill(theme::DARK, theme::accent(0.5)))
            } else {
                Span::styled(format!(" {txt} "), fg(MUTED))
            });
            row.push(Span::raw(" "));
        }
        let other = if sel == PRESETS.len() {
            Span::styled(
                format!(" {} ", duration::fmt_short(app.duration())),
                pill(theme::DARK, theme::accent(0.5)),
            )
        } else {
            Span::styled(" otro… ", fg(MUTED))
        };
        row.push(other);
        lines.push(Line::from(row));
        lines.push(Line::default());
    }

    let mark = |on: bool| {
        if on {
            Span::styled("● ", fg(OK))
        } else {
            Span::styled("○ ", fg(DIM))
        }
    };
    lines.push(Line::from(vec![
        label("Bloquear"),
        mark(true),
        Span::styled("teclado   ", fg(TEXT)),
        mark(app.mouse),
        Span::styled("mouse y touchpad", fg(if app.mouse { TEXT } else { DIM })),
    ]));
    lines.push(Line::from(vec![
        label("Salir"),
        Span::styled("mantén ", fg(TEXT)),
        Span::styled(app.unlock_key.to_string(), bold(theme::accent(1.0))),
        Span::styled(
            format!(
                " {} · o espera al final",
                duration::fmt_short(app.unlock_hold)
            ),
            fg(TEXT),
        ),
    ]));
    lines.push(Line::from(vec![
        label("Antes"),
        Span::styled("3 s de cuenta atrás para alejarte (cancelable)", fg(MUTED)),
    ]));
    lines.push(Line::default());
    lines.push(Line::from(vec![
        Span::raw("  "),
        key("⏎"),
        Span::styled(" empezar  ", fg(MUTED)),
        key("←→"),
        Span::styled(" duración  ", fg(MUTED)),
        key("t"),
        Span::styled(" escribir  ", fg(MUTED)),
        key("m"),
        Span::styled(" mouse  ", fg(MUTED)),
        key("q"),
        Span::styled(" salir", fg(MUTED)),
    ]));
    lines.push(Line::default());
    lines.push(Line::from(Span::styled(
        "  emergencia: Ctrl+Alt+F1…F12 (cambiar de VT) nunca se bloquea",
        fg(DIM),
    )));
    if let Some((msg, lvl)) = &app.message {
        let c = match lvl {
            Level::Ok => OK,
            Level::Warn => WARN,
            Level::Err => ERR,
        };
        lines.push(Line::from(Span::styled(format!("  {msg}"), fg(c))));
    }
    lines
}

fn countdown(secs_left: u64) -> Vec<Line<'static>> {
    vec![
        Line::default(),
        Line::default(),
        Line::from(Span::styled("Bloqueando en", fg(MUTED))).centered(),
        Line::default(),
        Line::from(Span::styled(format!("{secs_left}"), bold(WARN))).centered(),
        Line::default(),
        Line::from(Span::styled("aléjate del teclado", fg(TEXT))).centered(),
        Line::from(Span::styled("pulsa cualquier tecla para cancelar", fg(DIM))).centered(),
    ]
}

fn locked(
    app: &App,
    remaining: Duration,
    total: Duration,
    elapsed: f64,
    hold: f64,
) -> Vec<Line<'static>> {
    let what = if app.mouse {
        "teclado y mouse capturados"
    } else {
        "teclado capturado · mouse libre"
    };
    let mut hold_line = vec![Span::raw("  ")];
    hold_line.extend(bar(hold, BAR, theme::accent(1.0)));
    let mut time_line = vec![Span::raw("  ")];
    time_line.extend(bar(elapsed, BAR, theme::accent(0.0)));
    time_line.push(Span::styled(
        format!("  de {}", duration::fmt_short(total)),
        fg(DIM),
    ));
    vec![
        Line::default(),
        Line::from(vec![
            Span::styled("  ◉ BLOQUEADO", bold(WARN)),
            Span::styled(" · limpia tranquilo", fg(TEXT)),
        ]),
        Line::default(),
        Line::from(vec![
            Span::styled("  restante  ", fg(DIM)),
            Span::styled(duration::fmt_mmss(remaining), bold(TEXT)),
        ]),
        Line::from(time_line),
        Line::default(),
        Line::from(vec![
            Span::styled("  mantén ", fg(DIM)),
            Span::styled(app.unlock_key.to_string(), bold(theme::accent(1.0))),
            Span::styled(
                format!(" {} para salir antes", duration::fmt_short(app.unlock_hold)),
                fg(DIM),
            ),
        ]),
        Line::from(hold_line),
        Line::default(),
        Line::from(Span::styled(
            format!("  {what} · Ctrl+Alt+Fn sigue disponible"),
            fg(DIM),
        )),
    ]
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};

    use super::*;
    use crate::keys::Key;

    fn render(app: &App, view: View, w: u16, h: u16) -> String {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| draw(f, app, view)).unwrap();
        let buf = t.backend().buffer().clone();
        (0..h)
            .map(|y| {
                (0..w)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn app() -> App {
        App::new(None, true, Key::ESCAPE, Duration::from_secs(3))
    }

    const LOCKED: View = View::Run(Phase::Locked {
        remaining: Duration::from_secs(42),
        total: Duration::from_secs(60),
        elapsed: 0.3,
        hold: 0.5,
    });
    const COUNTDOWN: View = View::Run(Phase::Countdown { secs_left: 3 });

    #[test]
    fn pantalla_de_configuracion() {
        let s = render(&app(), View::Setup, 80, 24);
        for needle in [
            "Duración",
            "30s",
            "10m",
            "otro…",
            "mouse y touchpad",
            "Escape",
            "Ctrl+Alt+F1",
        ] {
            assert!(s.contains(needle), "falta {needle:?}\n{s}");
        }
    }

    #[test]
    fn entrada_libre_y_error() {
        let mut a = app();
        a.input = Some(super::super::app::Input {
            buf: "90".into(),
            error: Some("falta la unidad".into()),
        });
        let s = render(&a, View::Setup, 80, 24);
        assert!(s.contains("90▏") && s.contains("✗ falta la unidad"), "{s}");
    }

    #[test]
    fn cuenta_atras() {
        let s = render(&app(), COUNTDOWN, 80, 24);
        assert!(
            s.contains("Bloqueando en") && s.contains('3') && s.contains("cancelar"),
            "{s}"
        );
    }

    #[test]
    fn bloqueado() {
        let s = render(&app(), LOCKED, 80, 24);
        for needle in ["BLOQUEADO", "00:42", "de 1m", "mantén Escape 3s", "█", "░"] {
            assert!(s.contains(needle), "falta {needle:?}\n{s}");
        }
        let mut a = app();
        a.mouse = false;
        assert!(render(&a, LOCKED, 80, 24).contains("mouse libre"));
    }

    #[test]
    fn barra() {
        let txt = |r| {
            bar(r, 10, OK)
                .iter()
                .map(|s| s.content.to_string())
                .collect::<String>()
        };
        assert_eq!(txt(0.0), "░░░░░░░░░░");
        assert_eq!(txt(0.55), "█████░░░░░");
        assert_eq!(txt(0.99), "█████████░"); // no se llena antes de tiempo
        assert_eq!(txt(1.0), "██████████");
        assert_eq!(txt(7.0), "██████████");
    }

    #[test]
    fn no_entra_en_panico_en_ningun_tamano() {
        let mut a = app();
        a.set_message("✓ listo", Level::Ok);
        for (w, h) in [(1, 1), (10, 3), (30, 8), (66, 15), (200, 60)] {
            for v in [View::Setup, COUNTDOWN, LOCKED] {
                render(&a, v, w, h);
            }
        }
    }

    /// `cargo test vista_previa -- --ignored --nocapture` imprime las pantallas.
    #[test]
    #[ignore]
    fn vista_previa() {
        let mut a = app();
        a.set_message("✓ tiempo cumplido · teclado y mouse liberados", Level::Ok);
        for v in [View::Setup, COUNTDOWN, LOCKED] {
            println!("{}\n", render(&a, v, 74, 16));
        }
    }
}
