//! Unified usage-quota model (per-provider sections) for Claude, Codex and z.ai/GLM.
//!
//!   * Claude  -> GET https://api.anthropic.com/api/oauth/usage (cached, TTL).
//!                Fields: five_hour / seven_day / seven_day_{opus,sonnet}, each with
//!                `utilization` (percent) and `resets_at` (ISO-8601).
//!   * Codex   -> rate_limits block (primary = 5h, secondary = weekly) in the
//!                newest ~/.codex rollout file (local snapshot, may be stale).
//!   * z.ai    -> GET <base>/api/monitor/usage/quota/limit (cached, TTL). Auth is the
//!                raw API key (NO "Bearer" prefix). `data.limits[]` carry TOKENS_LIMIT
//!                (5h = unit3/num5, weekly = unit6/num1) and TIME_LIMIT (MCP tools).
//!                Key comes from opencode's account store.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Local, TimeZone};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::Config;
use crate::providers::{codex, Provider};

const CLAUDE_USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";

/// One usage window (a time-boxed quota). Provider lives on the enclosing section.
#[derive(Clone, Debug)]
pub struct QuotaWindow {
    pub label: String,
    pub used_percent: f32,
    pub resets_at: Option<i64>, // unix seconds
    /// The window's reset time has already passed since the snapshot was taken, so
    /// the recorded percentage no longer applies (local snapshot providers only).
    /// `used_percent` is forced to 0.
    pub reset: bool,
}

/// Local token totals (opencode) — shown as a summary, not a percentage bar.
#[derive(Clone, Debug, Default)]
pub struct TokenSummary {
    pub tokens_input: u64,
    pub tokens_output: u64,
    pub cost: f64,
}

/// A nested provider inside another's card (e.g. GLM / MiniMax under opencode).
#[derive(Clone, Debug)]
pub struct SubUsage {
    pub provider: Provider,
    pub badge: Option<String>,
    pub note: Option<String>,
    pub windows: Vec<QuotaWindow>,
    pub summary: Option<TokenSummary>,
}

/// Everything to render one provider's card.
#[derive(Clone, Debug)]
pub struct ProviderSection {
    pub provider: Provider,
    /// Plan tier badge, e.g. "max" (z.ai) or "plus" (codex).
    pub badge: Option<String>,
    /// Status caption — "as of …", "stale", or an error/availability note.
    pub note: Option<String>,
    pub windows: Vec<QuotaWindow>,
    pub summary: Option<TokenSummary>,
    /// Nested sub-providers (opencode → GLM, MiniMax).
    pub children: Vec<SubUsage>,
}

#[derive(Clone, Debug, Default)]
pub struct QuotaSnapshot {
    pub sections: Vec<ProviderSection>,
}

fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

fn iso_to_unix(s: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(s.trim()).ok().map(|dt| dt.timestamp())
}

fn fmt_local(ts: i64) -> String {
    Local
        .timestamp_opt(ts, 0)
        .single()
        .map(|dt| dt.format("%b %e %H:%M").to_string())
        .unwrap_or_default()
}

// ---- combined snapshot ----------------------------------------------------
pub fn snapshot(cfg: &Config) -> QuotaSnapshot {
    let mut sections = Vec::new();
    if cfg.providers.claude {
        if let Some(s) = claude_section(cfg) {
            sections.push(s);
        }
    }
    if cfg.providers.codex {
        if let Some(s) = codex_section() {
            sections.push(s);
        }
    }
    if cfg.providers.opencode {
        if let Some(s) = opencode_section(cfg) {
            sections.push(s);
        }
    }
    QuotaSnapshot { sections }
}

// ---- opencode (umbrella: GLM + MiniMax sub-providers) ----------------------
fn summary_from(t: &crate::providers::opencode::Totals) -> TokenSummary {
    TokenSummary { tokens_input: t.0, tokens_output: t.1, cost: t.2 }
}

fn opencode_section(cfg: &Config) -> Option<ProviderSection> {
    let totals = crate::providers::opencode::provider_totals();
    let mut children = Vec::new();

    // GLM (z.ai coding plan): live quota windows + opencode token totals.
    if cfg.providers.zai {
        let mut glm = zai_child(cfg);
        glm.summary = totals.get("zai-coding-plan").map(summary_from);
        if !glm.windows.is_empty() || glm.summary.is_some() {
            children.push(glm);
        }
    }
    // MiniMax: opencode token totals only (no public quota endpoint).
    if let Some(t) = totals.get("minimax") {
        children.push(SubUsage {
            provider: Provider::Minimax,
            badge: None,
            note: None,
            windows: Vec::new(),
            summary: Some(summary_from(t)),
        });
    }

    if children.is_empty() {
        return None;
    }
    Some(ProviderSection {
        provider: Provider::Opencode,
        badge: None,
        note: Some("usage by provider".to_string()),
        windows: Vec::new(),
        summary: None,
        children,
    })
}

