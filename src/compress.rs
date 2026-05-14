use std::sync::Arc;
use std::time::Duration;
use futures::StreamExt;
use oddonkey::OddOnkey;
use tokio::sync::{Mutex, mpsc, watch};
use crate::cfg::Config;
use crate::pty::{CapturedCommand, strip_ansi};

async fn build_oddonkey(config: &Config) -> anyhow::Result<OddOnkey> {
    let model_name = config.model_name().expect("model_name is missing in config");
    let mut builder = OddOnkey::builder(&model_name);
    if let Some(base_url) = config.model_base_url() {
        builder = builder.base_url(&base_url);
    }
    builder.build().await.map_err(|e| anyhow::anyhow!("OddOnkey init failed: {e}"))
}

/// Returns a stable string that identifies the active model+address pair.
/// Used to detect when config reload requires re-initialising OddOnkey.
fn model_key(config: &Config) -> String {
    format!(
        "{}@{}",
        config.model_name().unwrap_or_default(),
        config.model_addr().unwrap_or_default()
    )
}

/// Stage 2 of the pipeline: for each captured command, runs:
///   strip ANSI -> threshold check -> semantic_compress (LLM, with timeout) -> display
///
/// OddOnkey is initialised once at startup and re-initialised only when the
/// model name or address changes via `config reload`.
pub async fn pipeline_task(
    config_rx: watch::Receiver<Config>,
    mut rx: mpsc::Receiver<CapturedCommand>,
    display_tx: mpsc::Sender<Vec<u8>>,
) {
    let config = config_rx.borrow().clone();
    let mut active_key = model_key(&config);

    let model = match build_oddonkey(&config).await {
        Ok(m) => m,
        Err(e) => {
            tracing::error!("Failed to initialise OddOnkey: {e}");
            return;
        }
    };
    let model_arc: Arc<Mutex<OddOnkey>> = Arc::new(Mutex::new(model));

    while let Some(cap) = rx.recv().await {
        let config = config_rx.borrow().clone();

        // Re-init if model name or address changed on config reload.
        let new_key = model_key(&config);
        if new_key != active_key {
            match build_oddonkey(&config).await {
                Ok(new_model) => {
                    *model_arc.lock().await = new_model;
                    tracing::info!("OddOnkey reinitialised → {}", new_key);
                    active_key = new_key;
                }
                Err(e) => {
                    tracing::error!("Failed to reinit OddOnkey for '{}': {e}; keeping previous model", new_key);
                }
            }
        }

        let tx = display_tx.clone();
        let arc = Arc::clone(&model_arc);
        tokio::spawn(async move { process_pipeline(config, cap, arc, tx).await; });
    }
}

async fn process_pipeline(
    config: Config,
    cap: CapturedCommand,
    model: Arc<Mutex<OddOnkey>>,
    display_tx: mpsc::Sender<Vec<u8>>,
) {
    let clean = strip_ansi(&cap.bytes);
    semantic_compress(config, cap, model, display_tx, clean).await;
}

async fn semantic_compress(
    config: Config,
    cap: CapturedCommand,
    model: Arc<Mutex<OddOnkey>>,
    display_tx: mpsc::Sender<Vec<u8>>,
    clean: String,
) {
    let threshold = config.summarize_threshold_with_source().0.unwrap_or(250);
    let disabled = config.disable_summary_with_source().0.unwrap_or(false);
    let timeout = Duration::from_secs(config.summarize_timeout_secs_with_source().0.unwrap_or(3000));

    if disabled || clean.len() < threshold {
        // Short output or summaries disabled — display as-is.
        let mut out = b"\r\n".to_vec();
        out.extend_from_slice(&cap.bytes);
        if !cap.prompt.is_empty() {
            out.extend_from_slice(&cap.prompt);
        }
        let _ = display_tx.send(out).await;
        return;
    }

    let prompt_template = config.compress_prompt().expect("compress_prompt is missing in config");
    let prompt = prompt_template
        .replace("{cmd}", &cap.cmd)
        .replace("{clean_output}", &clean);

    // Lock, clear history (one-shot — no cross-command context), start stream.
    // The mutex is released as soon as the TokenStream is returned so concurrent
    // commands can each start their own stream independently.
    let stream_result = tokio::time::timeout(timeout, async move {
        let mut m = model.lock().await;
        m.clear_history();
        m.prompt_stream(&prompt).await.map_err(|e| anyhow::anyhow!("{e}"))
    })
    .await;

    match stream_result {
        Ok(Ok(mut stream)) => {
            let _ = display_tx.send(b"\r\n".to_vec()).await;
            loop {
                match tokio::time::timeout(timeout, stream.next()).await {
                    Ok(Some(Ok(token))) => {
                        let text = token.replace('\n', "\r\n");
                        let _ = display_tx.send(text.into_bytes()).await;
                    }
                    Ok(Some(Err(e))) => {
                        tracing::warn!("LLM stream error: {e}");
                        break;
                    }
                    Ok(None) | Err(_) => break,
                }
            }
            if !cap.prompt.is_empty() {
                let _ = display_tx.send(cap.prompt).await;
            }
        }
        Ok(Err(e)) => {
            let mut out = format!("\r\n[AURA: summarize error: {}]\r\n", e).into_bytes();
            out.extend_from_slice(&cap.bytes);
            if !cap.prompt.is_empty() {
                out.extend_from_slice(&cap.prompt);
            }
            let _ = display_tx.send(out).await;
        }
        Err(_) => {
            let mut out = b"\r\n[AURA: summarize timeout]\r\n".to_vec();
            out.extend_from_slice(&cap.bytes);
            if !cap.prompt.is_empty() {
                out.extend_from_slice(&cap.prompt);
            }
            let _ = display_tx.send(out).await;
        }
    }
}

