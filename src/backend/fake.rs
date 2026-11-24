//! Backend falso para tests: tiempo simulado compartido con `FakeClock`,
//! eventos guionizados y registro de capturas/liberaciones.

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, VecDeque},
    rc::Rc,
    time::{Duration, Instant},
};

use super::{BackendError, Device, GrabAttempt, InputBackend, InputEvent};
use crate::lock::Clock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Call {
    Grabbed(Device),
    Ungrabbed(Device),
}

/// Reloj simulado: `sleep` avanza el tiempo al instante.
#[derive(Clone)]
pub struct FakeClock {
    pub now: Rc<Cell<Instant>>,
}

impl Clock for FakeClock {
    fn now(&self) -> Instant {
        self.now.get()
    }
    fn sleep(&mut self, d: Duration) {
        self.now.set(self.now.get() + d);
    }
}

pub struct FakeBackend {
    now: Rc<Cell<Instant>>,
    pub log: Rc<RefCell<Vec<Call>>>,
    grabbed: Vec<Device>,
    /// El dispositivo está ocupado por "otro programa" hasta ese instante.
    busy_until: HashMap<Device, Instant>,
    /// `try_grab` de este dispositivo devuelve error de backend.
    fail_grab: Option<Device>,
    /// Eventos que llegan en instantes absolutos.
    events: VecDeque<(Instant, InputEvent)>,
    /// `poll` falla a partir de este instante (conexión perdida).
    poll_error_at: Option<Instant>,
}

impl FakeBackend {
    /// Devuelve el backend y un reloj que comparte su tiempo simulado.
    pub fn new() -> (Self, FakeClock, Instant) {
        let t0 = Instant::now();
        let now = Rc::new(Cell::new(t0));
        let b = Self {
            now: Rc::clone(&now),
            log: Rc::default(),
            grabbed: Vec::new(),
            busy_until: HashMap::new(),
            fail_grab: None,
            events: VecDeque::new(),
            poll_error_at: None,
        };
        (b, FakeClock { now }, t0)
    }

    pub fn busy_until(mut self, d: Device, until: Instant) -> Self {
        self.busy_until.insert(d, until);
        self
    }

    pub fn fail_grab(mut self, d: Device) -> Self {
        self.fail_grab = Some(d);
        self
    }

    pub fn event(mut self, at: Instant, ev: InputEvent) -> Self {
        self.events.push_back((at, ev));
        self
    }

    pub fn poll_error_at(mut self, at: Instant) -> Self {
        self.poll_error_at = Some(at);
        self
    }

    pub fn is_grabbed(&self, d: Device) -> bool {
        self.grabbed.contains(&d)
    }
}

impl InputBackend for FakeBackend {
    fn try_grab(&mut self, device: Device) -> Result<GrabAttempt, BackendError> {
        if self.fail_grab == Some(device) {
            return Err(BackendError(format!("fallo simulado al capturar {device}")));
        }
        if self
            .busy_until
            .get(&device)
            .is_some_and(|&t| self.now.get() < t)
        {
            return Ok(GrabAttempt::Busy);
        }
        if !self.grabbed.contains(&device) {
            self.grabbed.push(device);
            self.log.borrow_mut().push(Call::Grabbed(device));
        }
        Ok(GrabAttempt::Grabbed)
    }

    fn ungrab(&mut self, device: Device) {
        if let Some(i) = self.grabbed.iter().position(|&d| d == device) {
            self.grabbed.remove(i);
            self.log.borrow_mut().push(Call::Ungrabbed(device));
        }
    }

    fn poll(&mut self, timeout: Duration) -> Result<Option<InputEvent>, BackendError> {
        let now = self.now.get();
        let until = now + timeout;
        if let Some(at) = self.poll_error_at
            && at <= until
        {
            self.now.set(at.max(now));
            return Err(BackendError("conexión perdida (simulada)".into()));
        }
        if let Some(&(at, _)) = self.events.front()
            && at <= until
        {
            let (_, ev) = self.events.pop_front().expect("hay evento");
            self.now.set(at.max(now));
            return Ok(Some(ev));
        }
        self.now.set(until);
        Ok(None)
    }
}
