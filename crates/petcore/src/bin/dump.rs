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
        if snap.sections.is_empty() {
            println!("no quota data");
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        for sec in &snap.sections {
            let badge = sec.badge.as_deref().map(|b| format!(" [{}]", b)).unwrap_or_default();
            println!("\n{}{}", sec.provider.display_name(), badge);
            if let Some(note) = &sec.note {
                println!("  note: {}", note);
            }
            for w in &sec.windows {
                let resets = if w.reset {
                    "reset".to_string()
                } else {
                    match w.resets_at {
                        Some(r) if r > now => petcore::fmt_elapsed(Some((r - now) as f64)),
                        Some(_) => "soon".to_string(),
                        None => "-".to_string(),
                    }
                };
                println!("  {:<16} {:>5.1}%   resets in {}", w.label, w.used_percent, resets);
            }
            if let Some(sum) = &sec.summary {
                println!(
                    "  tokens: {} in / {} out · {}",
                    fmt_tokens(sum.tokens_input),
                    fmt_tokens(sum.tokens_output),
                    fmt_cost(Some(sum.cost))
                );
            }
            for child in &sec.children {
                let badge = child.badge.as_deref().map(|b| format!(" [{}]", b)).unwrap_or_default();
                let note = child.note.as_deref().map(|n| format!("  ({})", n)).unwrap_or_default();
                println!("  └ {}{}{}", child.provider.display_name(), badge, note);
                for w in &child.windows {
                    let resets = if w.reset {
                        "reset".to_string()
                    } else {
                        match w.resets_at {
                            Some(r) if r > now => petcore::fmt_elapsed(Some((r - now) as f64)),
                            Some(_) => "soon".to_string(),
                            None => "-".to_string(),
                        }
                    };
                    println!("      {:<14} {:>5.1}%   resets in {}", w.label, w.used_percent, resets);
                }
                if let Some(sum) = &child.summary {
                    println!(
                        "      tokens: {} in / {} out · {}",
                        fmt_tokens(sum.tokens_input),
                        fmt_tokens(sum.tokens_output),
                        fmt_cost(Some(sum.cost))
                    );
                }
            }
        }
        return;
    }

    if args.first().map(|s| s.as_str()) == Some("--opencode") {
        let list = petcore::find_opencode_sessions(Some(15));
        println!("opencode sessions: {}", list.len());
        for s in &list {
            println!(
                "  {:<8} {:<40} {} prompts · {} tok · {}",
                s.provider.label(),
                s.title.chars().take(40).collect::<String>(),
                s.total_prompts,
                fmt_tokens(s.total_tokens),
                fmt_cost(Some(s.cost)),
            );
        }
        // drill into the newest to verify per-prompt history
        if let Some(first) = list.first() {
            if let Some(full) = petcore::parse_session_any(Path::new(&first.session_id)) {
                println!("\n  drill into {}: {} prompts", full.title, full.prompts.len());
                for p in full.prompts.iter().take(5) {
                    println!(
                        "    {:>2} {:<46} {:>6} tok {:>7}",
                        p.index,
                        p.title.chars().take(46).collect::<String>(),
                        fmt_tokens(p.out_tokens),
                        fmt_elapsed(p.elapsed),
                    );
                }
            }
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
