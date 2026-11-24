use std::time::Duration;

use clap::Parser;

use crate::{
    duration,
    keys::{self, Key},
};

/// Códigos de salida (también documentados en `--help`).
pub mod exit {
    /// Bloqueo terminado (temporizador o combo) o comando informativo.
    pub const OK: u8 = 0;
    /// Error inesperado.
    pub const ERROR: u8 = 1;
    /// Uso inválido (argumentos). Lo emite clap.
    pub const USAGE: u8 = 2;
    /// No se pudo capturar teclado/mouse (sin X11, o lo tiene otro programa).
    pub const GRAB_FAILED: u8 = 3;
    /// Cancelado: cuenta atrás interrumpida o señal (SIGINT/SIGTERM/SIGHUP).
    pub const CANCELLED: u8 = 130;
}

const AFTER_HELP: &str = "\
Sin argumentos (en una terminal) abre la interfaz interactiva (TUI).

El teclado se bloquea siempre; el mouse y el touchpad, salvo con --no-mouse.
Antes de bloquear hay una cuenta atrás de 3 s, cancelable.
Para salir antes de tiempo: mantener la tecla de desbloqueo (por defecto Escape, 3 s).

Salida de emergencia: el cambio de terminal virtual (Ctrl+Alt+F1…F12) NO se bloquea.
Si el proceso muere (incluso con kill -9), el servidor X libera el teclado y el mouse.
NO se bloquea el botón de encendido (lo gestiona systemd-logind, no X11).
Mientras dura, ningún bloqueador de pantalla (i3lock, xss-lock…) puede activarse.

Códigos de salida:
  0    bloqueo terminado (temporizador o combo)
  1    error inesperado
  2    uso inválido
  3    no se pudo capturar teclado/mouse (sin X11, o lo tiene otro programa)
  130  cancelado (cuenta atrás interrumpida o SIGINT/SIGTERM/SIGHUP)";

#[derive(Parser, Debug)]
#[command(
    name = env!("CARGO_BIN_NAME"),
    version,
    about = "Bloquea teclado y mouse temporalmente para limpiar el equipo · X11",
    after_help = AFTER_HELP
)]
pub struct Cli {
    /// Duración del bloqueo: 30s, 1m, 2m30s… (máximo 60m) [por defecto: 1m]
    #[arg(short = 't', long, value_name = "DUR", value_parser = duration::parse_lock)]
    pub time: Option<Duration>,

    /// Tecla que hay que mantener para desbloquear (nombre de keysym: Escape, F12, space…)
    #[arg(long, value_name = "TECLA", default_value = "Escape", value_parser = keys::parse_key)]
    pub unlock_key: Key,

    /// Cuánto tiempo mantenerla: 1s..10s
    #[arg(long, value_name = "DUR", default_value = "3s", value_parser = duration::parse_hold)]
    pub unlock_hold: Duration,

    /// No bloquear el mouse ni el touchpad (solo el teclado)
    #[arg(long)]
    pub no_mouse: bool,

    /// Abrir la TUI usando estas opciones como valores iniciales
    #[arg(long)]
    pub tui: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, error::ErrorKind};

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("nodirt").chain(args.iter().copied()))
    }

    #[test]
    fn valores_por_defecto() {
        let c = parse(&[]).unwrap();
        assert_eq!(c.time, None);
        assert_eq!(c.unlock_key, Key::ESCAPE);
        assert_eq!(c.unlock_hold, Duration::from_secs(3));
        assert!(!c.no_mouse);
        assert!(!c.tui);
    }

    #[test]
    fn opciones() {
        let c = parse(&[
            "-t",
            "2m30s",
            "--unlock-key",
            "F12",
            "--unlock-hold",
            "5s",
            "--no-mouse",
            "--tui",
        ])
        .unwrap();
        assert_eq!(c.time, Some(Duration::from_secs(150)));
        assert_eq!(c.unlock_key, Key(0xffc9)); // F12
        assert_eq!(c.unlock_hold, Duration::from_secs(5));
        assert!(c.no_mouse);
        assert!(c.tui);
    }

    #[test]
    fn duraciones_invalidas_son_error_de_uso() {
        for args in [
            &["--time", "90"][..],
            &["--time", "61m"],
            &["--time", "0s"],
            &["--time", "abc"],
            &["--unlock-hold", "11s"],
            &["--unlock-hold", "0s"],
            &["--unlock-key", "F13"],
            &["--unlock-key", "ctrl"],
        ] {
            let e = parse(args).unwrap_err();
            assert_eq!(e.kind(), ErrorKind::ValueValidation, "{args:?}");
            assert_eq!(e.exit_code(), i32::from(exit::USAGE), "{args:?}");
        }
    }

    #[test]
    fn help_documenta_todo() {
        let help = Cli::command().render_long_help().to_string();
        for needle in [
            "--time",
            "--unlock-key",
            "--unlock-hold",
            "--no-mouse",
            "--tui",
            "--help",
            "--version",
            "Códigos de salida",
            "130",
            "Ctrl+Alt+F1",
        ] {
            assert!(help.contains(needle), "falta {needle:?} en --help");
        }
    }

    #[test]
    fn definicion_de_clap_valida() {
        Cli::command().debug_assert();
    }
}
