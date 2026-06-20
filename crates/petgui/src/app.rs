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

pub const PANEL_W: f32 = 480.0;
pub const PANEL_H: f32 = 540.0;
const TELEPORT_GLIDE: f64 = 0.9;
const GROW_TIME: f64 = 0.5;
const ATTENTION_POLL: f64 = 5.0;
const ATTENTION_WINDOW: f64 = 600.0;
const PET_CHECK: f64 = 60.0;
const PET_GROW_SCALE: f32 = 2.6;

#[derive(PartialEq, Clone, Copy)]
pub enum PetState {
    Normal,
    Petting,
    Celebrate,
}

#[derive(PartialEq, Clone, Copy)]
pub enum PanelView {
    List,
    History,
    Quota,
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
    pub frame_idx: usize,
    frame_timer: f64,
    pub base_size: f32,

    // window geometry (screen coords, top-left)
    pub pos: Pos2,
    pub scale: f32,
    pub size: f32,
    placed: bool,
    anim: Option<Anim>,
    last_pushed_pos: Pos2,
    last_pushed_size: f32,

    // data
    pub sessions: Vec<Session>,
    last_poll: f64,
    last_panel_refresh: f64,

    // attention
    pub attention: HashSet<String>,
    att_working: HashMap<String, bool>,

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
    pub panel_view: PanelView,
    pub panel_session: Option<String>,
    pub panel_pos: Pos2,
    pub search: String,
    pub today_only: bool,
    pub expanded: HashSet<usize>,
    pub expanded_for: Option<String>,
    pub footer_hint: String,

    pub quota: petcore::quota::QuotaSnapshot,
    last_quota_refresh: f64,
    quota_shared: Arc<Mutex<Option<petcore::quota::QuotaSnapshot>>>,
    quota_inflight: Arc<AtomicBool>,

    rng: u64,
    booted: f64,
}

fn now_unix() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
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

        let frames = crate::art::find_art(cfg.pet.max_size);
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
            frame_idx: 0,
            frame_timer: 0.0,
            base_size,
            pos,
            scale: 1.0,
            size: base_size,
            placed: false,
            anim: None,
            last_pushed_pos: Pos2::new(f32::MIN, f32::MIN),
            last_pushed_size: -1.0,
            sessions: Vec::new(),
            last_poll: -1e9,
            last_panel_refresh: -1e9,
            attention: HashSet::new(),
            att_working: HashMap::new(),
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
            panel_view: PanelView::List,
            panel_session: None,
            panel_pos: Pos2::ZERO,
            search: String::new(),
            today_only: false,
            expanded: HashSet::new(),
            expanded_for: None,
            footer_hint: String::new(),
            quota: petcore::quota::QuotaSnapshot::default(),
            last_quota_refresh: -1e9,
            quota_shared: Arc::new(Mutex::new(None)),
            quota_inflight: Arc::new(AtomicBool::new(false)),
            rng: seed,
            booted: 0.0,
        }
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
        self.refresh(Some(25));
        let now = now_unix();
        let mut new_attention = false;
        for s in &self.sessions {
            let path = s.path.to_string_lossy().to_string();
            if !s.open {
                self.att_working.remove(&path);
                self.attention.remove(&path);
                continue;
            }
            let was = self.att_working.get(&path).copied().unwrap_or(false);
            let now_working = s.working;
            let recent = s.last_prompt_ts.map(|t| t >= now - ATTENTION_WINDOW).unwrap_or(false)
                || (now - s.mtime) <= ATTENTION_WINDOW;
            if was && !now_working && s.waiting && recent && !self.attention.contains(&path) {
                self.attention.insert(path.clone());
                new_attention = true;
            }
            if now_working {
                self.attention.remove(&path);
            }
            self.att_working.insert(path, now_working);
        }
        if new_attention {
            self.bounce = 16.0;
        }
    }

    pub fn clear_attention(&mut self, path: &str) {
        self.attention.remove(path);
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
        let c = Pos2::new(m.x + m.w / 2.0, m.y + m.h / 2.0);
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
            // decide initial view from saved state
            if let Some(last) = self.state.last_session.clone() {
                if std::path::Path::new(&last).is_file() {
                    self.show_history(&last);
                } else {
                    self.panel_view = PanelView::List;
                }
            } else {
                self.panel_view = PanelView::List;
            }
            self.refresh(Some(40));
            self.refresh_quota();
        }
    }

    fn place_panel(&mut self, ctx: &egui::Context) {
        let mons = monitors::all(ctx);
        let m = monitors::monitor_at(&mons, self.center().x, self.center().y);
        let mut x = self.pos.x - PANEL_W - 12.0;
        if x < m.x + 8.0 {
            x = self.pos.x + self.size + 12.0;
        }
        x = x.clamp(m.x + 8.0, (m.right() - PANEL_W - 8.0).max(m.x + 8.0));
        let y = self.pos.y.clamp(m.y + 8.0, (m.bottom() - PANEL_H - 8.0).max(m.y + 8.0));
        self.panel_pos = Pos2::new(x, y);
    }

    pub fn show_history(&mut self, path: &str) {
        self.panel_view = PanelView::History;
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
        ctx.request_repaint_after(Duration::from_millis(33));
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

        // Sync our position from the real window unless we're animating it.
        if self.anim.is_none() {
            if let Some(rect) = ctx.input(|i| i.viewport().outer_rect) {
                self.pos = rect.min;
            }
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
            let limit = if self.search.is_empty() { Some(40) } else { None };
            self.refresh(limit);
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

        // draw the pet (pet.rs) directly into the root ui
        self.ui_pet(ui);

        // draw the panel viewport (panel.rs)
        if self.panel_open {
            self.show_panel_viewport(ctx);
        }
    }
}
