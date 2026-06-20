//! Claude Code transcript parser. Port of sessions.py.
//!
//! Reads ~/.claude/projects/<project>/*.jsonl (one level deep so subagent files
//! are never mistaken for a session) and returns `Session` structs. Robust
//! against partial/empty/garbage lines and missing fields.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::DateTime;
use serde_json::Value;

use super::{Prompt, Provider, Session, TokenBreakdown};
use crate::pricing;

/// A session counts as "running" if its file was touched within this many seconds.
pub const LIVE_WINDOW: f64 = 120.0;

const META_PREFIXES: &[&str] = &[
    "<local-command",
    "<bash-input",
    "<bash-stdout",
    "<bash-stderr",
    "<command-",
    "[request interrupted",
    "caveat:",
];

/// mtime-keyed parse cache: path -> (mtime, parsed session).
static CACHE: LazyLock<Mutex<HashMap<PathBuf, (f64, Session)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn projects_dir() -> PathBuf {
    directories::BaseDirs::new()
        .map(|b| b.home_dir().join(".claude/projects"))
        .unwrap_or_else(|| PathBuf::from(".claude/projects"))
}

fn mtime_secs(path: &Path) -> Option<f64> {
    let m = fs::metadata(path).ok()?;
    let t = m.modified().ok()?;
    Some(t.duration_since(UNIX_EPOCH).ok()?.as_secs_f64())
}

fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// Read JSONL entries, skipping blank lines and unparseable lines (e.g. a partial
/// last line during a live write). Only dict (object) values are yielded.
fn read_entries(path: &Path) -> Vec<Value> {
    let data = match fs::read_to_string(path) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for line in data.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<Value>(line) {
            if v.is_object() {
                out.push(v);
            }
        }
    }
    out
}

/// Parse an ISO-8601 timestamp to unix seconds (float). Handles a trailing "Z".
fn parse_ts(value: Option<&str>) -> Option<f64> {
    let s = value?.trim();
    if s.is_empty() {
        return None;
    }
    // chrono's RFC3339 parser accepts the "...Z" and offset forms Claude writes.
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.timestamp() as f64 + dt.timestamp_subsec_micros() as f64 / 1_000_000.0)
}

/// Human text of a user message, or None if it's a tool_result / empty.
fn message_text(msg: &Value) -> Option<String> {
    match msg.get("content") {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Array(blocks)) => {
            // A tool_result block means this isn't a typed prompt.
            for b in blocks {
                if b.get("type").and_then(Value::as_str) == Some("tool_result") {
                    return None;
                }
            }
            let joined: String = blocks
                .iter()
                .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .collect();
            let joined = joined.trim().to_string();
            if joined.is_empty() {
                None
            } else {
                Some(joined)
            }
        }
        _ => None,
    }
}

/// A prompt the human actually typed (promptSource=="typed"), with a legacy fallback.
fn is_real_prompt(entry: &Value, has_promptsource: bool) -> bool {
    if entry.get("type").and_then(Value::as_str) != Some("user") {
        return false;
    }
    if entry.get("isSidechain").and_then(Value::as_bool) == Some(true) {
        return false;
    }
    if has_promptsource {
        return entry.get("promptSource").and_then(Value::as_str) == Some("typed");
    }
    let Some(msg) = entry.get("message") else {
        return false;
    };
    let Some(text) = message_text(msg) else {
        return false;
    };
    let lowered = text.trim_start().to_lowercase();
    !META_PREFIXES.iter().any(|p| lowered.starts_with(p))
}

/// First non-empty line of text, optionally truncated to `width` chars.
fn first_line(text: &str, width: Option<usize>) -> String {
    let mut line = String::new();
    for ln in text.lines() {
        let ln = ln.trim();
        if !ln.is_empty() {
            line = ln.to_string();
            break;
        }
    }
    if line.is_empty() {
        line = "(empty prompt)".to_string();
    }
    if let Some(w) = width {
        let chars: Vec<char> = line.chars().collect();
        if chars.len() > w {
            line = chars[..w.saturating_sub(1)].iter().collect::<String>() + "…";
        }
    }
    line
}

/// Derive a readable project name from the encoded directory slug.
fn project_from_path(path: &Path) -> String {
    let slug = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("session");
    slug.split('-')
        .filter(|p| !p.is_empty())
        .last()
        .unwrap_or(slug)
        .to_string()
}

