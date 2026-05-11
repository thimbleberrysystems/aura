use anyhow::Context;
use crossterm::terminal::enable_raw_mode;
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
use aura::pty::{Mode, CommandState, RawGuard, current_pty_size, strip_ansi};
use aura::compress::call_ollama_summarize;
use tracing::{debug, error, info, warn};
use aura::cfg::load_config;
use aura::ingest::{now_millis, init_global_store, global_store, embed_text, store_text};
use aura::cmd;
use aura::server as cli_server;

// PTY types and helpers moved to `aura::pty` (Mode, CommandState, current_pty_size, RawGuard).

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load config — default: logging disabled. If enabled, tracing will honor RUST_LOG.
    let config = load_config();
    // Initialise ephemeral in-memory RAG store for this session.
    init_global_store();
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

    let _raw_guard = RawGuard(raw_mode_active);

    // ── Channels ──────────────────────────────────────────────────────────────
    // stdin bytes → PTY writer task
    let (stdin_tx, mut stdin_rx) = mpsc::channel::<Vec<u8>>(64);
    // bytes to display → stdout writer task  (PTY reader + flusher both send here)
    let (pty_out_tx, mut pty_out_rx) = mpsc::channel::<Vec<u8>>(128);

    // ── Control server ──────────────────────────────────────────────────────
    cli_server::start_control_server();

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
    let config_summarize = config.clone();
    tokio::spawn(async move {
        while let Some((cmd, captured, prompt)) = summarize_rx.recv().await {
            // Re-read all settings live on every command so runtime env var
            // changes (AURA_DISABLE_SUMMARY, AURA_SUMMARIZE_THRESHOLD,
            // AURA_SUMMARIZE_TIMEOUT_SECS, AURA_OLLAMA_BASE_URL, etc.) take
            // effect immediately without restarting aura.
            let summarize_threshold = config_summarize.summarize_threshold();
            let summarize_timeout = Duration::from_secs(config_summarize.summarize_timeout_secs());
            let ollama_url = config_summarize.ollama_base_url();
            let completion_model = config_summarize.completion_model();
            let embedding_model = config_summarize.embedding_model();
            let disabled = config_summarize.disable_summary();
            // AURA_DISABLE_RAG=1 skips all embedding/store operations.
            let rag_disabled = config_summarize.disable_rag();

            // Strip ANSI from captured to get clean text for LLM and length check.
            let clean = strip_ansi(&captured);

            // ── RAG query: retrieve past context before calling the LLM ──────────────
            // Only runs when summarization will actually happen and RAG is enabled.
            // Uses the dedicated embedding model (AURA_EMBEDDING_MODEL, default
            // nomic-embed-text) — NOT the completion model — to avoid dimension
            // mismatches and keep embedding fast.
            let context_chunks: Vec<String> = if !disabled && !rag_disabled && clean.len() >= summarize_threshold {
                let q = format!("Command: {}\n{}", cmd, &clean[..clean.len().min(512)]);
                match embed_text(&ollama_url, &embedding_model, &q).await {
                    Ok(emb) => {
                        let store = global_store();
                        let r = store.read().await;
                        let hits = r.top_k(&emb, 3);
                        debug!("rag: top_k returned {} hits", hits.len());
                        hits.into_iter().map(|(_, _, content)| content).collect()
                    }
                    Err(e) => { warn!("rag: embed query failed: {:#}", e); vec![] }
                }
            } else {
                vec![]
            };

            // ── Summarize ────────────────────────────────────────────────────────────
            // Returns (bytes_to_display, text_to_store_in_rag).
            let (display, to_store) = if disabled || clean.len() < summarize_threshold {
                // Summaries disabled via env OR short output — display as-is (original).
                (Some(captured.clone()), clean.clone())
            } else {
                // Call Ollama to summarize; fall back to original on any failure.
                let result = tokio::time::timeout(
                    summarize_timeout,
                    call_ollama_summarize(&ollama_url, &completion_model, &cmd, &clean, &context_chunks),
                )
                .await;

                match result {
                    Ok(Ok(summary)) => {
                        // Only use summary if it's shorter than the original clean text.
                        if summary.trim().eq_ignore_ascii_case("ORIGINAL")
                            || summary.len() >= clean.len()
                            || summary.trim().is_empty()
                        {
                            (Some(captured.clone()), clean.clone())
                        } else {
                            // Use the model reply directly; prompt instructs no preamble.
                            let body = summary.trim_end().to_string();
                            // Normalise LLM line endings: plain \n → \r\n so the
                            // cursor returns to column 0 in raw PTY mode.
                            let normalised = body.replace('\n', "\r\n");
                            let mut out = b"\r\n".to_vec();
                            out.extend_from_slice(b"[AURA] summarized (export AURA_DISABLE_SUMMARY=1 to disable)\r\n");
                            out.extend_from_slice(normalised.as_bytes());
                            out.extend_from_slice(b"\r\n");
                            (Some(out), body)
                        }
                    }
                    Ok(Err(err)) => {
                        // Ollama client returned an error — inform the user, then
                        // fall back to showing the original captured output.
                        let err_msg = format!("\r\n[AURA: summarize error: {}]\r\n", err);
                        let mut out = err_msg.into_bytes();
                        out.extend_from_slice(&captured);
                        (Some(out), clean.clone())
                    }
                    Err(_) => {
                        // Timeout — inform user and fall back to original output.
                        let mut out = b"\r\n[AURA: summarize timeout]\r\n".to_vec();
                        out.extend_from_slice(&captured);
                        (Some(out), clean.clone())
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

            // ── RAG: embed and store after display (fire and forget) ──────────────
            // Runs after the summary has been sent so it can never influence the
            // current call. Stores every non-empty command output (using the
            // distilled text when available, otherwise raw clean output).
            // Skipped when RAG is disabled or clean output is empty.
            if !rag_disabled && !to_store.trim().is_empty() {
                let store_content = format!("Command: {}\n{}", cmd, to_store);
                let base = ollama_url.clone();
                let emb_model = embedding_model.clone();
                tokio::spawn(async move {
                    if let Err(e) = store_text(&base, &emb_model, &store_content).await {
                        warn!("rag: store_text failed: {:#}", e);
                    }
                });
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

    // ── SIGINT passthrough ───────────────────────────────────────────────────
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

    
    Ok(())
}
