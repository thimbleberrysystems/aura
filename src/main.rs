use anyhow::Context;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, size as terminal_size};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::env;
use std::io::{self, Read, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio::signal::unix::{signal, SignalKind};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};
mod cfg;
use crate::cfg::load_config;
mod cli_server;
mod ingest;
mod help;
use ingest::now_millis;
use aura::context::AppContext;

/// Detect the current terminal size (columns x rows).
fn current_pty_size() -> PtySize {
    let (cols, rows) = terminal_size().unwrap_or((80, 24));
    PtySize { rows, cols, pixel_width: 0, pixel_height: 0 }
}

// ── State machine ─────────────────────────────────────────────────────────────
//
//  IDLE ──(stdin newline)──► RUNNING ──(200 ms PTY silence)──► IDLE + emit AURA
//  IDLE / RUNNING ──(alt-screen on: \x1b[?1049h)──► PASSTHROUGH
//  PASSTHROUGH ──(alt-screen off: \x1b[?1049l)──► IDLE
//
#[derive(Debug, Clone, PartialEq)]
enum Mode {
    /// Shell is idle/showing a prompt. PTY output forwarded to terminal as-is.
    Idle,
    /// User pressed Enter. PTY output is captured and suppressed from display.
    Running,
    /// Full-screen app (vim, less, top…) detected via alternate-screen sequence.
    /// Everything forwarded raw — no capture, no replacement.
    Passthrough,
}

struct CommandState {
    mode: Mode,
    /// Set on every PTY byte received while Running.
    last_pty_activity: Option<Instant>,
    /// Raw PTY bytes accumulated while Running (command output + trailing prompt).
    captured: Vec<u8>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load config — default: logging disabled. If enabled, tracing will honor RUST_LOG.
    let config = load_config();
    let env_filter = if config.logging_enabled() {
        tracing_subscriber::EnvFilter::from_default_env()
    } else {
        tracing_subscriber::EnvFilter::new("warn")
    };
    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_writer(std::io::stderr)
        .init();

    if !atty::is(atty::Stream::Stdin) {
        warn!("stdin is not a TTY — raw mode will not be entered");
    }

    let session_id = format!("aura-{}", now_millis());
    info!("session_id: {}", session_id);

    // ── Open PTY ─────────────────────────────────────────────────────────────
    let pty_system = native_pty_system();
    let initial_size = current_pty_size();
    info!("opening pty ({}x{})", initial_size.cols, initial_size.rows);

    let pty_pair = pty_system.openpty(initial_size).context("openpty failed")?;
    let master = Arc::new(Mutex::new(pty_pair.master));

    let shell = env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    info!("spawning shell: {}", shell);

    let cmd = CommandBuilder::new(&shell);
    let mut child = {
        let _m = master.lock().unwrap();
        pty_pair.slave.spawn_command(cmd).context("spawn_command failed")?
    };

    let mut pty_reader = {
        let m = master.lock().unwrap();
        m.try_clone_reader().context("clone reader failed")?
    };
    let pty_writer = {
        let m = master.lock().unwrap();
        m.take_writer().context("take writer failed")?
    };

    // ── Raw mode ──────────────────────────────────────────────────────────────
    let raw_mode_active = atty::is(atty::Stream::Stdin);
    if raw_mode_active {
        enable_raw_mode().context("enable_raw_mode failed")?;
        debug!("raw mode enabled");
    }

    struct RawGuard(bool);
    impl Drop for RawGuard {
        fn drop(&mut self) {
            if self.0 { let _ = disable_raw_mode(); }
        }
    }
    let _raw_guard = RawGuard(raw_mode_active);

    // ── Channels ──────────────────────────────────────────────────────────────
    // stdin bytes → PTY writer task
    let (stdin_tx, mut stdin_rx) = mpsc::channel::<Vec<u8>>(64);
    // bytes to display → stdout writer task  (PTY reader + flusher both send here)
    let (pty_out_tx, mut pty_out_rx) = mpsc::channel::<Vec<u8>>(128);

    // ── App context / control server ──────────────────────────────────────────
    let app_ctx = Arc::new(AppContext::new());
    cli_server::start_control_server(Arc::clone(&app_ctx));

    // ── Shared state ──────────────────────────────────────────────────────────
    let shared = Arc::new(Mutex::new(CommandState {
        mode: Mode::Idle,
        last_pty_activity: None,
        captured: Vec::new(),
    }));

    // How long PTY must be silent (while Running) before we emit AURA.
    let idle_timeout = Duration::from_millis(200);

