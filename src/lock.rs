//! Orquestador del bloqueo, genérico sobre `InputBackend` y `Clock`:
//! cuenta atrás → captura (todo o nada, con reintentos) → bucle → liberación.
//!
//! La liberación vive en `Drop` de `Held`: se ejecuta al terminar el temporizador,
//! al completar el combo, ante señal, ante error y durante el unwind de un panic.

use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

use crate::{
    backend::{BackendError, Device, GrabAttempt, InputBackend, InputEvent},
    keys::Key,
    timer::{START_DELAY, Timer},
    unlock::Unlock,
};

/// Cuánto se reintenta en total si otro programa tiene la captura.
pub const RETRY_FOR: Duration = Duration::from_secs(3);
pub const RETRY_EVERY: Duration = Duration::from_millis(100);
/// Periodo del bucle (refresco de UI y revisión de señales).
pub const TICK: Duration = Duration::from_millis(50);

pub trait Clock {
    fn now(&self) -> Instant;
    fn sleep(&mut self, d: Duration);
}

pub struct RealClock;

impl Clock for RealClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn sleep(&mut self, d: Duration) {
        std::thread::sleep(d);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LockError {
    #[error(
        "otro programa tiene capturado {0} (¿menú abierto, rofi/dmenu, un chord de sxhkd, \
         un arrastre en bspwm?) · reintenté {s} s; cierra eso y vuelve a probar",
        s = RETRY_FOR.as_secs()
    )]
    Busy(Device),
    #[error("error del sistema de entrada: {0}")]
    Backend(#[from] BackendError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    pub duration: Duration,
    pub unlock_key: Key,
    pub unlock_hold: Duration,
    pub mouse: bool,
}

impl Config {
    pub fn devices(&self) -> &'static [Device] {
        if self.mouse {
            &[Device::Keyboard, Device::Pointer]
        } else {
            &[Device::Keyboard]
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Terminó el temporizador.
    Expired,
    /// Se completó el combo.
    Unlocked,
    /// Se canceló durante la cuenta atrás (nunca se capturó nada).
    Cancelled,
    /// SIGINT/SIGTERM/SIGHUP.
    Signal,
}

/// Lo que la UI necesita para dibujar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Phase {
    Countdown {
        secs_left: u64,
    },
    Locked {
        remaining: Duration,
        total: Duration,
        /// Progreso del combo en `[0, 1]`.
        hold: f64,
    },
}

/// Respuesta de la UI en cada tick. Solo se respeta durante la cuenta atrás:
/// con el bloqueo activo, la única salida es temporizador, combo o señal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    Continue,
    Cancel,
}

/// Capturas activas. Al soltarse (`Drop`) libera en orden inverso.
pub struct Held<'a, B: InputBackend> {
    backend: &'a mut B,
    held: Vec<Device>,
}

impl<B: InputBackend> Held<'_, B> {
    pub fn backend(&mut self) -> &mut B {
        self.backend
    }

    pub fn devices(&self) -> &[Device] {
        &self.held
    }
}

impl<B: InputBackend> Drop for Held<'_, B> {
    fn drop(&mut self) {
        while let Some(d) = self.held.pop() {
            self.backend.ungrab(d);
        }
    }
}

/// Captura todos los `devices` o ninguno. Reintenta `Busy` hasta `RETRY_FOR` en total.
pub fn acquire<'a, B: InputBackend, C: Clock>(
    backend: &'a mut B,
    clock: &mut C,
    devices: &[Device],
) -> Result<Held<'a, B>, LockError> {
    let mut held = Held {
        backend,
        held: Vec::with_capacity(devices.len()),
    };
    let deadline = clock.now() + RETRY_FOR;
    for &d in devices {
        loop {
            // Si `?` o `return Err` salen de aquí, `held` se suelta y libera lo ya capturado.
            match held.backend.try_grab(d)? {
                GrabAttempt::Grabbed => {
                    held.held.push(d);
                    break;
                }
                GrabAttempt::Busy if clock.now() >= deadline => return Err(LockError::Busy(d)),
                GrabAttempt::Busy => clock.sleep(RETRY_EVERY),
            }
        }
    }
    Ok(held)
}

