use anyhow::Context;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, size as terminal_size};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use rig::providers::ollama;
use rig::client::{Nothing, CompletionClient as _};
use rig::completion::Prompt as _;
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

/// Strip ANSI escape sequences, returning clean UTF-8 text.
fn strip_ansi(bytes: &[u8]) -> String {
    // Walk bytes and drop ESC sequences: ESC [ ... final-byte  and  ESC non-[
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b {
            i += 1;
            if i < bytes.len() && bytes[i] == b'[' {
                // CSI sequence — skip until a byte in 0x40–0x7E
                i += 1;
                while i < bytes.len() && !(0x40..=0x7E).contains(&bytes[i]) {
                    i += 1;
                }
                i += 1; // skip final byte
            } else if i < bytes.len() {
                // Two-byte ESC sequence
                i += 1;
            }
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Call Ollama to summarize command output.  Returns the model's reply or an error.
async fn call_ollama_summarize(
    base_url: &str,
    model: &str,
    cmd: &str,
    clean_output: &str,
) -> anyhow::Result<String> {
    let client = ollama::Client::builder()
        .api_key(Nothing)
        .base_url(base_url)
        .build()?;
    let agent = client.agent(model).build();
    let prompt = format!(
        "Command: {cmd}\nOutput:\n{clean_output}\n\n\
         Summarize or shorten the output above without losing important information.\n\
         Rules:\n\
         - If the output is already very short or cannot be meaningfully shortened, reply with exactly: ORIGINAL\n\
         - Otherwise reply with only your shortened version, no preamble or explanation."
    );
    let reply = agent.prompt(&prompt).await?;
    Ok(reply)
}

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
    /// Printable text typed by the user for the current command (accumulated until newline).
    pending_cmd: String,
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
        pending_cmd: String::new(),
    }));

    // How long PTY must be silent (while Running) before we emit AURA.
    let idle_timeout = Duration::from_millis(200);

    // ── Stdin thread ──────────────────────────────────────────────────────────
    // Forwards raw bytes to the PTY unchanged (shell receives everything as-is).
    // Detects newline/CR to transition IDLE → RUNNING and records the typed command.
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
                    // Accumulate printable chars for the command text.
                    for &b in data.iter() {
                        if b == b'\n' || b == b'\r' {
                            // newline → commit and transition
                            let mut s = shared_stdin.lock().unwrap();
                            if s.mode == Mode::Idle {
                                s.mode = Mode::Running;
                                s.last_pty_activity = None;
                                s.captured.clear();
                                // pending_cmd is now the completed command; leave it for flusher.
                            } else {
                                // Already Running (e.g. multi-line); reset accumulated text.
                                s.pending_cmd.clear();
                            }
                        } else if b.is_ascii_graphic() || b == b' ' {
                            let mut s = shared_stdin.lock().unwrap();
                            if s.mode == Mode::Idle {
                                s.pending_cmd.push(b as char);
                            }
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

    // Channel: flusher → summarize task  (cmd_text, captured_bytes, prompt_bytes)
    let (summarize_tx, mut summarize_rx) =
        mpsc::channel::<(String, Vec<u8>, Vec<u8>)>(16);

    // ── Flusher thread ───────────────────────────────────────────────────────
    // Polls every 50 ms. When Running and PTY has been silent for idle_timeout,
    // extracts the trailing prompt and sends (cmd, captured, prompt) to the
    // summarize task for Ollama processing.
    let shared_flush = Arc::clone(&shared);
    let summarize_tx_flush = summarize_tx.clone();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_millis(50));
            let mut emit: Option<(String, Vec<u8>)> = None;
            {
                let mut s = shared_flush.lock().unwrap();
                if s.mode == Mode::Running {
                    if let Some(last) = s.last_pty_activity {
                        if Instant::now().duration_since(last) >= idle_timeout {
                            let captured = std::mem::take(&mut s.captured);
                            let cmd = std::mem::take(&mut s.pending_cmd);
                            s.mode = Mode::Idle;
                            s.last_pty_activity = None;
                            emit = Some((cmd, captured));
                        }
                    }
                }
            }
            if let Some((cmd, captured)) = emit {
                // Shell-agnostic prompt extraction: bytes after the last '\n'.
                let prompt: Vec<u8> = captured
                    .iter()
                    .rposition(|&b| b == b'\n')
                    .map(|pos| captured[pos + 1..].to_vec())
                    .unwrap_or_default();
                let _ = summarize_tx_flush.blocking_send((cmd, captured, prompt));
            }
        }
    });

    // ── Summarize task (async) ────────────────────────────────────────────────
    // Receives (cmd, captured_bytes, prompt_bytes) from the flusher.
    // If the clean output is under the threshold, displays it as-is.
    // Otherwise calls Ollama with a timeout; falls back to original on any failure.
    let pty_out_tx_summarize = pty_out_tx.clone();
    let summarize_threshold = config.summarize_threshold();
    let summarize_timeout = Duration::from_secs(config.summarize_timeout_secs());
    let ollama_url = config.ollama_base_url();
    let completion_model = config.completion_model();
    tokio::spawn(async move {
        while let Some((cmd, captured, prompt)) = summarize_rx.recv().await {
            // Strip ANSI from captured to get clean text for LLM and length check.
            let clean = strip_ansi(&captured);

            let display = if clean.len() < summarize_threshold {
                // Short output — display as-is (original).
                Some(captured.clone())
            } else {
                // Call Ollama to summarize; fall back to original on any failure.
                let result = tokio::time::timeout(
                    summarize_timeout,
                    call_ollama_summarize(&ollama_url, &completion_model, &cmd, &clean),
                )
                .await;

                match result {
                    Ok(Ok(summary)) => {
                        // Only use summary if it's shorter than the original clean text.
                        if summary.trim().eq_ignore_ascii_case("ORIGINAL")
                            || summary.len() >= clean.len()
                            || summary.trim().is_empty()
                        {
                            Some(captured.clone())
                        } else {
                            // Prepend a short indicator so the user knows this was
                            // produced by Ollama as a summary of a long output.
                            let mut out = b"\r\n".to_vec();
                            out.extend_from_slice(b"[AURA: summarized]\r\n");
                            out.extend_from_slice(summary.trim_end().as_bytes());
                            out.extend_from_slice(b"\r\n");
                            Some(out)
                        }
                    }
                    Ok(Err(err)) => {
                        // Ollama client returned an error — inform the user, then
                        // fall back to showing the original captured output.
                        let err_msg = format!("\r\n[AURA: summarize error: {}]\r\n", err);
                        let mut out = err_msg.into_bytes();
                        out.extend_from_slice(&captured.clone());
                        Some(out)
                    }
                    Err(_) => {
                        // Timeout — inform user and fall back to original output.
                        let mut out = b"\r\n[AURA: summarize timeout]\r\n".to_vec();
                        out.extend_from_slice(&captured.clone());
                        Some(out)
                    }
                }
            };

            if let Some(mut bytes) = display {
                // Ensure output starts on a new line (typed chars were echoed while Idle).
                let mut out = b"\r\n".to_vec();
                out.append(&mut bytes);
                // Re-append the prompt so the user sees their shell prompt.
                if !prompt.is_empty() {
                    out.extend_from_slice(&prompt);
                }
                let _ = pty_out_tx_summarize.send(out).await;
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