// ---- generic cache (Claude + z.ai live providers) -------------------------
#[derive(Clone, Serialize, Deserialize)]
struct CachedWindow {
    label: String,
    used_percent: f32,
    resets_at: Option<i64>,
}

#[derive(Default, Serialize, Deserialize)]
struct UsageCache {
    fetched_at: f64,
    windows: Vec<CachedWindow>,
    note: Option<String>,
    badge: Option<String>,
}

fn cache_path(file: &str) -> PathBuf {
    directories::ProjectDirs::from("", "", "cc-pet")
        .map(|d| d.config_dir().join(file))
        .unwrap_or_else(|| PathBuf::from(format!(".config/cc-pet/{file}")))
}

fn load_cache(file: &str) -> UsageCache {
    fs::read_to_string(cache_path(file))
        .ok()
        .and_then(|d| serde_json::from_str(&d).ok())
        .unwrap_or_default()
}

fn save_cache(file: &str, c: &UsageCache) {
    let p = cache_path(file);
    if let Some(parent) = p.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(d) = serde_json::to_string_pretty(c) {
        let _ = fs::write(&p, d);
    }
}

fn cache_to_windows(c: &UsageCache) -> Vec<QuotaWindow> {
    c.windows
        .iter()
        .map(|w| QuotaWindow {
            label: w.label.clone(),
            used_percent: w.used_percent,
            resets_at: w.resets_at,
            reset: false, // live providers are never stale
        })
        .collect()
}

// ---- Claude (live OAuth usage, cached) ------------------------------------
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

fn claude_section(cfg: &Config) -> Option<ProviderSection> {
    let cache = load_cache("claude-usage.json");
    // honor TTL — the endpoint rate-limits aggressively
    let (windows, note) =
        if now() - cache.fetched_at < cfg.quota.claude_poll_secs as f64 && !cache.windows.is_empty() {
            (cache_to_windows(&cache), cache.note.clone())
        } else {
            match claude_credentials() {
                Some((_tok, true)) => {
                    // token expired: don't risk refreshing/rotating; keep stale data + note
                    let note =
                        Some("Claude token expired — run any `claude` command to refresh".to_string());
                    (cache_to_windows(&cache), note)
                }
                Some((tok, false)) => match fetch_claude(&tok) {
                    Some(windows) => {
                        let c = UsageCache {
                            fetched_at: now(),
                            windows,
                            note: None,
                            badge: None,
                        };
                        save_cache("claude-usage.json", &c);
                        (cache_to_windows(&c), None)
                    }
                    None => (cache_to_windows(&cache), Some("Claude usage unavailable".to_string())),
                },
                None => (cache_to_windows(&cache), Some("no Claude credentials".to_string())),
            }
        };
    if windows.is_empty() && note.is_none() {
        return None;
    }
    Some(ProviderSection {
        provider: Provider::Claude,
        badge: None,
        note,
        windows,
        summary: None,
        children: Vec::new(),
    })
}

// ---- Codex (local rate_limits snapshot) -----------------------------------
fn codex_section() -> Option<ProviderSection> {
    let files = codex::rollout_files();
    let (_, path) = files.first()?;
    let data = fs::read_to_string(path).ok()?;

    // find the most recent rate_limits block + when it was recorded
    let mut latest: Option<Value> = None;
    let mut latest_ts: Option<i64> = None;
    for line in data.lines() {
        let Ok(o) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if let Some(rl) = o.get("payload").and_then(|p| p.get("rate_limits")) {
            if !rl.is_null() {
                latest = Some(rl.clone());
                latest_ts =
                    o.get("timestamp").and_then(Value::as_str).and_then(iso_to_unix).or(latest_ts);
            }
        }
    }
    let rl = latest?;
    let badge = rl.get("plan_type").and_then(Value::as_str).map(|s| s.to_string());
    let now_s = now() as i64;
    let mut windows = Vec::new();
    for (key, fallback_label) in [("primary", "5h"), ("secondary", "weekly")] {
        if let Some(w) = rl.get(key).filter(|w| !w.is_null()) {
            let mut pct = w.get("used_percent").and_then(Value::as_f64).unwrap_or(0.0) as f32;
            let resets = w.get("resets_at").and_then(Value::as_i64);
            let mins = w.get("window_minutes").and_then(Value::as_i64).unwrap_or(0);
            let label = match mins {
                300 => "5h".to_string(),
                10080 => "weekly".to_string(),
                m if m > 0 => format!("{}h", m / 60),
                _ => fallback_label.to_string(),
            };
            // The recorded percent is a snapshot from the last Codex run. If the
            // window's reset time has already passed, it has rolled over and the
            // real usage is back to ~0.
            let reset = resets.map(|r| r <= now_s).unwrap_or(false);
            if reset {
                pct = 0.0;
            }
            windows.push(QuotaWindow { label, used_percent: pct, resets_at: resets, reset });
        }
    }
    if windows.is_empty() {
        return None;
    }
    let note = latest_ts.map(|t| format!("as of {}", fmt_local(t)));
    Some(ProviderSection {
        provider: Provider::Codex,
        badge,
        note,
        windows,
        summary: None,
        children: Vec::new(),
    })
}

