//! Codex (OpenAI) session parser — reads ~/.codex/sessions/**/rollout-*.jsonl
//! (and archived_sessions/). Each line is {timestamp, type, payload}.
//!
//! Prompts  = event_msg payloads with type "user_message".
//! Tokens   = event_msg "token_count" payloads (info.total_token_usage cumulative).
//! Quotas   = the rate_limits block in those token_count payloads (see quota.rs).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::DateTime;
use serde_json::Value;

use super::{Prompt, Provider, Session, TokenBreakdown};

static CACHE: LazyLock<Mutex<HashMap<PathBuf, (f64, Session)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn sessions_dir() -> PathBuf {
    directories::BaseDirs::new()
        .map(|b| b.home_dir().join(".codex"))
        .unwrap_or_else(|| PathBuf::from(".codex"))
}

fn mtime_secs(path: &Path) -> Option<f64> {
    let m = fs::metadata(path).ok()?;
    Some(m.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_secs_f64())
}

fn parse_ts(value: Option<&str>) -> Option<f64> {
    let s = value?.trim();
    if s.is_empty() {
        return None;
    }
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.timestamp() as f64 + dt.timestamp_subsec_micros() as f64 / 1_000_000.0)
}

fn first_line(text: &str) -> String {
    for ln in text.lines() {
        let ln = ln.trim();
        if !ln.is_empty() {
            return ln.to_string();
        }
    }
    "(empty prompt)".to_string()
}

/// All rollout files, newest first, as (mtime, path).
pub fn rollout_files() -> Vec<(f64, PathBuf)> {
    let base = sessions_dir();
    let mut out = Vec::new();
    for sub in ["sessions/**/rollout-*.jsonl", "archived_sessions/rollout-*.jsonl"] {
        let pat = base.join(sub);
        if let Ok(paths) = glob::glob(&pat.to_string_lossy()) {
            for p in paths.flatten() {
                if let Some(m) = mtime_secs(&p) {
                    out.push((m, p));
                }
            }
        }
    }
    out.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    out
}

struct Turn {
    index: usize,
    title: String,
    full_text: String,
    prompt_dt: Option<f64>,
    last_dt: Option<f64>,
    cum_start: u64,
    out_tokens: u64,
    completed: bool,
}

