//! Display helpers shared with the GUI. Ported from sessions.py.

use crate::providers::Session;
use chrono::{Datelike, Duration, Local, TimeZone};

pub fn fmt_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1000 {
        format!("{:.1}k", n as f64 / 1000.0)
    } else {
        n.to_string()
    }
}

pub fn fmt_elapsed(seconds: Option<f64>) -> String {
    let Some(s) = seconds else {
        return "-".to_string();
    };
    let s = s.round().max(0.0) as u64;
    if s < 60 {
        format!("{}s", s)
    } else if s < 3600 {
        format!("{}m{:02}s", s / 60, s % 60)
    } else {
        format!("{}h{:02}m", s / 3600, (s % 3600) / 60)
    }
}

pub fn fmt_cost(cost: Option<f64>) -> String {
    let Some(c) = cost else {
        return "-".to_string();
    };
    if c >= 100.0 {
        format!("${:.0}", c)
    } else if c >= 1.0 {
        format!("${:.2}", c)
    } else {
        format!("${:.3}", c)
    }
}

/// Relative "time ago" for a unix timestamp: "just now", "5m ago", "2h ago",
/// "3d ago", else a short local date ("Apr 5"). Used for session recency and
/// per-prompt timestamps.
pub fn fmt_ago(ts: f64) -> String {
    let now = Local::now().timestamp() as f64;
    let d = now - ts;
    if d < 0.0 {
        return "just now".to_string();
    }
    let s = d as u64;
    if s < 45 {
        "just now".to_string()
    } else if s < 3600 {
        format!("{}m ago", (s + 30) / 60)
    } else if s < 86_400 {
        format!("{}h ago", s / 3600)
    } else if s < 7 * 86_400 {
        format!("{}d ago", s / 86_400)
    } else {
        match Local.timestamp_opt(ts as i64, 0) {
            chrono::LocalResult::Single(dt) => dt.format("%b %-d").to_string(),
            _ => "a while ago".to_string(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Totals {
    pub prompts: usize,
    pub tokens: u64,
    pub cost: f64,
}

fn local_date(mtime: f64) -> Option<chrono::NaiveDate> {
    match Local.timestamp_opt(mtime as i64, 0) {
        chrono::LocalResult::Single(dt) => Some(dt.date_naive()),
        _ => None,
    }
}

pub fn is_today(s: &Session) -> bool {
    local_date(s.mtime) == Some(Local::now().date_naive())
}

/// Unix timestamp (seconds) of this Monday at local midnight (Mon–Sun calendar week).
pub fn week_start_unix() -> f64 {
    let today = Local::now().date_naive();
    let days_since_monday = today.weekday().num_days_from_monday() as i64;
    let monday = today - Duration::days(days_since_monday);
    monday
        .and_hms_opt(0, 0, 0)
        .and_then(|ndt| Local.from_local_datetime(&ndt).single())
        .map(|dt| dt.timestamp() as f64)
        .unwrap_or(0.0)
}

pub fn is_this_week(s: &Session) -> bool {
    s.mtime >= week_start_unix()
}

/// Sum prompts/tokens/cost across sessions whose last activity is today (local).
pub fn today_totals(sessions: &[Session]) -> Totals {
    let today = Local::now().date_naive();
    let mut t = Totals::default();
    for s in sessions {
        if local_date(s.mtime) == Some(today) {
            t.prompts += s.total_prompts;
            t.tokens += s.total_tokens;
            t.cost += s.cost;
        }
    }
    t
}
