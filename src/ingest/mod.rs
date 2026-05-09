use anyhow::Result;
use rig::Embed;
use rig_sqlite::{Column, ColumnValue, SqliteVectorStore, SqliteVectorStoreTable};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::cfg::Config;

/// A sanitized text chunk produced from PTY input or PTY output.
#[derive(Debug, Clone)]
pub struct SanitizedChunk {
    pub session_id: String,
    pub ts: i64,
    pub text: String,
    /// "input" for user stdin, "output" for PTY stdout
    pub direction: String,
}

/// The document type stored in the SQLite vector store.
#[derive(Embed, Clone, Debug, Deserialize, Serialize)]
struct TerminalChunkDoc {
    id: String,
    session_id: String,
    ts: i64,
    direction: String,
    #[embed]
    content: String,
}

impl SqliteVectorStoreTable for TerminalChunkDoc {
    fn name() -> &'static str {
        "terminal_chunks"
    }

    fn schema() -> Vec<Column> {
        vec![
            Column::new("id", "TEXT PRIMARY KEY"),
            Column::new("session_id", "TEXT"),
               Column::new("direction", "TEXT"),
               Column::new("ts", "INTEGER"),
               Column::new("content", "TEXT"),
        ]
    }

    fn id(&self) -> String {
        self.id.clone()
    }

    fn column_values(&self) -> Vec<(&'static str, Box<dyn ColumnValue>)> {
        vec![
            ("id", Box::new(self.id.clone())),
            ("session_id", Box::new(self.session_id.clone())),
               ("direction", Box::new(self.direction.clone())),
               ("ts", Box::new(self.ts.to_string())),
               ("content", Box::new(self.content.clone())),
        ]
    }
}

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
    use tokio_rusqlite::ffi::sqlite3_auto_extension;
    use sqlite_vec::sqlite3_vec_init;
    use tokio_rusqlite::Connection;

    let model_name = cfg.embedding_model();
    let base_url = cfg.ollama_base_url();
    let sqlite_path = cfg.sqlite_path();
    let dims = cfg.embedding_dims();

    // Build Ollama client (Nothing = no API key needed for local Ollama).
    let client = ollama::Client::builder()
        .api_key(Nothing)
        .base_url(&base_url)
        .build()
        .map_err(|e| anyhow::anyhow!("ollama client: {e}"))?;

    let model = client.embedding_model_with_ndims(&model_name, dims as usize);

    // Convert batch into TerminalChunkDoc instances.
    let docs: Vec<TerminalChunkDoc> = batch
        .iter()
        .map(|c| TerminalChunkDoc {
            id: format!("{}-{}", c.session_id, c.ts),
            session_id: c.session_id.clone(),
            ts: c.ts,
            direction: c.direction.clone(),
            content: c.text.clone(),
        })
        .collect();

    // Build embeddings for all docs.
    let embeddings: Vec<(TerminalChunkDoc, _)> = EmbeddingsBuilder::new(model.clone())
        .documents(docs)?
        .build()
        .await
        .map_err(|e| anyhow::anyhow!("embed: {e}"))
        .map(|v: Vec<(TerminalChunkDoc, _)>| v)?;

    let n = embeddings.len();

    // Open the SQLite connection with sqlite-vec extension.
    // Safety: must be called before any connection is opened.
    unsafe {
        sqlite3_auto_extension(Some(std::mem::transmute(sqlite3_vec_init as *const ())));
    }

    let conn = Connection::open(&sqlite_path).await?;

    // Create the vector store (creates table if it doesn't exist).
    let store = SqliteVectorStore::new(conn, &model).await?;

    // Insert the embedded documents.
    store.add_rows(embeddings).await?;

    info!("ingest: stored {} chunks into SQLite at {}", n, sqlite_path);
    Ok(())
}

/// Build a millisecond-precision Unix timestamp.
pub fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
