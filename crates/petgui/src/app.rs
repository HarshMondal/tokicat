//! App state + the eframe update loop. Pet drawing lives in pet.rs, the panel in
//! panel.rs. This ports pet.py's App/Pet/Panel orchestration.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use egui::{Pos2, Vec2};

use petcore::config::Config;
use petcore::providers::Session;
use petcore::state::State;
use petcore::terminal::{self, Terminal};

use crate::art::Frame;
use crate::monitors;

pub const PANEL_W: f32 = 480.0; // default panel size; actual size lives in App.panel_w/h
pub const PANEL_H: f32 = 540.0;
pub const PANEL_MIN_W: f32 = 380.0;
pub const PANEL_MIN_H: f32 = 360.0;
const TELEPORT_GLIDE: f64 = 0.9;
const GROW_TIME: f64 = 0.5;
const ATTENTION_POLL: f64 = 5.0;
const ATTENTION_WINDOW: f64 = 600.0;
const PET_CHECK: f64 = 60.0;
const PET_GROW_SCALE: f32 = 2.6;
/// How often to re-assert the always-on-top window level. `with_always_on_top()` only
/// sets the WM hint at creation; many X11 WMs drop it when another window takes focus,
/// so we re-send WindowLevel::AlwaysOnTop on this cadence to keep the pet visible.
const TOPMOST_REASSERT: f64 = 0.5;
/// Per-provider cap for the session list. Used by every non-search refresh path so
/// the attention scan and the panel refresh don't fetch different-sized lists (which
/// would make rows flicker in/out between refreshes).
const SESSION_LIMIT: usize = 60;

#[derive(PartialEq, Clone, Copy)]
pub enum PetState {
    Normal,
    Petting,
    Celebrate,
}

/// Top-level panel tab. Provider tabs (Claude/Codex/Opencode) each do their own
/// list → click → prompt-history drill-in; Usage shows the quota cards.
#[derive(PartialEq, Clone, Copy)]
pub enum Tab {
    Usage,
    Claude,
    Codex,
    Opencode,
    Summary,
}

#[derive(PartialEq, Clone, Copy)]
pub enum SummaryPeriod {
    Today,
    ThisWeek,
}

pub struct Heart {
    pub x: f32,
    pub y: f32,
    pub vy: f32,
    pub r: f32,
    pub life: f32,
}

enum AnimDone {
    None,
    FinishPetting,
}

struct Anim {
    from_pos: Pos2,
    to_pos: Pos2,
    from_scale: f32,
    to_scale: f32,
    start: f64,
    dur: f64,
    on_done: AnimDone,
}

pub struct App {
    pub cfg: Config,
    pub state: State,
    pub term: Box<dyn Terminal>,

    // art
    pub frames: Vec<Frame>,
    pub textures: Vec<Option<egui::TextureHandle>>,
    pub logos: HashMap<petcore::Provider, egui::TextureHandle>,
    pub frame_idx: usize,
    frame_timer: f64,
    pub base_size: f32,

    // window geometry (screen coords, top-left)
    pub pos: Pos2,
    pub scale: f32,
    pub size: f32,
    placed: bool,
    anim: Option<Anim>,
    pub dragging: bool,
    last_pushed_pos: Pos2,
    last_pushed_size: f32,

    // data
    pub sessions: Vec<Session>,
    last_poll: f64,
    last_panel_refresh: f64,

    // attention
    pub attention: HashSet<String>,
    /// Last-seen completed-turn count per session path; attention fires when it grows.
    att_turns: HashMap<String, usize>,

    // personality
    pub pet_state: PetState,
    pub pets: u32,
    last_pet: f64,
    last_pet_check: f64,
    next_teleport: f64,
    home: Pos2,
    pub hearts: Vec<Heart>,
    pub wiggle: f32,
    pub bounce: f32,
    celebrate_start: f64,
    shrinking: bool,

