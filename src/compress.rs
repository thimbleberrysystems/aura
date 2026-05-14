use std::time::Duration;
use futures::StreamExt;
use genai::Client;
use genai::ServiceTarget;
use genai::chat::{ChatRequest, ChatStream, ChatStreamEvent};
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
///   strip ANSI -> threshold check -> semantic_compress (LLM, with timeout) -> display
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
        tokio::spawn(async move { process_pipeline(config, cap, tx).await; });
    }
}

async fn process_pipeline(config: Config, cap: CapturedCommand, display_tx: mpsc::Sender<Vec<u8>>) {
    let clean = strip_ansi(&cap.bytes);
    semantic_compress(config, cap, display_tx, clean).await;
}

async fn semantic_compress(
    config: Config,
    cap: CapturedCommand,
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

    let model_cfg = ModelConfig {
        name: config.model_name().expect("model_name is missing in config"),
        endpoint: config.model_endpoint(),
        api_key: config.model_api_key(),
    };
    let prompt_template = config.compress_prompt().expect("compress_prompt is missing in config");

    let stream_result = tokio::time::timeout(
        timeout,
        start_llm_stream(model_cfg, &prompt_template, &cap.cmd, &clean),
    ).await;

    match stream_result {
        Ok(Ok(mut stream)) => {
            let _ = display_tx.send(b"\r\n".to_vec()).await;
            loop {
                match tokio::time::timeout(timeout, stream.next()).await {
                    Ok(Some(Ok(ChatStreamEvent::Chunk(chunk)))) => {
                        let text = chunk.content.replace('\n', "\r\n");
                        let _ = display_tx.send(text.into_bytes()).await;
                    }
                    Ok(Some(Ok(_))) => {}
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

async fn start_llm_stream(
    model_cfg: ModelConfig,
    prompt_template: &str,
    cmd: &str,
    clean_output: &str,
) -> anyhow::Result<ChatStream> {
    let name = model_cfg.name.clone();
    let client = build_client(model_cfg);
    let prompt = prompt_template
        .replace("{cmd}", cmd)
        .replace("{clean_output}", clean_output);
    let stream_response = client.exec_chat_stream(&name, ChatRequest::from_user(prompt), None).await?;
    Ok(stream_response.stream)
}
