use anyhow::Context;
use config::File;
use serde::{Deserialize, Serialize};
use std::env;
use std::path::PathBuf;

#[derive(Deserialize, Serialize, Debug, Default, Clone)]
pub struct Config {
    pub logging: Option<LoggingConfig>,
    pub summary: Option<SummaryConfig>,
    pub compress: Option<CompressConfig>,
    pub server: Option<ServerConfig>,
    pub model: Option<ModelConfig>,
}

#[derive(Deserialize, Serialize, Debug, Default, Clone)]
pub struct LoggingConfig {
    pub enabled: Option<bool>,
}

#[derive(Deserialize, Serialize, Debug, Default, Clone)]
pub struct SummaryConfig {
    pub disable: Option<bool>,
    pub threshold: Option<usize>,
    pub timeout_secs: Option<u64>,
}

#[derive(Deserialize, Serialize, Debug, Default, Clone)]
pub struct CompressConfig {
    pub prompt: Option<String>,
}

#[derive(Deserialize, Serialize, Debug, Default, Clone)]
pub struct ServerConfig {
    pub control_tcp: Option<String>,
}

#[derive(Deserialize, Serialize, Debug, Default, Clone)]
pub struct ModelConfig {
    pub name: Option<String>,
    pub addr: Option<String>,
}

/// Where a configuration value came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Config,
    Missing,
}

impl Config {
    /// Logging enabled flag plus its source.
    pub fn logging_enabled_with_source(&self) -> (Option<bool>, Source) {
        if let Some(logging) = &self.logging {
            if let Some(enabled) = logging.enabled {
                return (Some(enabled), Source::Config);
            }
        }
        (None, Source::Missing)
    }

    /// Disable-summary value plus its source.
    pub fn disable_summary_with_source(&self) -> (Option<bool>, Source) {
        if let Some(summary) = &self.summary {
            if let Some(disable) = summary.disable {
                return (Some(disable), Source::Config);
            }
        }
        (None, Source::Missing)
    }

    /// Summarize threshold plus its source.
    pub fn summarize_threshold_with_source(&self) -> (Option<usize>, Source) {
        if let Some(summary) = &self.summary {
            if let Some(threshold) = summary.threshold {
                return (Some(threshold), Source::Config);
            }
        }
        (None, Source::Missing)
    }

    /// Summarize timeout plus its source.
    pub fn summarize_timeout_secs_with_source(&self) -> (Option<u64>, Source) {
        if let Some(summary) = &self.summary {
            if let Some(timeout_secs) = summary.timeout_secs {
                return (Some(timeout_secs), Source::Config);
            }
        }
        (None, Source::Missing)
    }

    /// Prompt template for LLM compression.
    pub fn compress_prompt(&self) -> Option<String> {
        self.compress.as_ref().and_then(|c| c.prompt.clone())
    }

    /// Control TCP address plus its source.
    pub fn control_tcp_with_source(&self) -> (Option<String>, Source) {
        if let Some(server) = &self.server {
            if let Some(control_tcp) = server.control_tcp.clone() {
                return (Some(control_tcp), Source::Config);
            }
        }
        (None, Source::Missing)
    }

    /// Control TCP address.
    pub fn control_tcp(&self) -> Option<String> {
        self.server.as_ref().and_then(|s| s.control_tcp.clone())
    }

    /// Model server address plus its source.
    pub fn model_addr_with_source(&self) -> (Option<String>, Source) {
        if let Some(model) = &self.model {
            if let Some(addr) = model.addr.clone() {
                return (Some(addr), Source::Config);
            }
        }
        (None, Source::Missing)
    }

    /// Model server address.
    pub fn model_addr(&self) -> Option<String> {
        self.model.as_ref().and_then(|m| m.addr.clone())
    }

    /// Model name.
    pub fn model_name(&self) -> Option<String> {
        self.model.as_ref().and_then(|m| m.name.clone())
    }

    /// Model base URL for Ollama (derived from addr, e.g. "http://127.0.0.1:11434").
    pub fn model_base_url(&self) -> Option<String> {
        self.model_addr().map(|addr| format!("http://{}", addr))
    }

    /// Validate that all required config values are present.
    pub fn validate(&self) -> anyhow::Result<()> {
        self.logging_enabled_with_source().0
            .context("logging.enabled is missing in config")?;
        self.disable_summary_with_source().0
            .context("summary.disable is missing in config")?;
        self.summarize_threshold_with_source().0
            .context("summary.threshold is missing in config")?;
        self.summarize_timeout_secs_with_source().0
            .context("summary.timeout_secs is missing in config")?;
        self.compress_prompt()
            .context("compress.prompt is missing in config")?;
        self.control_tcp()
            .context("server.control_tcp is missing in config")?;
        self.model_name()
            .context("model.name is missing in config")?;
        Ok(())
    }
}

fn config_file_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    if let Ok(cwd) = env::current_dir() {
        paths.extend(ancestor_config_paths(&cwd));
    }

    if let Ok(exe) = env::current_exe() {
        if let Some(dir) = exe.parent() {
            paths.extend(ancestor_config_paths(dir));
        }
    }

    if let Ok(xdg_config) = env::var("XDG_CONFIG_HOME") {
        paths.push(PathBuf::from(xdg_config).join("aura/aura.toml"));
    }

    if let Ok(home) = env::var("HOME") {
        paths.push(PathBuf::from(home).join(".config/aura/aura.toml"));
    }

    paths
}

fn ancestor_config_paths(start: &std::path::Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let mut current = start.to_path_buf();
    loop {
        paths.push(current.join("config/aura.toml"));
        if !current.pop() {
            break;
        }
    }
    paths
}

/// Load configuration from a file if present and validate required keys.
pub fn load_config() -> anyhow::Result<Config> {
    let mut builder = config::Config::builder();
    for path in config_file_paths() {
        builder = builder.add_source(File::from(path).required(false));
    }

    let config = builder.build()
        .and_then(|cfg| cfg.try_deserialize::<Config>())?;

    config.validate()?;
    Ok(config)
}
