//! Backend X11 hablando el protocolo directamente (x11rb `RustConnection`, Rust puro:
//! sin libX11/libxcb).
//!
//! - Teclado: `GrabKeyboard(root, owner_events=false)`. Las teclas llegan a ESTE cliente
//!   como `KeyPress`/`KeyRelease`; ninguna otra ventana las recibe.
//! - Mouse/touchpad: `GrabPointer(root, owner_events=false)` con máscara de botones
//!   (sin movimiento, para no inundar la cola). Lo no seleccionado se descarta.
//! - Autorepeat: `XkbPerClientFlags(DetectableAutoRepeat)` solo para esta conexión
//!   (no cambia nada global, no hay que restaurar). Si el servidor no lo soporta,
//!   se fusiona el par release+press con mismo keycode y timestamp.
//! - Ctrl+Alt+Fn (cambio de VT) lo procesa el servidor antes de entregar eventos:
//!   los grabs no lo bloquean.
//! - Si cambia el mapa del teclado (XKB `MapNotify`/`NewKeyboardNotify`, o
//!   `MappingNotify` sin XKB), se vuelve a leer.
//! - Si el proceso muere, al cerrarse el socket el servidor X suelta los grabs.

use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use x11rb::{
    CURRENT_TIME, NONE,
    connection::Connection,
    errors::{ConnectionError, ReplyError},
    protocol::{
        Event,
        xkb::{self, ConnectionExt as _, EventType, MapPart, PerClientFlag, SelectEventsAux},
        xproto::{ConnectionExt as _, EventMask, GrabMode, GrabStatus, Mapping, Window},
    },
    rust_connection::RustConnection,
};

use super::{BackendError, Device, GrabAttempt, InputBackend, InputEvent};
use crate::keys::Key;

/// Granularidad de espera de `poll` (la conexión no expone espera con timeout).
const POLL_STEP: Duration = Duration::from_millis(5);

/// Conexión activa, para que el panic hook pueda soltar los grabs aunque el
/// unwind no llegue a ejecutar `Drop` (p. ej. panic dentro de otro `Drop`).
static PANIC_CONN: Mutex<Option<Arc<RustConnection>>> = Mutex::new(None);

#[derive(Debug, thiserror::Error)]
pub enum ConnectError {
    #[error("no hay $DISPLAY · esta herramienta es solo para X11")]
    NoDisplay,
    #[error("sesión Wayland: los grabs de X11 no bloquean las apps nativas de Wayland")]
    Wayland,
    #[error("no pude conectar con el servidor X11: {0}")]
    Connect(String),
    #[error("error de X11 al preparar el backend: {0}")]
    Setup(String),
}

fn be<E: std::fmt::Display>(e: E) -> BackendError {
    BackendError(format!("X11: {e}"))
}

pub struct X11Backend {
    conn: Arc<RustConnection>,
    root: Window,
    keymap: HashMap<u8, Key>,
    detectable_repeat: bool,
    pending: VecDeque<Event>,
    keyboard: bool,
    pointer: bool,
}

impl X11Backend {
    pub fn connect() -> Result<Self, ConnectError> {
        if std::env::var("XDG_SESSION_TYPE").is_ok_and(|t| t == "wayland") {
            return Err(ConnectError::Wayland);
        }
        if std::env::var_os("DISPLAY").is_none_or(|d| d.is_empty()) {
            return Err(ConnectError::NoDisplay);
        }
        let (conn, screen) =
            x11rb::connect(None).map_err(|e| ConnectError::Connect(e.to_string()))?;
        let setup_err = |e: &dyn std::fmt::Display| ConnectError::Setup(e.to_string());
        let root = conn.setup().roots[screen].root;
        let keymap = load_keymap(&conn).map_err(|e| setup_err(&e))?;
        let detectable_repeat = enable_detectable_repeat(&conn);
        watch_keymap(&conn);
        let conn = Arc::new(conn);
        if let Ok(mut g) = PANIC_CONN.lock() {
            *g = Some(Arc::clone(&conn));
        }
        Ok(Self {
            conn,
            root,
            keymap,
            detectable_repeat,
            pending: VecDeque::new(),
            keyboard: false,
            pointer: false,
        })
    }

