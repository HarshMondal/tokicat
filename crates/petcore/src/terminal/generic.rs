//! Generic terminal backend — launch-only via a configurable command template.
//! No pane querying, so no live working/waiting detection.

use std::path::Path;
use std::process::Command;

use super::{Pane, Terminal};

pub struct GenericTerminal {
    spawn_template: String,
    new_template: String,
}

impl GenericTerminal {
    pub fn new(spawn_template: String, new_template: String) -> Self {
        GenericTerminal { spawn_template, new_template }
    }
}

/// Render a template with {cwd}/{cmd} and split into argv (whitespace split;
/// the {cmd} placeholder is passed as a single argument).
fn render(template: &str, cwd: &str, inner: &str) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();
    for tok in template.split_whitespace() {
        if tok == "{cmd}" {
            args.push(inner.to_string());
        } else {
            args.push(tok.replace("{cwd}", cwd).replace("{cmd}", inner));
        }
    }
    args
}

fn run(args: Vec<String>) -> bool {
    if args.is_empty() {
        return false;
    }
    Command::new(&args[0]).args(&args[1..]).spawn().is_ok()
}

impl Terminal for GenericTerminal {
    fn supports_live_status(&self) -> bool {
        false
    }

    fn list_panes(&self) -> Vec<Pane> {
        Vec::new()
    }

    fn focus(&self, _pane_id: &str) -> bool {
        false // can't focus; caller falls back to spawn
    }

    fn spawn(&self, cwd: &Path, inner: &str) -> bool {
        let cwd = cwd.to_string_lossy().to_string();
        run(render(&self.spawn_template, &cwd, inner))
    }
}

impl GenericTerminal {
    pub fn spawn_new(&self, cwd: &Path, inner: &str) -> bool {
        let cwd = cwd.to_string_lossy().to_string();
        run(render(&self.new_template, &cwd, inner))
    }
}
