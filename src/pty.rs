use crossterm::terminal::{disable_raw_mode, size as terminal_size};
use libc;
use portable_pty::{MasterPty, PtySize};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use termwiz::escape::parser::Parser as VteParser;
use termwiz::escape::Action;
use tokio::sync::mpsc;
use tracing::debug;

/// A complete command cycle captured from the PTY, ready for the pipeline.
pub struct CapturedCommand {
    pub cmd: String,
    pub bytes: Vec<u8>,
    pub prompt: Vec<u8>,
}

/// PTY state machine modes.
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

/// Stage 1 of the pipeline: owns all PTY I/O for the session.
///
/// Internally spawns a stdin reader thread and a PTY reader thread:
/// - `Idle` / `Passthrough`: raw PTY bytes go directly to `display_tx`.
/// - `Running` + shell returns to foreground (via `tcgetpgrp` / `process_group_leader`)
///   + brief PTY silence: captured bytes are packaged as a `CapturedCommand`
///   and sent to `cmd_tx` for the pipeline to process.
///
/// Falls back to a 200 ms idle-timeout if `process_group_leader` is not
/// implemented by the PTY backend.
///
/// Returns when the PTY reader closes (i.e. the shell has exited).
pub async fn capture_task(
    mut pty_reader: Box<dyn Read + Send>,
    pty_writer: Box<dyn Write + Send>,
    display_tx: mpsc::Sender<Vec<u8>>,
    cmd_tx: mpsc::Sender<CapturedCommand>,
    master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    shell_pgid: libc::pid_t,
) {
    let shared = Arc::new(Mutex::new(CommandState {
        mode: Mode::Idle,
        last_pty_activity: None,
        captured: Vec::new(),
        pending_cmd: String::new(),
    }));

    // ── Stdin reader thread ───────────────────────────────────────────────────
    // Reads raw keystrokes; forwards to PTY writer; tracks command boundaries.
    let (stdin_tx, mut stdin_rx) = mpsc::channel::<Vec<u8>>(64);
    let shared_stdin = Arc::clone(&shared);
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match std::io::stdin().read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let data = &buf[..n];
                    for &b in data {
                        if b == b'\n' || b == b'\r' {
                            let mut s = shared_stdin.lock().unwrap();
                            if s.mode == Mode::Idle {
                                s.mode = Mode::Running;
                                s.last_pty_activity = None;
                                s.captured.clear();
                            } else {
                                s.pending_cmd.clear();
                            }
                        } else if b.is_ascii_graphic() || b == b' ' {
                            let mut s = shared_stdin.lock().unwrap();
                            if s.mode == Mode::Idle {
                                s.pending_cmd.push(b as char);
                            }
                        }
                    }
                    if stdin_tx.blocking_send(data.to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });

    // ── PTY writer task ───────────────────────────────────────────────────────
    tokio::spawn(async move {
        let mut writer = pty_writer;
        while let Some(data) = stdin_rx.recv().await {
            if writer.write_all(&data).is_err() {
                break;
            }
        }
    });

    // ── PTY reader thread ─────────────────────────────────────────────────────
    // Forwards or captures bytes based on current Mode.
    // Signals `done_tx` when the PTY closes so the idle loop can exit.
    let shared_pty = Arc::clone(&shared);
    let display_tx_reader = display_tx.clone();
    let (done_tx, done_rx) = tokio::sync::oneshot::channel::<()>();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match pty_reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let data = &buf[..n];
                    let text = std::str::from_utf8(data).unwrap_or("");
                    let enters_alt = text.contains("\x1b[?1049h") || text.contains("\x1b[?1047h");
                    let exits_alt  = text.contains("\x1b[?1049l") || text.contains("\x1b[?1047l");

                    let mut s = shared_pty.lock().unwrap();
                    match s.mode {
                        Mode::Idle => {
                            if enters_alt { s.mode = Mode::Passthrough; }
                            drop(s);
                            let _ = display_tx_reader.blocking_send(data.to_vec());
                        }
                        Mode::Passthrough => {
                            if exits_alt { s.mode = Mode::Idle; }
                            drop(s);
                            let _ = display_tx_reader.blocking_send(data.to_vec());
                        }
                        Mode::Running => {
                            if enters_alt {
                                // Interactive app started mid-capture — flush and pass through.
                                let prev = std::mem::take(&mut s.captured);
                                s.mode = Mode::Passthrough;
                                s.last_pty_activity = None;
                                drop(s);
                                if !prev.is_empty() {
                                    let _ = display_tx_reader.blocking_send(prev);
                                }
                                let _ = display_tx_reader.blocking_send(data.to_vec());
                            } else {
                                s.captured.extend_from_slice(data);
                                s.last_pty_activity = Some(Instant::now());
                            }
                        }
                    }
                }
            }
        }
        let _ = done_tx.send(());
    });

    // ── Idle-detection loop ───────────────────────────────────────────────────
    // Polls every 50 ms.
    //
    // Primary trigger: `process_group_leader()` returns `shell_pgid`, meaning
    // the shell has become the foreground process group again (command exited).
    // A short silence window (50 ms) is required after that to let the PTY
    // reader thread drain any remaining output.
    //
    // Fallback: if `process_group_leader()` is unimplemented (returns `None`),
    // fall back to a pure 200 ms PTY-silence timeout.
    let mut done_rx = done_rx;
    loop {
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(50)) => {
                let mut emit: Option<(String, Vec<u8>)> = None;
                {
                    let mut s = shared.lock().unwrap();
                    if s.mode == Mode::Running {
                        let fg_pgid = master.lock().unwrap().process_group_leader();
                        let (shell_is_fg, has_pgid_support) = match fg_pgid {
                            Some(pgid) => (pgid == shell_pgid, true),
                            None       => (false, false),
                        };
                        // Silence duration since last PTY byte.
                        let silence = s.last_pty_activity
                            .map(|t| Instant::now().duration_since(t));
                        let should_emit = if has_pgid_support {
                            // Shell back in fg + at least 50 ms of silence.
                            shell_is_fg && silence.map(|d| d >= Duration::from_millis(50)).unwrap_or(true)
                        } else {
                            // Fallback: 200 ms of pure silence.
                            silence.map(|d| d >= Duration::from_millis(200)).unwrap_or(false)
                        };
                        if should_emit {
                            let captured = std::mem::take(&mut s.captured);
                            let cmd = std::mem::take(&mut s.pending_cmd);
                            s.mode = Mode::Idle;
                            s.last_pty_activity = None;
                            emit = Some((cmd, captured));
                        }
                    }
                }
                if let Some((cmd, captured)) = emit {
                    let prompt = captured
                        .iter()
                        .rposition(|&b| b == b'\n')
                        .map(|pos| captured[pos + 1..].to_vec())
                        .unwrap_or_default();
                    debug!("capture: cmd={:?} bytes={}", cmd, captured.len());
                    if cmd_tx.send(CapturedCommand { cmd, bytes: captured, prompt }).await.is_err() {
                        break;
                    }
                }
            }
            _ = &mut done_rx => break,
        }
    }
}
