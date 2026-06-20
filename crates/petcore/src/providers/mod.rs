//! Shared session/prompt data types and the provider abstraction.
//!
//! A `Session` is the unified shape produced by every provider (Claude, Codex).
//! The GUI renders these without caring which provider they came from.

pub mod claude;
pub mod codex;

use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider {
    Claude,
    Codex,
}

impl Provider {
    pub fn label(&self) -> &'static str {
        match self {
            Provider::Claude => "claude",
            Provider::Codex => "codex",
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
    pub total_tokens: u64, // sum of per-prompt out_tokens
    pub tokens: TokenBreakdown,
    pub cost: f64,
    pub last_prompt_ts: Option<f64>,
    pub last_completed: bool,
    pub wall_seconds: Option<f64>,
    pub prompts: Vec<Prompt>,
}
