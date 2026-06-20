//! User configuration (TOML) under ~/.config/cc-pet/config.toml.
//! All knobs that were top-of-file constants in the Python app live here, so the
//! growing tool stays configurable without recompiles.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub terminal: TerminalConfig,
    pub quota: QuotaConfig,
    pub pet: PetConfig,
    pub providers: ProvidersConfig,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct TerminalConfig {
    /// "wezterm" (full live-status) or "generic" (launch-only template).
    pub backend: String,
    /// Generic backend: command template for resuming/opening a session.
    /// Placeholders: {cwd} {cmd}
    pub spawn_template: String,
    /// Generic backend: command template for a brand-new session.
    pub new_template: String,
}

impl Default for TerminalConfig {
    fn default() -> Self {
        TerminalConfig {
            backend: "wezterm".to_string(),
            spawn_template: "wezterm start --cwd {cwd} -- bash -lc {cmd}".to_string(),
            new_template: "wezterm start --cwd {cwd} -- bash -lc {cmd}".to_string(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct QuotaConfig {
    /// How often to refetch the Claude OAuth usage endpoint (it rate-limits hard).
    pub claude_poll_secs: u64,
    /// Optional manual fallback limits (tokens) if the live API is unavailable.
    pub claude_daily_token_limit: Option<u64>,
    pub claude_weekly_token_limit: Option<u64>,
}

impl Default for QuotaConfig {
    fn default() -> Self {
        QuotaConfig {
            claude_poll_secs: 300,
            claude_daily_token_limit: None,
            claude_weekly_token_limit: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct PetConfig {
    pub max_size: u32,
    pub refresh_secs: u64,
    pub teleport_min_secs: u64,
    pub teleport_max_secs: u64,
    pub pet_interval_secs: u64,
    pub pets_required: u32,
}

impl Default for PetConfig {
    fn default() -> Self {
        PetConfig {
            max_size: 110,
            refresh_secs: 2,
            teleport_min_secs: 10 * 60,
            teleport_max_secs: 30 * 60,
            pet_interval_secs: 3 * 3600,
            pets_required: 10,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ProvidersConfig {
    pub claude: bool,
    pub codex: bool,
}

impl Default for ProvidersConfig {
    fn default() -> Self {
        ProvidersConfig { claude: true, codex: true }
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            terminal: TerminalConfig::default(),
            quota: QuotaConfig::default(),
            pet: PetConfig::default(),
            providers: ProvidersConfig::default(),
        }
    }
}

pub fn config_path() -> PathBuf {
    directories::ProjectDirs::from("", "", "cc-pet")
        .map(|d| d.config_dir().join("config.toml"))
        .unwrap_or_else(|| PathBuf::from(".config/cc-pet/config.toml"))
}

impl Config {
    pub fn load() -> Config {
        let p = config_path();
        if let Ok(data) = fs::read_to_string(&p) {
            if let Ok(c) = toml::from_str::<Config>(&data) {
                return c;
            }
        }
        let c = Config::default();
        c.save(); // write a documented default on first run
        c
    }

    pub fn save(&self) {
        let p = config_path();
        if let Some(parent) = p.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(data) = toml::to_string_pretty(self) {
            let _ = fs::write(&p, data);
        }
    }
}