    // panel
    pub panel_open: bool,
    pub tab: Tab,
    /// Drilled-into session id/path within a provider tab (None = list view).
    pub panel_session: Option<String>,
    pub panel_pos: Pos2,
    pub panel_w: f32,
    pub panel_h: f32,
    /// Pet→panel offset captured when the panel opens, so the panel stays docked to
    /// the pet as it moves (dragging the pet drags the panel). Clamped to `panel_mon`.
    panel_offset: Vec2,
    panel_mon: Option<monitors::MonRect>,
    pub last_panel_pushed: Pos2,
    // always-on-top re-assert timer + per-frame flag (read by the panel viewport)
    last_topmost: f64,
    pub reassert_top: bool,
    /// Whether the pointer is over the pet this frame (drives the hover bubble).
    pub pet_hovered: bool,
    /// Whether the "Needs you" popup dialog is open (separate small viewport).
    pub needs_open: bool,
    /// opencode tab: cached GLM/MiniMax session list + the All/GLM/MiniMax chip filter.
    pub oc_sessions: Vec<Session>,
    pub oc_filter: Option<petcore::Provider>,
    pub summary_period: SummaryPeriod,
    pub summary_provider: Option<petcore::Provider>,
    pub search: String,
    pub today_only: bool,
    pub expanded: HashSet<usize>,
    pub expanded_for: Option<String>,
    /// Whether the history view's detailed stats card is expanded.
    pub history_stats_open: bool,
    pub footer_hint: String,

    pub quota: petcore::quota::QuotaSnapshot,
    last_quota_refresh: f64,
    quota_shared: Arc<Mutex<Option<petcore::quota::QuotaSnapshot>>>,
    quota_inflight: Arc<AtomicBool>,

    rng: u64,
    booted: f64,

    // shutdown confirmation (Ctrl+Shift+U)
    pub quit_confirm: bool,
}

fn now_unix() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// Fire a desktop notification (+ optional sound) that a session is waiting on the
/// user. The session title is the summary so you know which one at a glance. Spawned
/// on detached threads that reap the children, so a missing/slow `notify-send` or
/// sound player never blocks (or zombifies) the UI thread.
fn notify_waiting(provider: &str, project: &str, title: &str, sound_command: &str) {
    let summary = if title.trim().is_empty() {
        format!("{provider} needs you")
    } else {
        title.to_string()
    };
    let body = format!("{provider} · {project} — waiting for you");
    std::thread::spawn(move || {
        let _ = std::process::Command::new("notify-send")
            .arg("--app-name=cc-pet")
            .arg(summary)
            .arg(body)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    });
    play_sound(sound_command);
}

