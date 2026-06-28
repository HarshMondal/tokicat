//! opencode local token accounting. opencode keeps per-message token/cost totals in
//! a SQLite store (`~/.local/share/opencode/opencode.db`, `message` table, JSON in the
//! `data` column with top-level `providerID` + `tokens` + `cost`). We read it
//! read-only/immutable so a live opencode process can't block us, and surface the
//! totals split per underlying provider (zai-coding-plan, minimax, …).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OpenFlags};
use serde_json::Value;

use super::{Prompt, Provider, Session, TokenBreakdown};

/// Parsed-session cache keyed by session id, invalidated by the row's time_updated.
/// Keeps the history view from re-querying the DB on every frame.
static CACHE: LazyLock<Mutex<HashMap<String, (i64, Session)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// `(tokens_input, tokens_output, cost)` for one provider.
pub type Totals = (u64, u64, f64);

pub fn db_path() -> Option<PathBuf> {
    Some(directories::BaseDirs::new()?.home_dir().join(".local/share/opencode/opencode.db"))
}

fn open_ro() -> Option<Connection> {
    let path = db_path()?;
    if !path.is_file() {
        return None;
    }
    // Immutable read-only open: never takes a lock, safe while opencode is running.
    let uri = format!("file:{}?mode=ro&immutable=1", path.to_string_lossy());
    Connection::open_with_flags(uri, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI)
        .ok()
}

/// Token/cost totals keyed by opencode `providerID` (e.g. "zai-coding-plan", "minimax").
/// Uses SQLite's JSON1 `json_extract` so we never parse message bodies in Rust.
pub fn provider_totals() -> HashMap<String, Totals> {
    let mut out = HashMap::new();
    let Some(conn) = open_ro() else {
        return out;
    };
    let sql = "SELECT json_extract(data,'$.providerID'), \
               COALESCE(SUM(json_extract(data,'$.tokens.input')),0), \
               COALESCE(SUM(json_extract(data,'$.tokens.output')),0), \
               COALESCE(SUM(json_extract(data,'$.cost')),0) \
               FROM message WHERE json_extract(data,'$.tokens') IS NOT NULL \
               GROUP BY json_extract(data,'$.providerID')";
    let Ok(mut stmt) = conn.prepare(sql) else {
        return out;
    };
    let rows = stmt.query_map([], |r| {
        let pid: Option<String> = r.get(0)?;
        let input: i64 = r.get(1)?;
        let output: i64 = r.get(2)?;
        let cost: f64 = r.get(3)?;
        Ok((pid, (input.max(0) as u64, output.max(0) as u64, cost)))
    });
    if let Ok(rows) = rows {
        for row in rows.flatten() {
            if let (Some(pid), totals) = row {
                out.insert(pid, totals);
            }
        }
    }
    out
}

// ---- session + prompt history -------------------------------------------
// opencode stores conversations in `session` (one row per chat) + `message`
// (role/providerID/tokens, ordered by time_created) + `part` (the text bodies).
// We turn that into the same `Session`/`Prompt` shape Claude/Codex produce, so the
// panel's history view renders GLM/MiniMax sessions with no provider-specific code.

fn now_secs() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

/// Map an opencode `providerID` to our Provider (None = not a GLM/MiniMax session).
fn provider_from_id(pid: &str) -> Option<Provider> {
    match pid {
        "zai-coding-plan" => Some(Provider::Zai),
        "minimax" | "minimax-coding-plan" => Some(Provider::Minimax),
        _ => None,
    }
}

