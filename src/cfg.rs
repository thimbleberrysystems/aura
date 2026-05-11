use serde::Deserialize;
use std::fs;
use std::path::Path;

#[derive(Deserialize, Debug, Default, Clone)]
pub struct Config {
    pub logging: Option<bool>,
    pub model: Option<String>,
}

impl Config {
    pub fn logging_enabled(&self) -> bool {
        if let Ok(v) = std::env::var("AURA_LOGGING") {
            let v = v.to_lowercase();
            return matches!(v.as_str(), "1" | "true" | "yes");
        }
        self.logging.unwrap_or(false)
    }

    /// The model string passed to genai. genai infers the provider automatically.
    /// e.g. "llama3.2" → Ollama, "gpt-4o" → OpenAI, "claude-3-5-sonnet" → Anthropic.
    /// Override with AURA_MODEL env var or set `model` in config/aura.toml.
    pub fn model(&self) -> String {
        if let Ok(v) = std::env::var("AURA_MODEL") {
            return v;
        }
        self.model
            .clone()
            .unwrap_or_else(|| "llama3.2".to_string())
    }

    /// If `AURA_DISABLE_SUMMARY` is set (1/true/yes), disable summarization.
    pub fn disable_summary(&self) -> bool {
        if let Ok(v) = std::env::var("AURA_DISABLE_SUMMARY") {
            let v = v.to_lowercase();
            return matches!(v.as_str(), "1" | "true" | "yes");
        }
        false
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

    /// Timeout in seconds for the model summarize call.
    /// Default: 3000. Override with AURA_SUMMARIZE_TIMEOUT_SECS.
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
