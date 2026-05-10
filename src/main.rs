use anyhow::Context;
use crossterm::{
    terminal::{disable_raw_mode, enable_raw_mode, size as terminal_size},
};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::env;
use std::io::{self, Read, Write};
use std::sync::{Arc, Mutex};
use termwiz::escape::parser::Parser;
use termwiz::escape::Action;
use tokio::io::AsyncWriteExt;
use tokio::signal::unix::{signal, SignalKind};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};
mod cfg;
use crate::cfg::load_config;
mod cli_server;
mod ingest;
mod help;
use ingest::{now_millis, start_ingest_worker};
use aura::context::AppContext;

/// Detect the current terminal size (columns x rows).
fn current_pty_size() -> PtySize {
    let (cols, rows) = terminal_size().unwrap_or((80, 24));
    PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load config — default: logging disabled. If enabled, tracing will honor RUST_LOG.
    let config = load_config();
    let env_filter = if config.logging_enabled() {
        tracing_subscriber::EnvFilter::from_default_env()
    } else {
        // set to WARN by default to suppress debug/info from `aura`
        tracing_subscriber::EnvFilter::new("warn")
    };

    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_writer(std::io::stderr)
        .init();

    // Warn if stdin is not a real TTY (e.g. piped input).
    if !atty::is(atty::Stream::Stdin) {
        warn!("stdin is not a TTY — raw mode will not be entered");
    }

    // MCP feature removed: no management control server is started.

    // ── Session ID ───────────────────────────────────────────────────────────
    let session_id = format!("aura-{}", now_millis());
    info!("session_id: {}", session_id);

    // ── Open PTY ────────────────────────────────────────────────────────────
    let pty_system = native_pty_system();
    let initial_size = current_pty_size();
    info!("opening pty ({}x{})", initial_size.cols, initial_size.rows);

    let pty_pair = pty_system.openpty(initial_size).context("openpty failed")?;

    // Wrap master in Arc<Mutex<>> so the resize handler can call .resize().
    let master = Arc::new(Mutex::new(pty_pair.master));

    let shell = env::var("SHELL").unwrap_or_else(|_| String::from("/bin/sh"));
    info!("spawning shell: {}", shell);

    let cmd = CommandBuilder::new(&shell);
    let mut child = {
        let _m = master.lock().unwrap();
        pty_pair
            .slave
            .spawn_command(cmd)
            .context("spawn_command failed")?
    };

    // Grab reader and writer from the master *before* entering raw mode.
    let mut pty_reader = {
        let m = master.lock().unwrap();
        m.try_clone_reader().context("clone reader failed")?
    };
    let pty_writer = {
        let m = master.lock().unwrap();
        m.take_writer().context("take writer failed")?
    };

    // ── Enter raw mode ───────────────────────────────────────────────────────
    let raw_mode_active = atty::is(atty::Stream::Stdin);
    if raw_mode_active {
        enable_raw_mode().context("enable_raw_mode failed")?;
        debug!("raw mode enabled");
    }

    // RAII guard: restores terminal on drop (even on panic).
    struct RawGuard(bool);
    impl Drop for RawGuard {
        fn drop(&mut self) {
            if self.0 {
                let _ = disable_raw_mode();
            }
        }
    }
    let _raw_guard = RawGuard(raw_mode_active);

    // ── Channels ─────────────────────────────────────────────────────────────
    // Channel: user stdin bytes → pty writer thread.
    let (stdin_tx, mut stdin_rx) = mpsc::channel::<Vec<u8>>(64);
    // Channel: pty output bytes → stdout writer task.
    let (pty_out_tx, mut pty_out_rx) = mpsc::channel::<Vec<u8>>(64);

    // Application context used by `/aura status` command.
    let app_ctx = Arc::new(AppContext::new());

    // Start the control server (UDS + TCP fallback) in the background.
    cli_server::start_control_server(Arc::clone(&app_ctx));

    // ── Ingestion worker ─────────────────────────────────────────────────────
    // Always start the ingestion worker (no runtime gating).
    info!("starting ingestion worker (always enabled)");
    let ingest_tx = Some(start_ingest_worker(config.clone()));

    // ── Thread: blocking stdin → async channel (forward) + ingestion (input)
    let stdin_tx2 = stdin_tx.clone();
    let ingest_tx2 = ingest_tx.clone();
    let _session_id_for_stdin = session_id.clone();
    std::thread::spawn(move || {
        let mut stdin = io::stdin();
        let mut buf = [0u8; 4096];
        let mut parser = Parser::new();
        let mut partial = String::new();
        loop {
            match stdin.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let data = buf[..n].to_vec();

                    // Sanitize user input for ingestion (plain text)
                    let mut plain = String::new();
                    parser.parse(&data, |action: Action| {
                        match &action {
                            Action::Print(c) => plain.push(*c),
                            Action::PrintString(s) => plain.push_str(s),
                            _ => {}
                        }
                    });

                    // Accumulate partial input and flush on newline or if large
                    partial.push_str(&plain);
                    while let Some(pos) = partial.find('\n') {
                        let line = partial.drain(..=pos).collect::<String>();
                        let trimmed = line.trim().to_string();
                        if !trimmed.is_empty() {
                            if let Some(ref tx) = ingest_tx2 {
                                let chunk = crate::ingest::SanitizedChunk { text: trimmed };
                                let _ = tx.blocking_send(chunk);
                            }
                        }
                    }
                    // If partial grows too large without newline, flush it as a chunk
                    if partial.len() > 256 {
                        let flushed = partial.split_off(0);
                        let trimmed = flushed.trim().to_string();
                        if !trimmed.is_empty() {
                            if let Some(ref tx) = ingest_tx2 {
                                let chunk = crate::ingest::SanitizedChunk { text: trimmed };
                                let _ = tx.blocking_send(chunk);
                            }
                        }
                    }

                    if stdin_tx2.blocking_send(data).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    error!("stdin read error: {}", e);
                    break;
                }
            }
        }
        // flush any remaining partial on exit
        if !partial.trim().is_empty() {
            if let Some(ref tx) = ingest_tx2 {
                let chunk = crate::ingest::SanitizedChunk { text: partial.trim().to_string() };
                let _ = tx.blocking_send(chunk);
            }
        }
    });

    // ── Thread: blocking pty reader → async channel ───────────────────────
    let _session_id_for_pty = session_id.clone();
    let ingest_tx_for_pty = ingest_tx.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        let mut parser = Parser::new();
        let mut partial = String::new();
        loop {
            match pty_reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let data = &buf[..n];

                    // Sanitize VT100 escape sequences → plain text for ingestion.
                    let mut plain = String::new();
                    parser.parse(data, |action: Action| {
                        match &action {
                            Action::Print(c) => plain.push(*c),
                            Action::PrintString(s) => plain.push_str(s),
                            Action::Control(ctrl) => {
                                use termwiz::escape::ControlCode;
                                if matches!(ctrl, ControlCode::LineFeed | ControlCode::CarriageReturn) {
                                    plain.push('\n');
                                }
                            }
                            _ => {}
                        }
                        debug!("vt action: {:?}", action);
                    });

                    // Accumulate and flush on newline or size threshold
                    partial.push_str(&plain);
                    while let Some(pos) = partial.find('\n') {
                        let line = partial.drain(..=pos).collect::<String>();
                        let trimmed = line.trim().to_string();
                        if !trimmed.is_empty() {
                            if let Some(ref tx) = ingest_tx_for_pty {
                                let chunk = crate::ingest::SanitizedChunk { text: trimmed };
                                let _ = tx.blocking_send(chunk);
                            }
                        }
                    }
                    if partial.len() > 4096 {
                        let flushed = partial.split_off(0);
                        let trimmed = flushed.trim().to_string();
                        if !trimmed.is_empty() {
                            if let Some(ref tx) = ingest_tx_for_pty {
                                let chunk = crate::ingest::SanitizedChunk { text: trimmed };
                                let _ = tx.blocking_send(chunk);
                            }
                        }
                    }

                    if pty_out_tx.blocking_send(data.to_vec()).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    error!("pty read error: {}", e);
                    break;
                }
            }
        }
        if !partial.trim().is_empty() {
            if let Some(ref tx) = ingest_tx_for_pty {
                let chunk = crate::ingest::SanitizedChunk { text: partial.trim().to_string() };
                let _ = tx.blocking_send(chunk);
            }
        }
    });

    // No aggregator: stdin and stdout lines are sent to ingest separately.

    tokio::spawn(async move {
        let mut pty_writer_owned = pty_writer;
        while let Some(data) = stdin_rx.recv().await {
            if let Err(e) = pty_writer_owned.write_all(&data) {
                error!("pty write error: {}", e);
                break;
            }
        }
    });

    // ── Task: pty channel → stdout ────────────────────────────────────────
    tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        while let Some(data) = pty_out_rx.recv().await {
            if let Err(e) = stdout.write_all(&data).await {
                error!("stdout write error: {}", e);
                break;
            }
            let _ = stdout.flush().await;
        }
    });

    // ── Signal: SIGWINCH → resize PTY ────────────────────────────────────
    let master_resize = Arc::clone(&master);
    tokio::spawn(async move {
        let mut sigwinch =
            signal(SignalKind::window_change()).expect("SIGWINCH handler failed");
        loop {
            sigwinch.recv().await;
            let new_size = current_pty_size();
            info!(
                "SIGWINCH: resizing pty to {}x{}",
                new_size.cols, new_size.rows
            );
            let m = master_resize.lock().unwrap();
            if let Err(e) = m.resize(new_size) {
                error!("pty resize error: {}", e);
            }
        }
    });

    // ── Signal: SIGINT passthrough ────────────────────────────────────────
    // In raw mode crossterm/the shell handles Ctrl-C, so we just log it.
    tokio::spawn(async move {
        let mut sigint = signal(SignalKind::interrupt()).expect("SIGINT handler failed");
        loop {
            sigint.recv().await;
            debug!("SIGINT received (passed through to shell via raw tty)");
        }
    });

    // ── Wait for child shell to exit ──────────────────────────────────────
    let status = tokio::task::spawn_blocking(move || child.wait()).await??;
    info!("shell exited: {:?}", status);

    if raw_mode_active {
        disable_raw_mode().context("disable_raw_mode failed")?;
        debug!("raw mode disabled");
    }

    Ok(())
}
