//! Dibujo. Solo lee estado: `App` (configuración) y `View` (fase actual).
//! Estructura de nosleep-tui: cabecera · "ahora" con reloj grande · duración y
//! bloqueo · seguridad · pie de atajos, más modales de ayuda y duración.

use std::time::Duration;

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Margin, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Clear, Paragraph},
};

use super::{
    app::{App, Choice, Level},
    digits::{self, Scale},
    theme::{
        BORDER, DARK, DIM, ERR, FAINT, KEY_BG, MUTED, OK, OK_DIM, TEXT, WARN, accent, bold, fg,
        pill,
    },
};
use crate::{
    duration,
    lock::{Phase, RELEASE_WAIT},
    timer::{PRESETS, START_DELAY},
};

pub const MIN_W: u16 = 60;
pub const MIN_H: u16 = 18;
const MAX_W: u16 = 140;
/// Desde aquí el reloj se dibuja al doble de tamaño.
const BIG_W: u16 = 120;
const BIG_H: u16 = 34;
/// Color del reloj en reposo.
const IDLE_CLOCK: Color = Color::Rgb(71, 85, 105);

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum View {
    /// Pantalla de configuración.
    Setup,
    /// Cuenta atrás o bloqueo activo, tal como lo reporta `lock::run`.
    Run(Phase),
}

impl View {
    fn locked(self) -> bool {
        matches!(
            self,
            View::Run(Phase::Locked { .. } | Phase::Releasing { .. })
        )
    }
}

pub fn draw(f: &mut Frame, app: &App, view: View) {
    let full = f.area();
    if full.width < MIN_W || full.height < MIN_H {
        too_small(f, full, view);
        return;
    }
    // En terminales muy anchas el contenido se centra en vez de estirarse.
    let w = full.width.min(MAX_W);
    let area = Rect::new(full.x + (full.width - w) / 2, full.y, w, full.height);

    let big = area.width >= BIG_W && area.height >= BIG_H;
    let hero_h = if big {
        16
    } else if area.height >= 24 {
        10
    } else {
        7
    };
    let [header, _, hero, mid, safety, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(u16::from(area.height >= 26)),
        Constraint::Length(hero_h),
        Constraint::Length(7),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(area);

    draw_header(f, header, app, view);
    draw_hero(f, hero, app, view, big);
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(54), Constraint::Percentage(46)]).areas(mid);
    draw_duration(f, left, app, view);
    draw_options(f, right, app, view);
    draw_safety(f, safety, view);
    draw_footer(f, footer, app, view);

    if view == View::Setup {
        if app.help {
            draw_help(f, area);
        } else if let Some(input) = &app.input {
            draw_input(f, area, app, &input.buf);
        }
    }
}

// ───────────────────────────── piezas ─────────────────────────────

fn panel<'a>(title: &'a str, hot: bool) -> Block<'a> {
    let border = if hot { accent(0.5) } else { BORDER };
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(fg(border))
        .title(Line::from(vec![
            Span::styled(" ", fg(border)),
            Span::styled(title, bold(if hot { TEXT } else { MUTED })),
            Span::styled(" ", fg(border)),
        ]))
}

fn pulse(tick: u64) -> bool {
    (tick / 5).is_multiple_of(2)
}

fn gradient(text: &str) -> Vec<Span<'static>> {
    let n = text.chars().count().max(2) - 1;
    text.chars()
        .enumerate()
        .map(|(i, c)| Span::styled(c.to_string(), bold(accent(i as f64 / n as f64))))
        .collect()
}

fn key(k: &str) -> Span<'static> {
    Span::styled(format!(" {k} "), Style::new().fg(TEXT).bg(KEY_BG))
}

fn width(spans: &[Span]) -> usize {
    spans.iter().map(Span::width).sum()
}

/// Barra `━` de `w` columnas: degradado de marca hasta `frac`, el resto tenue.
fn bar_spans(frac: f64, w: usize, solid: Option<Color>) -> Vec<Span<'static>> {
    // Redondeo hacia abajo: solo se llena del todo al 100 %.
    let filled = ((frac.clamp(0.0, 1.0) * w as f64) as usize).min(w);
    (0..w)
        .map(|i| {
            let c = if i < filled {
                solid.unwrap_or_else(|| accent(i as f64 / w.max(1) as f64))
            } else {
                FAINT
            };
            Span::styled("━", fg(c))
        })
        .collect()
}

