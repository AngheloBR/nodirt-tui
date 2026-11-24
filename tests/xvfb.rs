//! Integración en un servidor X virtual (Xvfb). NUNCA toca la sesión real:
//! cada test arranca su propio Xvfb (`-displayfd`, display libre elegido por el
//! servidor) y todos los procesos reciben ese `DISPLAY` explícitamente.
//!
//! Requiere `Xvfb` y `xdotool`. Ejecutar con:
//!     cargo test --test xvfb -- --ignored
//!
//! `xdotool` usa XTEST: sus eventos entran como si fueran hardware y pasan por
//! los grabs (a diferencia de `--window`, que usa XSendEvent y los saltaría).

use std::{
    io::{BufRead, BufReader, Read},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use x11rb::{
    COPY_DEPTH_FROM_PARENT, CURRENT_TIME,
    connection::Connection,
    protocol::{
        Event,
        xproto::{
            ConnectionExt as _, CreateWindowAux, EventMask, GrabMode, GrabStatus, InputFocus,
            WindowClass,
        },
    },
    rust_connection::RustConnection,
};

// ── servidor virtual ────────────────────────────────────────────────────────

struct Xvfb {
    child: Child,
    display: String,
}

impl Xvfb {
    fn start() -> Self {
        let mut child = Command::new("Xvfb")
            .args([
                "-displayfd",
                "1",
                "-nolisten",
                "tcp",
                "-screen",
                "0",
                "1024x768x24",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("no se pudo arrancar Xvfb (¿instalado?)");
        // Xvfb escribe el número de display cuando ya acepta conexiones.
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let display = format!(":{}", line.trim());
        assert!(
            line.trim().parse::<u32>().is_ok(),
            "displayfd raro: {line:?}"
        );
        // Barrera de seguridad: jamás operar sobre la sesión real.
        if let Ok(real) = std::env::var("DISPLAY") {
            assert_ne!(display, real, "Xvfb devolvió el display de la sesión real");
        }
        Self { child, display }
    }
}

impl Drop for Xvfb {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn xdo(x: &Xvfb, args: &[&str]) {
    let st = Command::new("xdotool")
        .args(args)
        .env("DISPLAY", &x.display)
        .status()
        .expect("no se pudo ejecutar xdotool");
    assert!(st.success(), "xdotool {args:?} falló");
}

fn xmodmap(x: &Xvfb, exprs: &[&str]) {
    let mut cmd = Command::new("xmodmap");
    for e in exprs {
        cmd.args(["-e", e]);
    }
    let st = cmd
        .env("DISPLAY", &x.display)
        .status()
        .expect("no se pudo ejecutar xmodmap");
    assert!(st.success(), "xmodmap {exprs:?} falló");
}

/// Teclea `a` y hace clic con el puntero sobre el testigo.
fn poke(x: &Xvfb) {
    xdo(x, &["mousemove", "200", "200"]);
    xdo(x, &["key", "a"]);
    xdo(x, &["click", "1"]);
}

// ── segundo cliente: testigo ────────────────────────────────────────────────

#[derive(Debug, Default, PartialEq, Eq)]
struct Seen {
    keys: usize,
    buttons: usize,
}

/// Ventana a pantalla completa con el foco: registra teclas y clics que le llegan.
struct Witness {
    conn: RustConnection,
}

impl Witness {
    fn new(x: &Xvfb) -> Self {
        let (conn, screen) = x11rb::connect(Some(&x.display)).unwrap();
        let s = &conn.setup().roots[screen];
        let (root, w, h) = (s.root, s.width_in_pixels, s.height_in_pixels);
        let win = conn.generate_id().unwrap();
        let mask = EventMask::KEY_PRESS | EventMask::KEY_RELEASE | EventMask::BUTTON_PRESS;
        conn.create_window(
            COPY_DEPTH_FROM_PARENT,
            win,
            root,
            0,
            0,
            w,
            h,
            0,
            WindowClass::INPUT_OUTPUT,
            0,
            &CreateWindowAux::new().event_mask(mask),
        )
        .unwrap();
        conn.map_window(win).unwrap();
        conn.get_input_focus().unwrap().reply().unwrap(); // ida y vuelta: ya es visible
        conn.set_input_focus(InputFocus::PARENT, win, CURRENT_TIME)
            .unwrap();
        conn.get_input_focus().unwrap().reply().unwrap();
        let w = Self { conn };
        w.take(Duration::from_millis(100));
        w
    }

    /// Espera `settle` y devuelve lo recibido desde la última llamada.
    fn take(&self, settle: Duration) -> Seen {
        thread::sleep(settle);
        let mut seen = Seen::default();
        while let Some(ev) = self.conn.poll_for_event().unwrap() {
            match ev {
                Event::KeyPress(_) => seen.keys += 1,
                Event::ButtonPress(_) => seen.buttons += 1,
                _ => {}
            }
        }
        seen
    }

    /// Lo inyecta `poke` y comprueba qué llegó.
    fn poke(&self, x: &Xvfb) -> Seen {
        poke(x);
        self.take(Duration::from_millis(300))
    }
}

const ALL: Seen = Seen {
    keys: 1,
    buttons: 1,
};
const NONE: Seen = Seen {
    keys: 0,
    buttons: 0,
};

// ── nodirt ──────────────────────────────────────────────────────────────────

struct Nodirt {
    child: Child,
    out: Arc<Mutex<String>>,
    err: Arc<Mutex<String>>,
}

fn collect(r: impl Read + Send + 'static) -> Arc<Mutex<String>> {
    let buf = Arc::new(Mutex::new(String::new()));
    let b = Arc::clone(&buf);
    thread::spawn(move || {
        let mut r = r;
        let mut chunk = [0u8; 512];
        while let Ok(n) = r.read(&mut chunk) {
            if n == 0 {
                break;
            }
            b.lock()
                .unwrap()
                .push_str(&String::from_utf8_lossy(&chunk[..n]));
        }
    });
    buf
}

impl Nodirt {
    fn spawn(x: &Xvfb, args: &[&str]) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_nodirt"))
            .args(args)
            .env("DISPLAY", &x.display)
            .env_remove("XDG_SESSION_TYPE")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let out = collect(child.stdout.take().unwrap());
        let err = collect(child.stderr.take().unwrap());
        Self { child, out, err }
    }

    fn out(&self) -> String {
        self.out.lock().unwrap().clone()
    }

    fn err(&self) -> String {
        self.err.lock().unwrap().clone()
    }

    /// Espera a que la salida contenga `needle`.
    fn wait_for(&self, needle: &str, timeout: Duration) {
        let t0 = Instant::now();
        while !self.out().contains(needle) {
            assert!(
                t0.elapsed() < timeout,
                "no apareció {needle:?} en {timeout:?}\nstdout: {}\nstderr: {}",
                self.out(),
                self.err()
            );
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn wait_locked(&self) {
        // 3 s de cuenta atrás + margen.
        self.wait_for("◉ bloqueado", Duration::from_secs(8));
        // Primer frame impreso justo tras el grab: deja que se asiente.
        thread::sleep(Duration::from_millis(150));
    }

    fn wait_exit(&mut self, timeout: Duration) -> ExitStatus {
        let t0 = Instant::now();
        loop {
            if let Some(st) = self.child.try_wait().unwrap() {
                thread::sleep(Duration::from_millis(50)); // vaciar lectores
                return st;
            }
            assert!(
                t0.elapsed() < timeout,
                "nodirt no terminó en {timeout:?}\nstdout: {}",
                self.out()
            );
            thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Nodirt {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ── escenarios ──────────────────────────────────────────────────────────────

#[test]
#[ignore = "requiere Xvfb y xdotool: cargo test --test xvfb -- --ignored"]
fn temporizador_bloquea_y_libera() {
    let x = Xvfb::start();
    let w = Witness::new(&x);
    assert_eq!(
        w.poke(&x),
        ALL,
        "control: sin bloqueo el testigo recibe todo"
    );

    let mut n = Nodirt::spawn(&x, &["-t", "3s"]);
    n.wait_locked();
    assert_eq!(w.poke(&x), NONE, "con bloqueo NO debe llegar nada");
    let st = n.wait_exit(Duration::from_secs(10));
    assert_eq!(st.code(), Some(0), "stdout: {}", n.out());
    assert!(n.out().contains("tiempo cumplido"), "{}", n.out());
    assert!(
        !n.out().contains("no soporta DetectableAutoRepeat"),
        "Xvfb debería soportar DetectableAutoRepeat"
    );
    assert_eq!(w.poke(&x), ALL, "tras el temporizador SÍ debe llegar");
}

#[test]
#[ignore = "requiere Xvfb y xdotool: cargo test --test xvfb -- --ignored"]
fn temporizador_con_tecla_pulsada_espera_a_que_se_suelte() {
    let x = Xvfb::start();
    let w = Witness::new(&x);
    let mut n = Nodirt::spawn(&x, &["-t", "2s"]);
    n.wait_locked();
    // Un paño apoyado sobre la 'a' mientras termina el temporizador.
    xdo(&x, &["keydown", "a"]);
    n.wait_for("suelta las teclas", Duration::from_secs(5));
    assert!(
        n.child.try_wait().unwrap().is_none(),
        "no debe liberar con una tecla pulsada"
    );
    thread::sleep(Duration::from_millis(500));
    xdo(&x, &["keyup", "a"]);
    let st = n.wait_exit(Duration::from_secs(3));
    assert_eq!(st.code(), Some(0), "stdout: {}", n.out());
    assert_eq!(
        w.poke(&x),
        ALL,
        "tras el temporizador SÍ llega, sin autorepeat de la 'a' colado"
    );
}

#[test]
#[ignore = "requiere Xvfb y xdotool: cargo test --test xvfb -- --ignored"]
fn combo_mantenido_libera_y_soltar_antes_no() {
    let x = Xvfb::start();
    let w = Witness::new(&x);
    let mut n = Nodirt::spawn(&x, &["-t", "60s"]);
    n.wait_locked();

    // Mantener solo 1 s: no desbloquea.
    xdo(&x, &["keydown", "Escape"]);
    thread::sleep(Duration::from_secs(1));
    xdo(&x, &["keyup", "Escape"]);
    thread::sleep(Duration::from_millis(500));
    assert!(
        n.child.try_wait().unwrap().is_none(),
        "soltar antes NO debe desbloquear"
    );
    assert_eq!(w.poke(&x), NONE);

    // Mantener 3,5 s: desbloquea.
    let t0 = Instant::now();
    xdo(&x, &["keydown", "Escape"]);
    thread::sleep(Duration::from_millis(3500));
    // Combo ya completo (3 s), pero la tecla sigue pulsada: debe seguir capturando
    // hasta que se suelte, para que su autorepeat no llegue al testigo.
    assert!(
        n.child.try_wait().unwrap().is_none(),
        "no debe liberar con la tecla aún pulsada"
    );
    assert!(n.out().contains("suelta Escape"), "{}", n.out());
    xdo(&x, &["keyup", "Escape"]);
    let st = n.wait_exit(Duration::from_secs(5));
    assert!(t0.elapsed() >= Duration::from_millis(3500));
    assert_eq!(st.code(), Some(0), "stdout: {}", n.out());
    assert!(n.out().contains("desbloqueado con Escape"), "{}", n.out());
    assert!(t0.elapsed() < Duration::from_secs(6));
    assert_eq!(
        w.poke(&x),
        ALL,
        "tras el combo SÍ debe llegar, y sin Escape de autorepeat colados"
    );
}

#[test]
#[ignore = "requiere Xvfb y xdotool: cargo test --test xvfb -- --ignored"]
fn kill_9_libera_los_grabs() {
    let x = Xvfb::start();
    let w = Witness::new(&x);
    let mut n = Nodirt::spawn(&x, &["-t", "60s"]);
    n.wait_locked();
    assert_eq!(w.poke(&x), NONE);

    n.child.kill().unwrap(); // SIGKILL: no corre ningún Drop ni hook
    let st = n.child.wait().unwrap();
    assert_eq!(st.code(), None, "debe morir por señal");
    assert_eq!(
        w.poke(&x),
        ALL,
        "el servidor X debe soltar los grabs al cerrarse el socket"
    );
}

#[test]
#[ignore = "requiere Xvfb y xdotool: cargo test --test xvfb -- --ignored"]
fn sigterm_libera_y_sale_con_130() {
    let x = Xvfb::start();
    let w = Witness::new(&x);
    let mut n = Nodirt::spawn(&x, &["-t", "60s"]);
    n.wait_locked();
    assert_eq!(w.poke(&x), NONE);

    let pid = n.child.id().to_string();
    assert!(
        Command::new("kill")
            .args(["-TERM", &pid])
            .status()
            .unwrap()
            .success()
    );
    let st = n.wait_exit(Duration::from_secs(3));
    assert_eq!(st.code(), Some(130), "stdout: {}", n.out());
    assert_eq!(w.poke(&x), ALL);
}

#[test]
#[ignore = "requiere Xvfb y xdotool: cargo test --test xvfb -- --ignored"]
fn sin_mouse_solo_bloquea_el_teclado() {
    let x = Xvfb::start();
    let w = Witness::new(&x);
    let mut n = Nodirt::spawn(&x, &["-t", "3s", "--no-mouse"]);
    n.wait_locked();
    assert_eq!(
        w.poke(&x),
        Seen {
            keys: 0,
            buttons: 1
        },
        "teclas bloqueadas, clics libres"
    );
    assert_eq!(n.wait_exit(Duration::from_secs(10)).code(), Some(0));
    assert_eq!(w.poke(&x), ALL);
}

#[test]
#[ignore = "requiere Xvfb y xdotool: cargo test --test xvfb -- --ignored"]
fn teclado_ocupado_por_otro_cliente_da_codigo_3() {
    let x = Xvfb::start();
    // "Otro programa" (un menú, rofi…) con el teclado capturado.
    let (other, screen) = x11rb::connect(Some(&x.display)).unwrap();
    let root = other.setup().roots[screen].root;
    let r = other
        .grab_keyboard(false, root, CURRENT_TIME, GrabMode::ASYNC, GrabMode::ASYNC)
        .unwrap()
        .reply()
        .unwrap();
    assert_eq!(r.status, GrabStatus::SUCCESS);

    let t0 = Instant::now();
    let mut n = Nodirt::spawn(&x, &["-t", "60s"]);
    let st = n.wait_exit(Duration::from_secs(12));
    let took = t0.elapsed();
    assert_eq!(st.code(), Some(3), "stderr: {}", n.err());
    assert!(n.err().contains("el teclado"), "{}", n.err());
    // 3 s de cuenta atrás + 3 s de reintentos (no infinito).
    assert!(
        took >= Duration::from_secs(6) && took < Duration::from_secs(9),
        "{took:?}"
    );
    // Todo o nada: no se quedó con el puntero.
    let p = other
        .grab_pointer(
            false,
            root,
            EventMask::BUTTON_PRESS,
            GrabMode::ASYNC,
            GrabMode::ASYNC,
            x11rb::NONE,
            x11rb::NONE,
            CURRENT_TIME,
        )
        .unwrap()
        .reply()
        .unwrap();
    assert_eq!(p.status, GrabStatus::SUCCESS, "el puntero quedó libre");
}

#[test]
#[ignore = "requiere Xvfb, xdotool y xmodmap: cargo test --test xvfb -- --ignored"]
fn cambio_del_mapa_de_teclado_durante_el_bloqueo() {
    let x = Xvfb::start();
    let w = Witness::new(&x);
    let mut n = Nodirt::spawn(&x, &["-t", "60s", "--unlock-key", "F12"]);
    n.wait_locked();
    // Intercambia F11 y F12 con el bloqueo activo (keycodes evdev 95 y 96).
    xmodmap(&x, &["keycode 95 = F12", "keycode 96 = F11"]);
    thread::sleep(Duration::from_millis(300));
    // Algunos Xvfb (p. ej. Fedora 45) ignoran los cambios de mapa: sin cambio
    // no hay nada que probar, y se dice en voz alta en vez de pasar en silencio.
    let map = Command::new("xmodmap")
        .arg("-pke")
        .env("DISPLAY", &x.display)
        .output()
        .unwrap();
    let map = String::from_utf8_lossy(&map.stdout);
    let k95 = map
        .lines()
        .find(|l| l.starts_with("keycode  95 "))
        .unwrap_or("");
    if !k95.contains("= F12") {
        eprintln!("OMITIDO: este Xvfb no aplicó el cambio de mapa ({k95:?})");
        return;
    }
    // xdotool busca F12 en el mapa nuevo: pulsa el keycode 95.
    xdo(&x, &["keydown", "F12"]);
    thread::sleep(Duration::from_millis(3500));
    xdo(&x, &["keyup", "F12"]);
    let st = n.wait_exit(Duration::from_secs(5));
    assert_eq!(st.code(), Some(0), "stdout: {}", n.out());
    assert!(n.out().contains("desbloqueado con F12"), "{}", n.out());
    assert_eq!(w.poke(&x), ALL);
}
