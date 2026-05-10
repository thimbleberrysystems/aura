use anyhow::Result;
use rig::client::{EmbeddingsClient, Nothing};
use rig::embeddings::EmbeddingsBuilder;
use rig::providers::ollama;
use rig::client::CompletionClient;
use rig::completion::Prompt;

use crate::ingest::{TerminalChunkDoc, global_store};

/// Perform a semantic search against the existing SQLite vector DB and return
/// the top matches as a plain-text reply. This does the same sqlite-vec
/// initialization and model construction used during ingest.
pub async fn handle_help(query: &str) -> Result<String> {
    let cfg = crate::cfg::load_config();
    let model_name = cfg.embedding_model();
    let base_url = cfg.ollama_base_url();
    let dims = cfg.embedding_dims();

    // Build Ollama client (Nothing = no API key needed for local Ollama).
    let client = ollama::Client::builder()
        .api_key(Nothing)
        .base_url(&base_url)
        .build()
        .map_err(|e| anyhow::anyhow!("ollama client: {e}"))?;

    let model = client.embedding_model_with_ndims(&model_name, dims as usize);

    // Compute embedding for the query
    let query_doc = TerminalChunkDoc { content: query.to_string() };
    let raw_q_embs = EmbeddingsBuilder::new(model.clone())
        .documents(vec![query_doc])?
        .build()
        .await
        .map_err(|e| anyhow::anyhow!("embed query: {e}"))?;

    let q_vec: Vec<f32> = if let Some((_d, emb)) = raw_q_embs.into_iter().next() {
        let e = emb.first();
        e.vec.into_iter().map(|x| x as f32).collect()
    } else {
        return Ok("No embedding produced for query.".to_string());
    };

    // Query the in-memory global store
    let store = global_store();
    let results = {
        let r = store.read().await;
        r.top_k(&q_vec, 5)
    };

    if results.is_empty() {
        return Ok("No relevant context found.".to_string());
    }

    // Build context text from top results (only the document text)
    let mut context_text = String::new();
    for (_id, _score, content) in &results {
        context_text.push_str(&format!("{}\n\n", content));
    }

    // Build prompt: include user query and the retrieved context
    let prompt = format!(
        "Use the following context to answer the user's question. If the answer is not contained in the context, say you don't know.\n\nContext:\n{}\nUser Question:\n{}\n\nAnswer:",
        context_text, query
    );

    // Completion model (from config)
    let completion_model = cfg.completion_model();

    // Build an agent and prompt it
    let agent = client
        .agent(completion_model)
        .preamble("You are a helpful assistant. Answer concisely using only the provided context.")
        .build();
    // Debug: print the exact prompt sent to the completion model
    eprintln!("=== PROMPT SENT TO MODEL START ===\n{}\n=== PROMPT SENT TO MODEL END ===", prompt);

    let response: String = agent.prompt(&prompt).await.map_err(|e| anyhow::anyhow!("completion: {e}"))?;

    Ok(response)
}
