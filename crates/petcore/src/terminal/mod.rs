//! Terminal abstraction. The pet opens/focuses/starts sessions through a
//! `Terminal` backend so the terminal is configurable (requirement #2).
//!
//! WezTerm is the full backend (live working/waiting detection via `wezterm cli
//! list`). A generic command-template backend covers any other terminal in
//! launch-only mode (no live status).

pub mod generic;
pub mod wezterm;

use std::path::Path;

use crate::config::Config;
use crate::providers::Session;

/// An open terminal pane, used to detect which sessions are live/working.
#[derive(Clone, Debug)]
pub struct Pane {
    pub pane_id: String,
    pub title: String,
    pub cwd: String,
    pub glyph: String, // first non-alnum title char (Claude's spinner while working)
}

pub trait Terminal {
    /// Open panes (empty if the terminal can't be queried).
    fn list_panes(&self) -> Vec<Pane>;
    /// Whether this backend can report open/working/waiting status.
    fn supports_live_status(&self) -> bool;
    /// Focus an already-open pane by id.
    fn focus(&self, pane_id: &str) -> bool;
    /// Open a new tab/window in `cwd` running `inner` (a shell command string).
    fn spawn(&self, cwd: &Path, inner: &str) -> bool;
}

/// Build the configured backend.
pub fn backend(cfg: &Config) -> Box<dyn Terminal> {
    match cfg.terminal.backend.as_str() {
        "generic" => Box::new(generic::GenericTerminal::new(
            cfg.terminal.spawn_template.clone(),
            cfg.terminal.new_template.clone(),
        )),
        _ => Box::new(wezterm::WezTerm),
    }
}

/// Braille spinner glyphs Claude Code shows in the tab title while working.
pub const SPINNER: &str = "⠁⠂⠃⠄⠅⠆⠇⠈⠉⠊⠋⠌⠍⠎⠏⠐⠑⠒⠓⠔⠕⠖⠗⠘⠙⠚⠛⠜⠝⠞⠟\
⠠⠡⠢⠣⠤⠥⠦⠧⠨⠩⠪⠫⠬⠭⠮⠯⠰⠱⠲⠳⠴⠵⠶⠷⠸⠹⠺⠻⠼⠽⠾⠿⡀⢀";

fn is_spinner(glyph: &str) -> bool {
    glyph.chars().next().map(|c| SPINNER.contains(c)).unwrap_or(false)
}

/// Minimal POSIX single-quote escaping for embedding a value in a shell command.
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Open or focus a session in the terminal. If it's already open, focus that
/// pane; otherwise spawn a new tab that resumes it (claude/codex) and drops to a
/// shell so the tab stays open. Mirrors `open_in_wezterm`.
pub fn open_session(term: &dyn Terminal, s: &Session) -> bool {
    if s.open {
        if let Some(pid) = &s.pane_id {
            if term.focus(pid) {
                return true;
            }
        }
    }
    let cwd = match &s.cwd {
        Some(c) if Path::new(c).is_dir() => c.clone(),
        _ => directories::BaseDirs::new()
            .map(|b| b.home_dir().to_string_lossy().to_string())
            .unwrap_or_else(|| "/".to_string()),
    };
    let resume_cli = match s.provider {
        crate::providers::Provider::Codex => "codex resume",
        _ => "claude --resume",
    };
    let inner = if s.session_id.is_empty() {
        "exec claude".to_string()
    } else {
        format!("{} {} ; exec bash", resume_cli, shell_quote(&s.session_id))
    };
    term.spawn(Path::new(&cwd), &inner)
}

/// Start a brand-new session in `cwd`.
pub fn new_session(term: &dyn Terminal, cwd: &Path) -> bool {
    let cwd = if cwd.is_dir() {
        cwd.to_path_buf()
    } else {
        directories::BaseDirs::new()
            .map(|b| b.home_dir().to_path_buf())
            .unwrap_or_else(|| Path::new("/").to_path_buf())
    };
    term.spawn(&cwd, "claude ; exec bash")
}

/// Annotate sessions with open/pane_id/working/waiting from matching panes.
/// Mirrors `_annotate_open` + the open-flag reset in `find_sessions`.
pub fn annotate_open(sessions: &mut [Session], panes: &[Pane]) {
    for s in sessions.iter_mut() {
        s.open = false;
        s.pane_id = None;
        s.working = false;
        s.waiting = false;
        let scwd = s.cwd.clone().unwrap_or_default();
        let scwd = scwd.trim_end_matches('/');
        let title = s.title.to_lowercase();
        for pane in panes {
            let pcwd = pane.cwd.trim_end_matches('/');
            if !pane.cwd.is_empty()
                && !scwd.is_empty()
                && pcwd == scwd
                && !title.is_empty()
                && pane.title.to_lowercase().contains(&title)
            {
                s.open = true;
                s.pane_id = Some(pane.pane_id.clone());
                s.working = is_spinner(&pane.glyph);
                s.waiting = !s.working && s.last_completed;
                break;
            }
        }
    }
}
