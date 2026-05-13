use anyhow::Context;
use crossterm::terminal::enable_raw_mode;
use libc;
use portable_pty::{native_pty_system, CommandBuilder};
use std::env;
use std::sync::{Arc, Mutex};
use tokio::io::AsyncWriteExt;
use tokio::signal::unix::{signal, SignalKind};
use tokio::sync::mpsc;
use tracing::{error, info, warn};
use aura::cfg::load_config;
use aura::compress::pipeline_task;
use aura::pty::{CapturedCommand, RawGuard, capture_task, current_pty_size};
use aura::server as cli_server;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = load_config();

    let env_filter = if config.logging_with_source().0 {
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

    // ── Open PTY & spawn shell ────────────────────────────────────────────────
    let pty_system = native_pty_system();
    let initial_size = current_pty_size();
    let pty_pair = pty_system.openpty(initial_size).context("openpty failed")?;
    let master = Arc::new(Mutex::new(pty_pair.master));

    let shell = env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    let cmd = CommandBuilder::new(&shell);
    let mut child = {
        let _m = master.lock().unwrap();
        pty_pair.slave.spawn_command(cmd).context("spawn_command failed")?
    };

    // Shell's process group id — used by capture_task to detect command end
    // via tcgetpgrp.  Interactive shells are always their own session/pgid
    // leader, so pgid == pid.
    let shell_pgid = child.process_id().unwrap_or(0) as libc::pid_t;

    let pty_reader = master.lock().unwrap().try_clone_reader().context("clone reader")?;
    let pty_writer = master.lock().unwrap().take_writer().context("take writer")?;

    // ── Raw mode ──────────────────────────────────────────────────────────────
    let raw_mode_active = atty::is(atty::Stream::Stdin);
    if raw_mode_active {
        enable_raw_mode().context("enable_raw_mode failed")?;
    }
    let _raw_guard = RawGuard(raw_mode_active);

    // ── Pipeline channels ─────────────────────────────────────────────────────
    //   Stage 1: capture_task  →[CapturedCommand]→  Stage 2: pipeline_task
    //   Both stages             →[Vec<u8>]→          Stage 3: display → stdout
    let (display_tx, mut display_rx) = mpsc::channel::<Vec<u8>>(128);
    let (cmd_tx, cmd_rx) = mpsc::channel::<CapturedCommand>(16);

    // ── Control server ────────────────────────────────────────────────────────
    cli_server::start_control_server();

    // ── SIGWINCH: resize PTY ──────────────────────────────────────────────────
    let master_resize = Arc::clone(&master);
    tokio::spawn(async move {
        let mut sigwinch = signal(SignalKind::window_change()).expect("SIGWINCH handler failed");
        loop {
            sigwinch.recv().await;
            let m = master_resize.lock().unwrap();
            if let Err(e) = m.resize(current_pty_size()) {
                error!("pty resize: {}", e);
            }
        }
    });

    // ── Stage 1: PTY capture ──────────────────────────────────────────────────
    tokio::spawn(capture_task(
        pty_reader,
        pty_writer,
        display_tx.clone(),
        cmd_tx,
        Arc::clone(&master),
        shell_pgid,
    ));

    // ── Stage 2: summarize ────────────────────────────────────────────────────
    tokio::spawn(pipeline_task(config, cmd_rx, display_tx));

    // ── Stage 3: display ──────────────────────────────────────────────────────
    tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        while let Some(data) = display_rx.recv().await {
            if let Err(e) = stdout.write_all(&data).await {
                error!("stdout write: {}", e);
                break;
            }
            let _ = stdout.flush().await;
        }
    });

    // ── Wait for shell to exit ────────────────────────────────────────────────
    let status = tokio::task::spawn_blocking(move || child.wait()).await??;
    info!("shell exited: {:?}", status);
    Ok(())
}
