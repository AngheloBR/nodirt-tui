//! Máquina de estados del combo "mantener tecla N segundos". Pura: recibe `now`.
//!
//! ```text
//! Idle ──press(target)──────────────▶ Holding{since}
//! Holding ──press(target) (repetición)▶ Holding        (se ignora, no reinicia)
//! Holding ──release(target) antes─────▶ Idle           (reinicia, progreso 0)
//! Holding ──release/tick ≥ since+hold─▶ Unlocked       (final)
//! cualquier estado ──otra tecla───────▶ sin cambio
//! Unlocked ──cualquier evento─────────▶ Unlocked
//! ```
//!
//! Que la tecla siga pulsada tras desbloquear lo gestiona `lock.rs`, que no
//! suelta la captura hasta que se liberen todas las teclas (con tope).
//!
//! Las repeticiones de autorepeat llegan como `press` sin `release` gracias a
//! `XkbSetDetectableAutoRepeat`; si el servidor no lo soporta, el backend debe
//! fusionar el par release+press con el mismo timestamp antes de llamar aquí.

use std::time::{Duration, Instant};

use crate::keys::Key;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Idle,
    Holding { since: Instant },
    Unlocked,
}

#[derive(Debug, Clone)]
pub struct Unlock {
    target: Key,
    hold: Duration,
    state: State,
}

impl Unlock {
    pub fn new(target: Key, hold: Duration) -> Self {
        Self {
            target,
            hold,
            state: State::Idle,
        }
    }

    pub fn press(&mut self, key: Key, now: Instant) {
        if key == self.target && self.state == State::Idle {
            self.state = State::Holding { since: now };
        }
    }

    pub fn release(&mut self, key: Key, now: Instant) {
        if key != self.target {
            return;
        }
        if let State::Holding { since } = self.state {
            // Si ya se cumplió el tiempo, soltar cuenta como completado
            // (no depender de que el tick llegue justo antes).
            self.state = if now.saturating_duration_since(since) >= self.hold {
                State::Unlocked
            } else {
                State::Idle
            };
        }
    }

    /// Avanza el reloj. Devuelve `true` si ya está desbloqueado.
    pub fn tick(&mut self, now: Instant) -> bool {
        if let State::Holding { since } = self.state
            && now.saturating_duration_since(since) >= self.hold
        {
            self.state = State::Unlocked;
        }
        self.is_unlocked()
    }

    pub fn is_unlocked(&self) -> bool {
        self.state == State::Unlocked
    }

    #[cfg(test)]
    pub fn is_holding(&self) -> bool {
        matches!(self.state, State::Holding { .. })
    }

    /// Progreso de la barra en `[0, 1]`.
    pub fn progress(&self, now: Instant) -> f64 {
        match self.state {
            State::Idle => 0.0,
            State::Unlocked => 1.0,
            State::Holding { since } => {
                let f =
                    now.saturating_duration_since(since).as_secs_f64() / self.hold.as_secs_f64();
                f.min(1.0)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ESC: Key = Key::ESCAPE;
    const A: Key = Key(0x61);
    const MS: Duration = Duration::from_millis(1);
    const S: Duration = Duration::from_secs(1);

    fn new() -> (Unlock, Instant) {
        (Unlock::new(ESC, 3 * S), Instant::now())
    }

    #[test]
    fn mantener_completo_desbloquea() {
        let (mut u, t0) = new();
        u.press(ESC, t0);
        assert!(u.is_holding());
        assert!(!u.tick(t0 + 2999 * MS));
        assert!(u.tick(t0 + 3 * S));
        assert!(u.is_unlocked());
    }

    #[test]
    fn soltar_antes_reinicia() {
        let (mut u, t0) = new();
        u.press(ESC, t0);
        u.release(ESC, t0 + 2 * S);
        assert!(!u.is_holding());
        assert!(u.progress(t0 + 2 * S).abs() < f64::EPSILON);
        // El tiempo previo no cuenta: hay que empezar de cero.
        u.press(ESC, t0 + 2500 * MS);
        assert!(!u.tick(t0 + 5 * S)); // 2.5 s desde el nuevo press
        assert!(u.tick(t0 + 5500 * MS)); // 3 s desde el nuevo press
    }

    #[test]
    fn otras_teclas_se_ignoran() {
        let (mut u, t0) = new();
        // Otras teclas no inician el combo.
        u.press(A, t0);
        assert!(!u.tick(t0 + 10 * S));
        assert!(!u.is_holding());
        // Ni lo reinician mientras se mantiene el target.
        u.press(ESC, t0 + 10 * S);
        u.press(A, t0 + 11 * S);
        u.release(A, t0 + 12 * S);
        assert!(u.is_holding());
        assert!(u.tick(t0 + 13 * S));
    }

    #[test]
    fn repeticiones_no_reinician() {
        let (mut u, t0) = new();
        u.press(ESC, t0);
        for i in 1..30 {
            u.press(ESC, t0 + i * 100 * MS); // autorepeat detectable
        }
        assert!(u.tick(t0 + 3 * S));
    }

    #[test]
    fn soltar_justo_al_cumplir_cuenta() {
        let (mut u, t0) = new();
        u.press(ESC, t0);
        u.release(ESC, t0 + 3 * S); // sin tick intermedio
        assert!(u.is_unlocked());
    }

    #[test]
    fn soltar_sin_haber_pulsado_no_hace_nada() {
        // p. ej. Esc pulsado antes de que empezara el bloqueo.
        let (mut u, t0) = new();
        u.release(ESC, t0);
        assert!(!u.is_holding() && !u.is_unlocked());
    }

    #[test]
    fn desbloqueado_es_final() {
        let (mut u, t0) = new();
        u.press(ESC, t0);
        assert!(u.tick(t0 + 3 * S));
        u.release(ESC, t0 + 4 * S);
        u.press(A, t0 + 5 * S);
        assert!(u.is_unlocked());
        assert!((u.progress(t0) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn progreso_de_la_barra() {
        let (mut u, t0) = new();
        assert!(u.progress(t0).abs() < f64::EPSILON);
        u.press(ESC, t0);
        assert!((u.progress(t0 + 1500 * MS) - 0.5).abs() < 1e-9);
        assert!((u.progress(t0 + 10 * S) - 1.0).abs() < f64::EPSILON); // acotado
    }

    #[test]
    fn otra_tecla_de_desbloqueo() {
        let mut u = Unlock::new(A, S);
        let t0 = Instant::now();
        u.press(ESC, t0);
        assert!(!u.tick(t0 + 5 * S));
        u.press(A, t0 + 5 * S);
        assert!(u.tick(t0 + 6 * S));
    }
}