/// Cuenta atrás de `START_DELAY`. `None` si terminó (hay que bloquear); `Some(..)` si se canceló o llegó una señal.
fn countdown<C: Clock>(
    clock: &mut C,
    stop: &AtomicBool,
    ui: &mut impl FnMut(Phase) -> Control,
) -> Option<Outcome> {
    let timer = Timer::new(clock.now(), START_DELAY);
    loop {
        if stop.load(Ordering::Relaxed) {
            return Some(Outcome::Signal);
        }
        let now = clock.now();
        if timer.expired(now) {
            return None;
        }
        let phase = Phase::Countdown {
            secs_left: timer.remaining_secs_ceil(now),
        };
        if ui(phase) == Control::Cancel {
            return Some(Outcome::Cancelled);
        }
        clock.sleep(TICK.min(timer.remaining(now)));
    }
}

/// Sesión completa: cuenta atrás, captura, bucle y liberación.
pub fn run<B: InputBackend, C: Clock>(
    backend: &mut B,
    clock: &mut C,
    cfg: &Config,
    stop: &AtomicBool,
    mut ui: impl FnMut(Phase) -> Control,
) -> Result<Outcome, LockError> {
    if let Some(out) = countdown(clock, stop, &mut ui) {
        return Ok(out);
    }

    let mut held = acquire(backend, clock, cfg.devices())?;
    let timer = Timer::new(clock.now(), cfg.duration);
    let mut unlock = Unlock::new(cfg.unlock_key, cfg.unlock_hold);

    // `held` se suelta al salir de esta función por cualquier camino.
    loop {
        let now = clock.now();
        if stop.load(Ordering::Relaxed) {
            return Ok(Outcome::Signal);
        }
        if unlock.tick(now) {
            return Ok(Outcome::Unlocked);
        }
        if timer.expired(now) {
            return Ok(Outcome::Expired);
        }
        // El valor devuelto se ignora a propósito: con el bloqueo activo no se cancela desde la UI.
        let _ = ui(Phase::Locked {
            remaining: timer.remaining(now),
            total: timer.total(),
            hold: unlock.progress(now),
        });
        match held.backend().poll(TICK.min(timer.remaining(now)))? {
            Some(InputEvent::KeyPress(k)) => unlock.press(k, clock.now()),
            Some(InputEvent::KeyRelease(k)) => unlock.release(k, clock.now()),
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        cell::Cell,
        panic::{AssertUnwindSafe, catch_unwind},
    };

    use super::*;
    use crate::backend::fake::{Call, FakeBackend};

    const S: Duration = Duration::from_secs(1);
    const ESC: Key = Key::ESCAPE;
    const A: Key = Key(0x61);
    use Call::{Grabbed, Ungrabbed};
    use Device::{Keyboard as K, Pointer as P};

    fn cfg(secs: u64, mouse: bool) -> Config {
        Config {
            duration: Duration::from_secs(secs),
            unlock_key: ESC,
            unlock_hold: 3 * S,
            mouse,
        }
    }

    fn go(b: &mut FakeBackend, c: &mut impl Clock, cfg: &Config) -> Result<Outcome, LockError> {
        run(b, c, cfg, &AtomicBool::new(false), |_| Control::Continue)
    }

    // ── captura todo o nada ────────────────────────────────────────────────

    #[test]
    fn captura_y_libera_en_orden_inverso() {
        let (mut b, mut c, _) = FakeBackend::new();
        let log = b.log.clone();
        {
            let held = acquire(&mut b, &mut c, &[K, P]).unwrap();
            assert_eq!(held.devices(), &[K, P]);
        }
        assert_eq!(
            *log.borrow(),
            [Grabbed(K), Grabbed(P), Ungrabbed(P), Ungrabbed(K)]
        );
        assert!(!b.is_grabbed(K) && !b.is_grabbed(P));
    }

    #[test]
    fn si_el_mouse_esta_ocupado_suelta_el_teclado() {
        let (b, mut c, t0) = FakeBackend::new();
        let mut b = b.busy_until(P, t0 + 100 * S);
        let log = b.log.clone();
        let err = acquire(&mut b, &mut c, &[K, P]).err().unwrap();
        assert_eq!(err, LockError::Busy(P));
        assert_eq!(*log.borrow(), [Grabbed(K), Ungrabbed(K)]);
        assert!(!b.is_grabbed(K));
        // Reintentó durante RETRY_FOR y se rindió (no infinito).
        let waited = c.now() - t0;
        assert!(waited >= RETRY_FOR && waited < RETRY_FOR + RETRY_EVERY * 2);
    }

    #[test]
    fn reintenta_si_se_libera_a_tiempo() {
        let (b, mut c, t0) = FakeBackend::new();
        let mut b = b.busy_until(K, t0 + 1500 * Duration::from_millis(1));
        let held = acquire(&mut b, &mut c, &[K, P]).unwrap();
        assert_eq!(held.devices(), &[K, P]);
    }

    #[test]
    fn error_de_backend_suelta_lo_capturado() {
        let (b, mut c, _) = FakeBackend::new();
        let mut b = b.fail_grab(P);
        let log = b.log.clone();
        let err = acquire(&mut b, &mut c, &[K, P]).err().unwrap();
        assert!(matches!(err, LockError::Backend(_)));
        assert_eq!(*log.borrow(), [Grabbed(K), Ungrabbed(K)]);
    }

    #[test]
    fn teclado_ocupado_da_error_claro() {
        let (b, mut c, t0) = FakeBackend::new();
        let mut b = b.busy_until(K, t0 + 100 * S);
        let msg = acquire(&mut b, &mut c, &[K, P]).err().unwrap().to_string();
        assert!(msg.contains("el teclado") && msg.contains("sxhkd"), "{msg}");
        assert!(b.log.borrow().is_empty());
    }

    // ── sesión completa ────────────────────────────────────────────────────

    #[test]
    fn el_temporizador_libera() {
        let (mut b, mut c, t0) = FakeBackend::new();
        let log = b.log.clone();
        assert_eq!(go(&mut b, &mut c, &cfg(10, true)), Ok(Outcome::Expired));
        assert_eq!(
            *log.borrow(),
            [Grabbed(K), Grabbed(P), Ungrabbed(P), Ungrabbed(K)]
        );
        // 3 s de cuenta atrás + 10 s de bloqueo.
        assert_eq!(c.now() - t0, 13 * S);
    }

    #[test]
    fn el_combo_libera() {
        let (b, mut c, t0) = FakeBackend::new();
        // Cuenta atrás termina en t0+3; Esc a los 5 s, desbloquea a los 8 s.
        let mut b = b.event(t0 + 5 * S, InputEvent::KeyPress(ESC));
        let log = b.log.clone();
        assert_eq!(go(&mut b, &mut c, &cfg(60, true)), Ok(Outcome::Unlocked));
        assert_eq!(log.borrow().last(), Some(&Ungrabbed(K)));
        let t = c.now() - t0;
        assert!(t >= 8 * S && t < 8 * S + TICK * 2, "{t:?}");
    }

    #[test]
    fn soltar_el_combo_antes_no_libera() {
        let (b, mut c, _) = FakeBackend::new();
        let t0 = c.now();
        let mut b = b
            .event(t0 + 4 * S, InputEvent::KeyPress(ESC))
            .event(t0 + 6 * S, InputEvent::KeyRelease(ESC))
            .event(t0 + 7 * S, InputEvent::KeyPress(A));
        assert_eq!(go(&mut b, &mut c, &cfg(20, true)), Ok(Outcome::Expired));
    }

    #[test]
    fn sin_mouse_solo_captura_teclado() {
        let (mut b, mut c, _) = FakeBackend::new();
        let log = b.log.clone();
        assert_eq!(go(&mut b, &mut c, &cfg(5, false)), Ok(Outcome::Expired));
        assert_eq!(*log.borrow(), [Grabbed(K), Ungrabbed(K)]);
    }

    #[test]
    fn cancelar_la_cuenta_atras_no_captura_nada() {
        let (mut b, mut c, t0) = FakeBackend::new();
        let log = b.log.clone();
        let mut seen = Vec::new();
        let out = run(
            &mut b,
            &mut c,
            &cfg(60, true),
            &AtomicBool::new(false),
            |p| {
                seen.push(p);
                if c_elapsed(&seen) {
                    Control::Cancel
                } else {
                    Control::Continue
                }
            },
        );
        assert_eq!(out, Ok(Outcome::Cancelled));
        assert!(log.borrow().is_empty());
        assert!(c.now() - t0 < START_DELAY);
        assert_eq!(seen[0], Phase::Countdown { secs_left: 3 });
    }

    /// Cancela tras unos cuantos ticks de cuenta atrás.
    fn c_elapsed(seen: &[Phase]) -> bool {
        seen.len() >= 10
    }

    #[test]
    fn la_ui_no_puede_cancelar_con_el_bloqueo_activo() {
        let (mut b, mut c, _) = FakeBackend::new();
        let out = run(
            &mut b,
            &mut c,
            &cfg(5, true),
            &AtomicBool::new(false),
            |p| {
                if matches!(p, Phase::Locked { .. }) {
                    Control::Cancel
                } else {
                    Control::Continue
                }
            },
        );
        assert_eq!(out, Ok(Outcome::Expired));
    }

    #[test]
    fn senal_durante_el_bloqueo_libera() {
        let (mut b, mut c, _) = FakeBackend::new();
        let log = b.log.clone();
        let stop = AtomicBool::new(false);
        let ticks = Cell::new(0);
        let out = run(&mut b, &mut c, &cfg(60, true), &stop, |p| {
            if matches!(p, Phase::Locked { .. }) {
                ticks.set(ticks.get() + 1);
                if ticks.get() == 5 {
                    stop.store(true, Ordering::Relaxed); // como haría signal-hook
                }
            }
            Control::Continue
        });
        assert_eq!(out, Ok(Outcome::Signal));
        assert_eq!(log.borrow().last(), Some(&Ungrabbed(K)));
        assert!(!b.is_grabbed(K) && !b.is_grabbed(P));
    }

    #[test]
    fn senal_durante_la_cuenta_atras() {
        let (mut b, mut c, _) = FakeBackend::new();
        let stop = AtomicBool::new(true);
        let out = run(&mut b, &mut c, &cfg(60, true), &stop, |_| Control::Continue);
        assert_eq!(out, Ok(Outcome::Signal));
        assert!(b.log.borrow().is_empty());
    }

    #[test]
    fn conexion_perdida_libera_y_devuelve_error() {
        let (b, mut c, t0) = FakeBackend::new();
        let mut b = b.poll_error_at(t0 + 5 * S);
        let log = b.log.clone();
        let out = go(&mut b, &mut c, &cfg(60, true));
        assert!(matches!(out, Err(LockError::Backend(_))));
        assert_eq!(log.borrow().last(), Some(&Ungrabbed(K)));
    }

    #[test]
    fn panic_en_la_ui_libera_por_drop() {
        let (mut b, mut c, _) = FakeBackend::new();
        let log = b.log.clone();
        let res = catch_unwind(AssertUnwindSafe(|| {
            run(
                &mut b,
                &mut c,
                &cfg(60, true),
                &AtomicBool::new(false),
                |p| {
                    if matches!(p, Phase::Locked { .. }) {
                        panic!("panic simulado en la UI");
                    }
                    Control::Continue
                },
            )
        }));
        assert!(res.is_err());
        assert_eq!(
            *log.borrow(),
            [Grabbed(K), Grabbed(P), Ungrabbed(P), Ungrabbed(K)]
        );
    }

    #[test]
    fn la_ui_recibe_el_progreso() {
        let (b, mut c, t0) = FakeBackend::new();
        let mut b = b.event(t0 + 4 * S, InputEvent::KeyPress(ESC));
        let mut max_hold: f64 = 0.0;
        let mut first_locked = None;
        let out = run(
            &mut b,
            &mut c,
            &cfg(60, true),
            &AtomicBool::new(false),
            |p| {
                if let Phase::Locked {
                    hold,
                    total,
                    remaining,
                } = p
                {
                    first_locked.get_or_insert((total, remaining));
                    max_hold = max_hold.max(hold);
                }
                Control::Continue
            },
        );
        assert_eq!(out, Ok(Outcome::Unlocked));
        assert_eq!(first_locked, Some((60 * S, 60 * S)));
        assert!(max_hold > 0.9 && max_hold < 1.0, "{max_hold}");
    }
}