/// Play the configured notification sound command (detached). Empty = disabled.
fn play_sound(sound_command: &str) {
    let parts: Vec<String> = sound_command.split_whitespace().map(String::from).collect();
    let Some((prog, args)) = parts.split_first().map(|(p, a)| (p.clone(), a.to_vec())) else {
        return; // empty command → no sound
    };
    std::thread::spawn(move || {
        let _ = std::process::Command::new(prog)
            .args(args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    });
}

fn smoothstep(f: f64) -> f64 {
    let f = f.clamp(0.0, 1.0);
    f * f * (3.0 - 2.0 * f)
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let cfg = Config::load();
        let state = State::load();
        crate::theme::apply(&cc.egui_ctx);
        crate::panel::set_ui_scale(cfg.ui.scale);

        let frames = crate::art::find_art(cfg.pet.max_size);
        let logos = crate::logos::load(&cc.egui_ctx);
        let base = frames
            .first()
            .map(|f| f.image.size[0].max(f.image.size[1]) as f32)
            .unwrap_or(cfg.pet.max_size as f32);
        let base_size = base.clamp(48.0, cfg.pet.max_size as f32);
        let n = frames.len();

        let last_pet = state.last_pet_time.unwrap_or_else(now_unix);
        let pos = Pos2::new(
            state.pet_x.unwrap_or(1500) as f32,
            state.pet_y.unwrap_or(60) as f32,
        );
        let ui_scale = cfg.ui.scale.clamp(0.8, 2.0);
        let state_panel_w = state.panel_w.unwrap_or(PANEL_W * ui_scale).max(PANEL_MIN_W);
        let state_panel_h = state.panel_h.unwrap_or(PANEL_H * ui_scale).max(PANEL_MIN_H);

        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E3779B97F4A7C15)
            | 1;

        App {
            term: terminal::backend(&cfg),
            cfg,
            state,
            frames,
            textures: vec![None; n],
            logos,
            frame_idx: 0,
            frame_timer: 0.0,
            base_size,
            pos,
            scale: 1.0,
            size: base_size,
            placed: false,
            anim: None,
            dragging: false,
            last_pushed_pos: Pos2::new(f32::MIN, f32::MIN),
            last_pushed_size: -1.0,
            sessions: Vec::new(),
            last_poll: -1e9,
            last_panel_refresh: -1e9,
            attention: HashSet::new(),
            att_turns: HashMap::new(),
            pet_state: PetState::Normal,
            pets: 0,
            last_pet,
            last_pet_check: 0.0,
            next_teleport: 0.0,
            home: pos,
            hearts: Vec::new(),
            wiggle: 0.0,
            bounce: 0.0,
            celebrate_start: 0.0,
            shrinking: false,
            panel_open: false,
            tab: Tab::Usage,
            panel_session: None,
            panel_pos: Pos2::ZERO,
            panel_w: state_panel_w,
            panel_h: state_panel_h,
            panel_offset: Vec2::ZERO,
            panel_mon: None,
            last_panel_pushed: Pos2::new(f32::MIN, f32::MIN),
            last_topmost: -1e9,
            reassert_top: false,
            pet_hovered: false,
            needs_open: false,
            oc_sessions: Vec::new(),
            oc_filter: None,
            summary_period: SummaryPeriod::ThisWeek,
            summary_provider: None,
            search: String::new(),
            today_only: false,
            expanded: HashSet::new(),
            expanded_for: None,
            history_stats_open: false,
            footer_hint: String::new(),
            quota: petcore::quota::QuotaSnapshot::default(),
            last_quota_refresh: -1e9,
            quota_shared: Arc::new(Mutex::new(None)),
            quota_inflight: Arc::new(AtomicBool::new(false)),
            rng: seed,
            booted: 0.0,
            quit_confirm: false,
        }
    }

    /// The shutdown chord: Ctrl+Shift+U. Only fires for the viewport that currently
    /// holds keyboard focus, so it is checked in both the pet and panel viewports.
    pub(crate) fn quit_chord(ctx: &egui::Context) -> bool {
        ctx.input(|i| {
            i.modifiers.ctrl && i.modifiers.shift && !i.modifiers.alt && i.key_pressed(egui::Key::U)
        })
    }

    pub fn rand01(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        (x >> 11) as f32 / (1u64 << 53) as f32
    }

    pub fn any_live(&self) -> bool {
        self.sessions.iter().any(|s| s.is_live)
    }

    /// Cancel any in-progress position/scale animation (e.g. when dragging).
    pub fn cancel_animation(&mut self) {
        self.anim = None;
    }

    /// Limit for the session list: capped normally, full pool while searching
    /// (search needs every match, not just the most recent). Shared by all refresh
    /// paths so they never fetch different-sized lists and flicker against each other.
    pub fn list_limit(&self) -> Option<usize> {
        if self.search.is_empty() && self.tab != Tab::Summary { Some(SESSION_LIMIT) } else { None }
    }

    // ---- data refresh + attention ---------------------------------------
    pub fn refresh(&mut self, limit: Option<usize>) {
        let mut sessions = petcore::find_sessions(limit);
        if self.term.supports_live_status() {
            let panes = self.term.list_panes();
            terminal::annotate_open(&mut sessions, &panes);
        }
        self.sessions = sessions;
    }

    fn poll_attention(&mut self) {
        let lim = self.list_limit();
        self.refresh(lim);
        // Include opencode (GLM/MiniMax) sessions so attention works for every provider,
        // not just the Claude/Codex transcripts in self.sessions.
        self.reload_opencode();
        // The pane the user is actually looking at — so we don't alert for (or keep
        // alerting) a session they've already opened in the terminal themselves.
        let focused = if self.term.supports_live_status() {
            self.term.focused_pane()
        } else {
            None
        };
        let now = now_unix();
        let mut new_attention = false;
        // Scan Claude/Codex + opencode together. Trigger = a session's completed-turn
        // count just grew (an agent finished a turn) — a transcript signal that works
        // across providers, instead of the Claude-only terminal spinner.
        let scan: Vec<Session> =
            self.sessions.iter().chain(self.oc_sessions.iter()).cloned().collect();
        for s in &scan {
            let path = s.path.to_string_lossy().to_string();
            let is_focused = s.pane_id.is_some() && s.pane_id.as_deref() == focused.as_deref();
            let recent = s.last_prompt_ts.map(|t| t >= now - ATTENTION_WINDOW).unwrap_or(false)
                || (now - s.mtime) <= ATTENTION_WINDOW;
            let prev = self.att_turns.get(&path).copied();
            let grew = prev.map(|p| s.completed_turns > p).unwrap_or(false);
            // A turn just finished → alert, unless you're already viewing that pane.
            // (prev=None is the startup baseline — record without alerting.)
            if grew && recent && !is_focused && !self.attention.contains(&path) {
                self.attention.insert(path.clone());
                notify_waiting(
                    s.provider.display_name(),
                    &s.project,
                    &s.title,
                    &self.cfg.notify.sound_command,
                );
                new_attention = true;
            }
            // Clear once you switch to its pane (Claude/Codex; opencode clears via the
            // Needs tab / opening it, since it has no pane mapping).
            if is_focused {
                self.attention.remove(&path);
            }
            self.att_turns.insert(path, s.completed_turns);
        }
        if new_attention {
            self.bounce = 16.0;
        }
    }

    /// Resolve the configured "new session" command for a provider (Zai/MiniMax/
    /// opencode all use the opencode command).
    pub fn new_cmd(&self, p: petcore::Provider) -> String {
        let s = &self.cfg.sessions;
        match p {
            petcore::Provider::Claude => s.new_claude.clone(),
            petcore::Provider::Codex => s.new_codex.clone(),
            _ => s.new_opencode.clone(),
        }
    }

    /// Resolve the configured "resume session" command template (`{id}`) for a provider.
    pub fn resume_cmd(&self, p: petcore::Provider) -> String {
        let s = &self.cfg.sessions;
        match p {
            petcore::Provider::Claude => s.resume_claude.clone(),
            petcore::Provider::Codex => s.resume_codex.clone(),
            _ => s.resume_opencode.clone(),
        }
    }

    pub fn clear_attention(&mut self, path: &str) {
        self.attention.remove(path);
    }

    /// Sessions currently flagged as waiting on the user, newest-finished first.
    /// Searches Claude/Codex (self.sessions) and opencode (self.oc_sessions).
    pub fn waiting_sessions(&self) -> Vec<Session> {
        let mut out: Vec<Session> = self
            .sessions
            .iter()
            .chain(self.oc_sessions.iter())
            .filter(|s| self.attention.contains(&s.path.to_string_lossy().to_string()))
            .cloned()
            .collect();
        out.sort_by(|a, b| b.mtime.partial_cmp(&a.mtime).unwrap_or(std::cmp::Ordering::Equal));
        out
    }

    /// Click handler when the pet has pending attention: toggle the small "Needs you"
    /// popup beside the cat (clicking again closes it). Falls back to the panel if
    /// nothing is actually waiting.
    pub fn attention_click(&mut self, _ctx: &egui::Context) {
        if self.waiting_sessions().is_empty() {
            self.toggle_panel(_ctx);
            return;
        }
        self.needs_open = !self.needs_open;
    }

    /// Focus a waiting session's terminal and clear its attention badge.
    pub fn focus_waiting(&mut self, s: &Session) {
        let path = s.path.to_string_lossy().to_string();
        let tmpl = self.resume_cmd(s.provider);
        petcore::terminal::open_session(self.term.as_ref(), s, &tmpl);
        self.clear_attention(&path);
    }

    /// Kick off an async quota refresh (Claude fetch can block, so it runs on a
    /// background thread). The TTL inside quota::snapshot prevents over-fetching.
    pub fn refresh_quota(&mut self) {
        if self.quota_inflight.swap(true, Ordering::SeqCst) {
            return; // already running
        }
        let cfg = self.cfg.clone();
        let shared = self.quota_shared.clone();
        let inflight = self.quota_inflight.clone();
        std::thread::spawn(move || {
            let snap = petcore::quota::snapshot(&cfg);
            *shared.lock().unwrap() = Some(snap);
            inflight.store(false, Ordering::SeqCst);
        });
    }

    fn drain_quota(&mut self) {
        if let Some(snap) = self.quota_shared.lock().unwrap().take() {
            self.quota = snap;
        }
    }

    // ---- animation ------------------------------------------------------
    fn center(&self) -> Pos2 {
        Pos2::new(self.pos.x + self.size / 2.0, self.pos.y + self.size / 2.0)
    }

    fn start_anim(&mut self, to_center: Pos2, to_scale: f32, dur: f64, t: f64, on_done: AnimDone) {
        let from_size = self.size;
        let to_size = self.base_size * to_scale;
        // convert center targets to top-left, interpolating top-left directly
        let from_pos = self.pos;
        let to_pos = Pos2::new(to_center.x - to_size / 2.0, to_center.y - to_size / 2.0);
        let _ = from_size;
        self.anim = Some(Anim {
            from_pos,
            to_pos,
            from_scale: self.scale,
            to_scale,
            start: t,
            dur,
            on_done,
        });
    }

    fn step_anim(&mut self, t: f64) {
        let Some(a) = &self.anim else { return };
        let f = if a.dur <= 0.0 { 1.0 } else { (t - a.start) / a.dur };
        let e = smoothstep(f) as f32;
        self.scale = a.from_scale + (a.to_scale - a.from_scale) * e;
        self.size = (self.base_size * self.scale).max(8.0);
        self.pos = Pos2::new(
            a.from_pos.x + (a.to_pos.x - a.from_pos.x) * e,
            a.from_pos.y + (a.to_pos.y - a.from_pos.y) * e,
        );
        if f >= 1.0 {
            let done = match self.anim.take() {
                Some(a) => a.on_done,
                None => AnimDone::None,
            };
            match done {
                AnimDone::FinishPetting => {
                    self.pet_state = PetState::Normal;
                    self.last_pet = now_unix();
                    self.state.last_pet_time = Some(self.last_pet);
                    self.state.save();
                    self.shrinking = false;
                }
                AnimDone::None => {}
            }
        }
    }

    fn clamp_center(mons: &[monitors::MonRect], cx: f32, cy: f32, size: f32) -> Pos2 {
        let m = monitors::monitor_at(mons, cx, cy);
        let half = size / 2.0;
        let cx = cx.clamp(m.x + half, (m.right() - half).max(m.x + half));
        let cy = cy.clamp(m.y + half, (m.bottom() - half).max(m.y + half));
        Pos2::new(cx, cy)
    }

    // ---- teleport / petting --------------------------------------------
    fn do_teleport(&mut self, ctx: &egui::Context, t: f64) {
        if self.pet_state != PetState::Normal || self.anim.is_some() || self.panel_open {
            return;
        }
        let mons = monitors::all(ctx);
        // pick a random monitor, then a random point within it
        let idx = (self.rand01() * mons.len() as f32) as usize;
        let m = mons.get(idx.min(mons.len().saturating_sub(1))).copied().unwrap_or(mons[0]);
        let cx = m.x + self.size + self.rand01() * (m.w - 2.0 * self.size).max(1.0);
        let cy = m.y + self.size + self.rand01() * (m.h - 2.0 * self.size).max(1.0);
        let c = Self::clamp_center(&mons, cx, cy, self.size);
        self.start_anim(c, 1.0, TELEPORT_GLIDE, t, AnimDone::None);
    }

    fn enter_petting(&mut self, ctx: &egui::Context, t: f64) {
        self.panel_open = false;
        self.pet_state = PetState::Petting;
        self.pets = 0;
        self.home = self.center();
        let mons = monitors::all(ctx);
        let m = monitors::monitor_at(&mons, self.center().x, self.center().y);
        // Center on this monitor, but clamp so the *grown* pet stays fully on one
        // screen — never straddling a boundary even under fractional/mixed-DPI
        // rounding where a single global pixels_per_point can be slightly off.
        let grown = self.base_size * PET_GROW_SCALE;
        let c = Self::clamp_center(&mons, m.x + m.w / 2.0, m.y + m.h / 2.0, grown);
        self.start_anim(c, PET_GROW_SCALE, GROW_TIME, t, AnimDone::None);
    }

    pub fn on_pet(&mut self) {
        self.pets += 1;
        self.wiggle = 10.0;
        self.spawn_hearts(3);
        if self.pets >= self.cfg.pet.pets_required {
            self.pet_state = PetState::Celebrate;
            self.spawn_hearts(14);
            self.wiggle = 16.0;
            self.celebrate_start = now_unix();
        }
    }

    pub fn spawn_hearts(&mut self, n: usize) {
        for _ in 0..n {
            let x = self.size / 2.0 + (self.rand01() - 0.5) * self.size * 0.5;
            let y = self.size * 0.5 + (self.rand01() - 0.5) * 20.0;
            let vy = 0.6 + self.rand01() * 1.0;
            let r = 4.0 + self.rand01() * 5.0;
            self.hearts.push(Heart { x, y, vy, r, life: 1.0 });
        }
    }

    fn step_hearts(&mut self, dt: f32) {
        for h in &mut self.hearts {
            h.y -= h.vy * dt * 60.0;
            h.life -= dt / 0.9;
        }
        self.hearts.retain(|h| h.life > 0.0);
    }

    pub fn toggle_panel(&mut self, ctx: &egui::Context) {
        self.panel_open = !self.panel_open;
        if self.panel_open {
            self.anim = None;
            self.place_panel(ctx);
            // open straight to the usage-limits tab
            self.tab = Tab::Usage;
            self.panel_session = None;
            self.refresh(Some(SESSION_LIMIT));
            self.refresh_quota();
        }
    }

    /// Switch tabs: reset the per-tab drill state and lazily load opencode sessions.
    pub fn switch_tab(&mut self, tab: Tab) {
        self.tab = tab;
        self.panel_session = None;
        self.footer_hint.clear();
        if tab == Tab::Opencode && self.oc_sessions.is_empty() {
            self.reload_opencode();
        }
        if tab == Tab::Summary {
            self.refresh(None);
            self.reload_opencode();
        }
    }

    /// Reload the GLM/MiniMax session list from opencode's DB (cheap metadata query).
    pub fn reload_opencode(&mut self) {
        self.oc_sessions = petcore::find_opencode_sessions(Some(60));
    }

    /// Refresh everything the header's ⟳ button should pull: quota + sessions (+opencode).
    pub fn refresh_all(&mut self) {
        self.refresh_quota();
        let limit = self.list_limit();
        self.refresh(limit);
        self.reload_opencode();
    }

    fn place_panel(&mut self, ctx: &egui::Context) {
        let mons = monitors::all(ctx);
        let m = monitors::monitor_at(&mons, self.center().x, self.center().y);
        let mut x = self.pos.x - self.panel_w - 12.0;
        if x < m.x + 8.0 {
            x = self.pos.x + self.size + 12.0;
        }
        x = x.clamp(m.x + 8.0, (m.right() - self.panel_w - 8.0).max(m.x + 8.0));
        let y = self.pos.y.clamp(m.y + 8.0, (m.bottom() - self.panel_h - 8.0).max(m.y + 8.0));
        self.panel_pos = Pos2::new(x, y);
        // Dock to the pet: remember the offset + monitor so the panel follows the pet
        // as it's dragged, without re-querying xrandr every frame.
        self.panel_offset = self.panel_pos - self.pos;
        self.panel_mon = Some(m);
        self.last_panel_pushed = Pos2::new(f32::MIN, f32::MIN);
    }

    /// Keep the panel docked to the pet at its open-time offset. `anchor` is the pet's
    /// current top-left (live window rect during a drag, logical pos otherwise).
    fn follow_panel(&mut self, anchor: Pos2) {
        let mut p = anchor + self.panel_offset;
        if let Some(m) = self.panel_mon {
            p.x = p.x.clamp(m.x + 8.0, (m.right() - self.panel_w - 8.0).max(m.x + 8.0));
            p.y = p.y.clamp(m.y + 8.0, (m.bottom() - self.panel_h - 8.0).max(m.y + 8.0));
        }
        self.panel_pos = p;
    }

    pub fn show_history(&mut self, path: &str) {
        self.panel_session = Some(path.to_string());
        if self.expanded_for.as_deref() != Some(path) {
            self.expanded.clear();
            self.expanded_for = Some(path.to_string());
        }
    }

    // ---- window push ----------------------------------------------------
    fn push_window(&mut self, ctx: &egui::Context) {
        if (self.pos.x - self.last_pushed_pos.x).abs() > 0.5
            || (self.pos.y - self.last_pushed_pos.y).abs() > 0.5
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(self.pos));
            self.last_pushed_pos = self.pos;
        }
        if (self.size - self.last_pushed_size).abs() > 0.5 {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::splat(self.size)));
            self.last_pushed_size = self.size;
        }
    }
}

