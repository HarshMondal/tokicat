//! CLI smoke test for the data layer (no GUI), mirroring `python3 sessions.py`.
//!
//!   cargo run -p petcore --bin dump            # recent sessions
//!   cargo run -p petcore --bin dump <path>     # one session
//!   cargo run -p petcore --bin dump --quota    # quota snapshot (features phase)

use std::path::Path;

use petcore::providers::claude;
use petcore::{fmt_cost, fmt_elapsed, fmt_tokens};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.first().map(|s| s.as_str()) == Some("--quota") {
        let cfg = petcore::config::Config::load();
        let snap = petcore::quota::snapshot(&cfg);
        if let Some(note) = &snap.note {
            println!("note: {}", note);
        }
        if snap.windows.is_empty() {
            println!("no quota data");
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        for w in &snap.windows {
            let resets = match w.resets_at {
                Some(r) => petcore::fmt_elapsed(Some((r - now) as f64)),
                None => "-".to_string(),
            };
            println!(
                "{:>6}  {:<16} {:>5.1}%   resets in {}",
                w.provider.label(),
                w.label,
                w.used_percent,
                resets
            );
        }
        return;
    }

    let sessions = if let Some(path) = args.first() {
        match claude::parse_session(Path::new(path)) {
            Some(s) => vec![s],
            None => {
                eprintln!("could not parse {}", path);
                vec![]
            }
        }
    } else {
        petcore::find_sessions(Some(25))
    };

    if sessions.is_empty() {
        println!("no sessions found");
        return;
    }

    for s in &sessions {
        let live = if s.is_live { " [LIVE]" } else { "" };
        let branch = s.branch.as_deref().map(|b| format!("@{}", b)).unwrap_or_default();
        println!(
            "\n== {} ({}{}){} [{}]",
            s.title,
            s.project,
            branch,
            live,
            s.provider.label()
        );
        let start = s.prompts.len().saturating_sub(12);
        for p in &s.prompts[start..] {
            let metric = if p.running {
                "running...".to_string()
            } else {
                format!("{:>6} tok  {:>7}", fmt_tokens(p.out_tokens), fmt_elapsed(p.elapsed))
            };
            let title: String = p.title.chars().take(46).collect();
            println!("  {:>2} {:<46} {}", p.index, title, metric);
        }
        println!(
            "  -- {} prompts | {} tok | {} | {}",
            s.total_prompts,
            fmt_tokens(s.total_tokens),
            fmt_elapsed(s.wall_seconds),
            fmt_cost(Some(s.cost)),
        );
    }
}