pub fn parse_session(path: &Path) -> Option<Session> {
    let mtime = mtime_secs(path)?;
    if let Some((cm, cached)) = CACHE.lock().unwrap().get(path) {
        if *cm == mtime {
            return Some(cached.clone());
        }
    }

    let data = fs::read_to_string(path).ok()?;

    let mut session_id: Option<String> = None;
    let mut cwd: Option<String> = None;
    let mut model: Option<String> = None;
    let mut tok = TokenBreakdown::default();
    let mut cum_out: u64 = 0;
    let mut turns: Vec<Turn> = Vec::new();

    let finalize_turn = |t: &mut Turn, cum: u64| {
        t.out_tokens = cum.saturating_sub(t.cum_start);
    };

    for line in data.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(o) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let otype = o.get("type").and_then(Value::as_str);
        let ts = parse_ts(o.get("timestamp").and_then(Value::as_str));
        let payload = o.get("payload");

        if otype == Some("session_meta") {
            if let Some(p) = payload {
                session_id = session_id.or_else(|| p.get("id").and_then(Value::as_str).map(String::from));
                cwd = cwd.or_else(|| p.get("cwd").and_then(Value::as_str).map(String::from));
                model = model.or_else(|| p.get("model").and_then(Value::as_str).map(String::from));
            }
            continue;
        }

        let ptype = payload.and_then(|p| p.get("type")).and_then(Value::as_str);

        if otype == Some("event_msg") && ptype == Some("user_message") {
            if let Some(prev) = turns.last_mut() {
                finalize_turn(prev, cum_out);
            }
            let msg = payload
                .and_then(|p| p.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            turns.push(Turn {
                index: turns.len() + 1,
                title: first_line(&msg),
                full_text: msg,
                prompt_dt: ts,
                last_dt: ts,
                cum_start: cum_out,
                out_tokens: 0,
                completed: false,
            });
            continue;
        }

        if otype == Some("event_msg") && ptype == Some("token_count") {
            if let Some(info) = payload.and_then(|p| p.get("info")) {
                if let Some(total) = info.get("total_token_usage") {
                    cum_out = total.get("output_tokens").and_then(Value::as_u64).unwrap_or(cum_out);
                    tok.output = cum_out;
                    tok.input = total.get("input_tokens").and_then(Value::as_u64).unwrap_or(tok.input);
                    tok.cache_read = total
                        .get("cached_input_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(tok.cache_read);
                }
            }
            if let Some(t) = turns.last_mut() {
                if ts.is_some() {
                    t.last_dt = ts;
                }
            }
            continue;
        }

        if otype == Some("event_msg") && ptype == Some("task_complete") {
            if let Some(t) = turns.last_mut() {
                t.completed = true;
                if ts.is_some() {
                    t.last_dt = ts;
                }
            }
            continue;
        }

        // keep last_dt fresh for the running turn
        if let (Some(t), Some(_)) = (turns.last_mut(), ts) {
            t.last_dt = ts;
        }
    }
    if let Some(last) = turns.last_mut() {
        finalize_turn(last, cum_out);
    }

    let n = turns.len();
    let mut prompts = Vec::with_capacity(n);
    for (i, t) in turns.iter().enumerate() {
        let newest = i == n - 1;
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
            ts: t.prompt_dt,
            model: None,
            running: newest && !t.completed && t.out_tokens == 0,
            completed: t.completed,
        });
    }

    let starts: Vec<f64> = turns.iter().filter_map(|t| t.prompt_dt).collect();
    let mut ends: Vec<f64> = turns.iter().filter_map(|t| t.last_dt).collect();
    ends.extend(starts.iter().copied());
    let wall = if !starts.is_empty() && !ends.is_empty() {
        Some(ends.iter().cloned().fold(f64::MIN, f64::max) - starts.iter().cloned().fold(f64::MAX, f64::min))
    } else {
        None
    };

    let title = turns
        .first()
        .map(|t| {
            let l = &t.title;
            if l.chars().count() > 60 {
                l.chars().take(59).collect::<String>() + "…"
            } else {
                l.clone()
            }
        })
        .or_else(|| session_id.as_ref().map(|s| s.chars().take(8).collect()))
        .unwrap_or_else(|| "codex".to_string());

    let project = cwd
        .as_ref()
        .map(|c| Path::new(c).file_name().and_then(|n| n.to_str()).unwrap_or(c).to_string())
        .unwrap_or_else(|| "codex".to_string());

    let last_prompt_dt = turns.last().and_then(|t| t.prompt_dt);
    let total_tokens: u64 = prompts.iter().map(|p| p.out_tokens).sum();
    let last_completed = prompts.last().map(|p| !p.running).unwrap_or(true);

    let session = Session {
        provider: Provider::Codex,
        path: path.to_path_buf(),
        session_id: session_id.unwrap_or_else(|| {
            path.file_stem().and_then(|s| s.to_str()).unwrap_or("codex").to_string()
        }),
        cwd,
        project,
        branch: None,
        title,
        model: model.clone(),
        model_family: "codex".to_string(),
        mtime,
        is_live: false,
        open: false,
        pane_id: None,
        working: false,
        waiting: false,
        total_prompts: prompts.len(),
        completed_turns: prompts.iter().filter(|p| p.completed).count(),
        total_tokens,
        tokens: tok,
        cost: 0.0, // Codex isn't priced in the Claude PRICING table
        last_prompt_ts: last_prompt_dt,
        last_completed,
        wall_seconds: wall,
        prompts,
    };

    CACHE.lock().unwrap().insert(path.to_path_buf(), (mtime, session.clone()));
    Some(session)
}

/// Recent Codex sessions, newest first.
pub fn find_sessions(limit: Option<usize>) -> Vec<Session> {
    let mut files = rollout_files();
    if let Some(lim) = limit {
        files.truncate(lim.max(1));
    }
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
    let mut out = Vec::new();
    for (mtime, p) in files {
        if let Some(mut s) = parse_session(&p) {
            s.is_live = (now - mtime) <= super::claude::LIVE_WINDOW;
            // skip empty rollouts (no user prompts) to keep the list useful
            if s.total_prompts > 0 {
                out.push(s);
            }
        }
    }
    out
}
