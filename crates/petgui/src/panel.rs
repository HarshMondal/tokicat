//! Panel viewport: session list + prompt-history scoreboard. Methods on `App`.
//! Ports the `Panel` class from pet.py.

use egui::{Align, Color32, Layout, RichText, Sense};

use petcore::providers::Session;
use petcore::{fmt_cost, fmt_elapsed, fmt_tokens, is_today, today_totals};

use crate::app::{App, PanelView, PANEL_H, PANEL_W};
use crate::theme;

impl App {
    /// Render the panel as an immediate child viewport (shares &mut self).
    // CentralPanel::show(&Context) is the documented pattern for child viewports.
    #[allow(deprecated)]
    pub fn show_panel_viewport(&mut self, ctx: &egui::Context) {
        let vb = egui::ViewportBuilder::default()
            .with_title("cc-pet")
            .with_decorations(false)
            .with_always_on_top()
            .with_transparent(true)
            .with_resizable(false)
            .with_inner_size([PANEL_W, PANEL_H])
            .with_position(self.panel_pos);
        let id = egui::ViewportId::from_hash_of("cc-pet-panel");
        ctx.show_viewport_immediate(id, vb, |ctx, _class| {
            theme::apply(ctx);
            if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.panel_open = false;
            }
            if ctx.input(|i| i.viewport().close_requested()) {
                self.panel_open = false;
            }
            let frame = egui::Frame::default()
                .fill(theme::PANEL_BG)
                .inner_margin(16.0)
                .corner_radius(egui::CornerRadius::same(16));
            egui::CentralPanel::default().frame(frame).show(ctx, |ui| match self.panel_view {
                PanelView::List => self.ui_list(ui),
                PanelView::History => self.ui_history(ui),
                PanelView::Quota => self.ui_quota(ui),
            });
        });
    }

    fn results(&self) -> Vec<Session> {
        let q = self.search.to_lowercase();
        let mut out: Vec<Session> = self
            .sessions
            .iter()
            .filter(|s| {
                if q.is_empty() {
                    return true;
                }
                let hay = format!(
                    "{} {} {} {}",
                    s.title,
                    s.project,
                    s.branch.clone().unwrap_or_default(),
                    s.cwd.clone().unwrap_or_default()
                )
                .to_lowercase();
                hay.contains(&q)
            })
            .filter(|s| !self.today_only || is_today(s))
            .cloned()
            .collect();
        out.sort_by(|a, b| b.mtime.partial_cmp(&a.mtime).unwrap_or(std::cmp::Ordering::Equal));
        out
    }

    // ---- list view ------------------------------------------------------
    fn ui_list(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Claude sessions").color(theme::HDR_TITLE).size(17.0).strong());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button(RichText::new("✕").size(16.0).color(theme::SUB)).clicked() {
                    self.panel_open = false;
                }
                if ui.button(RichText::new("◔ Usage").color(theme::ACCENT_GREEN)).clicked() {
                    self.panel_view = PanelView::Quota;
                    self.refresh_quota();
                }
                if ui.button(RichText::new("+ New").color(theme::BLUE)).clicked() {
                    // open in the most-recent session's folder
                    if let Some(cwd) = self.sessions.iter().find_map(|s| s.cwd.clone()) {
                        self.new_session_action(&cwd);
                    }
                }
                let today_label = RichText::new("Today")
                    .color(if self.today_only { theme::BLUE } else { theme::SUB });
                if ui.button(today_label).clicked() {
                    self.today_only = !self.today_only;
                }
            });
        });

        let resp = ui.add(
            egui::TextEdit::singleline(&mut self.search)
                .hint_text("search title, folder or branch…")
                .desired_width(f32::INFINITY),
        );
        if resp.changed() {
            // searching needs the full pool; refetch once on change
            let limit = if self.search.is_empty() { Some(40) } else { None };
            self.refresh(limit);
        }
        ui.add_space(6.0);

        let results = self.results();
        let max_tok = results.iter().map(|s| s.total_tokens).max().unwrap_or(0);
        let pins = self.state.pinned.clone();

        let pinned: Vec<Session> = results.iter().filter(|s| pins.iter().any(|p| p == &s.path.to_string_lossy())).cloned().collect();
        let mut active: Vec<Session> = results
            .iter()
            .filter(|s| s.open && !pins.iter().any(|p| p == &s.path.to_string_lossy()))
            .cloned()
            .collect();
        active.sort_by_key(|s| if s.working { 0 } else if s.waiting { 1 } else { 2 });
        let rest: Vec<Session> = results
            .iter()
            .filter(|s| !s.open && !pins.iter().any(|p| p == &s.path.to_string_lossy()))
            .cloned()
            .collect();

        let count = results.len();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            if results.is_empty() {
                let msg = if !self.search.is_empty() {
                    "No sessions match."
                } else if self.today_only {
                    "No sessions today."
                } else {
                    "No Claude Code sessions found yet."
                };
                ui.add_space(12.0);
                ui.label(RichText::new(msg).color(theme::SUB).size(14.0));
                return;
            }
            self.section(ui, "PINNED", &pinned, max_tok);
            self.section(ui, "ACTIVE", &active, max_tok);
            self.section(ui, if self.search.is_empty() { "RECENT" } else { "MATCHES" }, &rest, max_tok);
        });

        ui.add_space(6.0);
        let footer = if self.today_only {
            let t = today_totals(&results);
            format!("today: {} prompts · {} tok · {}", t.prompts, fmt_tokens(t.tokens), fmt_cost(Some(t.cost)))
        } else if !self.footer_hint.is_empty() {
            self.footer_hint.clone()
        } else {
            format!("{} sessions · ● working  ● waiting · ⮒ open · ★ pin", count)
        };
        ui.label(RichText::new(footer).color(theme::SUB).size(11.0));
    }

    fn section(&mut self, ui: &mut egui::Ui, label: &str, items: &[Session], max_tok: u64) {
        if items.is_empty() {
            return;
        }
        ui.add_space(6.0);
        ui.label(RichText::new(label).color(theme::SUB).size(10.0).strong());
        for s in items {
            self.session_row(ui, s, max_tok);
        }
    }

    fn session_row(&mut self, ui: &mut egui::Ui, s: &Session, _max_tok: u64) {
        let path = s.path.to_string_lossy().to_string();
        let pinned = self.state.is_pinned(&path);
        egui::Frame::default().inner_margin(egui::Margin::symmetric(2, 4)).show(ui, |ui| {
            ui.horizontal(|ui| {
                let (dot, col) = if s.working {
                    ("●", theme::LIVE_GREEN)
                } else if s.waiting {
                    ("●", theme::DOT_WAIT)
                } else {
                    ("●", theme::DOT_OFF)
                };
                ui.label(RichText::new(dot).color(col).size(13.0));
                let name = ui.add(
                    egui::Label::new(RichText::new(&s.title).color(theme::SESS_NAME).size(14.0).strong())
                        .truncate()
                        .sense(Sense::click()),
                );
                if name.clicked() {
                    self.open_history(&path);
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let star = if pinned { "★" } else { "☆" };
                    let star_col = if pinned { theme::DOT_WAIT } else { theme::DOT_OFF };
                    if ui.button(RichText::new(star).color(star_col)).clicked() {
                        self.state.toggle_pin(&path);
                        self.state.save();
                    }
                    let open_label = if s.open { "⮒ focus" } else { "⮒ open" };
                    if ui.button(RichText::new(open_label).color(theme::ACCENT_GREEN).size(12.0)).clicked() {
                        self.open_action(s);
                    }
                });
            });
            ui.horizontal(|ui| {
                let meta = if let Some(b) = &s.branch {
                    format!("{}  ⎇ {}", s.project, b)
                } else {
                    s.project.clone()
                };
                if s.provider == petcore::Provider::Codex {
                    ui.label(RichText::new(" codex ").size(10.0).color(Color32::from_rgb(16, 18, 24)).background_color(theme::BLUE));
                }
                ui.add(egui::Label::new(RichText::new(meta).color(theme::SUB).size(12.0)).truncate());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if s.provider != petcore::Provider::Codex {
                        ui.label(RichText::new(fmt_cost(Some(s.cost))).color(theme::ACCENT_GREEN).size(11.0));
                    }
                    ui.label(
                        RichText::new(format!("{} · {} tok", s.total_prompts, fmt_tokens(s.total_tokens)))
                            .color(theme::SUB)
                            .size(12.0),
                    );
                });
            });
        });
    }

    fn open_history(&mut self, path: &str) {
        self.clear_attention(path);
        self.show_history(path);
        self.state.last_session = Some(path.to_string());
        self.state.save();
    }

    fn open_action(&mut self, s: &Session) {
        self.clear_attention(&s.path.to_string_lossy());
        let ok = petcore::terminal::open_session(self.term.as_ref(), s);
        let verb = if s.open { "focused" } else { "opened" };
        self.footer_hint = if ok {
            format!("{} “{}” in your terminal…", verb, s.project)
        } else {
            "couldn't reach the terminal — is it running?".to_string()
        };
    }

    fn new_session_action(&mut self, cwd: &str) {
        let ok = petcore::terminal::new_session(self.term.as_ref(), std::path::Path::new(cwd));
        self.footer_hint = if ok {
            format!("started new session in {}", cwd)
        } else {
            "couldn't reach the terminal".to_string()
        };
    }

    // ---- history view ---------------------------------------------------
    fn ui_history(&mut self, ui: &mut egui::Ui) {
        let path = match &self.panel_session {
            Some(p) => p.clone(),
            None => {
                self.panel_view = PanelView::List;
                return;
            }
        };
        let session = petcore::parse_session_any(std::path::Path::new(&path));

        ui.horizontal(|ui| {
            if ui.button(RichText::new("‹ Sessions").color(theme::ACCENT_GREEN)).clicked() {
                self.panel_view = PanelView::List;
            }
            if let Some(s) = &session {
                ui.add(egui::Label::new(RichText::new(&s.title).color(theme::HDR_TITLE).size(17.0).strong()).truncate());
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button(RichText::new("✕").size(16.0).color(theme::SUB)).clicked() {
                    self.panel_open = false;
                }
            });
        });
        ui.add_space(6.0);

        let Some(s) = session else {
            ui.label(RichText::new("No prompts in this session yet.").color(theme::SUB));
            return;
        };
        if s.prompts.is_empty() {
            ui.label(RichText::new("No prompts in this session yet.").color(theme::SUB));
            return;
        }
        let max_out = s.prompts.iter().map(|p| p.out_tokens).max().unwrap_or(0);

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for p in &s.prompts {
                let full = if p.full_text.is_empty() { p.title.clone() } else { p.full_text.clone() };
                let expandable = full.contains('\n') || full.chars().count() > 90;
                let expanded = expandable && self.expanded.contains(&p.index);

                let frame = egui::Frame::default().inner_margin(egui::Margin::symmetric(6, 6));
                frame.show(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        let caret = if expandable { if expanded { "▾" } else { "▸" } } else { " " };
                        ui.label(RichText::new(caret).color(theme::DOT_OFF).monospace());
                        ui.label(RichText::new(format!("{:>2}", p.index)).color(theme::DOT_OFF).monospace());

                        ui.vertical(|ui| {
                            ui.set_width(ui.available_width() - 96.0);
                            let text = if expanded { full.clone() } else { p.title.clone() };
                            let col = if p.running { theme::HDR_TITLE } else { theme::PTITLE };
                            let label = ui.add(
                                egui::Label::new(RichText::new(text).color(col).size(14.0))
                                    .wrap()
                                    .sense(Sense::click()),
                            );
                            if expandable && label.clicked() {
                                if expanded {
                                    self.expanded.remove(&p.index);
                                } else {
                                    self.expanded.insert(p.index);
                                }
                            }
                        });

                        ui.with_layout(Layout::top_down(Align::RIGHT), |ui| {
                            if p.running {
                                ui.label(
                                    RichText::new(" running… ")
                                        .background_color(theme::LIVE_GREEN)
                                        .color(Color32::from_rgb(16, 18, 24))
                                        .size(12.0)
                                        .strong(),
                                );
                            } else {
                                let c = theme::heat_color(p.out_tokens, max_out);
                                ui.label(RichText::new(format!("{} tok", fmt_tokens(p.out_tokens))).color(c).monospace().size(13.0));
                                ui.label(RichText::new(fmt_elapsed(p.elapsed)).color(theme::SUB).monospace().size(12.0));
                            }
                        });
                    });
                });
            }
        });

        ui.add_space(6.0);
        egui::Frame::default()
            .fill(Color32::from_rgba_unmultiplied(255, 255, 255, 12))
            .inner_margin(egui::Margin::symmetric(14, 9))
            .corner_radius(egui::CornerRadius::same(10))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let cells = [
                        (s.total_prompts.to_string(), "prompts"),
                        (fmt_tokens(s.total_tokens), "output tokens"),
                        (fmt_elapsed(s.wall_seconds), "wall-clock"),
                    ];
                    for (num, lbl) in cells {
                        ui.vertical(|ui| {
                            ui.label(RichText::new(num).color(theme::HDR_TITLE).size(16.0).strong());
                            ui.label(RichText::new(lbl).color(theme::SUB).size(11.0));
                        });
                        ui.add_space(18.0);
                    }
                });
            });
    }

    // ---- quota / usage view ---------------------------------------------
    fn ui_quota(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button(RichText::new("‹ Sessions").color(theme::ACCENT_GREEN)).clicked() {
                self.panel_view = PanelView::List;
            }
            ui.label(RichText::new("Usage limits").color(theme::HDR_TITLE).size(17.0).strong());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button(RichText::new("✕").size(16.0).color(theme::SUB)).clicked() {
                    self.panel_open = false;
                }
                if ui.button(RichText::new("⟳").color(theme::SUB)).clicked() {
                    self.refresh_quota();
                }
            });
        });
        ui.add_space(8.0);

        if let Some(note) = self.quota.note.clone() {
            ui.label(RichText::new(note).color(theme::DOT_WAIT).size(12.0));
            ui.add_space(6.0);
        }
        if self.quota.windows.is_empty() {
            ui.label(RichText::new("Fetching usage…").color(theme::SUB).size(13.0));
            return;
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        let windows = self.quota.windows.clone();
        let mut last_provider: Option<petcore::Provider> = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for w in &windows {
                if last_provider != Some(w.provider) {
                    ui.add_space(8.0);
                    ui.label(RichText::new(w.provider.label().to_uppercase()).color(theme::SUB).size(10.0).strong());
                    last_provider = Some(w.provider);
                }
                self.quota_bar(ui, w, now);
            }
        });
    }

    fn quota_bar(&self, ui: &mut egui::Ui, w: &petcore::quota::QuotaWindow, now: i64) {
        let pct = (w.used_percent / 100.0).clamp(0.0, 1.0);
        let color = if w.used_percent >= 90.0 {
            theme::TOK_HEAVY
        } else if w.used_percent >= 70.0 {
            theme::TOK_MID
        } else {
            theme::LIVE_GREEN
        };
        ui.horizontal(|ui| {
            ui.label(RichText::new(&w.label).color(theme::SESS_NAME).size(13.0));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let resets = match w.resets_at {
                    Some(r) if r > now => format!("resets in {}", fmt_elapsed(Some((r - now) as f64))),
                    Some(_) => "resets soon".to_string(),
                    None => String::new(),
                };
                ui.label(RichText::new(resets).color(theme::SUB).size(11.0));
                ui.add_space(8.0);
                ui.label(RichText::new(format!("{:.0}%", w.used_percent)).color(color).size(13.0).strong());
            });
        });
        // bar track + fill
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 8.0), Sense::hover());
        let painter = ui.painter();
        painter.rect_filled(rect, 4.0, Color32::from_rgba_unmultiplied(255, 255, 255, 20));
        let mut fill = rect;
        fill.set_width(rect.width() * pct);
        painter.rect_filled(fill, 4.0, color);
        ui.add_space(8.0);
    }
}
