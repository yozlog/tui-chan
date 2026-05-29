use std::sync::mpsc;
use std::time::Duration;
use std::thread;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    Char(char),
    Ctrl(char),
    Alt(char),
    Backspace,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    BackTab,
    Delete,
    Insert,
    Esc,
    F(u8),
    Null,
}

pub(crate) struct Events {
    rx: mpsc::Receiver<Event<Key>>,
    tx: mpsc::Sender<Event<Key>>,
}

pub(crate) enum Event<I> {
    Input(I),
    Tick,
}

impl Events {
    pub(crate) fn new() -> Events {
        Events::with_config(Config::default())
    }

    fn with_config(config: Config) -> Events {
        let (tx, rx) = mpsc::channel();

        let _input_handle = {
            let tx = tx.clone();
            thread::spawn(move || {
                loop {
                    if let Ok(true) = crossterm::event::poll(Duration::from_millis(50)) {
                        if let Ok(crossterm::event::Event::Key(key_event)) = crossterm::event::read() {
                            if key_event.kind == crossterm::event::KeyEventKind::Press {
                                if let Some(key) = to_custom_key(key_event) {
                                    if tx.send(Event::Input(key)).is_err() {
                                        return;
                                    }
                                    if key == config.exit_key {
                                        return;
                                    }
                                }
                            }
                        }
                    }
                }
            })
        };

        let _tick_handle = {
            let tx = tx.clone();
            thread::spawn(move || loop {
                if tx.send(Event::Tick).is_err() {
                    break;
                }
                thread::sleep(config.tick_rate);
            })
        };

        Events {
            rx,
            tx,
        }
    }

    pub(crate) fn tx(&self) -> mpsc::Sender<Event<Key>> {
        self.tx.clone()
    }

    pub(crate) fn next(&self) -> Result<Event<Key>, mpsc::RecvError> {
        self.rx.recv()
    }


}

#[derive(Debug, Clone, Copy)]
struct Config {
    exit_key: Key,
    tick_rate: Duration,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            exit_key: Key::Char('q'),
            tick_rate: Duration::from_millis(250),
        }
    }
}

fn to_custom_key(event: crossterm::event::KeyEvent) -> Option<Key> {
    use crossterm::event::{KeyCode, KeyModifiers};

    let is_ctrl = event.modifiers.contains(KeyModifiers::CONTROL);
    let is_alt = event.modifiers.contains(KeyModifiers::ALT);

    match event.code {
        KeyCode::Char(c) => {
            if is_ctrl {
                Some(Key::Ctrl(c))
            } else if is_alt {
                Some(Key::Alt(c))
            } else {
                Some(Key::Char(c))
            }
        }
        KeyCode::Backspace => Some(Key::Backspace),
        KeyCode::Left => Some(Key::Left),
        KeyCode::Right => Some(Key::Right),
        KeyCode::Up => Some(Key::Up),
        KeyCode::Down => Some(Key::Down),
        KeyCode::Home => Some(Key::Home),
        KeyCode::End => Some(Key::End),
        KeyCode::PageUp => Some(Key::PageUp),
        KeyCode::PageDown => Some(Key::PageDown),
        KeyCode::BackTab => Some(Key::BackTab),
        KeyCode::Delete => Some(Key::Delete),
        KeyCode::Insert => Some(Key::Insert),
        KeyCode::Esc => Some(Key::Esc),
        KeyCode::F(num) => Some(Key::F(num)),
        KeyCode::Null => Some(Key::Null),
        KeyCode::Enter => Some(Key::Char('\n')),
        KeyCode::Tab => Some(Key::Char('\t')),
        _ => None,
    }
}