fn too_small(f: &mut Frame, area: Rect, view: View) {
    let [_, mid, _] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(3),
        Constraint::Fill(1),
    ])
    .areas(area);
    // Aun en miniatura, con el bloqueo activo se ve lo esencial.
    let first = match view {
        View::Run(Phase::Locked { remaining, .. }) => Line::styled(
            format!("◉ bloqueado · {}", duration::fmt_mmss(remaining)),
            bold(WARN),
        ),
        View::Run(Phase::Countdown { secs_left }) => {
            Line::styled(format!("bloqueando en {secs_left}"), bold(WARN))
        }
        View::Run(Phase::Releasing { .. }) => Line::styled("✓ suelta las teclas", bold(OK)),
        View::Setup => Line::styled("terminal demasiado pequeña", bold(WARN)),
    };
    let p = Paragraph::new(vec![
        first,
        Line::styled(
            format!("{}×{} · necesito {MIN_W}×{MIN_H}", area.width, area.height),
            fg(DIM),
        ),
    ])
    .alignment(Alignment::Center);
    f.render_widget(p, mid);
}

// ───────────────────────────── cabecera ─────────────────────────────

fn draw_header(f: &mut Frame, area: Rect, app: &App, view: View) {
    let area = area.inner(Margin::new(1, 0));
    let dot = match view {
        View::Setup => DIM,
        View::Run(Phase::Countdown { .. }) => WARN,
        _ if pulse(app.tick) => accent(0.0),
        _ => accent(0.6),
    };
    let mut left = vec![Span::styled("◉ ", fg(dot))];
    left.extend(gradient(env!("CARGO_BIN_NAME")));
    left.push(Span::styled(
        format!("  v{}", env!("CARGO_PKG_VERSION")),
        fg(FAINT),
    ));

    let locked = view.locked();
    let right = vec![
        Span::styled("● ", fg(if locked { accent(0.0) } else { DIM })),
        Span::styled("X11", fg(MUTED)),
        Span::styled("  ·  ", fg(FAINT)),
        Span::styled(
            if app.mouse {
                "teclado + mouse"
            } else {
                "solo teclado"
            },
            fg(MUTED),
        ),
    ];

    let free = usize::from(area.width).saturating_sub(width(&left) + width(&right) + 4);
    let toast = (view == View::Setup)
        .then_some(app.message.as_ref())
        .flatten()
        .map(|(msg, lvl)| {
            let c = match lvl {
                Level::Ok => OK,
                Level::Warn => WARN,
                Level::Err => ERR,
            };
            let mut s = msg.clone();
            if s.chars().count() > free {
                s = s.chars().take(free.saturating_sub(1)).collect::<String>() + "…";
            }
            Line::styled(s, fg(c))
        });

    f.render_widget(Paragraph::new(Line::from(left.clone())), area);
    f.render_widget(
        Paragraph::new(Line::from(right.clone())).alignment(Alignment::Right),
        area,
    );
    if let Some(t) = toast {
        // Centrado en el hueco entre ambos lados, nunca encima de ellos.
        let lw = u16::try_from(width(&left) + 2).unwrap_or(u16::MAX);
        let rw = u16::try_from(width(&right) + 2).unwrap_or(u16::MAX);
        let gap = Rect::new(
            area.x.saturating_add(lw),
            area.y,
            area.width.saturating_sub(lw.saturating_add(rw)),
            1,
        );
        f.render_widget(Paragraph::new(t).alignment(Alignment::Center), gap);
    }
}

// ───────────────────────────── ahora ─────────────────────────────

