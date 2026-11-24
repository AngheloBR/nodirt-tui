// TEMPORAL (M1): timer, presets y códigos de salida aún no están cableados.
// Quitar en M5, cuando CLI y TUI los usen; a partir de ahí clippy debe pasar sin esto.
#![allow(dead_code)]

mod backend;
mod cli;
mod duration;
mod keys;
mod lock;
mod timer;
mod unlock;

use std::process::ExitCode;

use clap::Parser;
use cli::{Cli, exit};

fn main() -> color_eyre::Result<ExitCode> {
    color_eyre::install()?;
    let cli = Cli::parse();

    let dur = cli.time.unwrap_or(timer::DEFAULT_LOCK);
    let mouse = if cli.no_mouse { "no" } else { "sí" };
    println!(
        "{} · bloqueo {} · desbloqueo: mantener {} {} · mouse: {mouse}",
        env!("CARGO_BIN_NAME"),
        duration::fmt_short(dur),
        cli.unlock_key,
        duration::fmt_short(cli.unlock_hold),
    );
    // M1: aún no hay backend de entrada; el bloqueo llega en M3/M4.
    eprintln!("el bloqueo aún no está implementado (M1)");
    Ok(ExitCode::from(exit::ERROR))
}
