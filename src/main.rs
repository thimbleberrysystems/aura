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

    // ── Thread: blocking stdin → async channel ────────────────────────────
    let stdin_tx2 = stdin_tx.clone();
    std::thread::spawn(move || {
        let mut stdin = io::stdin();
        let mut buf = [0u8; 1024];
        loop {
            match stdin.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if stdin_tx2.blocking_send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    error!("stdin read error: {}", e);
                    break;
                }
            }
        }
    });

    // ── Thread: blocking pty reader → async channel ───────────────────────
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        let mut parser = Parser::new();
        loop {
            match pty_reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let data = &buf[..n];
                    // Feed into termwiz VT parser for debug tracing.
                    parser.parse(data, |action: Action| {
                        debug!("vt action: {:?}", action);
                    });
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
    });

    // ── Task: stdin channel → pty writer ─────────────────────────────────
    let mut pty_writer_owned = pty_writer;
    tokio::spawn(async move {
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
