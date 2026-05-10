use anyhow::Result;
use rig::Embed;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, info, warn};
use once_cell::sync::OnceCell;

use crate::cfg::Config;

/// A sanitized text chunk produced from PTY input or PTY output.
#[derive(Debug, Clone)]
pub struct SanitizedChunk {
    pub text: String,
}

/// The document type stored in the in-memory vector store.
#[derive(Embed, Clone, Debug, Deserialize, Serialize)]
pub struct TerminalChunkDoc {
    #[embed]
    pub content: String,
}

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
            }).collect();
        scores.sort_unstable_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        if scores.len() > k { scores.truncate(k); }
        scores
    }
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b.iter()).map(|(x,y)| x * y).sum()
}
fn norm(a: &[f32]) -> f32 { dot(a,a).sqrt() }

static GLOBAL_STORE: OnceCell<Arc<RwLock<InMemoryStore>>> = OnceCell::new();

pub fn init_global_store() {
    GLOBAL_STORE.get_or_init(|| Arc::new(RwLock::new(InMemoryStore::new())));
}

pub fn global_store() -> Arc<RwLock<InMemoryStore>> {
    GLOBAL_STORE.get().expect("global store not initialized").clone()
}
// ---------------------------------------------------------------------------------------

/// Start the background ingestion worker.
/// Returns a sender; the caller pushes `SanitizedChunk`s onto it.
pub fn start_ingest_worker(cfg: Config) -> mpsc::Sender<SanitizedChunk> {
    let (tx, rx) = mpsc::channel::<SanitizedChunk>(1024);
    let cfg = Arc::new(cfg);
    tokio::spawn(run_worker(rx, cfg));
    tx
}

async fn run_worker(mut rx: mpsc::Receiver<SanitizedChunk>, cfg: Arc<Config>) {
    let max_batch = 32usize;
    let max_wait = Duration::from_millis(500);

    loop {
        let first = match rx.recv().await {
            Some(c) => c,
            None => {
                info!("ingest channel closed; worker exiting");
                return;
            }
        };

        let mut batch = Vec::with_capacity(max_batch);
        batch.push(first);

        let deadline = tokio::time::Instant::now() + max_wait;
        loop {
            if batch.len() >= max_batch {
                break;
            }
            match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Some(chunk)) => batch.push(chunk),
                Ok(None) => {
                    process_batch(&cfg, batch).await;
                    return;
                }
                Err(_) => break,
            }
        }

        process_batch(&cfg, batch).await;
    }
}

async fn process_batch(cfg: &Config, batch: Vec<SanitizedChunk>) {
    if batch.is_empty() {
        return;
    }
    debug!("ingest: processing batch of {} chunks", batch.len());
    if let Err(e) = embed_and_store(cfg, batch).await {
        warn!("ingest: embed_and_store error: {:#}", e);
    }
}

async fn embed_and_store(cfg: &Config, batch: Vec<SanitizedChunk>) -> Result<()> {
    use rig::client::{EmbeddingsClient, Nothing};
    use rig::embeddings::EmbeddingsBuilder;
    use rig::providers::ollama;

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

    // Convert batch into TerminalChunkDoc instances (store only content for embedding).
    fn looks_like_prompt(s: &str) -> bool {
        let t = s.trim();
        if t.is_empty() { return false }
        // crude heuristic: username@host:path$ or ends with >
        if (t.ends_with('$') || t.ends_with('>')) && t.contains('@') && t.contains(':') {
            return true;
        }
        false
    }

    let docs: Vec<TerminalChunkDoc> = batch
        .iter()
        .map(|c| TerminalChunkDoc { content: c.text.clone() })
        .filter(|d| {
            let t = d.content.trim();
            if t.is_empty() { return false }
            if looks_like_prompt(t) { return false }
            // require at least one printable non-control char
            if !t.chars().any(|c| !c.is_control()) { return false }
            true
        })
        .collect();

    if docs.is_empty() {
        debug!("ingest: no non-empty documents to embed");
        return Ok(());
    }

    // Debug: print the documents being embedded
    eprintln!("=== EMBEDDING DOCUMENTS ({} docs) ===", docs.len());
    for (i, d) in docs.iter().enumerate() {
        eprintln!("--- doc[{}] (len={}) ---\n'{}'\n", i, d.content.len(), d.content);
    }
    // trailing newline for clarity
    eprintln!("");

    // Build embeddings for all docs.
    let raw_embeddings = EmbeddingsBuilder::new(model.clone())
        .documents(docs)?
        .build()
        .await
        .map_err(|e| anyhow::anyhow!("embed: {e}"))?;

    // Convert Rig Embedding (f64) -> Vec<f32> and collect pairs
    let embeddings: Vec<(TerminalChunkDoc, Vec<f32>)> = raw_embeddings
        .into_iter()
        .map(|(doc, emb)| {
            // emb: OneOrMany<Embedding> - take first embedding and convert f64->f32
            let e = emb.first();
            let v: Vec<f32> = e.vec.into_iter().map(|x| x as f32).collect();
            (doc, v)
        })
        .collect();

    let n = embeddings.len();

    use std::sync::atomic::{AtomicU64, Ordering};
    static ID_COUNTER: AtomicU64 = AtomicU64::new(1);

    // Initialize global in-memory store (ephemeral)
    init_global_store();

    // Prepare batch for in-memory store: generate internal ids, keep only content+embedding
    let mut batch_for_mem: Vec<(String, Vec<f32>, String)> = Vec::with_capacity(embeddings.len());
    for (doc, emb) in embeddings {
        let emb_vec: Vec<f32> = emb;
        let id = format!("{}-{}", now_millis(), ID_COUNTER.fetch_add(1, Ordering::SeqCst));
        batch_for_mem.push((id, emb_vec, doc.content.clone()));
    }

    let store = global_store();
    {
        let mut w = store.write().await;
        w.add_batch(batch_for_mem).await;
    }

    info!("ingest: stored {} chunks into in-memory store", n);
    Ok(())
}

/// Build a millisecond-precision Unix timestamp.
pub fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
