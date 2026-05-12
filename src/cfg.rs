use serde::Deserialize;
use std::fs;
use std::path::Path;

#[derive(Deserialize, Debug, Default, Clone)]
pub struct Config {
    pub logging: Option<bool>,
    pub model: Option<String>,
}

/// Where a configuration value came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Env,
    Config,
    Default,
}

pub const DEFAULT_CONTROL_TCP: &str = "127.0.0.1:40001";

impl Config {
    pub fn logging_enabled(&self) -> bool {
        self.logging_with_source().0
    }

    /// The model string passed to genai. genai infers the provider automatically.
    /// e.g. "llama3.2" → Ollama, "gpt-4o" → OpenAI, "claude-3-5-sonnet" → Anthropic.
    /// Override with AURA_MODEL env var or set `model` in config/aura.toml.
    pub fn model(&self) -> String {
        self.model_with_source().0
    }

    /// If `AURA_DISABLE_SUMMARY` is set (1/true/yes), disable summarization.
    pub fn disable_summary(&self) -> bool {
        self.disable_summary_with_source().0
    }

    /// Maximum byte length of clean stdout before we bother calling Ollama.
    /// If the output is shorter than this, we display it as-is.
    /// Default: 250. Override with AURA_SUMMARIZE_THRESHOLD.
    pub fn summarize_threshold(&self) -> usize {
        self.summarize_threshold_with_source().0
    }

    /// Timeout in seconds for the model summarize call.
    /// Default: 3000. Override with AURA_SUMMARIZE_TIMEOUT_SECS.
    pub fn summarize_timeout_secs(&self) -> u64 {
        self.summarize_timeout_secs_with_source().0
    }

    /// Logging value plus its source.
    pub fn logging_with_source(&self) -> (bool, Source) {
        if let Ok(v) = std::env::var("AURA_LOGGING") {
            let v = v.to_lowercase();
            return (
                matches!(v.as_str(), "1" | "true" | "yes"),
                Source::Env,
            );
        }
        if let Some(b) = self.logging {
            return (b, Source::Config);
        }
        (false, Source::Default)
    }

    /// Model value plus its source.
    pub fn model_with_source(&self) -> (String, Source) {
        if let Ok(v) = std::env::var("AURA_MODEL") {
            return (v, Source::Env);
        }
        if let Some(m) = &self.model {
            return (m.clone(), Source::Config);
        }
        ("llama3.2".to_string(), Source::Default)
    }

    /// Disable-summary value plus its source.
    pub fn disable_summary_with_source(&self) -> (bool, Source) {
        if let Ok(v) = std::env::var("AURA_DISABLE_SUMMARY") {
            let v = v.to_lowercase();
            return (matches!(v.as_str(), "1" | "true" | "yes"), Source::Env);
        }
        (false, Source::Default)
    }

    /// Summarize threshold plus its source.
    pub fn summarize_threshold_with_source(&self) -> (usize, Source) {
        if let Ok(v) = std::env::var("AURA_SUMMARIZE_THRESHOLD") {
            if let Ok(n) = v.parse::<usize>() { return (n, Source::Env); }
        }
        (250, Source::Default)
    }

    /// Summarize timeout plus its source.
    pub fn summarize_timeout_secs_with_source(&self) -> (u64, Source) {
        if let Ok(v) = std::env::var("AURA_SUMMARIZE_TIMEOUT_SECS") {
            if let Ok(n) = v.parse::<u64>() { return (n, Source::Env); }
        }
        (3000, Source::Default)
    }

    /// Control TCP address plus its source.
    pub fn control_tcp_with_source(&self) -> (String, Source) {
        if let Ok(v) = std::env::var("AURA_CONTROL_TCP") {
            return (v, Source::Env);
        }
        (DEFAULT_CONTROL_TCP.to_string(), Source::Default)
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
