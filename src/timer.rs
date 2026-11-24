//! Temporizador puro: recibe `now` en cada consulta, así se testea sin dormir.
//! Se usa tanto para la cuenta atrás de inicio como para el bloqueo.

use std::time::{Duration, Instant};

/// Cuenta atrás antes de bloquear (fija, cancelable).
pub const START_DELAY: Duration = Duration::from_secs(3);
/// Duración por defecto del bloqueo.
pub const DEFAULT_LOCK: Duration = Duration::from_secs(60);
/// Presets de la TUI.
pub const PRESETS: [Duration; 5] = [
    Duration::from_secs(30),
    Duration::from_secs(60),
    Duration::from_secs(2 * 60),
    Duration::from_secs(5 * 60),
    Duration::from_secs(10 * 60),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timer {
    start: Instant,
    total: Duration,
}

impl Timer {
    pub fn new(start: Instant, total: Duration) -> Self {
        Self { start, total }
    }

    pub fn total(&self) -> Duration {
        self.total
    }

    pub fn elapsed(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.start).min(self.total)
    }

    pub fn remaining(&self, now: Instant) -> Duration {
        self.total - self.elapsed(now)
    }

    pub fn expired(&self, now: Instant) -> bool {
        self.elapsed(now) >= self.total
    }

    /// Fracción transcurrida en `[0, 1]`.
    pub fn progress(&self, now: Instant) -> f64 {
        if self.total.is_zero() {
            return 1.0;
        }
        self.elapsed(now).as_secs_f64() / self.total.as_secs_f64()
    }

    /// Segundos restantes redondeados hacia arriba (para mostrar "3, 2, 1").
    pub fn remaining_secs_ceil(&self, now: Instant) -> u64 {
        let r = self.remaining(now);
        r.as_secs() + u64::from(r.subsec_nanos() > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: Duration = Duration::from_secs(1);

    #[test]
    fn cuenta_y_expira() {
        let t0 = Instant::now();
        let t = Timer::new(t0, 10 * S);
        assert_eq!(t.remaining(t0), 10 * S);
        assert!(!t.expired(t0));
        assert_eq!(t.remaining(t0 + 4 * S), 6 * S);
        assert!(!t.expired(t0 + 9 * S));
        assert!(t.expired(t0 + 10 * S));
        assert_eq!(t.remaining(t0 + 10 * S), Duration::ZERO);
    }

    #[test]
    fn no_desborda_pasado_el_final_ni_antes_del_inicio() {
        let t0 = Instant::now();
        let t = Timer::new(t0 + 5 * S, 10 * S);
        // `now` anterior al inicio: nada transcurrido.
        assert_eq!(t.elapsed(t0), Duration::ZERO);
        assert_eq!(t.remaining(t0), 10 * S);
        // Muy pasado el final: se queda en 0, no entra en pánico.
        assert_eq!(t.remaining(t0 + 1000 * S), Duration::ZERO);
        assert!((t.progress(t0 + 1000 * S) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn progreso() {
        let t0 = Instant::now();
        let t = Timer::new(t0, 4 * S);
        assert!(t.progress(t0).abs() < f64::EPSILON);
        assert!((t.progress(t0 + S) - 0.25).abs() < 1e-9);
        assert!((t.progress(t0 + 2 * S) - 0.5).abs() < 1e-9);
        assert!((Timer::new(t0, Duration::ZERO).progress(t0) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn segundos_redondeados_hacia_arriba() {
        let t0 = Instant::now();
        let t = Timer::new(t0, START_DELAY);
        assert_eq!(t.remaining_secs_ceil(t0), 3);
        assert_eq!(t.remaining_secs_ceil(t0 + Duration::from_millis(100)), 3);
        assert_eq!(t.remaining_secs_ceil(t0 + Duration::from_millis(1001)), 2);
        assert_eq!(t.remaining_secs_ceil(t0 + 3 * S), 0);
    }

    #[test]
    fn presets_dentro_del_tope() {
        for p in PRESETS {
            assert!(p >= crate::duration::MIN_LOCK && p <= crate::duration::MAX_LOCK);
        }
        assert!(PRESETS.contains(&DEFAULT_LOCK));
    }
}
