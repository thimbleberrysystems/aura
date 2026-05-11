use std::time::Duration;
use rig::providers::ollama;
use rig::client::{Nothing, CompletionClient as _};
use rig::completion::Prompt;
use tokio::sync::mpsc;
use crate::cfg::Config;
use crate::pty::{CapturedCommand, strip_ansi};

/// Stage 2 of the pipeline: for each captured command, runs:
///   strip ANSI -> threshold check -> Ollama summarize (with timeout) -> display
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
    let threshold = config.summarize_threshold();
    let disabled = config.disable_summary();
    let ollama_url = config.ollama_base_url();
    let completion_model = config.completion_model();
    let timeout = Duration::from_secs(config.summarize_timeout_secs());

    let display_bytes = if disabled || clean.len() < threshold {
        // Short output or summaries disabled — display as-is.
        cap.bytes.clone()
    } else {
        match tokio::time::timeout(timeout, call_ollama_summarize(&ollama_url, &completion_model, &cap.cmd, &clean)).await {
            Ok(Ok(summary)) if is_useful(&summary, &clean) => {
                let normalised = summary.trim_end().replace('\n', "\r\n");
                let mut out = b"\r\n[AURA] summarized (export AURA_DISABLE_SUMMARY=1 to disable)\r\n".to_vec();
                out.extend_from_slice(normalised.as_bytes());
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

/// Call Ollama to summarize command output. Returns the model reply or an error.
pub async fn call_ollama_summarize(
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
        "Distill this terminal output for another LLM.\n\
Discard: progress bars, UI noise, ANSI codes, and repetitive in-progress logs.\n\
Preserve: Error messages, stack traces, exit codes, and unique identifiers (IPs, IDs, paths).\n\
Constraint: Output ONLY the distilled data. No conversational filler. No leading preamble.\n\
Goal: You are a compressor. Reduce text size while preserving important info for an LLM reader.\n\
If the output is already concise, return it as-is.\n\
No unnecessary line breaks, no preamble like \"Summary:\". Just return the distilled text.\n\
Command: {cmd}\n\
<BEGIN_OUTPUT>\n\
{clean_output}\n\
<END_OUTPUT>",
        cmd = cmd,
        clean_output = clean_output,
    );
    Ok(agent.prompt(&prompt).await?)
}
