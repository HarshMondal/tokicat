//! Shared session/prompt data types and the provider abstraction.
//!
//! A `Session` is the unified shape produced by every provider (Claude, Codex).
//! The GUI renders these without caring which provider they came from.

pub mod claude;
pub mod codex;
pub mod opencode;

use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Provider {
    Claude,
    Codex,
    /// z.ai / GLM Coding Plan — live quota, shown as an opencode sub-provider.
    Zai,
    /// MiniMax — opencode sub-provider (local token totals; no public quota API).
    Minimax,
    /// opencode — umbrella client; its card nests the GLM/MiniMax sub-providers.
    Opencode,
}

impl Provider {
    /// Short lowercase key (asset filenames, config, logs).
    pub fn label(&self) -> &'static str {
        match self {
            Provider::Claude => "claude",
            Provider::Codex => "codex",
            Provider::Zai => "glm",
            Provider::Minimax => "minimax",
            Provider::Opencode => "opencode",
        }
    }

    /// Human-facing name shown in the usage UI.
    pub fn display_name(&self) -> &'static str {
        match self {
            Provider::Claude => "Claude",
            Provider::Codex => "Codex",
            Provider::Zai => "GLM Coding Plan",
            Provider::Minimax => "MiniMax",
            Provider::Opencode => "opencode",
        }
    }
}

/// Session-wide token totals (deduped by message id), used for cost.
#[derive(Clone, Copy, Debug, Default)]
pub struct TokenBreakdown {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write_5m: u64,
    pub cache_write_1h: u64,
}

/// One prompt-turn: your typed prompt plus the assistant work until the next one.
#[derive(Clone, Debug)]
pub struct Prompt {
    pub index: usize,
    pub title: String,
    pub full_text: String,
    pub out_tokens: u64,
    pub elapsed: Option<f64>, // seconds
    pub ts: Option<f64>,      // unix seconds, when the prompt was sent
    /// Raw model id that handled this turn (None → fall back to the session model).
    /// Lets the history view show which model answered each prompt.
    pub model: Option<String>,
    pub running: bool,
    pub completed: bool,
}

/// A fully parsed session. Fields mirror the dict produced by sessions.py.
#[derive(Clone, Debug)]
pub struct Session {
    pub provider: Provider,
    pub path: PathBuf,
    pub session_id: String,
    pub cwd: Option<String>,
    pub project: String,
    pub branch: Option<String>,
    pub title: String,
    pub model: Option<String>,
    pub model_family: String,
    pub mtime: f64, // unix seconds
    // Live / terminal flags — filled by find_sessions()/terminal annotation.
    pub is_live: bool,
    pub open: bool,
    pub pane_id: Option<String>,
    pub working: bool,
    pub waiting: bool,
    pub total_prompts: usize,
    /// Number of fully-finished turns (assistant responses that completed). Drives
    /// the cross-provider attention trigger: when it increases, a turn just finished.
    pub completed_turns: usize,
    pub total_tokens: u64, // sum of per-prompt out_tokens
    pub tokens: TokenBreakdown,
    pub cost: f64,
    pub last_prompt_ts: Option<f64>,
    pub last_completed: bool,
    pub wall_seconds: Option<f64>,
    pub prompts: Vec<Prompt>,
}