fn draw_hero(f: &mut Frame, area: Rect, app: &App, view: View, big: bool) {
    let block = panel("ahora", view.locked());
    let inner = block.inner(area).inner(Margin::new(2, 0));
    f.render_widget(block, area);

    let clock = match view {
        View::Setup => duration::fmt_mmss(app.duration()),
        View::Run(Phase::Countdown { secs_left }) => format!("00:{secs_left:02}"),
        View::Run(Phase::Locked { remaining, .. }) => duration::fmt_mmss(remaining),
        View::Run(Phase::Releasing { .. }) => "00:00".into(),
    };
    let urgent = matches!(view, View::Run(Phase::Locked { remaining, .. }) if remaining < Duration::from_secs(10));
    let info = info_lines(app, view);

    if inner.height >= 8 {
        let want = if big { Scale::BIG } else { Scale::WIDE };
        let rows_h = u16::try_from(want.rows()).unwrap_or(5);
        let [_, top, _, bar] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(rows_h),
            Constraint::Fill(1),
            Constraint::Length(1),
        ])
        .areas(inner);
        let [dig, _, side] = Layout::horizontal([
            Constraint::Fill(1),
            Constraint::Length(2),
            Constraint::Length(28),
        ])
        .areas(top);
        let scale = [want, Scale::WIDE, Scale::SMALL]
            .into_iter()
            .find(|s| digits::width(&clock, *s) <= usize::from(dig.width))
            .unwrap_or(Scale::SMALL);
        let rows = digits::render(&clock, scale);
        let last = rows.len().saturating_sub(1).max(1) as f64;
        let lines: Vec<Line> = rows
            .into_iter()
            .enumerate()
            .map(|(i, r)| {
                let c = match view {
                    View::Setup => IDLE_CLOCK,
                    View::Run(Phase::Countdown { .. }) => WARN,
                    View::Run(Phase::Releasing { .. }) => OK,
                    _ if urgent => WARN,
                    _ => accent(i as f64 / last),
                };
                Line::styled(r, fg(c))
            })
            .collect();
        // La columna de estado se centra verticalmente junto al reloj.
        let pad = top.height.saturating_sub(5) / 2;
        let side = Rect::new(side.x, side.y + pad, side.width, side.height - pad);
        f.render_widget(Paragraph::new(lines), dig);
        f.render_widget(Paragraph::new(info), side);
        draw_bar(f, bar, view);
    } else {
        // Versión compacta para terminales bajas.
        let [_, line1, line2, bar] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(inner);
        let mut first = vec![
            Span::styled(clock, bold(if view.locked() { accent(0.2) } else { DIM })),
            Span::raw("   "),
        ];
        first.extend(info.first().cloned().unwrap_or_default().spans);
        f.render_widget(Paragraph::new(Line::from(first)), line1);
        if let Some(l) = info.get(2) {
            f.render_widget(Paragraph::new(l.clone()), line2);
        }
        draw_bar(f, bar, view);
    }
}

fn info_lines(app: &App, view: View) -> Vec<Line<'static>> {
    let k = app.unlock_key.to_string();
    let hold = duration::fmt_short(app.unlock_hold);
    match view {
        View::Setup => vec![
            Line::from(Span::styled(" ○ LISTO ", pill(MUTED, KEY_BG))),
            Line::default(),
            Line::from(vec![
                Span::styled("bloquea  ", fg(DIM)),
                Span::styled(
                    if app.mouse {
                        "teclado + mouse"
                    } else {
                        "solo teclado"
                    },
                    fg(TEXT),
                ),
            ]),
            Line::from(vec![
                Span::styled("salida   ", fg(DIM)),
                Span::styled(format!("{k} {hold}"), bold(accent(0.4))),
            ]),
            Line::from(vec![
                Span::styled("antes    ", fg(DIM)),
                Span::styled(
                    format!("{} s para alejarte", START_DELAY.as_secs()),
                    fg(MUTED),
                ),
            ]),
        ],
        View::Run(Phase::Countdown { .. }) => vec![
            Line::from(Span::styled(" ◔ PREPARANDO ", pill(DARK, WARN))),
            Line::default(),
            Line::styled("aléjate del teclado", bold(TEXT)),
            Line::styled("cualquier tecla cancela", fg(MUTED)),
        ],
        View::Run(Phase::Locked { hold: h, .. }) => {
            let dot = if pulse(app.tick) { "●" } else { "○" };
            let mut holdbar = vec![Span::styled(format!("{k:<8} "), bold(accent(0.4)))];
            holdbar.extend(bar_spans(h, 12, Some(accent(0.8))));
            holdbar.push(Span::styled(
                format!(" {:>3}%", (h * 100.0).floor() as u32),
                fg(MUTED),
            ));
            vec![
                Line::from(Span::styled(
                    format!(" {dot} BLOQUEADO "),
                    pill(DARK, accent(0.0)),
                )),
                Line::default(),
                Line::styled("limpia tranquilo", bold(TEXT)),
                Line::styled(format!("salir: mantén {k} {hold}"), fg(DIM)),
                Line::from(holdbar),
            ]
        }
        View::Run(Phase::Releasing { unlocked }) => vec![
            Line::from(Span::styled(" ✓ LIBERANDO ", pill(DARK, OK))),
            Line::default(),
            Line::styled(
                if unlocked {
                    format!("suelta {k}")
                } else {
                    "suelta las teclas".into()
                },
                bold(TEXT),
            ),
            Line::styled(
                format!("así nada se cuela (máx. {} s)", RELEASE_WAIT.as_secs()),
                fg(DIM),
            ),
        ],
    }
}