// ---- z.ai / GLM Coding Plan (live, cached) --------------------------------
/// Pull the z.ai coding-plan API key from opencode's account store.
fn zai_credential() -> Option<String> {
    let path = directories::BaseDirs::new()?
        .home_dir()
        .join(".local/share/opencode/account.json");
    let v: Value = serde_json::from_str(&fs::read_to_string(path).ok()?).ok()?;
    let accounts = v.get("accounts")?.as_object()?;
    for acc in accounts.values() {
        if acc.get("serviceID").and_then(Value::as_str) == Some("zai-coding-plan") {
            if let Some(k) = acc.get("credential").and_then(|c| c.get("key")).and_then(Value::as_str)
            {
                return Some(k.to_string());
            }
        }
    }
    None
}

/// Map one `data.limits[]` entry to a (sort-priority, window). Returns None for
/// entries we don't surface.
fn zai_limit_to_window(lim: &Value) -> Option<(u8, CachedWindow)> {
    let typ = lim.get("type").and_then(Value::as_str).unwrap_or("");
    let unit = lim.get("unit").and_then(Value::as_i64);
    let number = lim.get("number").and_then(Value::as_i64);
    let pct = lim.get("percentage").and_then(Value::as_f64).unwrap_or(0.0) as f32;
    let resets = lim.get("nextResetTime").and_then(Value::as_i64).map(|ms| ms / 1000);
    let (prio, label) = match (typ, unit, number) {
        ("TOKENS_LIMIT", Some(3), Some(5)) => (0, "5h tokens"),
        ("TOKENS_LIMIT", Some(6), Some(1)) => (1, "weekly tokens"),
        ("TOKENS_LIMIT", _, _) => (2, "tokens"),
        ("TIME_LIMIT", _, _) => (3, "MCP tools"),
        _ => return None,
    };
    Some((prio, CachedWindow { label: label.to_string(), used_percent: pct, resets_at: resets }))
}

fn fetch_zai(base: &str, key: &str) -> Option<(Vec<CachedWindow>, Option<String>)> {
    let url = format!("{}/api/monitor/usage/quota/limit", base.trim_end_matches('/'));
    let resp = ureq::get(&url)
        .set("Authorization", key) // raw token — NO "Bearer" prefix
        .set("Accept-Language", "en-US,en")
        .set("Content-Type", "application/json")
        .timeout(Duration::from_secs(15))
        .call()
        .ok()?;
    let v: Value = resp.into_json().ok()?;
    let data = v.get("data")?;
    let badge = data.get("level").and_then(Value::as_str).map(|s| s.to_string());
    let limits = data.get("limits")?.as_array()?;
    let mut scored: Vec<(u8, CachedWindow)> = limits.iter().filter_map(zai_limit_to_window).collect();
    scored.sort_by_key(|(p, _)| *p);
    let out: Vec<CachedWindow> = scored.into_iter().map(|(_, w)| w).collect();
    if out.is_empty() {
        None
    } else {
        Some((out, badge))
    }
}

/// GLM live quota as an opencode sub-provider. Always returns a SubUsage (possibly
/// with empty windows + a note); the caller attaches opencode token totals.
fn zai_child(cfg: &Config) -> SubUsage {
    let cache = load_cache("zai-usage.json");
    let fresh = now() - cache.fetched_at < cfg.quota.zai_poll_secs as f64 && !cache.windows.is_empty();
    let (windows, badge, note) = if fresh {
        (cache_to_windows(&cache), cache.badge.clone(), cache.note.clone())
    } else {
        match zai_credential() {
            None => {
                let note = if cache.windows.is_empty() {
                    Some("live quota unavailable (no z.ai credentials)".to_string())
                } else {
                    Some("showing last fetch".to_string())
                };
                (cache_to_windows(&cache), cache.badge.clone(), note)
            }
            Some(key) => match fetch_zai(&cfg.quota.zai_base_url, &key) {
                Some((windows, badge)) => {
                    let c = UsageCache { fetched_at: now(), windows, note: None, badge };
                    save_cache("zai-usage.json", &c);
                    (cache_to_windows(&c), c.badge.clone(), None)
                }
                None => {
                    let note = Some("live quota unavailable".to_string());
                    (cache_to_windows(&cache), cache.badge.clone(), note)
                }
            },
        }
    };
    SubUsage { provider: Provider::Zai, badge, note, windows, summary: None }
}