    pub fn detectable_repeat(&self) -> bool {
        self.detectable_repeat
    }

    fn next_event(&mut self) -> Result<Option<Event>, BackendError> {
        if let Some(e) = self.pending.pop_front() {
            return Ok(Some(e));
        }
        self.conn.poll_for_event().map_err(be)
    }

    fn key(&self, keycode: u8) -> Key {
        // Keycode sin keysym: valor imposible como keysym para que nunca coincida.
        self.keymap.get(&keycode).copied().unwrap_or(Key(0))
    }

    /// Traduce un evento; `None` si no interesa (botones del mouse, etc.).
    fn translate(&mut self, ev: Event) -> Result<Option<InputEvent>, BackendError> {
        match ev {
            Event::KeyPress(e) => Ok(Some(InputEvent::KeyPress(self.key(e.detail)))),
            Event::KeyRelease(e) => {
                if !self.detectable_repeat {
                    // Autorepeat clásico: release+press con el mismo keycode y tiempo.
                    match self.conn.poll_for_event().map_err(be)? {
                        Some(Event::KeyPress(p)) if p.detail == e.detail && p.time == e.time => {
                            return Ok(Some(InputEvent::KeyPress(self.key(p.detail))));
                        }
                        Some(other) => self.pending.push_back(other),
                        None => {}
                    }
                }
                Ok(Some(InputEvent::KeyRelease(self.key(e.detail))))
            }
            // Un error asíncrono del servidor: abortar es lo seguro (Drop suelta todo).
            // La distribución cambió (setxkbmap, xmodmap…): releer el mapa para
            // que la tecla de salida se siga reconociendo.
            // Con XKB activo el servidor no manda el MappingNotify del núcleo:
            // avisa con estos eventos (suscritos en `watch_keymap`).
            Event::XkbMapNotify(_) | Event::XkbNewKeyboardNotify(_) => {
                self.keymap = load_keymap(&self.conn).map_err(be)?;
                Ok(None)
            }
            Event::MappingNotify(e) if e.request == Mapping::KEYBOARD => {
                self.keymap = load_keymap(&self.conn).map_err(be)?;
                Ok(None)
            }
            Event::Error(e) => Err(be(format!("{e:?}"))),
            _ => Ok(None),
        }
    }
}

fn load_keymap(conn: &RustConnection) -> Result<HashMap<u8, Key>, ReplyError> {
    let (min, max) = (conn.setup().min_keycode, conn.setup().max_keycode);
    let reply = conn.get_keyboard_mapping(min, max - min + 1)?.reply()?;
    let per = usize::from(reply.keysyms_per_keycode.max(1));
    Ok(reply
        .keysyms
        .chunks(per)
        .zip(min..=max)
        .filter(|(syms, _)| syms[0] != 0)
        .map(|(syms, kc)| (kc, Key(syms[0])))
        .collect())
}

/// `true` si el servidor aceptó DetectableAutoRepeat para esta conexión.
fn enable_detectable_repeat(conn: &RustConnection) -> bool {
    let ok = conn
        .xkb_use_extension(1, 0)
        .ok()
        .and_then(|c| c.reply().ok())
        .is_some_and(|r| r.supported);
    if !ok {
        return false;
    }
    let flag = PerClientFlag::DETECTABLE_AUTO_REPEAT;
    conn.xkb_per_client_flags(
        xkb::ID::USE_CORE_KBD.into(),
        flag,
        flag,
        0u32.into(),
        0u32.into(),
        0u32.into(),
    )
    .ok()
    .and_then(|c| c.reply().ok())
    .is_some_and(|r| u32::from(r.value) & u32::from(flag) != 0)
}

/// Pide los avisos de XKB cuando cambia el mapa de teclas (p. ej. `setxkbmap`).
/// Si falla no es grave: solo no se notarían cambios de distribución en pleno bloqueo.
fn watch_keymap(conn: &RustConnection) {
    let events = EventType::NEW_KEYBOARD_NOTIFY | EventType::MAP_NOTIFY;
    let _ = conn.xkb_select_events(
        xkb::ID::USE_CORE_KBD.into(),
        0u16.into(),
        events,
        MapPart::KEY_SYMS,
        MapPart::KEY_SYMS,
        &SelectEventsAux::new(),
    );
}

