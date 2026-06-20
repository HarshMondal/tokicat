//! WezTerm backend — full live-status support. Port of the WezTerm bits of
//! sessions.py (`wezterm_panes`, `_norm_cwd`) and pet.py (`_wezterm_spawn`).

use std::path::Path;
use std::process::Command;

use serde_json::Value;

use super::{Pane, Terminal};

pub struct WezTerm;

/// 'file://host/home/harsh/Foo/' -> '/home/harsh/Foo'.
fn norm_cwd(uri: &str) -> String {
    if uri.is_empty() {
        return String::new();
    }
    let s = if let Some(rest) = uri.strip_prefix("file://") {
        // drop the host segment up to the first '/'
        match rest.find('/') {
            Some(i) => &rest[i..],
            None => "",
        }
    } else {
        uri
    };
    s.trim_end_matches('/').to_string()
}

impl Terminal for WezTerm {
    fn supports_live_status(&self) -> bool {
        true
    }

    fn list_panes(&self) -> Vec<Pane> {
        let out = match Command::new("wezterm")
            .args(["cli", "list", "--format", "json"])
            .output()
        {
            Ok(o) if o.status.success() => o,
            _ => return Vec::new(),
        };
        let data: Value = match serde_json::from_slice(&out.stdout) {
            Ok(v) => v,
            Err(_) => return Vec::new(),
        };
        let mut panes = Vec::new();
        if let Some(arr) = data.as_array() {
            for p in arr {
                let title = p.get("title").and_then(Value::as_str).unwrap_or("").trim().to_string();
                let glyph = title
                    .chars()
                    .next()
                    .filter(|c| !c.is_alphanumeric())
                    .map(|c| c.to_string())
                    .unwrap_or_default();
                let pane_id = match p.get("pane_id") {
                    Some(Value::Number(n)) => n.to_string(),
                    Some(Value::String(s)) => s.clone(),
                    _ => continue,
                };
                let cwd = norm_cwd(p.get("cwd").and_then(Value::as_str).unwrap_or(""));
                panes.push(Pane { pane_id, title, cwd, glyph });
            }
        }
        panes
    }

    fn focus(&self, pane_id: &str) -> bool {
        Command::new("wezterm")
            .args(["cli", "activate-pane", "--pane-id", pane_id])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn spawn(&self, cwd: &Path, inner: &str) -> bool {
        let cwd = cwd.to_string_lossy().to_string();
        // new tab in the running WezTerm
        let spawned = Command::new("wezterm")
            .args(["cli", "spawn", "--cwd", &cwd, "--", "bash", "-lc", inner])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if spawned {
            return true;
        }
        // fallback: brand-new window
        Command::new("wezterm")
            .args(["start", "--cwd", &cwd, "--", "bash", "-lc", inner])
            .spawn()
            .is_ok()
    }
}