fn draw_bar(f: &mut Frame, area: Rect, view: View) {
    let w = usize::from(area.width);
    let mut spans = Vec::new();
    let with_label = |frac: f64, solid: Option<Color>, spans: &mut Vec<Span<'static>>| {
        let label = format!(" {:>3}%", (frac * 100.0).floor() as u32);
        spans.extend(bar_spans(frac, w.saturating_sub(label.len()), solid));
        spans.push(Span::styled(label, fg(MUTED)));
    };
    match view {
        View::Setup => {
            let hint = vec![
                Span::raw(" "),
                key("⏎"),
                Span::styled(" bloquear", fg(MUTED)),
            ];
            let bw = w.saturating_sub(width(&hint));
            spans.push(Span::styled("━".repeat(bw), fg(FAINT)));
            spans.extend(hint);
        }
        View::Run(Phase::Countdown { secs_left }) => {
            let total = START_DELAY.as_secs().max(1) as f64;
            with_label(1.0 - secs_left as f64 / total, Some(WARN), &mut spans);
        }
        View::Run(Phase::Locked { elapsed, .. }) => with_label(elapsed, None, &mut spans),
        View::Run(Phase::Releasing { .. }) => with_label(1.0, Some(OK), &mut spans),
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

// ───────────────────────────── paneles ─────────────────────────────

fn draw_duration(f: &mut Frame, area: Rect, app: &App, view: View) {
    let block = panel("duración", false);
    let inner = block.inner(area).inner(Margin::new(2, 0));
    f.render_widget(block, area);
    let editable = view == View::Setup;

    let mut chips = Vec::new();
    let mut keys = Vec::new();
    for (i, p) in PRESETS.iter().enumerate() {
        let sel = app.choice == Choice::Preset(i);
        let text = format!(" {} ", duration::fmt_short(*p));
        let w = text.chars().count();
        chips.push(if sel {
            Span::styled(text, pill(DARK, accent(0.35)))
        } else {
            Span::styled(text, fg(if editable { MUTED } else { FAINT }))
        });
        let num = (i + 1).to_string();
        let pad = w - 1;
        keys.push(Span::styled(
            format!("{}{num}{}", " ".repeat(pad / 2), " ".repeat(pad - pad / 2)),
            fg(if sel { accent(0.35) } else { FAINT }),
        ));
        chips.push(Span::raw(" "));
        keys.push(Span::raw(" "));
    }

    let custom = match app.choice {
        Choice::Custom(d) => Line::from(vec![
            Span::styled(" ✦ ", fg(accent(0.35))),
            Span::styled(
                format!(" {} ", duration::fmt_short(d)),
                pill(DARK, accent(0.35)),
            ),
            Span::styled("  personalizado", fg(DIM)),
        ]),
        Choice::Preset(_) => Line::from(vec![
            key("t"),
            Span::styled(" personalizado (30s, 2m30s…)", fg(DIM)),
        ]),
    };
    let summary = if editable {
        Line::from(vec![
            Span::styled(duration::fmt_short(app.duration()), bold(TEXT)),
            Span::styled(" · máximo 60 min", fg(DIM)),
        ])
    } else {
        Line::styled("fija durante el bloqueo", fg(DIM))
    };
    let lines = vec![
        Line::default(),
        Line::from(chips),
        Line::from(keys),
        custom,
        summary,
    ];
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_options(f: &mut Frame, area: Rect, app: &App, view: View) {
    let block = panel("bloqueo", false);
    let inner = block.inner(area).inner(Margin::new(2, 0));
    f.render_widget(block, area);
    let w = usize::from(inner.width);
    let editable = view == View::Setup;

    let row = |on: bool, label: &str, k: Option<&str>| {
        let switch = if on {
            Span::styled("  ON ", pill(DARK, OK))
        } else {
            Span::styled(" OFF ", Style::new().fg(DIM).bg(KEY_BG))
        };
        let mut spans = vec![
            switch,
            Span::raw("  "),
            Span::styled(label.to_owned(), fg(if on { TEXT } else { MUTED })),
        ];
        let tail = match k {
            Some(k) if editable => key(k),
            Some(_) => Span::raw(""),
            None => Span::styled("siempre", fg(FAINT)),
        };
        let pad = w.saturating_sub(width(&spans) + tail.width() + 1);
        spans.push(Span::raw(" ".repeat(pad)));
        spans.push(tail);
        Line::from(spans)
    };

    let mut exit = vec![
        Span::styled("salida ", fg(DIM)),
        Span::styled(app.unlock_key.to_string(), bold(accent(0.4))),
        Span::styled(
            format!(" · mantener {}", duration::fmt_short(app.unlock_hold)),
            fg(MUTED),
        ),
    ];
    let pad = w.saturating_sub(width(&exit) + 1);
    exit.push(Span::raw(" ".repeat(pad)));

    let lines = vec![
        Line::default(),
        row(true, "Teclado", None),
        row(app.mouse, "Mouse y touchpad", Some("m")),
        Line::default(),
        Line::from(exit),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_safety(f: &mut Frame, area: Rect, view: View) {
    let block = panel("seguridad", false);
    let inner = block.inner(area).inner(Margin::new(2, 0));
    f.render_widget(block, area);
    let item = |icon: &'static str, c: Color, head: &'static str, tail: &'static str| {
        Line::from(vec![
            Span::styled(format!("{icon} "), fg(c)),
            Span::styled(head, fg(TEXT)),
            Span::styled(tail, fg(DIM)),
        ])
    };
    let mut lines = vec![
        Line::default(),
        item(
            "◆",
            accent(0.3),
            "Ctrl+Alt+F1…F12",
            "  salida de emergencia: nunca se bloquea",
        ),
        item(
            "◆",
            accent(0.3),
            "si nodirt muere",
            "  (aun con kill -9) X11 suelta todo al instante",
        ),
        item(
            "◆",
            accent(0.3),
            "al terminar",
            "  espera a que sueltes las teclas",
        ),
        item(
            "!",
            WARN,
            "botón de encendido",
            "  no se bloquea: evítalo al limpiar",
        ),
        item(
            "!",
            WARN,
            "bloqueador de pantalla",
            "  no puede activarse mientras dure",
        ),
    ];
    if view.locked() {
        lines.push(Line::styled(
            "  ● la terminal no recibe teclas: el combo se lee de X11",
            fg(OK_DIM),
        ));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_footer(f: &mut Frame, area: Rect, app: &App, view: View) {
    let area = area.inner(Margin::new(1, 0));
    let k = app.unlock_key.to_string();
    let hold = format!("mantener {}", duration::fmt_short(app.unlock_hold));
    // (prioridad, tecla, texto): si no cabe, se quitan los de menor prioridad.
    let items: Vec<(i32, &str, &str)> = match view {
        View::Setup => vec![
            (0, "⏎", "bloquear"),
            (1, "1-5", "duración"),
            (2, "←→", "elegir"),
            (2, "t", "personalizar"),
            (1, "m", "mouse"),
            (1, "?", "ayuda"),
            (0, "q", "salir"),
        ],
        View::Run(Phase::Countdown { .. }) => vec![(0, "cualquier tecla", "cancelar")],
        View::Run(Phase::Locked { .. }) => vec![
            (0, k.as_str(), hold.as_str()),
            (1, "Ctrl+Alt+F1…F12", "emergencia"),
        ],
        View::Run(Phase::Releasing { .. }) => vec![(0, "↑", "suelta las teclas")],
    };
    let max = usize::from(area.width);
    let cost = |it: &(i32, &str, &str)| it.1.chars().count() + it.2.chars().count() + 5;
    let mut items = items;
    let mut lvl = 3;
    while items.iter().map(cost).sum::<usize>() > max && lvl > 0 {
        items.retain(|it| it.0 < lvl);
        lvl -= 1;
    }
    let mut spans = Vec::new();
    for (_, k, label) in items {
        spans.push(key(k));
        spans.push(Span::styled(format!(" {label}  "), fg(DIM)));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

// ───────────────────────────── modales ─────────────────────────────

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width.saturating_sub(4));
    let h = h.min(area.height.saturating_sub(2));
    Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    )
}

fn modal_block(title: &str) -> Block<'_> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(fg(accent(0.2)))
        .title(Line::from(vec![
            Span::raw(" "),
            Span::styled(title, bold(TEXT)),
            Span::raw(" "),
        ]))
}

fn draw_help(f: &mut Frame, area: Rect) {
    let rows: [(&str, &str); 8] = [
        ("⏎ / espacio", "bloquear (tras 3 s de cuenta atrás)"),
        ("1 … 5", "30s · 1m · 2m · 5m · 10m"),
        ("← →", "elegir duración"),
        ("t", "duración personalizada (máx. 60m)"),
        ("m", "bloquear también mouse y touchpad"),
        ("?", "esta ayuda"),
        ("q / esc", "salir"),
        ("bloqueado", "mantén la tecla de salida para terminar"),
    ];
    let mut lines = vec![Line::default()];
    for (k, d) in rows {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(format!("{k:<13}"), bold(accent(0.4))),
            Span::styled(d, fg(TEXT)),
        ]));
    }
    lines.push(Line::default());
    lines.push(Line::styled("  cualquier tecla para cerrar", fg(DIM)));
    let r = centered(area, 60, 13);
    f.render_widget(Clear, r);
    f.render_widget(Paragraph::new(lines).block(modal_block("atajos")), r);
}

