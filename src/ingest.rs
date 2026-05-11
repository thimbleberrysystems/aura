use anyhow::Result;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};
use once_cell::sync::OnceCell;

// --- In-memory vector store (ephemeral) -------------------------------------------------

pub struct StoredChunk {
    pub id: String,
    pub embedding: Vec<f32>,
    pub content: String,
}

pub struct InMemoryStore {
    items: Vec<StoredChunk>,
}

impl InMemoryStore {
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    pub async fn add_batch(&mut self, docs: Vec<(String, Vec<f32>, String)>) {
        for (id, emb, content) in docs {
            self.items.push(StoredChunk { id, embedding: emb, content });
        }
    }

    pub fn top_k(&self, q_emb: &[f32], k: usize) -> Vec<(String, f32, String)> {
        let q_norm = norm(q_emb).max(1e-6);
        let mut scores: Vec<(String, f32, String)> = self.items.iter()
            .map(|it| {
                let s = dot(q_emb, &it.embedding) / (q_norm * norm(&it.embedding).max(1e-6));
                (it.id.clone(), s, it.content.clone())
            })
            .collect();
        scores.sort_unstable_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        if scores.len() > k { scores.truncate(k); }
        scores
    }
}

fn dot(a: &[f32], b: &[f32]) -> f32 { a.iter().zip(b.iter()).map(|(x, y)| x * y).sum() }
fn norm(a: &[f32]) -> f32 { dot(a, a).sqrt() }

static GLOBAL_STORE: OnceCell<Arc<RwLock<InMemoryStore>>> = OnceCell::new();

pub fn init_global_store() {
    GLOBAL_STORE.get_or_init(|| Arc::new(RwLock::new(InMemoryStore::new())));
}

pub fn global_store() -> Arc<RwLock<InMemoryStore>> {
    GLOBAL_STORE.get().expect("global store not initialized").clone()
}

// --- Core embedding & storage -----------------------------------------------------------

/// Embed a string via Ollama's /api/embeddings endpoint.
pub async fn embed_text(base_url: &str, model: &str, text: &str) -> Result<Vec<f32>> {
    let url = format!("{}/api/embeddings", base_url.trim_end_matches('/'));
    let resp: serde_json::Value = reqwest::Client::new()
        .post(&url)
        .json(&serde_json::json!({ "model": model, "prompt": text }))
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("embed HTTP request failed: {e}"))?
        .json()
        .await
        .map_err(|e| anyhow::anyhow!("embed HTTP response not JSON: {e}"))?;

    let arr = resp["embedding"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!(
            "Ollama /api/embeddings returned no 'embedding' field — is model '{}' pulled?", model
        ))?;

    Ok(arr.iter().map(|v| v.as_f64().unwrap_or(0.0) as f32).collect())
}

/// Embed `text` and persist it to the global in-memory store.
pub async fn store_text(base_url: &str, model: &str, text: &str) -> Result<()> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static STORE_ID: AtomicU64 = AtomicU64::new(1);

    init_global_store();
    let emb = embed_text(base_url, model, text).await?;
    let id = format!("{}-{}", now_millis(), STORE_ID.fetch_add(1, Ordering::SeqCst));
    global_store().write().await.add_batch(vec![(id, emb, text.to_string())]).await;
    info!("rag: stored chunk (len={})", text.len());
    Ok(())
}

// --- Pipeline-facing API ----------------------------------------------------------------

/// Query the RAG store: embed the query and return the top-k matching contents.
pub async fn rag_query(base_url: &str, model: &str, cmd: &str, clean: &str) -> Vec<String> {
    let q = format!("Command: {}\n{}", cmd, &clean[..clean.len().min(512)]);
    match embed_text(base_url, model, &q).await {
        Ok(emb) => {
            let store = global_store();
            let r = store.read().await;
            let hits = r.top_k(&emb, 3);
            debug!("rag: top_k returned {} hits", hits.len());
            hits.into_iter().map(|(_, _, c)| c).collect()
        }
        Err(e) => {
            warn!("rag: query failed: {:#}", e);
            vec![]
        }
    }
}

/// Embed `content` and store it in the RAG store (logs warnings on failure).
pub async fn rag_store(base_url: &str, model: &str, content: &str) {
    if let Err(e) = store_text(base_url, model, content).await {
        warn!("rag: store failed: {:#}", e);
    }
}

// --- Utilities --------------------------------------------------------------------------

/// Millisecond-precision Unix timestamp.
pub fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
