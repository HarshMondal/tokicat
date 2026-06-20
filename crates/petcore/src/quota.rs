//! Unified usage-quota model (daily/weekly windows) for Claude and Codex.
//!
//!   * Claude  -> GET https://api.anthropic.com/api/oauth/usage (cached, TTL).
//!                Fields: five_hour / seven_day / seven_day_{opus,sonnet}, each with
//!                `utilization` (percent) and `resets_at` (ISO-8601).
//!   * Codex   -> rate_limits block (primary = 5h, secondary = weekly) in the
//!                newest ~/.codex rollout file.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::DateTime;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::Config;
use crate::providers::{codex, Provider};

const CLAUDE_USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";

#[derive(Clone, Debug)]
pub struct QuotaWindow {
    pub provider: Provider,
    pub label: String,
    pub used_percent: f32,
    pub resets_at: Option<i64>, // unix seconds
}

#[derive(Clone, Debug, Default)]
pub struct QuotaSnapshot {
    pub windows: Vec<QuotaWindow>,
    pub note: Option<String>,
}

fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

fn iso_to_unix(s: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(s.trim()).ok().map(|dt| dt.timestamp())
}

// ---- combined snapshot ----------------------------------------------------
pub fn snapshot(cfg: &Config) -> QuotaSnapshot {
    let mut windows = Vec::new();
    let mut note = None;
    if cfg.providers.claude {
        let (mut w, n) = claude_windows(cfg);
        windows.append(&mut w);
        note = n;
    }
    if cfg.providers.codex {
        windows.extend(codex_windows());
    }
    QuotaSnapshot { windows, note }
}

// ---- Claude (live OAuth usage, cached) ------------------------------------
#[derive(Clone, Serialize, Deserialize)]
struct CachedWindow {
    label: String,
    used_percent: f32,
    resets_at: Option<i64>,
}

#[derive(Default, Serialize, Deserialize)]
struct ClaudeCache {
    fetched_at: f64,
    windows: Vec<CachedWindow>,
    note: Option<String>,
}

fn cache_path() -> PathBuf {
    directories::ProjectDirs::from("", "", "cc-pet")
        .map(|d| d.config_dir().join("claude-usage.json"))
        .unwrap_or_else(|| PathBuf::from(".config/cc-pet/claude-usage.json"))
}

fn load_cache() -> ClaudeCache {
    fs::read_to_string(cache_path())
        .ok()
        .and_then(|d| serde_json::from_str(&d).ok())
        .unwrap_or_default()
}

fn save_cache(c: &ClaudeCache) {
    let p = cache_path();
    if let Some(parent) = p.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(d) = serde_json::to_string_pretty(c) {
        let _ = fs::write(&p, d);
    }
}

fn cache_to_windows(c: &ClaudeCache) -> Vec<QuotaWindow> {
    c.windows
        .iter()
        .map(|w| QuotaWindow {
            provider: Provider::Claude,
            label: w.label.clone(),
            used_percent: w.used_percent,
            resets_at: w.resets_at,
        })
        .collect()
}

fn claude_credentials() -> Option<(String, bool)> {
    let path = directories::BaseDirs::new()?.home_dir().join(".claude/.credentials.json");
    let data = fs::read_to_string(path).ok()?;
    let v: Value = serde_json::from_str(&data).ok()?;
    let o = v.get("claudeAiOauth")?;
    let tok = o.get("accessToken").and_then(Value::as_str)?.to_string();
    let exp_ms = o.get("expiresAt").and_then(Value::as_i64).unwrap_or(0);
    let expired = (exp_ms as f64) / 1000.0 <= now();
    Some((tok, expired))
}

fn fetch_claude(tok: &str) -> Option<Vec<CachedWindow>> {
    let resp = ureq::get(CLAUDE_USAGE_URL)
        .set("Authorization", &format!("Bearer {tok}"))
        .set("anthropic-beta", "oauth-2025-04-20")
        .set("User-Agent", "claude-code/1.0")
        .timeout(Duration::from_secs(15))
        .call()
        .ok()?;
    let v: Value = resp.into_json().ok()?;
    let mut out = Vec::new();
    for (key, label) in [
        ("five_hour", "5h"),
        ("seven_day", "weekly"),
        ("seven_day_opus", "weekly · opus"),
        ("seven_day_sonnet", "weekly · sonnet"),
    ] {
        if let Some(w) = v.get(key).filter(|w| !w.is_null()) {
            let pct = w.get("utilization").and_then(Value::as_f64).unwrap_or(0.0) as f32;
            let resets = w.get("resets_at").and_then(Value::as_str).and_then(iso_to_unix);
            out.push(CachedWindow { label: label.to_string(), used_percent: pct, resets_at: resets });
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn claude_windows(cfg: &Config) -> (Vec<QuotaWindow>, Option<String>) {
    let cache = load_cache();
    // honor TTL — the endpoint rate-limits aggressively
    if now() - cache.fetched_at < cfg.quota.claude_poll_secs as f64 && !cache.windows.is_empty() {
        return (cache_to_windows(&cache), cache.note.clone());
    }
    match claude_credentials() {
        Some((_tok, true)) => {
            // token expired: don't risk refreshing/rotating; keep stale data + note
            let note = Some("Claude token expired — run any `claude` command to refresh".to_string());
            (cache_to_windows(&cache), note)
        }
        Some((tok, false)) => match fetch_claude(&tok) {
            Some(windows) => {
                let c = ClaudeCache { fetched_at: now(), windows, note: None };
                save_cache(&c);
                (cache_to_windows(&c), None)
            }
            None => (cache_to_windows(&cache), Some("Claude usage unavailable".to_string())),
        },
        None => (cache_to_windows(&cache), Some("no Claude credentials".to_string())),
    }
}

// ---- Codex (local rate_limits) --------------------------------------------
fn codex_windows() -> Vec<QuotaWindow> {
    let files = codex::rollout_files();
    let Some((_, path)) = files.first() else {
        return Vec::new();
    };
    let Ok(data) = fs::read_to_string(path) else {
        return Vec::new();
    };
    // find the most recent rate_limits block
    let mut latest: Option<Value> = None;
    for line in data.lines() {
        let Ok(o) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if let Some(rl) = o.get("payload").and_then(|p| p.get("rate_limits")) {
            if !rl.is_null() {
                latest = Some(rl.clone());
            }
        }
    }
    let Some(rl) = latest else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (key, fallback_label) in [("primary", "5h"), ("secondary", "weekly")] {
        if let Some(w) = rl.get(key).filter(|w| !w.is_null()) {
            let pct = w.get("used_percent").and_then(Value::as_f64).unwrap_or(0.0) as f32;
            let resets = w.get("resets_at").and_then(Value::as_i64);
            let mins = w.get("window_minutes").and_then(Value::as_i64).unwrap_or(0);
            let label = match mins {
                300 => "5h".to_string(),
                10080 => "weekly".to_string(),
                m if m > 0 => format!("{}h", m / 60),
                _ => fallback_label.to_string(),
            };
            out.push(QuotaWindow { provider: Provider::Codex, label, used_percent: pct, resets_at: resets });
        }
    }
    out
}
