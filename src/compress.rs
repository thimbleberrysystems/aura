use std::time::Duration;
use genai::Client;
use genai::ServiceTarget;
use genai::chat::ChatRequest;
use genai::resolver::{AuthData, ServiceTargetResolver};
use tokio::sync::mpsc;
use crate::cfg::Config;
use crate::pty::{CapturedCommand, strip_ansi};

struct ModelConfig {
    name: String,
    endpoint: Option<String>,
    api_key: Option<String>,
}

fn build_client(cfg: ModelConfig) -> Client {
    let resolver = ServiceTargetResolver::from_resolver_fn(move |mut target: ServiceTarget| {
        if let Some(ref addr) = cfg.endpoint {
            target.endpoint = genai::resolver::Endpoint::from_owned(addr.as_str());
        }
        if let Some(ref key) = cfg.api_key {
            target.auth = AuthData::from_single(key.clone());
        }
        Ok(target)
    });
    Client::builder().with_service_target_resolver(resolver).build()
}

/// Stage 2 of the pipeline: for each captured command, runs:
///   strip ANSI -> threshold check -> model summarize (with timeout) -> display
///
/// One tokio task is spawned per command so commands are processed concurrently.
pub async fn pipeline_task(
    config: Config,
    mut rx: mpsc::Receiver<CapturedCommand>,
    display_tx: mpsc::Sender<Vec<u8>>,
) {
    while let Some(cap) = rx.recv().await {
        let config = config.clone();
        let tx = display_tx.clone();
        tokio::spawn(async move { process_command(config, cap, tx).await; });
    }
}

async fn process_command(config: Config, cap: CapturedCommand, display_tx: mpsc::Sender<Vec<u8>>) {
    let clean = strip_ansi(&cap.bytes);
    let threshold = config.summarize_threshold_with_source().0;
    let disabled = config.disable_summary_with_source().0;
    let timeout = Duration::from_secs(config.summarize_timeout_secs_with_source().0);
    let model_cfg = ModelConfig {
        name: config.model_name(),
        endpoint: config.model_endpoint(),
        api_key: config.model_api_key(),
    };

    let display_bytes = if disabled || clean.len() < threshold {
        // Short output or summaries disabled — display as-is.
        cap.bytes.clone()
    } else {
        match tokio::time::timeout(timeout, call_semantic_compressor(model_cfg, &cap.cmd, &clean)).await {
            Ok(Ok(summary)) if is_useful(&summary, &clean) => {
                let normalised = summary.trim_end().replace('\n', "\r\n");
                let mut out = normalised.into_bytes();
                out.extend_from_slice(b"\r\n");
                out
            }
            Ok(Ok(_)) => cap.bytes.clone(),
            Ok(Err(e)) => {
                let mut out = format!("\r\n[AURA: summarize error: {}]\r\n", e).into_bytes();
                out.extend_from_slice(&cap.bytes);
                out
            }
            Err(_) => {
                let mut out = b"\r\n[AURA: summarize timeout]\r\n".to_vec();
                out.extend_from_slice(&cap.bytes);
                out
            }
        }
    };

    // Prepend \r\n and re-append the shell prompt.
    let mut out = b"\r\n".to_vec();
    out.extend_from_slice(&display_bytes);
    if !cap.prompt.is_empty() {
        out.extend_from_slice(&cap.prompt);
    }
    let _ = display_tx.send(out).await;
}

/// Returns true if the LLM summary is actually shorter and non-trivial.
fn is_useful(summary: &str, original: &str) -> bool {
    !summary.trim().is_empty()
        && !summary.trim().eq_ignore_ascii_case("ORIGINAL")
        && summary.len() < original.len()
}

/// Distill terminal output using the configured model.
async fn call_semantic_compressor(
    model_cfg: ModelConfig,
    cmd: &str,
    clean_output: &str,
) -> anyhow::Result<String> {
    let name = model_cfg.name.clone();
    let client = build_client(model_cfg);
    let prompt = format!(
        "Shorten this terminal output, retaining essential information only for another LLM.\n\
Command: {cmd}\n\
<BEGIN_OUTPUT>\n\
{clean_output}\n\
<END_OUTPUT>",
        cmd = cmd,
        clean_output = clean_output,
    );
    let response = client.exec_chat(&name, ChatRequest::from_user(prompt), None).await?;
    Ok(response.into_first_text().unwrap_or_default())
}