fn attempt(status: GrabStatus) -> Result<GrabAttempt, BackendError> {
    match status {
        GrabStatus::SUCCESS => Ok(GrabAttempt::Grabbed),
        GrabStatus::ALREADY_GRABBED | GrabStatus::FROZEN => Ok(GrabAttempt::Busy),
        other => Err(be(format!("grab rechazado: {other:?}"))),
    }
}

impl InputBackend for X11Backend {
    fn try_grab(&mut self, device: Device) -> Result<GrabAttempt, BackendError> {
        let status = match device {
            Device::Keyboard => {
                self.conn
                    .grab_keyboard(
                        false,
                        self.root,
                        CURRENT_TIME,
                        GrabMode::ASYNC,
                        GrabMode::ASYNC,
                    )
                    .map_err(be)?
                    .reply()
                    .map_err(be)?
                    .status
            }
            Device::Pointer => {
                self.conn
                    .grab_pointer(
                        false,
                        self.root,
                        EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE,
                        GrabMode::ASYNC,
                        GrabMode::ASYNC,
                        NONE,
                        NONE,
                        CURRENT_TIME,
                    )
                    .map_err(be)?
                    .reply()
                    .map_err(be)?
                    .status
            }
        };
        let res = attempt(status)?;
        if res == GrabAttempt::Grabbed {
            match device {
                Device::Keyboard => self.keyboard = true,
                Device::Pointer => self.pointer = true,
            }
        }
        Ok(res)
    }

    fn ungrab(&mut self, device: Device) {
        let r: Result<(), ConnectionError> = match device {
            Device::Keyboard if self.keyboard => {
                self.keyboard = false;
                self.conn.ungrab_keyboard(CURRENT_TIME).map(drop)
            }
            Device::Pointer if self.pointer => {
                self.pointer = false;
                self.conn.ungrab_pointer(CURRENT_TIME).map(drop)
            }
            _ => return,
        };
        // Sin flush el ungrab podría quedarse en el buffer de salida.
        let _ = r.and_then(|()| self.conn.flush());
    }

    fn poll(&mut self, timeout: Duration) -> Result<Option<InputEvent>, BackendError> {
        let deadline = Instant::now() + timeout;
        loop {
            while let Some(ev) = self.next_event()? {
                if let Some(ie) = self.translate(ev)? {
                    return Ok(Some(ie));
                }
            }
            let now = Instant::now();
            if now >= deadline {
                return Ok(None);
            }
            thread::sleep(POLL_STEP.min(deadline - now));
        }
    }
}

impl Drop for X11Backend {
    fn drop(&mut self) {
        self.ungrab(Device::Pointer);
        self.ungrab(Device::Keyboard);
        if let Ok(mut g) = PANIC_CONN.lock() {
            *g = None;
        }
    }
}

/// Encadena un panic hook que suelta los grabs antes que nada.
/// Redundante con `Drop` (unwind) y con el cierre del socket; es la tercera red.
pub fn install_panic_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // try_lock: si el panic ocurrió con el mutex tomado, no bloquear.
        if let Ok(g) = PANIC_CONN.try_lock()
            && let Some(conn) = g.as_ref()
        {
            let _ = conn.ungrab_keyboard(CURRENT_TIME);
            let _ = conn.ungrab_pointer(CURRENT_TIME);
            let _ = conn.flush();
        }
        prev(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estados_de_grab() {
        assert_eq!(attempt(GrabStatus::SUCCESS), Ok(GrabAttempt::Grabbed));
        assert_eq!(attempt(GrabStatus::ALREADY_GRABBED), Ok(GrabAttempt::Busy));
        assert_eq!(attempt(GrabStatus::FROZEN), Ok(GrabAttempt::Busy));
        assert!(attempt(GrabStatus::NOT_VIEWABLE).is_err());
        assert!(attempt(GrabStatus::INVALID_TIME).is_err());
    }
}
