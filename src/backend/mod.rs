//! Abstracción del sistema de entrada. `lock.rs` orquesta (reintentos, todo o nada,
//! temporizador, combo) sobre este trait; los backends solo hacen un intento por llamada.

#[cfg(test)]
pub mod fake;
pub mod x11;

use std::{fmt, time::Duration};

use crate::keys::Key;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Device {
    Keyboard,
    /// Mouse y touchpad (todo lo que cuelga del pointer maestro).
    Pointer,
}

impl fmt::Display for Device {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Device::Keyboard => "el teclado",
            Device::Pointer => "el mouse",
        })
    }
}

/// Resultado de un único intento de captura.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrabAttempt {
    Grabbed,
    /// Otro cliente lo tiene capturado (`AlreadyGrabbed` / `Frozen`): reintentar.
    Busy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputEvent {
    KeyPress(Key),
    KeyRelease(Key),
}

/// Error no recuperable del backend (conexión perdida, protocolo…).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct BackendError(pub String);

pub trait InputBackend {
    /// Un solo intento de capturar `device`. Sin reintentos.
    fn try_grab(&mut self, device: Device) -> Result<GrabAttempt, BackendError>;

    /// Libera `device`. Idempotente y sin fallar (se llama desde `Drop`).
    fn ungrab(&mut self, device: Device);

    /// Espera hasta `timeout` por un evento de teclado. `None` = no hubo.
    fn poll(&mut self, timeout: Duration) -> Result<Option<InputEvent>, BackendError>;
}
