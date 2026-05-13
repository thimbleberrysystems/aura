use serde::Deserialize;
use std::fs;
use std::path::Path;

#[derive(Deserialize, Debug, Default, Clone)]
pub struct Config {
    pub logging: Option<bool>,
}

/// Where a configuration value came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Env,
    Config,
    Default,
}

pub const DEFAULT_CONTROL_TCP: &str = "127.0.0.1:40001";
pub const DEFAULT_MODEL_ADDR: &str = "127.0.0.1:11434";

impl Config {
    /// Model name for genai (e.g. "llama3.2", "gpt-4o"). Override with AURA_MODEL_NAME.
    pub fn model_name(&self) -> String {
        std::env::var("AURA_MODEL_NAME").unwrap_or_else(|_| "llama3.2".to_string())
    }

    /// Model endpoint URL. Override with AURA_MODEL_ENDPOINT.
    pub fn model_endpoint(&self) -> Option<String> {
        std::env::var("AURA_MODEL_ENDPOINT").ok()
    }

    /// Model API key. Override with AURA_MODEL_API_KEY.
    pub fn model_api_key(&self) -> Option<String> {
        std::env::var("AURA_MODEL_API_KEY").ok()
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

    /// Prompt template for LLM compression. Override with AURA_COMPRESS_PROMPT.
    /// The string may contain `{cmd}`, `{clean_output}`, and `{context_block}` placeholders.
    pub fn compress_prompt(&self) -> String {
        std::env::var("AURA_COMPRESS_PROMPT").unwrap_or_else(|_| {
            "Summarize the below shell output (will be used by other LLM), retaining essential information only for another LLM.\
\nCommand: {cmd}\n<BEGIN_OUTPUT>\n{clean_output}\n<END_OUTPUT>".to_string()
        })
    }

    /// Control TCP address plus its source.
    pub fn control_tcp_with_source(&self) -> (String, Source) {
        if let Ok(v) = std::env::var("AURA_CONTROL_TCP") {
            return (v, Source::Env);
        }
        (DEFAULT_CONTROL_TCP.to_string(), Source::Default)
    }

    /// Model server address plus its source (host:port). Default: 127.0.0.1:11434
    pub fn model_addr_with_source(&self) -> (String, Source) {
        if let Ok(v) = std::env::var("AURA_MODEL_ADDR") {
            return (v, Source::Env);
        }
        (DEFAULT_MODEL_ADDR.to_string(), Source::Default)
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
