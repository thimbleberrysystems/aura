use std::time::Duration;
use rig::providers::ollama;
use rig::client::{Nothing, CompletionClient as _};
use rig::completion::Prompt;
use tokio::sync::mpsc;
use tracing::debug;
use crate::cfg::Config;
use crate::pty::{CapturedCommand, strip_ansi};
use crate::ingest::{rag_query, rag_store};

/// Stage 2 of the pipeline: for each captured command, runs:
///   RAG query -> Ollama summarize -> display -> RAG store
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
    let rag_disabled = config.disable_rag();
    let ollama_url = config.ollama_base_url();
    let completion_model = config.completion_model();
    let embedding_model = config.embedding_model();
    let timeout = Duration::from_secs(config.summarize_timeout_secs());

    // Step 1: RAG query (skipped when summarization is disabled or output is short).
    let context = if !disabled && !rag_disabled && clean.len() >= threshold {
        rag_query(&ollama_url, &embedding_model, &cap.cmd, &clean).await
    } else {
        vec![]
    };

    // Step 2: Summarize.
    let (display_bytes, to_store) = if disabled || clean.len() < threshold {
        (cap.bytes.clone(), clean.clone())
    } else {
        match tokio::time::timeout(
            timeout,
            call_ollama_summarize(&ollama_url, &completion_model, &cap.cmd, &clean, &context),
        )
        .await
        {
            Ok(Ok(summary)) if is_useful(&summary, &clean) => {
                let body = summary.trim_end().to_string();
                let normalised = body.replace('\n', "\r\n");
                let mut out = b"\r\n[AURA] summarized (export AURA_DISABLE_SUMMARY=1 to disable)\r\n".to_vec();
                out.extend_from_slice(normalised.as_bytes());
                out.extend_from_slice(b"\r\n");
                (out, body)
            }
            Ok(Ok(_)) => (cap.bytes.clone(), clean.clone()),
            Ok(Err(e)) => {
                let msg = format!("\r\n[AURA: summarize error: {}]\r\n", e);
                let mut out = msg.into_bytes();
                out.extend_from_slice(&cap.bytes);
                (out, clean.clone())
            }
            Err(_) => {
                let mut out = b"\r\n[AURA: summarize timeout]\r\n".to_vec();
                out.extend_from_slice(&cap.bytes);
                (out, clean.clone())
            }
        }
    };

    // Step 3: Send to display (prepend \r\n, re-append shell prompt).
    let mut out = b"\r\n".to_vec();
    out.extend_from_slice(&display_bytes);
    if !cap.prompt.is_empty() {
        out.extend_from_slice(&cap.prompt);
    }
    let _ = display_tx.send(out).await;

    // Step 4: RAG store (fire and forget — never blocks display).
    if !rag_disabled && !to_store.trim().is_empty() {
        let content = format!("Command: {}\n{}", cap.cmd, to_store);
        tokio::spawn(async move {
            rag_store(&ollama_url, &embedding_model, &content).await;
        });
    }
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
    context_chunks: &[String],
) -> anyhow::Result<String> {
    let client = ollama::Client::builder()
        .api_key(Nothing)
        .base_url(base_url)
        .build()?;
    let agent = client.agent(model).build();
    let context_block = if context_chunks.is_empty() {
        String::new()
    } else {
        let items = context_chunks
            .iter()
            .enumerate()
            .map(|(i, c)| format!("[{}] {}", i + 1, c))
            .collect::<Vec<_>>()
            .join("\n---\n");
        format!("Previous Context (similar past commands, for reference only):\n{}\n\n", items)
    };
    debug!("rag: injecting {} context chunks", context_chunks.len());
    let prompt = format!(
        "Distill this terminal output for another LLM.\n\
Discard: progress bars, UI noise, ANSI codes, and repetitive in-progress logs.\n\
Preserve: Error messages, stack traces, exit codes, and unique identifiers (IPs, IDs, paths).\n\
Constraint: Output ONLY the distilled data. No conversational filler. No leading preamble.\n\
Goal: You are a compressor. Reduce text size while preserving important info for an LLM reader.\n\
If the output is already concise, return it as-is.\n\
Optional: Use previous context only to understand what details matter. If not useful, ignore it.\n\
No unnecessary line breaks, no preamble like 'Summary:'. Just return the distilled text.\n\
{context_block}\n\
Command: {cmd}\n\
<BEGIN_OUTPUT>\n\
{clean_output}\n\
<END_OUTPUT>",
        context_block = context_block,
        cmd = cmd,
        clean_output = clean_output,
    );
    Ok(agent.prompt(&prompt).await?)
}
