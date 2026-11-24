//! Estado de la pantalla de configuración. Puro: recibe teclas de crossterm y
//! devuelve una acción; no toca la terminal ni X11.

use std::time::Duration;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::{
    duration,
    keys::Key,
    lock::Config,
    timer::{DEFAULT_LOCK, PRESETS},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Preset(usize),
    Custom(Duration),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Ok,
    Warn,
    Err,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    None,
    Start,
    Quit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Input {
    pub buf: String,
    pub error: Option<String>,
}

pub struct App {
    pub choice: Choice,
    pub mouse: bool,
    pub unlock_key: Key,
    pub unlock_hold: Duration,
    pub input: Option<Input>,
    pub message: Option<(String, Level)>,
}

impl App {
    pub fn new(
        time: Option<Duration>,
        mouse: bool,
        unlock_key: Key,
        unlock_hold: Duration,
    ) -> Self {
        let d = time.unwrap_or(DEFAULT_LOCK);
        let choice = PRESETS
            .iter()
            .position(|&p| p == d)
            .map_or(Choice::Custom(d), Choice::Preset);
        Self {
            choice,
            mouse,
            unlock_key,
            unlock_hold,
            input: None,
            message: None,
        }
    }

    pub fn duration(&self) -> Duration {
        match self.choice {
            Choice::Preset(i) => PRESETS[i],
            Choice::Custom(d) => d,
        }
    }

    pub fn config(&self) -> Config {
        Config {
            duration: self.duration(),
            unlock_key: self.unlock_key,
            unlock_hold: self.unlock_hold,
            mouse: self.mouse,
        }
    }

    pub fn set_message(&mut self, text: impl Into<String>, level: Level) {
        self.message = Some((text.into(), level));
    }

    /// Índice seleccionado en la fila de opciones (presets + "otro").
    pub fn selected(&self) -> usize {
        match self.choice {
            Choice::Preset(i) => i,
            Choice::Custom(_) => PRESETS.len(),
        }
    }

    pub fn on_key(&mut self, k: KeyEvent) -> Action {
        let ctrl_c = k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c');
        if self.input.is_some() {
            return self.on_input_key(k, ctrl_c);
        }
        if ctrl_c {
            return Action::Quit;
        }
        self.message = None;
        match k.code {
            KeyCode::Char('q') | KeyCode::Esc => Action::Quit,
            KeyCode::Enter | KeyCode::Char(' ') => Action::Start,
            KeyCode::Left | KeyCode::Char('h') => {
                self.move_preset(-1);
                Action::None
            }
            KeyCode::Right | KeyCode::Char('l') => {
                self.move_preset(1);
                Action::None
            }
            KeyCode::Char(c @ '1'..='5') => {
                self.choice = Choice::Preset(usize::from(c as u8 - b'1'));
                Action::None
            }
            KeyCode::Char('t' | 'e') => {
                self.input = Some(Input {
                    buf: String::new(),
                    error: None,
                });
                Action::None
            }
            KeyCode::Char('m') => {
                self.mouse = !self.mouse;
                Action::None
            }
            _ => Action::None,
        }
    }

    fn move_preset(&mut self, delta: isize) {
        let i = match self.choice {
            Choice::Preset(i) => i as isize + delta,
            // Desde "otro": ← va al último preset, → al primero.
            Choice::Custom(_) if delta < 0 => PRESETS.len() as isize - 1,
            Choice::Custom(_) => 0,
        };
        self.choice = Choice::Preset(i.clamp(0, PRESETS.len() as isize - 1) as usize);
    }

    fn on_input_key(&mut self, k: KeyEvent, ctrl_c: bool) -> Action {
        let Some(input) = self.input.as_mut() else {
            return Action::None;
        };
        if ctrl_c || k.code == KeyCode::Esc {
            self.input = None;
            return Action::None;
        }
        match k.code {
            KeyCode::Enter => match duration::parse_lock(&input.buf) {
                Ok(d) => {
                    self.choice = PRESETS
                        .iter()
                        .position(|&p| p == d)
                        .map_or(Choice::Custom(d), Choice::Preset);
                    self.input = None;
                }
                Err(e) => input.error = Some(e.to_string()),
            },
            KeyCode::Backspace => {
                input.buf.pop();
                input.error = None;
            }
            KeyCode::Char(c) if input.buf.len() < 12 && (c.is_ascii_alphanumeric() || c == ' ') => {
                input.buf.push(c);
                input.error = None;
            }
            _ => {}
        }
        Action::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, KeyModifiers::NONE)
    }

    fn app() -> App {
        App::new(None, true, Key::ESCAPE, Duration::from_secs(3))
    }

    fn type_str(a: &mut App, s: &str) {
        for c in s.chars() {
            a.on_key(key(KeyCode::Char(c)));
        }
    }

    #[test]
    fn por_defecto_un_minuto() {
        let a = app();
        assert_eq!(a.choice, Choice::Preset(1));
        assert_eq!(a.duration(), Duration::from_secs(60));
    }

    #[test]
    fn duracion_inicial_de_la_cli() {
        let a = App::new(
            Some(Duration::from_secs(300)),
            true,
            Key::ESCAPE,
            Duration::from_secs(3),
        );
        assert_eq!(a.choice, Choice::Preset(3));
        let a = App::new(
            Some(Duration::from_secs(90)),
            true,
            Key::ESCAPE,
            Duration::from_secs(3),
        );
        assert_eq!(a.choice, Choice::Custom(Duration::from_secs(90)));
    }

    #[test]
    fn flechas_y_numeros() {
        let mut a = app();
        a.on_key(key(KeyCode::Right));
        assert_eq!(a.duration(), Duration::from_secs(120));
        for _ in 0..10 {
            a.on_key(key(KeyCode::Right));
        }
        assert_eq!(a.duration(), Duration::from_secs(600)); // tope en el último
        a.on_key(key(KeyCode::Char('1')));
        assert_eq!(a.duration(), Duration::from_secs(30));
        a.on_key(key(KeyCode::Left));
        assert_eq!(a.duration(), Duration::from_secs(30)); // tope en el primero
    }

    #[test]
    fn entrada_libre_valida() {
        let mut a = app();
        a.on_key(key(KeyCode::Char('t')));
        type_str(&mut a, "2m30s");
        a.on_key(key(KeyCode::Enter));
        assert!(a.input.is_none());
        assert_eq!(a.choice, Choice::Custom(Duration::from_secs(150)));
        assert_eq!(a.selected(), PRESETS.len());
    }

    #[test]
    fn entrada_libre_invalida_muestra_error_y_no_cambia() {
        let mut a = app();
        a.on_key(key(KeyCode::Char('t')));
        type_str(&mut a, "90");
        a.on_key(key(KeyCode::Enter));
        let err = a.input.as_ref().unwrap().error.clone().unwrap();
        assert!(err.contains("falta la unidad"), "{err}");
        assert_eq!(a.duration(), Duration::from_secs(60));
        // Tope de 60 min.
        a.on_key(key(KeyCode::Backspace));
        a.on_key(key(KeyCode::Backspace));
        type_str(&mut a, "2h");
        a.on_key(key(KeyCode::Enter));
        assert!(
            a.input
                .as_ref()
                .unwrap()
                .error
                .as_ref()
                .unwrap()
                .contains("máximo")
        );
        // Esc cierra sin aplicar.
        a.on_key(key(KeyCode::Esc));
        assert!(a.input.is_none());
        assert_eq!(a.duration(), Duration::from_secs(60));
    }

    #[test]
    fn entrada_libre_que_coincide_con_preset() {
        let mut a = app();
        a.on_key(key(KeyCode::Char('t')));
        type_str(&mut a, "5m");
        a.on_key(key(KeyCode::Enter));
        assert_eq!(a.choice, Choice::Preset(3));
    }

    #[test]
    fn las_teclas_de_la_entrada_no_disparan_acciones() {
        let mut a = app();
        a.on_key(key(KeyCode::Char('t')));
        // 'q' y 'm' son texto dentro de la entrada, no salir/mouse.
        assert_eq!(a.on_key(key(KeyCode::Char('q'))), Action::None);
        assert_eq!(a.on_key(key(KeyCode::Char('m'))), Action::None);
        assert!(a.mouse);
        assert_eq!(a.input.as_ref().unwrap().buf, "qm");
    }

    #[test]
    fn mouse_empezar_y_salir() {
        let mut a = app();
        a.on_key(key(KeyCode::Char('m')));
        assert!(!a.config().mouse);
        assert_eq!(a.on_key(key(KeyCode::Enter)), Action::Start);
        assert_eq!(a.on_key(key(KeyCode::Char('q'))), Action::Quit);
        assert_eq!(a.on_key(key(KeyCode::Esc)), Action::Quit);
        assert_eq!(
            a.on_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Action::Quit
        );
    }
}
