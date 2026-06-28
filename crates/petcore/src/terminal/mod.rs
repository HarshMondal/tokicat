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
    /// The pane id the user is currently focused on, if the backend can report it.
    /// Used to auto-clear a session's attention once the user views it themselves.
    fn focused_pane(&self) -> Option<String> {
        None
    }
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

/// A pane title that's just a plain shell / tty (i.e. NOT an agent session).
fn is_shell_title(title: &str) -> bool {
    let t = title.trim().to_lowercase();
    t.is_empty()
        || matches!(t.as_str(), "bash" | "zsh" | "fish" | "sh" | "-bash" | "-zsh")
        || t.starts_with("/dev/")
}

/// Best-matching open pane for a session, or None. Matches by cwd, then scores by
/// how strongly the pane title identifies the session: the session title (Claude),
/// the project/dir basename (idle Codex/opencode panes are titled this way), or a
/// working spinner; plain shell panes are penalised.
pub fn find_pane<'a>(panes: &'a [Pane], s: &Session) -> Option<&'a Pane> {
    let scwd = s.cwd.clone().unwrap_or_default();
    let scwd = scwd.trim_end_matches('/');
    if scwd.is_empty() {
        return None;
    }
    let title = s.title.to_lowercase();
    let project = s.project.to_lowercase();
    let mut best: Option<(i32, &Pane)> = None;
    for pane in panes {
        if pane.cwd.trim_end_matches('/') != scwd {
            continue;
        }
        let ptitle = pane.title.to_lowercase();
        let mut score = 0;
        if !title.is_empty() && ptitle.contains(&title) {
            score += 3;
        }
        if !project.is_empty() && ptitle.contains(&project) {
            score += 2;
        }
        if is_spinner(&pane.glyph) {
            score += 1;
        }
        if is_shell_title(&pane.title) {
            score -= 3;
        }
        if score > 0 && best.map(|(b, _)| score > b).unwrap_or(true) {
            best = Some((score, pane));
        }
    }
    best.map(|(_, p)| p)
}

/// Minimal POSIX single-quote escaping for embedding a value in a shell command.
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Open or focus a session in the terminal. If it's already open, focus that
/// pane; otherwise spawn a new tab that resumes it (claude/codex) and drops to a
/// shell so the tab stays open. Mirrors `open_in_wezterm`.
pub fn open_session(term: &dyn Terminal, s: &Session, resume_tmpl: &str) -> bool {
    if s.open {
        if let Some(pid) = &s.pane_id {
            if term.focus(pid) {
                return true;
            }
        }
    }
    // Even if annotate_open didn't flag it open, try to find the session's pane (idle
    // Codex/opencode panes are titled with the dir name, not the session title) so we
    // reuse it instead of spawning a duplicate.
    let panes = term.list_panes();
    if let Some(p) = find_pane(&panes, s) {
        if term.focus(&p.pane_id) {
            return true;
        }
    }
    let cwd = match &s.cwd {
        Some(c) if Path::new(c).is_dir() => c.clone(),
        _ => directories::BaseDirs::new()
            .map(|b| b.home_dir().to_string_lossy().to_string())
            .unwrap_or_else(|| "/".to_string()),
    };
    let id_part = if s.session_id.is_empty() {
        String::new()
    } else {
        shell_quote(&s.session_id)
    };
    let cmd = resume_tmpl.replace("{id}", &id_part);
    let inner = format!("{} ; exec bash", cmd.trim());
    term.spawn(Path::new(&cwd), &inner)
}

/// Start a brand-new session in `cwd` using the provider's configured command.
pub fn new_session(term: &dyn Terminal, cwd: &Path, new_cmd: &str) -> bool {
    let cwd = if cwd.is_dir() {
        cwd.to_path_buf()
    } else {
        directories::BaseDirs::new()
            .map(|b| b.home_dir().to_path_buf())
            .unwrap_or_else(|| Path::new("/").to_path_buf())
    };
    let inner = format!("{} ; exec bash", new_cmd);
    term.spawn(&cwd, &inner)
}

/// Annotate sessions with open/pane_id/working/waiting from matching panes.
/// Mirrors `_annotate_open` + the open-flag reset in `find_sessions`.
pub fn annotate_open(sessions: &mut [Session], panes: &[Pane]) {
    for s in sessions.iter_mut() {
        s.open = false;
        s.pane_id = None;
        s.working = false;
        s.waiting = false;
        if let Some(p) = find_pane(panes, s) {
            let (pid, working) = (p.pane_id.clone(), is_spinner(&p.glyph));
            s.open = true;
            s.pane_id = Some(pid);
            s.working = working;
            s.waiting = !working && s.last_completed;
        }
    }
}
