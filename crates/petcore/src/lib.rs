//! petcore — data layer for the desktop pet.
//!
//! No GUI. Parses Claude Code and Codex session transcripts into a unified
//! `Session` shape, estimates cost, tracks usage quotas, and drives terminal
//! control. Mirrors (and supersedes) the old Python `sessions.py`.

pub mod config;
pub mod fmt;
pub mod pricing;
pub mod providers;
pub mod quota;
pub mod state;
pub mod terminal;

pub use fmt::{fmt_ago, fmt_cost, fmt_elapsed, fmt_tokens, is_this_week, is_today, today_totals, Totals, week_start_unix};
// re-export so the GUI can call terminal/session helpers conveniently
pub use terminal as term;
pub use providers::{Prompt, Provider, Session, TokenBreakdown};

/// Parse one session, picking the provider from its path/id
/// (opencode `ses_*` id -> opencode DB, ~/.codex/... -> Codex, else Claude).
pub fn parse_session_any(path: &std::path::Path) -> Option<Session> {
    let s = path.to_string_lossy();
    if providers::opencode::is_opencode_id(&s) {
        providers::opencode::parse_session(&s)
    } else if s.contains("/.codex/") {
        providers::codex::parse_session(path)
    } else {
        providers::claude::parse_session(path)
    }
}

/// Recent opencode (GLM/MiniMax) sessions, newest first. Kept separate from
/// `find_sessions` so the panel only touches the opencode DB when that tab is open.
pub fn find_opencode_sessions(limit: Option<usize>) -> Vec<Session> {
    providers::opencode::find_sessions(limit)
}

/// Recent sessions across all enabled providers, newest first.
/// `limit` caps the count **per provider** (each provider's own `find_sessions`
/// applies it), so one busy provider can't crowd another out of the merged list.
pub fn find_sessions(limit: Option<usize>) -> Vec<Session> {
    let mut all = providers::claude::find_sessions(limit);
    all.extend(providers::codex::find_sessions(limit));
    all.sort_by(|a, b| b.mtime.partial_cmp(&a.mtime).unwrap_or(std::cmp::Ordering::Equal));
    all
}
