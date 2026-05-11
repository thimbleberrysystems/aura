use rig::providers::ollama;
use rig::client::{Nothing, CompletionClient as _};
use rig::completion::Prompt;
use tracing::debug;

/// Call Ollama to summarize command output. Returns the model's reply or an error.
/// `context_chunks` are semantically similar past summaries retrieved from the RAG store.
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
    debug!("rag: injecting {} context chunks (block len={})", context_chunks.len(), context_block.len());
    let prompt = format!(
        r#"Distill this terminal output for another LLM.
Discard: progress bars, UI noise, ANSI codes, and repetitive 'in-progress' logs.
Preserve: Error messages, stack traces, exit codes, and unique identifiers (IPs, IDs, paths).
Constraint: Output ONLY the distilled data. No conversational filler. No leading preamble.
Goal: Remember, you are a compressor which reduces text size, but still preserves important info. Your output will be read by another LLM, so make it suitable for LLM. No additional or extra info should be added.
If the output is already concise, return it as-is.
Optional: If previous context is provided, use it only to understand what details are important. Prioritise the current output. If previous context is not useful, ignore it silently.
Dont add unnecessary line breaks, and do not add any preamble like "Summary:" or "Distilled output:". Just return the distilled text.
{context_block}
Command: {cmd}
<BEGIN_OUTPUT>
{clean_output}
<END_OUTPUT>"#,
        context_block = context_block,
        cmd = cmd,
        clean_output = clean_output
    );
    let reply = agent.prompt(&prompt).await?;
    Ok(reply)
}
