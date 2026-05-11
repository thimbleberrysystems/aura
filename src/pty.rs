use crossterm::terminal::{disable_raw_mode, size as terminal_size};
use portable_pty::PtySize;
use std::time::Instant;
use termwiz::escape::parser::Parser as VteParser;
use termwiz::escape::Action;

/// PTY state machine modes and shared command state used by the PTY threads.
#[derive(Debug, Clone, PartialEq)]
pub enum Mode {
    Idle,
    Running,
    Passthrough,
}

pub struct CommandState {
    pub mode: Mode,
    pub last_pty_activity: Option<Instant>,
    pub captured: Vec<u8>,
    pub pending_cmd: String,
}

/// RAII guard that disables raw mode on drop when it was enabled.
pub struct RawGuard(pub bool);
impl Drop for RawGuard {
    fn drop(&mut self) {
        if self.0 {
            let _ = disable_raw_mode();
        }
    }
}

/// Strip ANSI/VT escape sequences using the termwiz parser, returning clean UTF-8 text.
pub fn strip_ansi(bytes: &[u8]) -> String {
    let mut parser = VteParser::new();
    let mut text = String::new();
    parser.parse(bytes, |action| match action {
        Action::Print(c) => text.push(c),
        Action::PrintString(s) => text.push_str(&s),
        _ => {}
    });
    text
}

/// Detect the current terminal size (columns x rows).
pub fn current_pty_size() -> PtySize {
    let (cols, rows) = terminal_size().unwrap_or((80, 24));
    PtySize { rows, cols, pixel_width: 0, pixel_height: 0 }
}