fn draw_input(f: &mut Frame, area: Rect, app: &App, buf: &str) {
    let cursor = if pulse(app.tick) { "▏" } else { " " };
    let preview = if buf.trim().is_empty() {
        Line::styled("ej. 45s · 2m30s · 15m · 1h", fg(DIM))
    } else {
        match duration::parse_lock(buf) {
            Ok(d) => Line::from(vec![
                Span::styled("✓ ", fg(OK)),
                Span::styled(duration::fmt_short(d), bold(TEXT)),
                Span::styled(" de bloqueo", fg(DIM)),
            ]),
            Err(e) => Line::styled(format!("✗ {e}"), fg(ERR)),
        }
    };
    let mut p = vec![Span::raw("    ")];
    p.extend(preview.spans);
    let out = vec![
        Line::default(),
        Line::from(vec![
            Span::styled("  › ", bold(accent(0.3))),
            Span::styled(buf.to_owned(), bold(TEXT)),
            Span::styled(cursor, fg(accent(0.6))),
        ]),
        Line::from(p),
        Line::default(),
        Line::from(vec![
            Span::raw("  "),
            key("⏎"),
            Span::styled(" aplicar   ", fg(MUTED)),
            key("esc"),
            Span::styled(" cancelar", fg(MUTED)),
        ]),
    ];
    let r = centered(area, 58, 7);
    f.render_widget(Clear, r);
    f.render_widget(
        Paragraph::new(out).block(modal_block("duración personalizada")),
        r,
    );
}

