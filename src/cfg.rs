use serde::Deserialize;
use std::fs;
use std::path::Path;

#[derive(Deserialize, Debug, Default)]
pub struct Config {
    pub logging: Option<bool>,
}

impl Config {
    pub fn logging_enabled(&self) -> bool {
        self.logging.unwrap_or(false)
    }
}

/// Load configuration from `config/aura.toml` if present. If parsing fails or file
/// is missing, return default `Config` with logging disabled.
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