impl eframe::App for App {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0] // transparent pet window
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx_owned = ui.ctx().clone();
        let ctx = &ctx_owned;
        // Repaint at the display's rate while the pet is moving (drag or animation) so it
        // tracks the cursor smoothly; throttle to ~30 fps when idle to save CPU.
        if self.dragging || self.anim.is_some() {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(Duration::from_millis(33));
        }
        let t = ctx.input(|i| i.time);
        let dt = ctx.input(|i| i.stable_dt).min(0.1);
        if self.booted == 0.0 {
            self.booted = t;
            self.last_pet_check = t;
            self.next_teleport = t
                + (self.cfg.pet.teleport_min_secs as f64)
                + self.rand01() as f64
                    * (self.cfg.pet.teleport_max_secs - self.cfg.pet.teleport_min_secs) as f64;
        }

        // Place at default top-right on first frame if no saved position.
        if !self.placed {
            self.placed = true;
            if self.state.pet_x.is_none() || self.state.pet_y.is_none() {
                let mons = monitors::all(ctx);
                let m = mons[0];
                self.pos = Pos2::new(m.right() - self.base_size - 40.0, m.y + 60.0);
            }
            self.size = self.base_size * self.scale;
            self.push_window(ctx);
        }

        // Live pet-window top-left (updates even mid-drag); used to dock the panel.
        let live_pos = ctx.input(|i| i.viewport().outer_rect).map(|r| r.min);
        // Sync our logical position from the real window unless we're animating or
        // dragging it. During a WM-native drag the window manager owns the position;
        // reading it back into self.pos fights the move and causes visible jitter.
        if self.anim.is_none() && !self.dragging {
            if let Some(p) = live_pos {
                self.pos = p;
            }
        }
        // Dock the panel to the pet so dragging the pet drags the panel too.
        if self.panel_open {
            let anchor = if self.dragging { live_pos.unwrap_or(self.pos) } else { self.pos };
            self.follow_panel(anchor);
        }

        // advance gif frame
        if !self.frames.is_empty() {
            self.frame_timer += dt as f64 * 1000.0;
            let cur = self.frames[self.frame_idx].delay_ms as f64;
            if self.frame_timer >= cur {
                self.frame_timer = 0.0;
                self.frame_idx = (self.frame_idx + 1) % self.frames.len();
            }
        }

        // animation + decay
        self.step_anim(t);
        self.step_hearts(dt);
        self.wiggle = if self.wiggle > 0.05 { self.wiggle * 0.85 } else { 0.0 };
        if !self.attention.is_empty() && self.bounce < 1.0 {
            self.bounce = 14.0;
        }
        self.bounce = if self.bounce > 0.3 { self.bounce * 0.9 } else { 0.0 };

        // celebrate -> shrink back home
        if self.pet_state == PetState::Celebrate
            && !self.shrinking
            && self.anim.is_none()
            && now_unix() - self.celebrate_start > 0.45
        {
            self.shrinking = true;
            let home = self.home;
            self.start_anim(home, 1.0, GROW_TIME, t, AnimDone::FinishPetting);
        }

        // attention poll
        if t - self.last_poll >= ATTENTION_POLL {
            self.last_poll = t;
            self.poll_attention();
        }
        // quota refresh (async; TTL-gated). Refresh more eagerly while the panel
        // is open, lazily otherwise.
        self.drain_quota();
        let quota_interval = if self.panel_open { 20.0 } else { 120.0 };
        if t - self.last_quota_refresh >= quota_interval {
            self.last_quota_refresh = t;
            self.refresh_quota();
        }
        // panel refresh
        if self.panel_open && t - self.last_panel_refresh >= self.cfg.pet.refresh_secs as f64 {
            self.last_panel_refresh = t;
            let limit = self.list_limit();
            self.refresh(limit);
            if self.tab == Tab::Opencode || self.tab == Tab::Summary {
                self.reload_opencode();
            }
        }

        // teleport
        if t >= self.next_teleport {
            self.do_teleport(ctx, t);
            self.next_teleport = t
                + self.cfg.pet.teleport_min_secs as f64
                + self.rand01() as f64
                    * (self.cfg.pet.teleport_max_secs - self.cfg.pet.teleport_min_secs) as f64;
        }
        // petting check
        if t - self.last_pet_check >= PET_CHECK {
            self.last_pet_check = t;
            if self.pet_state == PetState::Normal
                && self.anim.is_none()
                && now_unix() - self.last_pet >= self.cfg.pet.pet_interval_secs as f64
            {
                self.enter_petting(ctx, t);
            }
        }

        if self.anim.is_some() {
            self.push_window(ctx);
        }

        // Shutdown chord while the pet itself is focused. The confirmation modal lives
        // in the panel viewport (the 110px pet window is too small for it), so open the
        // panel to host it.
        if Self::quit_chord(ctx) {
            self.quit_confirm = true;
            self.panel_open = true;
        }

        // Keep the pet (and panel) above other windows: periodically re-assert the
        // always-on-top level, which X11 WMs tend to drop when another window is focused.
        self.reassert_top = t - self.last_topmost >= TOPMOST_REASSERT;
        if self.reassert_top {
            self.last_topmost = t;
            ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(
                egui::WindowLevel::AlwaysOnTop,
            ));
        }

        // draw the pet (pet.rs) directly into the root ui
        self.ui_pet(ui);

        // auto-close the "Needs you" popup once nothing is waiting
        if self.attention.is_empty() {
            self.needs_open = false;
        }
        // "Needs you" popup (separate small dialog beside the cat)
        if self.needs_open {
            self.show_needs_viewport(ctx);
        }

        // hover bubble: glanceable context for waiting sessions (suppressed while the
        // panel or the needs popup is open, since those already show everything).
        if self.pet_hovered
            && !self.attention.is_empty()
            && !self.panel_open
            && !self.needs_open
            && !self.dragging
        {
            self.show_bubble_viewport(ctx);
        }

        // draw the panel viewport (panel.rs)
        if self.panel_open {
            self.show_panel_viewport(ctx);
        }
    }
}
