//! TUI. En la pantalla de configuración y en la cuenta atrás las teclas llegan por
//! stdin (crossterm). Con el bloqueo activo, el teclado está capturado por X11:
//! la terminal no recibe nada y la única entrada son los eventos X11 que procesa
//! `lock::run` (combo). Aquí solo se redibuja.

pub mod app;
mod digits;
#[cfg(test)]
mod snapshot;
mod theme;
mod ui;

use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use ratatui::{
    DefaultTerminal,
    crossterm::event::{self, Event, KeyEventKind},
};

use crate::{
    backend::x11::{self, X11Backend},
    lock::{self, Control, LockError, Outcome, Phase, RealClock},
};
use app::{Action, App, Level};
use ui::View;

const FRAME: Duration = Duration::from_millis(100);

/// `true` si la app terminó por una señal.
pub fn run(mut app: App, stop: &AtomicBool) -> color_eyre::Result<bool> {
    // `ratatui::init` instala un hook que restaura la terminal ante panic;
    // el nuestro se instala después para correr ANTES y soltar los grabs primero.
    let mut terminal = ratatui::init();
    x11::install_panic_hook();
    let res = event_loop(&mut terminal, &mut app, stop);
    ratatui::restore();
    res?;
    Ok(stop.load(Ordering::Relaxed))
}

fn event_loop(
    terminal: &mut DefaultTerminal,
    app: &mut App,
    stop: &AtomicBool,
) -> color_eyre::Result<()> {
    while !stop.load(Ordering::Relaxed) {
        terminal.draw(|f| ui::draw(f, app, View::Setup))?;
        app.tick = app.tick.wrapping_add(1);
        if event::poll(FRAME)?
            && let Event::Key(k) = event::read()?
            && k.kind == KeyEventKind::Press
        {
            match app.on_key(k) {
                Action::Quit => break,
                Action::Start => start(terminal, app, stop),
                Action::None => {}
            }
        }
    }
    Ok(())
}

/// Ejecuta un bloqueo completo y deja el resultado como mensaje en `app`.
fn start(terminal: &mut DefaultTerminal, app: &mut App, stop: &AtomicBool) {
    let cfg = app.config();
    let mut backend = match X11Backend::connect() {
        Ok(b) => b,
        Err(e) => {
            app.set_message(format!("✗ {e}"), Level::Err);
            return;
        }
    };

    let res = lock::run(&mut backend, &mut RealClock, &cfg, stop, |p| {
        // Un fallo al dibujar no debe alargar ni romper el bloqueo: se ignora.
        let _ = terminal.draw(|f| ui::draw(f, app, View::Run(p)));
        // Avanza el pulso de la cabecera y del estado.
        app.tick = app.tick.wrapping_add(1);
        if matches!(p, Phase::Countdown { .. }) && key_pressed() {
            return Control::Cancel;
        }
        Control::Continue
    });
    // Suelta grabs y cierra la conexión X11 ya, antes de volver a la configuración.
    drop(backend);

    let (msg, lvl) = match res {
        Ok(Outcome::Expired) => (
            "✓ tiempo cumplido · teclado y mouse liberados".into(),
            Level::Ok,
        ),
        Ok(Outcome::Unlocked) => (
            format!(
                "✓ desbloqueado con {} · teclado y mouse liberados",
                cfg.unlock_key
            ),
            Level::Ok,
        ),
        Ok(Outcome::Cancelled) => ("· cancelado · no se bloqueó nada".into(), Level::Warn),
        Ok(Outcome::Signal) => ("· señal recibida · liberado".into(), Level::Warn),
        Err(e @ (LockError::Busy(_) | LockError::Backend(_))) => (format!("✗ {e}"), Level::Err),
    };
    app.set_message(msg, lvl);
}

/// ¿Hay una pulsación pendiente en la terminal? Ante error, `true`: cancelar es lo seguro.
fn key_pressed() -> bool {
    match event::poll(Duration::ZERO) {
        Ok(false) => false,
        Ok(true) => match event::read() {
            Ok(Event::Key(k)) => k.kind == KeyEventKind::Press,
            Ok(_) => false,
            Err(_) => true,
        },
        Err(_) => true,
    }
}