struct Turn {
    index: usize,
    title: String,
    full_text: String,
    prompt_dt: Option<f64>,
    out_tokens: u64,
    last_dt: Option<f64>,
    ids: HashSet<String>,
    completed: bool,
}

/// Fully parse one session into a `Session` (mtime-cached). None on read error.
pub fn parse_session(path: &Path) -> Option<Session> {
    let mtime = mtime_secs(path)?;
    if let Some((cm, cached)) = CACHE.lock().unwrap().get(path) {
        if *cm == mtime {
            return Some(cached.clone());
        }
    }

    let entries = read_entries(path);

    let has_promptsource = entries.iter().any(|e| {
        e.get("type").and_then(Value::as_str) == Some("user") && e.get("promptSource").is_some()
    });

    let mut ai_title: Option<String> = None;
    let mut cwd: Option<String> = None;
    let mut branch: Option<String> = None;
    let mut session_id: Option<String> = None;
    let mut slug: Option<String> = None;
    let mut first_prompt_text: Option<String> = None;
    let mut model: Option<String> = None;
    let mut tok = TokenBreakdown::default();

    let mut turns: Vec<Turn> = Vec::new();

    for entry in &entries {
        let etype = entry.get("type").and_then(Value::as_str);

        // Cheap metadata harvested along the way.
        if etype == Some("ai-title") {
            if let Some(t) = entry.get("aiTitle").and_then(Value::as_str) {
                ai_title = Some(t.to_string());
            }
        }
        if cwd.is_none() {
            if let Some(v) = entry.get("cwd").and_then(Value::as_str) {
                cwd = Some(v.to_string());
            }
        }
        if branch.is_none() {
            if let Some(v) = entry.get("gitBranch").and_then(Value::as_str) {
                branch = Some(v.to_string());
            }
        }
        if session_id.is_none() {
            if let Some(v) = entry.get("sessionId").and_then(Value::as_str) {
                session_id = Some(v.to_string());
            }
        }
        if slug.is_none() {
            if let Some(v) = entry.get("slug").and_then(Value::as_str) {
                slug = Some(v.to_string());
            }
        }

        if is_real_prompt(entry, has_promptsource) {
            let text = entry
                .get("message")
                .and_then(message_text)
                .unwrap_or_default();
            if first_prompt_text.is_none() {
                first_prompt_text = Some(text.clone());
            }
            turns.push(Turn {
                index: turns.len() + 1,
                title: first_line(&text, None),
                full_text: text,
                prompt_dt: parse_ts(entry.get("timestamp").and_then(Value::as_str)),
                out_tokens: 0,
                last_dt: None,
                ids: HashSet::new(),
                completed: false,
            });
            continue;
        }

        if etype == Some("assistant") {
            let Some(current) = turns.last_mut() else {
                continue;
            };
            let Some(msg) = entry.get("message") else {
                continue;
            };
            if !msg.is_object() {
                continue;
            }
            if let Some(ts) = parse_ts(entry.get("timestamp").and_then(Value::as_str)) {
                current.last_dt = Some(ts);
            }
            if let Some(m) = msg.get("model").and_then(Value::as_str) {
                model = Some(m.to_string());
            }
            let usage = msg.get("usage");
            let mid = msg.get("id").and_then(Value::as_str);
            if let Some(mid) = mid {
                if !current.ids.contains(mid) {
                    current.ids.insert(mid.to_string());
                    let u = usage.cloned().unwrap_or(Value::Null);
                    let out = u.get("output_tokens").and_then(Value::as_u64).unwrap_or(0);
                    current.out_tokens += out;
                    // Session-wide totals for cost (deduped by id).
                    tok.output += out;
                    tok.input += u.get("input_tokens").and_then(Value::as_u64).unwrap_or(0);
                    tok.cache_read += u
                        .get("cache_read_input_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    let cc = u.get("cache_creation");
                    let cw1h = cc
                        .and_then(|c| c.get("ephemeral_1h_input_tokens"))
                        .and_then(Value::as_u64);
                    let cw5 = cc
                        .and_then(|c| c.get("ephemeral_5m_input_tokens"))
                        .and_then(Value::as_u64);
                    if cw1h.is_some() || cw5.is_some() {
                        tok.cache_write_1h += cw1h.unwrap_or(0);
                        tok.cache_write_5m += cw5.unwrap_or(0);
                    } else {
                        // no breakdown: treat as 5-min cache writes
                        tok.cache_write_5m += u
                            .get("cache_creation_input_tokens")
                            .and_then(Value::as_u64)
                            .unwrap_or(0);
                    }
                }
            }
            if let Some(sr) = msg.get("stop_reason").and_then(Value::as_str) {
                if matches!(sr, "end_turn" | "stop_sequence" | "max_tokens") {
                    current.completed = true;
                }
            }
        }
    }

    // Finalize prompts.
    let n = turns.len();
    let mut prompts: Vec<Prompt> = Vec::with_capacity(n);
    for (i, t) in turns.iter().enumerate() {
        let newest = i == n - 1;
        let has_resp = !t.ids.is_empty();
        let elapsed = match (t.prompt_dt, t.last_dt) {
            (Some(p), Some(l)) => Some(l - p),
            _ => None,
        };
        prompts.push(Prompt {
            index: t.index,
            title: t.title.clone(),
            full_text: t.full_text.clone(),
            out_tokens: t.out_tokens,
            elapsed,
            running: newest && !has_resp,
            completed: t.completed,
        });
    }

    let starts: Vec<f64> = turns.iter().filter_map(|t| t.prompt_dt).collect();
    let mut ends: Vec<f64> = turns.iter().filter_map(|t| t.last_dt).collect();
    ends.extend(starts.iter().copied());
    let wall = if !starts.is_empty() && !ends.is_empty() {
        let max_e = ends.iter().cloned().fold(f64::MIN, f64::max);
        let min_s = starts.iter().cloned().fold(f64::MAX, f64::min);
        Some(max_e - min_s)
    } else {
        None
    };

    let title = ai_title
        .clone()
        .or_else(|| slug.clone())
        .or_else(|| first_prompt_text.as_ref().map(|t| first_line(t, Some(60))))
        .or_else(|| session_id.as_ref().map(|s| s.chars().take(8).collect()))
        .unwrap_or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("session")
                .chars()
                .take(8)
                .collect()
        });

    let project = cwd
        .as_ref()
        .map(|c| {
            Path::new(c)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(c)
                .to_string()
        })
        .unwrap_or_else(|| project_from_path(path));

    let family = pricing::model_family(model.as_deref());
    let cost = pricing::cost(&tok, &family);

    let last_prompt_dt = turns.last().and_then(|t| t.prompt_dt);
    let total_tokens: u64 = prompts.iter().map(|p| p.out_tokens).sum();
    let last_completed = prompts.last().map(|p| !p.running).unwrap_or(true);

    let session = Session {
        provider: Provider::Claude,
        path: path.to_path_buf(),
        session_id: session_id.unwrap_or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("session")
                .to_string()
        }),
        cwd,
        project,
        branch,
        title,
        model,
        model_family: family,
        mtime,
        is_live: false,
        open: false,
        pane_id: None,
        working: false,
        waiting: false,
        total_prompts: prompts.len(),
        total_tokens,
        tokens: tok,
        cost,
        last_prompt_ts: last_prompt_dt,
        last_completed,
        wall_seconds: wall,
        prompts,
    };

    CACHE
        .lock()
        .unwrap()
        .insert(path.to_path_buf(), (mtime, session.clone()));
    Some(session)
}

/// Recent top-level sessions, newest first. `limit` caps the count; None = all.
/// Globs one level deep so subagent files are never included. `is_live` is set
/// fresh on each call (terminal open/working/waiting are annotated separately).
pub fn find_sessions(limit: Option<usize>) -> Vec<Session> {
    let pattern = projects_dir().join("*").join("*.jsonl");
    let pattern = pattern.to_string_lossy().to_string();
    let mut paths_with_mtime: Vec<(f64, PathBuf)> = Vec::new();
    if let Ok(paths) = glob::glob(&pattern) {
        for p in paths.flatten() {
            if let Some(m) = mtime_secs(&p) {
                paths_with_mtime.push((m, p));
            }
        }
    }
    paths_with_mtime.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    if let Some(lim) = limit {
        paths_with_mtime.truncate(lim.max(1));
    }

    let now = now_secs();
    let mut out = Vec::new();
    for (mtime, p) in paths_with_mtime {
        if let Some(mut s) = parse_session(&p) {
            s.is_live = (now - mtime) <= LIVE_WINDOW;
            out.push(s);
        }
    }
    out
}