/// Pull the providerID out of a session's `model` JSON (`{"id":..,"providerID":..}`).
fn provider_from_model(model_json: &str) -> Option<Provider> {
    let v: Value = serde_json::from_str(model_json).ok()?;
    provider_from_id(v.get("providerID")?.as_str()?)
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

fn project_of(directory: &str) -> String {
    Path::new(directory)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(directory)
        .to_string()
}

/// True if this opencode session id can be parsed by us (cheap prefix gate so
/// `parse_session_any` can route without touching the DB).
pub fn is_opencode_id(id: &str) -> bool {
    id.starts_with("ses_")
}

/// Recent GLM/MiniMax sessions (cheap metadata only, no prompt bodies), newest first.
pub fn find_sessions(limit: Option<usize>) -> Vec<Session> {
    let Some(conn) = open_ro() else {
        return Vec::new();
    };
    let lim = limit.map(|l| l.max(1) as i64).unwrap_or(200);
    let sql = "SELECT s.id, s.title, s.directory, s.model, s.cost, s.time_updated, \
        (SELECT COUNT(*) FROM message m WHERE m.session_id = s.id \
            AND json_extract(m.data,'$.role') = 'user') AS prompts, \
        (SELECT COALESCE(SUM(json_extract(m.data,'$.tokens.output')),0) FROM message m \
            WHERE m.session_id = s.id AND json_extract(m.data,'$.role') = 'assistant') AS outtok, \
        (SELECT COUNT(*) FROM message m WHERE m.session_id = s.id \
            AND json_extract(m.data,'$.role') = 'assistant' \
            AND json_extract(m.data,'$.finish') IS NOT NULL \
            AND json_extract(m.data,'$.finish') != 'tool-calls') AS done \
        FROM session s \
        WHERE s.parent_id IS NULL \
          AND json_extract(s.model,'$.providerID') IN ('zai-coding-plan','minimax','minimax-coding-plan') \
        ORDER BY s.time_updated DESC LIMIT ?1";
    let Ok(mut stmt) = conn.prepare(sql) else {
        return Vec::new();
    };
    let now = now_secs();
    let rows = stmt.query_map([lim], |r| {
        let id: String = r.get(0)?;
        let title: String = r.get(1)?;
        let directory: String = r.get::<_, Option<String>>(2)?.unwrap_or_default();
        let model_json: String = r.get::<_, Option<String>>(3)?.unwrap_or_default();
        let cost: f64 = r.get(4)?;
        let updated_ms: i64 = r.get(5)?;
        let prompts: i64 = r.get(6)?;
        let outtok: i64 = r.get(7)?;
        let done: i64 = r.get(8)?;
        Ok((id, title, directory, model_json, cost, updated_ms, prompts, outtok, done))
    });
    let mut out = Vec::new();
    if let Ok(rows) = rows {
        for (id, title, directory, model_json, cost, updated_ms, prompts, outtok, done) in rows.flatten() {
            if prompts <= 0 {
                continue; // skip empty/system-only sessions
            }
            let Some(provider) = provider_from_model(&model_json) else {
                continue;
            };
            let mtime = updated_ms as f64 / 1000.0;
            let model = serde_json::from_str::<Value>(&model_json)
                .ok()
                .and_then(|v| v.get("id").and_then(Value::as_str).map(String::from));
            out.push(Session {
                provider,
                path: PathBuf::from(&id), // opencode "path" is the DB session id
                session_id: id,
                cwd: (!directory.is_empty()).then(|| directory.clone()),
                project: project_of(&directory),
                branch: None,
                title: if title.is_empty() { "opencode session".to_string() } else { title },
                model,
                model_family: provider.label().to_string(),
                mtime,
                is_live: (now - mtime) <= super::claude::LIVE_WINDOW,
                open: false,
                pane_id: None,
                working: false,
                waiting: false,
                total_prompts: prompts as usize,
                completed_turns: done.max(0) as usize,
                total_tokens: outtok.max(0) as u64,
                tokens: TokenBreakdown { output: outtok.max(0) as u64, ..Default::default() },
                cost,
                last_prompt_ts: Some(mtime),
                last_completed: true,
                wall_seconds: None,
                prompts: Vec::new(),
            });
        }
    }
    out
}

struct OcTurn {
    title: String,
    full_text: String,
    start_ms: i64,
    last_ms: i64,
    out_tokens: u64,
    completed: bool,
}

/// Full prompt-by-prompt history for one opencode session id.
pub fn parse_session(session_id: &str) -> Option<Session> {
    let conn = open_ro()?;
    let (title, directory, model_json, cost, updated_ms): (String, String, String, f64, i64) = conn
        .query_row(
            "SELECT title, directory, model, cost, time_updated FROM session WHERE id = ?1",
            [session_id],
            |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                    r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    r.get(3)?,
                    r.get(4)?,
                ))
            },
        )
        .ok()?;
    let provider = provider_from_model(&model_json)?;
    if let Some((cu, cached)) = CACHE.lock().unwrap().get(session_id) {
        if *cu == updated_ms {
            return Some(cached.clone());
        }
    }
    let model = serde_json::from_str::<Value>(&model_json)
        .ok()
        .and_then(|v| v.get("id").and_then(Value::as_str).map(String::from));

    // Text bodies of the user messages, keyed by message id (concatenated if split).
    let mut texts: HashMap<String, String> = HashMap::new();
    if let Ok(mut pstmt) = conn.prepare(
        "SELECT p.message_id, json_extract(p.data,'$.text') FROM part p \
         JOIN message m ON p.message_id = m.id \
         WHERE m.session_id = ?1 AND json_extract(m.data,'$.role') = 'user' \
           AND json_extract(p.data,'$.type') = 'text' ORDER BY p.time_created",
    ) {
        let rows = pstmt.query_map([session_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?.unwrap_or_default()))
        });
        if let Ok(rows) = rows {
            for (mid, text) in rows.flatten() {
                let e = texts.entry(mid).or_default();
                if !e.is_empty() {
                    e.push('\n');
                }
                e.push_str(&text);
            }
        }
    }

    // Walk messages in time order, grouping each user message + the assistant work
    // that follows it into one turn.
    let mut stmt = conn
        .prepare("SELECT id, data FROM message WHERE session_id = ?1 ORDER BY time_created")
        .ok()?;
    let rows = stmt
        .query_map([session_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .ok()?;

    let mut turns: Vec<OcTurn> = Vec::new();
    for (mid, data) in rows.flatten() {
        let Ok(v) = serde_json::from_str::<Value>(&data) else {
            continue;
        };
        let role = v.get("role").and_then(Value::as_str).unwrap_or("");
        let created = v.get("time").and_then(|t| t.get("created")).and_then(Value::as_i64);
        let completed = v.get("time").and_then(|t| t.get("completed")).and_then(Value::as_i64);
        if role == "user" {
            let text = texts.get(&mid).cloned().unwrap_or_default();
            let start = created.unwrap_or(0);
            turns.push(OcTurn {
                title: first_line(&text),
                full_text: text,
                start_ms: start,
                last_ms: start,
                out_tokens: 0,
                completed: false,
            });
        } else if role == "assistant" {
            if let Some(t) = turns.last_mut() {
                t.out_tokens += v
                    .get("tokens")
                    .and_then(|tk| tk.get("output"))
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                if let Some(c) = completed.or(created) {
                    t.last_ms = t.last_ms.max(c);
                }
                let finish = v.get("finish").and_then(Value::as_str).unwrap_or("");
                if !finish.is_empty() && finish != "tool-calls" {
                    t.completed = true;
                }
            }
        }
    }

    let n = turns.len();
    let prompts: Vec<Prompt> = turns
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let newest = i + 1 == n;
            let elapsed = if t.last_ms > t.start_ms {
                Some((t.last_ms - t.start_ms) as f64 / 1000.0)
            } else {
                None
            };
            Prompt {
                index: i + 1,
                title: t.title.clone(),
                full_text: t.full_text.clone(),
                out_tokens: t.out_tokens,
                elapsed,
                ts: if t.start_ms > 0 { Some(t.start_ms as f64 / 1000.0) } else { None },
                model: None,
                running: newest && !t.completed && t.out_tokens == 0,
                completed: t.completed,
            }
        })
        .collect();

    let total_tokens: u64 = prompts.iter().map(|p| p.out_tokens).sum();
    let starts: Vec<i64> = turns.iter().map(|t| t.start_ms).filter(|&m| m > 0).collect();
    let ends: Vec<i64> = turns.iter().map(|t| t.last_ms).filter(|&m| m > 0).collect();
    let wall = match (starts.iter().min(), ends.iter().max()) {
        (Some(&s), Some(&e)) if e > s => Some((e - s) as f64 / 1000.0),
        _ => None,
    };
    let mtime = updated_ms as f64 / 1000.0;

    let session = Session {
        provider,
        path: PathBuf::from(session_id),
        session_id: session_id.to_string(),
        cwd: (!directory.is_empty()).then(|| directory.clone()),
        project: project_of(&directory),
        branch: None,
        title: if title.is_empty() { "opencode session".to_string() } else { title },
        model,
        model_family: provider.label().to_string(),
        mtime,
        is_live: (now_secs() - mtime) <= super::claude::LIVE_WINDOW,
        open: false,
        pane_id: None,
        working: false,
        waiting: false,
        total_prompts: prompts.len(),
        completed_turns: prompts.iter().filter(|p| p.completed).count(),
        total_tokens,
        tokens: TokenBreakdown { output: total_tokens, ..Default::default() },
        cost,
        last_prompt_ts: turns.last().map(|t| t.start_ms as f64 / 1000.0),
        last_completed: prompts.last().map(|p| !p.running).unwrap_or(true),
        wall_seconds: wall,
        prompts,
    };

    CACHE.lock().unwrap().insert(session_id.to_string(), (updated_ms, session.clone()));
    Some(session)
}
