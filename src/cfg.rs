use serde::Deserialize;
use std::fs;
use std::path::Path;

#[derive(Deserialize, Debug, Default, Clone)]
pub struct Config {
    pub logging: Option<bool>,
    pub embedding_model: Option<String>,
    pub ollama_base_url: Option<String>,
    pub embedding_dims: Option<u32>,
}

impl Config {
    pub fn logging_enabled(&self) -> bool {
        if let Ok(v) = std::env::var("AURA_LOGGING") {
            let v = v.to_lowercase();
            return matches!(v.as_str(), "1" | "true" | "yes");
        }
        self.logging.unwrap_or(false)
    }

    pub fn embedding_model(&self) -> String {
        if let Ok(v) = std::env::var("AURA_EMBEDDING_MODEL") {
            return v;
        }
        self.embedding_model
            .clone()
            .unwrap_or_else(|| "nomic-embed-text".to_string())
    }

    pub fn ollama_base_url(&self) -> String {
        if let Ok(v) = std::env::var("AURA_OLLAMA_BASE_URL") {
            return v;
        }
        self.ollama_base_url
            .clone()
            .unwrap_or_else(|| "http://localhost:11434".to_string())
    }

    // sqlite_path removed: we no longer persist to SQLite in ephemeral mode.

    pub fn embedding_dims(&self) -> u32 {
        if let Ok(v) = std::env::var("AURA_EMBEDDING_DIMS") {
            if let Ok(n) = v.parse::<u32>() {
                return n;
            }
        }
        self.embedding_dims.unwrap_or(768)
    }

    pub fn completion_model(&self) -> String {
        if let Ok(v) = std::env::var("AURA_COMPLETION_MODEL") {
            return v;
        }
        "llama3".to_string()
    }

    /// Maximum byte length of clean stdout before we bother calling Ollama.
    /// If the output is shorter than this, we display it as-is.
    /// Default: 250. Override with AURA_SUMMARIZE_THRESHOLD.
    pub fn summarize_threshold(&self) -> usize {
        if let Ok(v) = std::env::var("AURA_SUMMARIZE_THRESHOLD") {
            if let Ok(n) = v.parse::<usize>() { return n; }
        }
        250
    }

    /// Timeout in seconds for the Ollama summarize call.
    /// Default: 30. Override with AURA_SUMMARIZE_TIMEOUT_SECS.
    pub fn summarize_timeout_secs(&self) -> u64 {
        if let Ok(v) = std::env::var("AURA_SUMMARIZE_TIMEOUT_SECS") {
            if let Ok(n) = v.parse::<u64>() { return n; }
        }
        3000
    }
}

/// Load configuration from `config/aura.toml` if present.
pub fn load_config() -> Config {
    let path = Path::new("config/aura.toml");
    if !path.exists() {
        return Config::default();
    }
    match fs::read_to_string(path) {
        Ok(s) => match toml::from_str::<Config>(&s) {
            Ok(c) => c,
            Err(_) => Config::default(),
        },
        Err(_) => Config::default(),
    }
}
