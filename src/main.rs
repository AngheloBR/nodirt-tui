mod backend;
mod cli;
mod duration;
mod keys;
mod lock;
mod timer;
mod tui;
mod unlock;

use std::{
    io::{IsTerminal, Write},
    process::ExitCode,
    sync::{Arc, atomic::AtomicBool},
};

use backend::x11::{self, X11Backend};
use clap::Parser;
use cli::{Cli, exit};
use lock::{Config, Control, LockError, Outcome, Phase, RealClock};

const BAR_WIDTH: usize = 10;

fn main() -> color_eyre::Result<ExitCode> {
    color_eyre::install()?;
    let cli = Cli::parse();

    let stop = Arc::new(AtomicBool::new(false));
    for sig in [
        signal_hook::consts::SIGINT,
        signal_hook::consts::SIGTERM,
        signal_hook::consts::SIGHUP,
    ] {
        signal_hook::flag::register(sig, Arc::clone(&stop))?;
    }

    let bare = std::env::args_os().len() == 1;
    let interactive = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    if cli.tui || (bare && interactive) {
        if !interactive {
            eprintln!("✗ --tui necesita una terminal interactiva");
            return Ok(ExitCode::from(exit::USAGE));
        }
        let app = tui::app::App::new(cli.time, !cli.no_mouse, cli.unlock_key, cli.unlock_hold);
        let by_signal = tui::run(app, &stop)?;
        return Ok(ExitCode::from(if by_signal {
            exit::CANCELLED
        } else {
            exit::OK
        }));
    }

    let cfg = Config {
        duration: cli.time.unwrap_or(timer::DEFAULT_LOCK),
        unlock_key: cli.unlock_key,
        unlock_hold: cli.unlock_hold,
        mouse: !cli.no_mouse,
    };

    // Después de color_eyre: el nuestro corre primero y suelta los grabs.
    x11::install_panic_hook();

    // Conectar ANTES de la cuenta atrás: si no hay X11, fallar ya.
    let mut backend = match X11Backend::connect() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("✗ {e}");
            return Ok(ExitCode::from(exit::GRAB_FAILED));
        }
    };

    let what = if cfg.mouse {
        "teclado + mouse"
    } else {
        "teclado"
    };
    println!(
        "{} · {what} · {} · salir antes: mantén {} {}",
        env!("CARGO_BIN_NAME"),
        duration::fmt_short(cfg.duration),
        cfg.unlock_key,
        duration::fmt_short(cfg.unlock_hold),
    );
    if !backend.detectable_repeat() {
        println!("  · el servidor no soporta DetectableAutoRepeat; uso el método alternativo");
    }

    let mut last = String::new();
    let res = lock::run(&mut backend, &mut RealClock, &cfg, &stop, |p| {
        let line = render(p, &cfg);
        if line != last {
            print!("\r\x1b[2K{line}");
            let _ = std::io::stdout().flush();
            last = line;
        }
        Control::Continue
    });
    drop(backend);
    println!();

    let code = match res {
        Ok(Outcome::Expired) => {
            println!("✓ tiempo cumplido · teclado y mouse liberados");
            exit::OK
        }
        Ok(Outcome::Unlocked) => {
            println!(
                "✓ desbloqueado con {} · teclado y mouse liberados",
                cfg.unlock_key
            );
            exit::OK
        }
        Ok(Outcome::Cancelled | Outcome::Signal) => {
            println!("· cancelado · nada queda bloqueado");
            exit::CANCELLED
        }
        Err(e @ LockError::Busy(_)) => {
            eprintln!("✗ {e}");
            exit::GRAB_FAILED
        }
        Err(e @ LockError::Backend(_)) => {
            eprintln!("✗ {e}");
            exit::ERROR
        }
    };
    Ok(ExitCode::from(code))
}

fn render(p: Phase, cfg: &Config) -> String {
    match p {
        Phase::Countdown { secs_left } => {
            format!("  bloqueando en {secs_left} s · Ctrl+C para cancelar")
        }
        Phase::Releasing { unlocked: true } => {
            format!("  ✓ combo completado · suelta {}", cfg.unlock_key)
        }
        Phase::Releasing { unlocked: false } => "  ✓ tiempo cumplido · suelta las teclas".into(),
        Phase::Locked {
            remaining, hold, ..
        } => {
            // Redondeo hacia abajo: la barra solo se llena del todo al desbloquear.
            let filled = ((hold * BAR_WIDTH as f64) as usize).min(BAR_WIDTH);
            format!(
                "  ◉ bloqueado · restante {} · mantén {} para salir [{}{}]",
                duration::fmt_mmss(remaining),
                cfg.unlock_key,
                "█".repeat(filled),
                "░".repeat(BAR_WIDTH - filled),
            )
        }
    }
}
