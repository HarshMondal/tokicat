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
    pub ui: UiConfig,
    pub sessions: SessionsConfig,
    pub notify: NotifyConfig,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionsConfig {
    /// Command to start a brand-new session, per provider. Defaults grant full access
    /// (skip approval/permission prompts). `{cwd}` is the working dir (the terminal
    /// already spawns in cwd, so it's usually unneeded).
    pub new_claude: String,
    pub new_codex: String,
    pub new_opencode: String,
    /// Command to resume an existing session. `{id}` is replaced with the session id.
    pub resume_claude: String,
    pub resume_codex: String,
    pub resume_opencode: String,
}

impl Default for SessionsConfig {
    fn default() -> Self {
        SessionsConfig {
            new_claude: "claude --dangerously-skip-permissions".to_string(),
            new_codex: "codex --dangerously-bypass-approvals-and-sandbox".to_string(),
            new_opencode: "opencode --model minimax/MiniMax-M2.5".to_string(),
            resume_claude: "claude --resume {id}".to_string(),
            resume_codex: "codex resume {id}".to_string(),
            resume_opencode: "opencode --session {id}".to_string(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct NotifyConfig {
    /// Shell command run (detached) when a session starts needing the user. Empty
    /// disables sound. Default uses the XDG sound theme (no fragile file path).
    pub sound_command: String,
}

impl Default for NotifyConfig {
    fn default() -> Self {
        NotifyConfig { sound_command: "canberra-gtk-play -i complete".to_string() }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    /// Overall scale for the panel's text and default window size. 1.0 = original;
    /// raise for bigger, more readable UI (clamped to a sane range at use sites).
    pub scale: f32,
}

impl Default for UiConfig {
    fn default() -> Self {
        UiConfig { scale: 1.15 }
    }
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
    /// How often to refetch the z.ai / GLM usage endpoint.
    pub zai_poll_secs: u64,
    /// z.ai API base URL ("https://api.z.ai" global, "https://open.bigmodel.cn" CN).
    pub zai_base_url: String,
}

impl Default for QuotaConfig {
    fn default() -> Self {
        QuotaConfig {
            claude_poll_secs: 300,
            claude_daily_token_limit: None,
            claude_weekly_token_limit: None,
            zai_poll_secs: 300,
            zai_base_url: "https://api.z.ai".to_string(),
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
    /// z.ai / GLM Coding Plan live quota (uses the opencode-stored API key).
    pub zai: bool,
    /// opencode local token totals (read from its SQLite store).
    pub opencode: bool,
}

impl Default for ProvidersConfig {
    fn default() -> Self {
        ProvidersConfig { claude: true, codex: true, zai: true, opencode: true }
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            terminal: TerminalConfig::default(),
            quota: QuotaConfig::default(),
            pet: PetConfig::default(),
            providers: ProvidersConfig::default(),
            ui: UiConfig::default(),
            sessions: SessionsConfig::default(),
            notify: NotifyConfig::default(),
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