#[cfg(test)]
pub(super) mod tests {
    use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};

    use super::*;
    use crate::keys::Key;

    pub fn render_buf(app: &App, view: View, w: u16, h: u16) -> Buffer {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| draw(f, app, view)).unwrap();
        t.backend().buffer().clone()
    }

    fn render(app: &App, view: View, w: u16, h: u16) -> String {
        let buf = render_buf(app, view, w, h);
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

    pub const LOCKED: View = View::Run(Phase::Locked {
        remaining: Duration::from_secs(42),
        total: Duration::from_secs(60),
        elapsed: 0.3,
        hold: 0.5,
    });
    pub const COUNTDOWN: View = View::Run(Phase::Countdown { secs_left: 3 });
    const RELEASING: View = View::Run(Phase::Releasing { unlocked: true });

    #[test]
    fn pantalla_de_configuracion() {
        let s = render(&app(), View::Setup, 92, 28);
        for needle in [
            "nodirt",
            "ahora",
            "duración",
            "30s",
            "10m",
            "bloqueo",
            "Mouse y touchpad",
            "seguridad",
            "Ctrl+Alt+F1…F12",
            "LISTO",
            "bloquear",
        ] {
            assert!(s.contains(needle), "falta {needle:?}\n{s}");
        }
    }

    #[test]
    fn modal_de_duracion_con_vista_previa() {
        let mut a = app();
        a.input = Some(super::super::app::Input {
            buf: "90".into(),
            error: None,
        });
        let s = render(&a, View::Setup, 92, 28);
        assert!(s.contains("duración personalizada"), "{s}");
        assert!(s.contains("✗ falta la unidad"), "{s}");
        a.input.as_mut().unwrap().buf = "2m30s".into();
        assert!(render(&a, View::Setup, 92, 28).contains("✓ 2m 30s"));
    }

    #[test]
    fn el_aviso_no_pisa_la_cabecera() {
        let mut a = app();
        a.set_message("✓ tiempo cumplido · teclado y mouse liberados", Level::Ok);
        for w in [60, 80, 92] {
            let s = render(&a, View::Setup, w, 24);
            let header = s.lines().next().unwrap();
            assert!(header.contains("teclado + mouse"), "{w}: {header}");
            assert!(header.contains("nodirt"), "{w}: {header}");
        }
    }

    #[test]
    fn modal_de_ayuda() {
        let mut a = app();
        a.help = true;
        let s = render(&a, View::Setup, 92, 28);
        assert!(s.contains("atajos") && s.contains("personalizada"), "{s}");
    }

    #[test]
    fn cuenta_atras() {
        let s = render(&app(), COUNTDOWN, 92, 28);
        assert!(s.contains("PREPARANDO") && s.contains("cancelar"), "{s}");
    }

    #[test]
    fn bloqueado() {
        let s = render(&app(), LOCKED, 92, 28);
        for needle in [
            "BLOQUEADO",
            "limpia tranquilo",
            "mantén Escape 3s",
            "50%",
            "30%",
            "emergencia",
        ] {
            assert!(s.contains(needle), "falta {needle:?}\n{s}");
        }
        let mut a = app();
        a.mouse = false;
        assert!(render(&a, LOCKED, 92, 28).contains("solo teclado"));
    }

    #[test]
    fn soltar_tras_terminar() {
        let s = render(&app(), RELEASING, 92, 28);
        assert!(
            s.contains("LIBERANDO") && s.contains("suelta Escape"),
            "{s}"
        );
        let s = render(
            &app(),
            View::Run(Phase::Releasing { unlocked: false }),
            92,
            28,
        );
        assert!(s.contains("suelta las teclas"), "{s}");
    }

    #[test]
    fn terminal_pequena_muestra_lo_esencial() {
        let s = render(&app(), LOCKED, 40, 10);
        assert!(s.contains("bloqueado · 00:42"), "{s}");
        assert!(render(&app(), View::Setup, 40, 10).contains("demasiado pequeña"));
    }

    #[test]
    fn barra() {
        let txt = |r| {
            bar_spans(r, 10, None)
                .iter()
                .map(|s| {
                    if s.style.fg == Some(FAINT) {
                        '·'
                    } else {
                        '━'
                    }
                })
                .collect::<String>()
        };
        assert_eq!(txt(0.0), "··········");
        assert_eq!(txt(0.55), "━━━━━·····");
        assert_eq!(txt(0.99), "━━━━━━━━━·"); // no se llena antes de tiempo
        assert_eq!(txt(1.0), "━━━━━━━━━━");
    }

    #[test]
    fn no_entra_en_panico_en_ningun_tamano() {
        let mut a = app();
        a.set_message("✓ listo", Level::Ok);
        for (w, h) in [(1, 1), (40, 10), (60, 18), (80, 24), (120, 40), (250, 70)] {
            for v in [View::Setup, COUNTDOWN, LOCKED, RELEASING] {
                render(&a, v, w, h);
            }
            a.help = true;
            render(&a, View::Setup, w, h);
            a.help = false;
        }
    }
}