    // ── Stdin thread ──────────────────────────────────────────────────────────
    // Forwards raw bytes to the PTY unchanged (shell receives everything as-is).
    // Detects newline/CR to transition IDLE → RUNNING.
    let stdin_tx2 = stdin_tx.clone();
    let shared_stdin = Arc::clone(&shared);
    std::thread::spawn(move || {
        let mut stdin = io::stdin();
        let mut buf = [0u8; 4096];
        loop {
            match stdin.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let data = &buf[..n];
                    // Newline or CR means the user submitted a command.
                    let has_newline = data.iter().any(|&b| b == b'\n' || b == b'\r');
                    if has_newline {
                        let mut s = shared_stdin.lock().unwrap();
                        if s.mode == Mode::Idle {
                            s.mode = Mode::Running;
                            s.last_pty_activity = None;
                            s.captured.clear();
                        }
                    }
                    // Always forward raw bytes to PTY.
                    if stdin_tx2.blocking_send(data.to_vec()).is_err() { break; }
                }
                Err(e) => { error!("stdin read: {}", e); break; }
            }
        }
    });

    // ── PTY reader thread ────────────────────────────────────────────────────
    // Idle/Passthrough  → forward raw bytes to pty_out_tx (shown in terminal).
    // Running           → capture bytes, suppress display, update last_pty_activity.
    // Alternate-screen  → switch to/from Passthrough mode.
    let shared_pty = Arc::clone(&shared);
    let pty_out_tx_reader = pty_out_tx.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match pty_reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let data = &buf[..n];

                    // Detect alternate-screen sequences (best-effort; valid for UTF-8 terminals).
                    let text = std::str::from_utf8(data).unwrap_or("");
                    let enters_alt = text.contains("\x1b[?1049h") || text.contains("\x1b[?1047h");
                    let exits_alt  = text.contains("\x1b[?1049l") || text.contains("\x1b[?1047l");

                    let mut s = shared_pty.lock().unwrap();
                    match s.mode {
                        Mode::Idle => {
                            if enters_alt { s.mode = Mode::Passthrough; }
                            drop(s);
                            let _ = pty_out_tx_reader.blocking_send(data.to_vec());
                        }
                        Mode::Passthrough => {
                            if exits_alt { s.mode = Mode::Idle; }
                            drop(s);
                            let _ = pty_out_tx_reader.blocking_send(data.to_vec());
                        }
                        Mode::Running => {
                            if enters_alt {
                                // Interactive app started while we were capturing.
                                // Flush what we already captured so the alt-screen
                                // setup sequence arrives complete at the terminal.
                                let prev = std::mem::take(&mut s.captured);
                                s.mode = Mode::Passthrough;
                                s.last_pty_activity = None;
                                drop(s);
                                if !prev.is_empty() {
                                    let _ = pty_out_tx_reader.blocking_send(prev);
                                }
                                let _ = pty_out_tx_reader.blocking_send(data.to_vec());
                            } else {
                                // Normal command output — capture and suppress.
                                s.captured.extend_from_slice(data);
                                s.last_pty_activity = Some(Instant::now());
                                // Do NOT send to pty_out_tx_reader.
                            }
                        }
                    }
                }
                Err(e) => { error!("pty read: {}", e); break; }
            }
        }
    });

    // ── Flusher thread ───────────────────────────────────────────────────────
    // Polls every 50 ms. When Running and PTY has been silent for idle_timeout:
    //   1. Extracts the trailing prompt from captured bytes (last partial line
    //      after the final newline — shell-agnostic heuristic).
    //   2. Emits "AURA\r\n" to the display channel.
    //   3. Re-emits the prompt bytes so the user sees their shell prompt.
    //   4. Switches back to Idle.
    let shared_flush = Arc::clone(&shared);
    let pty_out_tx_flush = pty_out_tx.clone();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_millis(50));
            let mut emit: Option<Vec<u8>> = None;
            {
                let mut s = shared_flush.lock().unwrap();
                if s.mode == Mode::Running {
                    if let Some(last) = s.last_pty_activity {
                        if Instant::now().duration_since(last) >= idle_timeout {
                            emit = Some(std::mem::take(&mut s.captured));
                            s.mode = Mode::Idle;
                            s.last_pty_activity = None;
                        }
                    }
                }
            }
            if let Some(captured) = emit {
                // The prompt is the last partial line (bytes after the final '\n').
                // This is shell-agnostic: every shell ends its prompt without a newline.
                let prompt: Vec<u8> = if let Some(pos) = captured.iter().rposition(|&b| b == b'\n') {
                    captured[pos + 1..].to_vec()
                } else {
                    Vec::new()
                };

                // Move to a new line (typed chars were echoed while still Idle),
                // then show replacement string.
                let _ = pty_out_tx_flush.blocking_send(b"\r\nAURA\r\n".to_vec());

                // Re-show the shell prompt so the user can keep typing.
                if !prompt.is_empty() {
                    let _ = pty_out_tx_flush.blocking_send(prompt);
                }

                // TODO: send `captured` to ingest worker for embedding.
                debug!("captured {} bytes for ingest", captured.len());
            }
        }
    });

    // ── PTY writer task (async) ───────────────────────────────────────────────
    tokio::spawn(async move {
        let mut pty_writer_owned = pty_writer;
        while let Some(data) = stdin_rx.recv().await {
            if let Err(e) = pty_writer_owned.write_all(&data) {
                error!("pty write: {}", e);
                break;
            }
        }
    });

    // ── Stdout writer task (async) ────────────────────────────────────────────
    tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        while let Some(data) = pty_out_rx.recv().await {
            if let Err(e) = stdout.write_all(&data).await {
                error!("stdout write: {}", e);
                break;
            }
            let _ = stdout.flush().await;
        }
    });

    // ── SIGWINCH → resize PTY ─────────────────────────────────────────────────
    let master_resize = Arc::clone(&master);
    tokio::spawn(async move {
        let mut sigwinch = signal(SignalKind::window_change()).expect("SIGWINCH handler failed");
        loop {
            sigwinch.recv().await;
            let new_size = current_pty_size();
            info!("SIGWINCH: resizing pty to {}x{}", new_size.cols, new_size.rows);
            let m = master_resize.lock().unwrap();
            if let Err(e) = m.resize(new_size) { error!("pty resize: {}", e); }
        }
    });

    // ── SIGINT passthrough ────────────────────────────────────────────────────
    tokio::spawn(async move {
        let mut sigint = signal(SignalKind::interrupt()).expect("SIGINT handler failed");
        loop {
            sigint.recv().await;
            debug!("SIGINT received (passed through to shell via raw tty)");
        }
    });

    // ── Wait for child shell to exit ──────────────────────────────────────────
    let status = tokio::task::spawn_blocking(move || child.wait()).await??;
    info!("shell exited: {:?}", status);

    if raw_mode_active {
        disable_raw_mode().context("disable_raw_mode failed")?;
        debug!("raw mode disabled");
    }

    Ok(())
}
