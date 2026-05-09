use serde::Deserialize;
use std::fs;
use std::path::Path;

#[derive(Deserialize, Debug, Default)]
pub struct Config {
    pub logging: Option<bool>,
}

impl Config {
    pub fn logging_enabled(&self) -> bool {
        // Environment variable override: if AURA_LOGGING is set to 1/true/yes
        // enable logging. Otherwise fall back to config file value or disabled.
        if let Ok(v) = std::env::var("AURA_LOGGING") {
            let v = v.to_lowercase();
            return matches!(v.as_str(), "1" | "true" | "yes");
        }
        self.logging.unwrap_or(false)
    }
}

/// Load configuration from `config/aura.toml` if present. If parsing fails or file
/// is missing, return default `Config` with logging disabled. Environment
/// variable `AURA_LOGGING` overrides the config file.
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
