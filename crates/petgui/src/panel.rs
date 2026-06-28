//! Panel viewport: session list + prompt-history scoreboard. Methods on `App`.
//! Ports the `Panel` class from pet.py.

use egui::{Align, Color32, CornerRadius, Layout, Rect, RichText, Sense, Stroke};

use petcore::providers::Session;
use petcore::{fmt_ago, fmt_cost, fmt_elapsed, fmt_tokens, is_this_week, is_today, today_totals, Provider};

use crate::app::{App, SummaryPeriod, Tab, PANEL_MIN_H, PANEL_MIN_W};
use crate::theme;

use std::collections::HashMap;

use std::sync::OnceLock;

/// Panel UI scale (from config `[ui] scale`). Set once at startup; multiplies font
/// sizes so the whole panel can be made larger without touching the pet window's
/// geometry (egui's global zoom would skew the pet's screen-positioning math).
static UI_SCALE: OnceLock<f32> = OnceLock::new();

pub fn set_ui_scale(scale: f32) {
    let _ = UI_SCALE.set(scale.clamp(0.8, 2.0));
}

/// Scale a base font size by the configured UI scale.
fn sz(x: f32) -> f32 {
    x * UI_SCALE.get().copied().unwrap_or(1.0)
}

#[derive(Clone, Copy)]
enum IconKind {
    Close,
    Refresh,
    Back,
}

/// A compact icon button with a subtle rounded hover background. The glyph is drawn
/// as vector shapes (not a font character) so it renders identically everywhere —
/// the bundled font lacks ✕/⟳, which otherwise show as tofu boxes.
fn icon_button(ui: &mut egui::Ui, kind: IconKind) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(28.0, 28.0), Sense::click());
    let hovered = resp.hovered();
    if hovered {
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(8),
            Color32::from_rgba_unmultiplied(255, 255, 255, 22),
        );
    }
    let col = if hovered { theme::HDR_TITLE } else { theme::SUB };
    let p = ui.painter();
    let c = rect.center();
    match kind {
        IconKind::Close => {
            let r = 5.5;
            let s = Stroke::new(1.7, col);
            p.line_segment([c + egui::vec2(-r, -r), c + egui::vec2(r, r)], s);
            p.line_segment([c + egui::vec2(-r, r), c + egui::vec2(r, -r)], s);
        }
        IconKind::Back => {
            // left-pointing chevron ‹
            let r = 4.6;
            let s = Stroke::new(1.9, col);
            p.line_segment([c + egui::vec2(r * 0.55, -r), c + egui::vec2(-r * 0.55, 0.0)], s);
            p.line_segment([c + egui::vec2(-r * 0.55, 0.0), c + egui::vec2(r * 0.55, r)], s);
        }
        IconKind::Refresh => {
            let r = 6.0;
            let s = Stroke::new(1.7, col);
            // ~300° arc leaving a gap, with an arrowhead at the leading end
            let a0 = 0.6_f32;
            let a1 = a0 + std::f32::consts::PI * 1.7;
            let n = 22;
            let pts: Vec<egui::Pos2> = (0..=n)
                .map(|i| {
                    let a = a0 + (a1 - a0) * i as f32 / n as f32;
                    c + egui::vec2(a.cos(), a.sin()) * r
                })
                .collect();
            let end = *pts.last().unwrap();
            p.add(egui::Shape::line(pts, s));
            // arrowhead: back along the tangent + spread along the radial normal
            let tangent = egui::vec2(-a1.sin(), a1.cos());
            let normal = egui::vec2(a1.cos(), a1.sin());
            let h = 4.0;
            p.add(egui::Shape::convex_polygon(
                vec![
                    end,
                    end - tangent * h + normal * (h * 0.55),
                    end - tangent * h - normal * (h * 0.55),
                ],
                col,
                Stroke::NONE,
            ));
        }
    }
    resp
}

/// Vector "go/open" arrow (replaces the ⮒ glyph, which the bundled font lacks).
fn draw_arrow_icon(p: &egui::Painter, rect: Rect, col: Color32) {
    let s = Stroke::new(1.6, col);
    let y = rect.center().y;
    p.line_segment([egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)], s);
    let a = rect.height() * 0.28;
    p.line_segment([egui::pos2(rect.right(), y), egui::pos2(rect.right() - a, y - a)], s);
    p.line_segment([egui::pos2(rect.right(), y), egui::pos2(rect.right() - a, y + a)], s);
}

/// Vector git-branch glyph (replaces ⎇): a trunk with two nodes and a branch node.
fn draw_branch_icon(p: &egui::Painter, rect: Rect, col: Color32) {
    let s = Stroke::new(1.4, col);
    let r = rect.width().min(rect.height()) * 0.13;
    let xl = rect.left() + rect.width() * 0.30;
    let xr = rect.left() + rect.width() * 0.72;
    let top = rect.top() + rect.height() * 0.22;
    let bot = rect.bottom() - rect.height() * 0.22;
    let mid = rect.center().y;
    p.line_segment([egui::pos2(xl, top + r), egui::pos2(xl, bot - r)], s); // trunk
    p.circle_stroke(egui::pos2(xl, top), r, s);
    p.circle_stroke(egui::pos2(xl, bot), r, s);
    p.circle_stroke(egui::pos2(xr, top), r, s); // branch node
    p.line_segment([egui::pos2(xl, mid), egui::pos2(xr, top + r)], s); // branch off the trunk
}

/// Vector folder/repo glyph for the history header's repo label.
fn draw_folder_icon(p: &egui::Painter, rect: Rect, col: Color32) {
    let s = Stroke::new(1.4, col);
    let w = rect.width();
    let h = rect.height();
    let x0 = rect.left() + w * 0.12;
    let x1 = rect.right() - w * 0.12;
    let yt = rect.top() + h * 0.30;
    let yb = rect.bottom() - h * 0.20;
    // tab
    let tabx = x0 + w * 0.34;
    p.line_segment([egui::pos2(x0, yt), egui::pos2(tabx, yt)], s);
    p.line_segment([egui::pos2(tabx, yt), egui::pos2(tabx + w * 0.12, yt - h * 0.16)], s);
    p.line_segment([egui::pos2(tabx + w * 0.12, yt - h * 0.16), egui::pos2(x1, yt - h * 0.16)], s);
    // body (open rectangle)
    p.line_segment([egui::pos2(x0, yt), egui::pos2(x0, yb)], s);
    p.line_segment([egui::pos2(x0, yb), egui::pos2(x1, yb)], s);
    p.line_segment([egui::pos2(x1, yb), egui::pos2(x1, yt - h * 0.16)], s);
}

/// Vector clock glyph for the history header's recency label.
fn draw_clock_icon(p: &egui::Painter, rect: Rect, col: Color32) {
    let s = Stroke::new(1.4, col);
    let c = rect.center();
    let r = rect.width().min(rect.height()) * 0.40;
    p.circle_stroke(c, r, s);
    p.line_segment([c, c + egui::vec2(0.0, -r * 0.62)], s); // minute hand
    p.line_segment([c, c + egui::vec2(r * 0.5, 0.0)], s); // hour hand
}

/// A small rounded chip carrying a model name, tinted with the provider's accent.
fn model_chip(ui: &mut egui::Ui, label: &str, accent: Color32) {
    let galley = ui.painter().layout_no_wrap(label.to_owned(), egui::FontId::proportional(sz(11.0)), accent);
    let padx = sz(7.0);
    let w = galley.size().x + padx * 2.0;
    let h = sz(17.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, h), Sense::hover());
    let fill = Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 28);
    ui.painter().rect_filled(rect, CornerRadius::same(6), fill);
    ui.painter().galley(egui::pos2(rect.left() + padx, rect.center().y - galley.size().y / 2.0), galley, accent);
}

/// The model that handled a specific prompt, as a short label. Falls back to the
/// session's model when the per-prompt model wasn't recorded (Codex/opencode).
fn prompt_model_label(s: &Session, p: &petcore::Prompt) -> Option<String> {
    match (&p.model, s.provider) {
        (Some(m), Provider::Claude) => Some(petcore::pricing::model_family(Some(m))),
        (Some(m), _) => Some(m.clone()),
        (None, _) => model_label(s),
    }
}

/// Short model label for a row: Claude → family ("sonnet"); others → the model id
/// ("glm-5.1", "MiniMax-M2.5"), falling back to the family.
fn model_label(s: &Session) -> Option<String> {
    let family = s.model_family.trim();
    match s.provider {
        Provider::Claude => (!family.is_empty()).then(|| family.to_string()),
        _ => s
            .model
            .clone()
            .filter(|m| !m.trim().is_empty())
            .or_else(|| (!family.is_empty()).then(|| family.to_string())),
    }
}

/// The disambiguating provider chip for a row (only GLM/MiniMax, which share the
/// opencode tab; each other provider already has its own tab).
fn provider_chip(s: &Session) -> Option<(&'static str, Color32)> {
    match s.provider {
        Provider::Zai => Some(("glm", theme::ZAI_ACCENT)),
        Provider::Minimax => Some(("minimax", theme::MINIMAX_ACCENT)),
        _ => None,
    }
}

/// Minimal status dot: green when the agent is actively working, grey when idle.
/// (The "just finished, look at it" nudge lives on the pet's attention badge.)
fn status_widget(ui: &mut egui::Ui, s: &Session) {
    let d = sz(10.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(d, d), Sense::hover());
    let col = if s.working { theme::LIVE_GREEN } else { theme::DOT_OFF };
    ui.painter().circle_filled(rect.center(), d * 0.30, col);
}

/// A muted "·" separator between metadata fields.
fn dot_sep(ui: &mut egui::Ui) {
    ui.label(RichText::new("·").color(theme::DOT_OFF).size(sz(12.0)));
}

/// One big-number-over-label cell in the summary stats header.
fn stat_cell(ui: &mut egui::Ui, value: &str, label: &str, color: Color32) {
    ui.vertical(|ui| {
        ui.label(RichText::new(value).color(color).size(sz(18.0)).strong());
        ui.label(RichText::new(label).color(theme::SUB).size(sz(11.0)));
    });
}

/// A section header (label + count + hairline rule) for the summary lists. Mirrors
/// the `App::section` header but works for non-`Session` rows.
fn summary_header(ui: &mut egui::Ui, label: &str, count: usize, accent: Color32) {
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).color(accent).size(sz(11.0)).strong());
        ui.add_space(6.0);
        ui.label(RichText::new(count.to_string()).color(theme::SUB).size(sz(11.0)));
        ui.add_space(8.0);
        let (r, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), Sense::hover());
        let y = r.center().y;
        ui.painter().line_segment(
            [egui::pos2(r.left(), y), egui::pos2(r.right(), y)],
            Stroke::new(1.0, theme::DOT_OFF),
        );
    });
    ui.add_space(4.0);
}

/// Vector magnifier icon for the search bar.
fn draw_search_icon(p: &egui::Painter, rect: Rect, col: Color32) {
    let s = Stroke::new(1.6, col);
    let r = rect.width() * 0.30;
    let c = rect.left_top() + egui::vec2(rect.width() * 0.40, rect.height() * 0.40);
    p.circle_stroke(c, r, s);
    let d = std::f32::consts::FRAC_1_SQRT_2;
    let p0 = c + egui::vec2(d, d) * r;
    let p1 = c + egui::vec2(d, d) * (r + rect.width() * 0.32);
    p.line_segment([p0, p1], s);
}

/// Pin star, drawn into a reserved rect (click handled by the card).
fn draw_star(p: &egui::Painter, rect: Rect, pinned: bool, hover: bool) {
    let col = if pinned {
        theme::DOT_WAIT
    } else if hover {
        theme::SUB
    } else {
        theme::DOT_OFF
    };
    let glyph = if pinned { "★" } else { "☆" };
    p.text(rect.center(), egui::Align2::CENTER_CENTER, glyph, egui::FontId::proportional(sz(15.0)), col);
}

/// A green "→ Focus" pill button with a hover highlight (used in the Needs popup).
fn focus_pill(ui: &mut egui::Ui) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(
        "Focus".to_owned(),
        egui::FontId::proportional(sz(12.0)),
        theme::ACCENT_GREEN,
    );
    let icon = sz(12.0);
    let w = sz(9.0) + icon + sz(5.0) + galley.size().x + sz(9.0);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, sz(24.0)), Sense::click());
    let bg = Color32::from_rgba_unmultiplied(154, 230, 180, if resp.hovered() { 58 } else { 30 });
    ui.painter().rect_filled(rect, CornerRadius::same(7), bg);
    let ic = Rect::from_min_size(
        egui::pos2(rect.left() + sz(9.0), rect.center().y - icon / 2.0),
        egui::vec2(icon, icon),
    );
    draw_arrow_icon(ui.painter(), ic, theme::ACCENT_GREEN);
    ui.painter().galley(
        egui::pos2(ic.right() + sz(5.0), rect.center().y - galley.size().y / 2.0),
        galley,
        theme::ACCENT_GREEN,
    );
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

/// A small expand chevron drawn as two strokes: points right (`›`) when collapsed,
/// down (`⌄`) when open. Vector-drawn so it never renders as font tofu.
fn draw_chevron(p: &egui::Painter, rect: Rect, col: Color32, open: bool) {
    let c = rect.center();
    let r = sz(3.6);
    let s = Stroke::new(sz(1.5), col);
    if open {
        p.line_segment([c + egui::vec2(-r, -r * 0.5), c + egui::vec2(0.0, r * 0.5)], s);
        p.line_segment([c + egui::vec2(0.0, r * 0.5), c + egui::vec2(r, -r * 0.5)], s);
    } else {
        p.line_segment([c + egui::vec2(-r * 0.5, -r), c + egui::vec2(r * 0.5, 0.0)], s);
        p.line_segment([c + egui::vec2(r * 0.5, 0.0), c + egui::vec2(-r * 0.5, r)], s);
    }
}

/// A subtle round step-indicator chip showing a prompt's index in the history list.
/// Replaces the old monospace `{:>2}` gutter. Returns the allocated width.
fn index_chip(ui: &mut egui::Ui, idx: usize, accent: bool) -> f32 {
    let d = sz(22.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(d, d), Sense::hover());
    let fill = Color32::from_rgba_unmultiplied(255, 255, 255, 14);
    ui.painter().circle_filled(rect.center(), d * 0.5, fill);
    let col = if accent { theme::HDR_TITLE } else { theme::SUB };
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        idx.to_string(),
        egui::FontId::proportional(sz(11.0)),
        col,
    );
    d
}

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
            .with_resizable(false) // we drive size ourselves via the corner grip
            .with_inner_size([self.panel_w, self.panel_h])
            .with_position(self.panel_pos);
        let id = egui::ViewportId::from_hash_of("cc-pet-panel");
        ctx.show_viewport_immediate(id, vb, |ctx, _class| {
            theme::apply(ctx);
            // Keep the panel above other windows too (same cadence as the pet).
            if self.reassert_top {
                ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(
                    egui::WindowLevel::AlwaysOnTop,
                ));
            }
            // Follow the pet: nudge the live panel window to its docked position. (The
            // builder's with_position only reliably applies at creation, so move it
            // explicitly each time the docked position changes.)
            if (self.panel_pos.x - self.last_panel_pushed.x).abs() > 0.5
                || (self.panel_pos.y - self.last_panel_pushed.y).abs() > 0.5
            {
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(self.panel_pos));
                self.last_panel_pushed = self.panel_pos;
            }
            // Shutdown chord while the panel is focused.
            if App::quit_chord(ctx) {
                self.quit_confirm = true;
            }
            // While the quit modal is up it takes Esc (cancel); otherwise Esc closes
            // the panel as before.
            if !self.quit_confirm && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.panel_open = false;
            }
            if ctx.input(|i| i.viewport().close_requested()) {
                self.panel_open = false;
            }
            let frame = egui::Frame::default()
                .fill(theme::PANEL_BG)
                .inner_margin(16.0)
                .corner_radius(CornerRadius::same(16));
            egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
                self.tab_bar(ui);
                ui.add_space(10.0);
                match self.tab {
                    Tab::Usage => self.ui_quota(ui),
                    Tab::Claude => self.ui_provider_tab(ui, Provider::Claude),
                    Tab::Codex => self.ui_provider_tab(ui, Provider::Codex),
                    Tab::Opencode => self.ui_opencode_tab(ui),
                    Tab::Summary => self.ui_summary(ui),
                }
                self.resize_grip(ui, ctx);
            });
            self.quit_modal(ctx);
        });
    }

    /// Top tab strip (Usage · Claude · Codex · opencode) with right-aligned
    /// refresh/close icon buttons. The active tab is underlined in its brand accent.
    fn tab_bar(&mut self, ui: &mut egui::Ui) {
        let tabs = [
            (Tab::Usage, "Usage", theme::ACCENT_GREEN),
            (Tab::Claude, "Claude", theme::CLAUDE_ACCENT),
            (Tab::Codex, "Codex", theme::CODEX_ACCENT),
            (Tab::Opencode, "opencode", theme::OPENCODE_ACCENT),
            (Tab::Summary, "Summary", theme::BLUE),
        ];
        ui.horizontal(|ui| {
            for (tab, label, accent) in tabs {
                let active = self.tab == tab;
                let col = if active { accent } else { theme::SUB };
                let resp = ui.add(
                    egui::Label::new(RichText::new(label).color(col).size(sz(15.0)).strong())
                        .sense(Sense::click()),
                );
                if active {
                    let r = resp.rect;
                    ui.painter().hline(
                        r.left()..=r.right(),
                        r.bottom() + 3.0,
                        Stroke::new(2.0, accent),
                    );
                }
                if resp.clicked() && !active {
                    self.switch_tab(tab);
                }
                ui.add_space(16.0);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if icon_button(ui, IconKind::Close).clicked() {
                    self.panel_open = false;
                }
                if icon_button(ui, IconKind::Refresh).clicked() {
                    self.refresh_all();
                }
            });
        });
    }

    /// Bottom-right drag handle that resizes the (decorationless) panel window.
    fn resize_grip(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let area = ui.max_rect();
        let g = 16.0;
        let grip = Rect::from_min_max(area.max - egui::vec2(g, g), area.max);
        let resp = ui.interact(grip, ui.id().with("resize-grip"), Sense::drag());
        // diagonal hatch lines, brighter on hover
        let col = if resp.hovered() || resp.dragged() { theme::SUB } else { theme::DOT_OFF };
        for off in [3.0_f32, 7.0, 11.0] {
            ui.painter().line_segment(
                [egui::pos2(grip.max.x - off, grip.max.y), egui::pos2(grip.max.x, grip.max.y - off)],
                Stroke::new(1.5, col),
            );
        }
        if resp.hovered() || resp.dragged() {
            ctx.set_cursor_icon(egui::CursorIcon::ResizeNwSe);
        }
        if resp.dragged() {
            let d = resp.drag_delta();
            let (max_w, max_h) = self.panel_max_size(ctx);
            self.panel_w = (self.panel_w + d.x).clamp(PANEL_MIN_W, max_w);
            self.panel_h = (self.panel_h + d.y).clamp(PANEL_MIN_H, max_h);
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                self.panel_w,
                self.panel_h,
            )));
        }
        if resp.drag_stopped() {
            self.state.panel_w = Some(self.panel_w);
            self.state.panel_h = Some(self.panel_h);
            self.state.save();
        }
    }

    /// Largest panel size that still fits on the panel's current monitor.
    fn panel_max_size(&self, ctx: &egui::Context) -> (f32, f32) {
        let mons = crate::monitors::all(ctx);
        let c = self.panel_pos;
        let m = crate::monitors::monitor_at(&mons, c.x + self.panel_w * 0.5, c.y + self.panel_h * 0.5);
        ((m.w - 16.0).max(PANEL_MIN_W), (m.h - 16.0).max(PANEL_MIN_H))
    }

    /// Wall-clock seconds, for "finished N ago" stamps.
    fn now_secs() -> f64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0)
    }

    // ---- "Needs you" popup: a small dialog beside the cat ------------------
    #[allow(deprecated)]
    pub fn show_needs_viewport(&mut self, ctx: &egui::Context) {
        let waiting = self.waiting_sessions();
        if waiting.is_empty() {
            self.needs_open = false;
            return;
        }
        let n = waiting.len();
        let shown = n.min(5) as f32;
        let bw = sz(320.0);
        let bh = sz(60.0) + shown * sz(58.0);
        let cx = self.pos.x + self.size * 0.5;
        let by = if self.pos.y < bh + 20.0 { self.pos.y + self.size + 8.0 } else { self.pos.y - bh - 8.0 };
        let bx = cx - bw * 0.5;

        let vb = egui::ViewportBuilder::default()
            .with_title("cc-pet")
            .with_decorations(false)
            .with_always_on_top()
            .with_transparent(true)
            .with_resizable(false)
            .with_inner_size([bw, bh])
            .with_position([bx, by]);
        let id = egui::ViewportId::from_hash_of("cc-pet-needs");
        let now = Self::now_secs();
        ctx.show_viewport_immediate(id, vb, |ctx, _class| {
            theme::apply(ctx);
            if self.reassert_top {
                ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(egui::WindowLevel::AlwaysOnTop));
            }
            if ctx.input(|i| i.key_pressed(egui::Key::Escape)) || ctx.input(|i| i.viewport().close_requested()) {
                self.needs_open = false;
            }
            let frame = egui::Frame::default()
                .fill(theme::PANEL_BG)
                .inner_margin(egui::Margin::symmetric(14, 12))
                .corner_radius(CornerRadius::same(14));
            egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
                // header: ● Needs you  N ............................. ✕
                ui.horizontal(|ui| {
                    let (dr, _) = ui.allocate_exact_size(egui::vec2(sz(12.0), sz(12.0)), Sense::hover());
                    ui.painter().circle_filled(dr.center(), sz(12.0) * 0.32, theme::ATTENTION);
                    ui.add_space(7.0);
                    ui.label(RichText::new("Needs you").color(theme::HDR_TITLE).size(sz(15.0)).strong());
                    ui.add_space(6.0);
                    ui.label(RichText::new(n.to_string()).color(theme::SUB).size(sz(13.0)));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if icon_button(ui, IconKind::Close).clicked() {
                            self.needs_open = false;
                        }
                    });
                });
                ui.add_space(8.0);
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    for s in &waiting {
                        egui::Frame::default()
                            .fill(theme::CARD_BG)
                            .inner_margin(egui::Margin::symmetric(10, 8))
                            .corner_radius(CornerRadius::same(8))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                // right-aligned Focus first so the title truncates before it
                                ui.horizontal(|ui| {
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        if focus_pill(ui).clicked() {
                                            self.focus_waiting(s);
                                        }
                                        ui.add_space(8.0);
                                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                            let (dr, _) = ui.allocate_exact_size(egui::vec2(sz(10.0), sz(10.0)), Sense::hover());
                                            ui.painter().circle_filled(dr.center(), sz(10.0) * 0.30, theme::DOT_WAIT);
                                            ui.add_space(6.0);
                                            ui.vertical(|ui| {
                                                ui.add(egui::Label::new(RichText::new(&s.title).color(theme::SESS_NAME).size(sz(13.5)).strong()).truncate());
                                                let ago = fmt_elapsed(Some((now - s.mtime).max(0.0)));
                                                ui.add(egui::Label::new(
                                                    RichText::new(format!("{} · {} · {ago} ago", s.provider.display_name(), s.project))
                                                        .color(theme::SUB)
                                                        .size(sz(11.0)),
                                                ).truncate());
                                            });
                                        });
                                    });
                                });
                            });
                        ui.add_space(5.0);
                    }
                });
            });
        });
    }

    // ---- hover bubble: glanceable context next to the pet ---------------
    #[allow(deprecated)]
    pub fn show_bubble_viewport(&mut self, ctx: &egui::Context) {
        let waiting = self.waiting_sessions();
        if waiting.is_empty() {
            return;
        }
        let shown = waiting.len().min(3);
        let bw = 250.0;
        let bh = 14.0 + shown as f32 * 34.0 + if waiting.len() > 3 { 16.0 } else { 0.0 };
        let cx = self.pos.x + self.size * 0.5;
        // above the pet, or below if it'd clip the top of the screen
        let by = if self.pos.y < bh + 20.0 { self.pos.y + self.size + 8.0 } else { self.pos.y - bh - 8.0 };
        let bx = cx - bw * 0.5;

        let vb = egui::ViewportBuilder::default()
            .with_title("cc-pet")
            .with_decorations(false)
            .with_always_on_top()
            .with_transparent(true)
            .with_resizable(false)
            .with_inner_size([bw, bh])
            .with_position([bx, by]);
        let id = egui::ViewportId::from_hash_of("cc-pet-bubble");
        let now = Self::now_secs();
        ctx.show_viewport_immediate(id, vb, |ctx, _class| {
            theme::apply(ctx);
            if self.reassert_top {
                ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(egui::WindowLevel::AlwaysOnTop));
            }
            let frame = egui::Frame::default()
                .fill(theme::PANEL_BG)
                .inner_margin(10.0)
                .corner_radius(CornerRadius::same(12));
            egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
                for s in waiting.iter().take(3) {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("●").color(theme::DOT_WAIT).size(sz(11.0)));
                        ui.vertical(|ui| {
                            ui.add(egui::Label::new(RichText::new(&s.title).color(theme::SESS_NAME).size(sz(13.0)).strong()).truncate());
                            let ago = fmt_elapsed(Some((now - s.mtime).max(0.0)));
                            ui.label(
                                RichText::new(format!("{} · {} · finished {ago} ago", s.provider.display_name(), s.project))
                                    .color(theme::SUB)
                                    .size(sz(10.0)),
                            );
                        });
                    });
                }
                if waiting.len() > 3 {
                    ui.label(RichText::new(format!("+{} more waiting", waiting.len() - 3)).color(theme::SUB).size(sz(10.0)));
                }
            });
        });
    }

    /// Centered "Quit cc-pet?" confirmation, drawn on top of the panel. Triggered by
    /// the Ctrl+Shift+U shortcut. Buttons work regardless of keyboard focus; Enter
    /// confirms and Esc cancels when the panel holds focus.
    fn quit_modal(&mut self, ctx: &egui::Context) {
        if !self.quit_confirm {
            return;
        }
        let mut cancel = false;
        let mut quit = false;
        egui::Window::new("quit-confirm")
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(
                egui::Frame::popup(&ctx.global_style())
                    .fill(theme::PANEL_BG)
                    .inner_margin(18.0)
                    .corner_radius(egui::CornerRadius::same(14)),
            )
            .show(ctx, |ui| {
                ui.set_max_width(260.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("Quit cc-pet?").size(sz(16.0)).strong());
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new("Usage tracking isn't lost — it resumes from\nyour session files next time you launch.")
                            .size(sz(11.0))
                            .color(theme::SUB),
                    );
                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        if ui.button(RichText::new("Cancel").size(sz(13.0))).clicked() {
                            cancel = true;
                        }
                        if ui
                            .button(RichText::new("⏻ Quit").size(sz(13.0)).color(Color32::WHITE))
                            .clicked()
                        {
                            quit = true;
                        }
                    });
                });
            });
        if ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
            quit = true;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            cancel = true;
        }
        if cancel {
            self.quit_confirm = false;
        }
        if quit {
            self.state.save();
            std::process::exit(0);
        }
    }

    /// Apply the search + Today filters to an (already provider-selected) slice.
    fn filtered(&self, src: &[Session]) -> Vec<Session> {
        let q = self.search.to_lowercase();
        let mut out: Vec<Session> = src
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

    /// Prominent full-width search bar (magnifier icon + frameless field in a rounded
    /// pill). Shared by every session tab. Refetches the full pool on change.
    fn search_bar(&mut self, ui: &mut egui::Ui) {
        egui::Frame::default()
            .fill(Color32::from_rgba_unmultiplied(255, 255, 255, 16))
            .inner_margin(egui::Margin::symmetric(10, 7))
            .corner_radius(CornerRadius::same(9))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let (ir, _) = ui.allocate_exact_size(egui::vec2(sz(16.0), sz(16.0)), Sense::hover());
                    draw_search_icon(ui.painter(), ir, theme::SUB);
                    ui.add_space(7.0);
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut self.search)
                            .hint_text(RichText::new("Search sessions…").color(theme::SUB).size(sz(14.0)))
                            .frame(egui::Frame::default())
                            .font(egui::FontId::proportional(sz(14.0)))
                            .desired_width(ui.available_width()),
                    );
                    if resp.changed() {
                        let limit = self.list_limit();
                        self.refresh(limit);
                    }
                });
            });
    }

    // ---- Claude / Codex tab: list -> history ----------------------------
    fn ui_provider_tab(&mut self, ui: &mut egui::Ui, provider: Provider) {
        if self.panel_session.is_some() {
            self.ui_history(ui);
            return;
        }
        self.search_bar(ui);
        ui.add_space(7.0);
        ui.horizontal(|ui| {
            if self.chip(ui, "Today", self.today_only, theme::BLUE).clicked() {
                self.today_only = !self.today_only;
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.add(egui::Button::new(RichText::new("+ New").color(theme::BLUE).size(sz(13.0))).frame(false)).clicked() {
                    let cwd = self
                        .sessions
                        .iter()
                        .find(|s| s.provider == provider)
                        .and_then(|s| s.cwd.clone())
                        .or_else(|| self.sessions.iter().find_map(|s| s.cwd.clone()));
                    if let Some(cwd) = cwd {
                        self.new_session_action(&cwd, provider);
                    }
                }
            });
        });
        ui.add_space(7.0);

        let src: Vec<Session> = self.sessions.iter().filter(|s| s.provider == provider).cloned().collect();
        let results = self.filtered(&src);
        let empty = if !self.search.is_empty() {
            "No sessions match."
        } else if self.today_only {
            "No sessions today."
        } else {
            "No sessions found yet."
        };
        self.session_list(ui, &results, true, empty);
    }

    // ---- opencode tab: GLM/MiniMax chips + list -> history --------------
    fn ui_opencode_tab(&mut self, ui: &mut egui::Ui) {
        if self.panel_session.is_some() {
            self.ui_history(ui);
            return;
        }
        self.search_bar(ui);
        ui.add_space(7.0);
        ui.horizontal(|ui| {
            let chips = [
                (None, "All"),
                (Some(Provider::Zai), "GLM"),
                (Some(Provider::Minimax), "MiniMax"),
            ];
            for (filt, label) in chips {
                let active = self.oc_filter == filt;
                let accent = match filt {
                    Some(p) => theme::provider_accent(p),
                    None => theme::ACCENT_GREEN,
                };
                if self.chip(ui, label, active, accent).clicked() {
                    self.oc_filter = filt;
                }
                ui.add_space(6.0);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.add(egui::Button::new(RichText::new("+ New").color(theme::BLUE).size(sz(13.0))).frame(false)).clicked() {
                    let cwd = self
                        .oc_sessions
                        .iter()
                        .find_map(|s| s.cwd.clone())
                        .or_else(|| self.sessions.iter().find_map(|s| s.cwd.clone()))
                        .unwrap_or_else(|| ".".to_string());
                    self.new_session_action(&cwd, Provider::Minimax);
                }
            });
        });
        ui.add_space(7.0);

        let src: Vec<Session> = self
            .oc_sessions
            .iter()
            .filter(|s| self.oc_filter.map(|p| s.provider == p).unwrap_or(true))
            .cloned()
            .collect();
        let results = self.filtered(&src);
        let empty = if self.oc_sessions.is_empty() {
            "No opencode (GLM/MiniMax) sessions found."
        } else {
            "No sessions match."
        };
        // terminal=true → opencode rows get an "open" button that resumes via
        // `opencode --session <id>` (no live status, so the status dot stays grey).
        self.session_list(ui, &results, true, empty);
    }

    // ---- summary tab: model usage + costly sessions across providers ----
    fn ui_summary(&mut self, ui: &mut egui::Ui) {
        // Snapshot the session pools up front so the card click handler can take
        // `&mut self` later without colliding with these borrows.
        let sessions_snap = self.sessions.clone();
        let oc_sessions_snap = self.oc_sessions.clone();
        let period = self.summary_period;
        let prov_filter = self.summary_provider;

        // Period toggle row (Today / This Week).
        ui.horizontal(|ui| {
            if self.chip(ui, "Today", period == SummaryPeriod::Today, theme::DOT_WAIT).clicked() {
                self.summary_period = SummaryPeriod::Today;
            }
            ui.add_space(6.0);
            if self.chip(ui, "This Week", period == SummaryPeriod::ThisWeek, theme::DOT_WAIT).clicked() {
                self.summary_period = SummaryPeriod::ThisWeek;
            }
        });
        ui.add_space(7.0);

        // Provider filter row (All / Claude / Codex / opencode).
        ui.horizontal(|ui| {
            let chips = [
                (None, "All", theme::ACCENT_GREEN),
                (Some(Provider::Claude), "Claude", theme::CLAUDE_ACCENT),
                (Some(Provider::Codex), "Codex", theme::CODEX_ACCENT),
                (Some(Provider::Opencode), "opencode", theme::OPENCODE_ACCENT),
            ];
            for (filt, label, accent) in chips {
                if self.chip(ui, label, prov_filter == filt, accent).clicked() {
                    self.summary_provider = filt;
                }
                ui.add_space(6.0);
            }
        });
        ui.add_space(10.0);

        // Build the filtered working set from both pools.
        let filtered: Vec<Session> = sessions_snap
            .iter()
            .chain(oc_sessions_snap.iter())
            .filter(|s| match prov_filter {
                None => true,
                Some(Provider::Claude) => s.provider == Provider::Claude,
                Some(Provider::Codex) => s.provider == Provider::Codex,
                Some(Provider::Opencode) => !matches!(s.provider, Provider::Claude | Provider::Codex),
                Some(_) => true,
            })
            .filter(|s| match period {
                SummaryPeriod::Today => is_today(s),
                SummaryPeriod::ThisWeek => is_this_week(s),
            })
            .cloned()
            .collect();

        let total_sessions = filtered.len();
        let total_cost: f64 = filtered.iter().map(|s| s.cost).sum();
        let total_tokens: u64 = filtered.iter().map(|s| s.tokens.input + s.tokens.output).sum();

        // Model stats: model label -> (session_count, input_tokens, output_tokens).
        let mut model_map: HashMap<String, (usize, u64, u64)> = HashMap::new();
        for s in &filtered {
            let key = model_label(s).unwrap_or_else(|| s.provider.display_name().to_string());
            let e = model_map.entry(key).or_insert((0, 0, 0));
            e.0 += 1;
            e.1 += s.tokens.input;
            e.2 += s.tokens.output;
        }
        let mut model_rows: Vec<(String, usize, u64, u64)> =
            model_map.into_iter().map(|(k, (n, i, o))| (k, n, i, o)).collect();
        model_rows.sort_by(|a, b| (b.2 + b.3).cmp(&(a.2 + a.3)));

        // Top-5 costliest sessions.
        let mut costly = filtered.clone();
        costly.sort_by(|a, b| b.cost.partial_cmp(&a.cost).unwrap_or(std::cmp::Ordering::Equal));
        costly.truncate(5);

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // Aggregate stats header card.
                egui::Frame::default()
                    .fill(theme::CARD_BG)
                    .inner_margin(egui::Margin::symmetric(14, 11))
                    .corner_radius(CornerRadius::same(10))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        if filtered.is_empty() {
                            ui.label(
                                RichText::new("No sessions in this period.")
                                    .color(theme::SUB)
                                    .size(sz(13.0)),
                            );
                        } else {
                            ui.horizontal(|ui| {
                                stat_cell(ui, &total_sessions.to_string(), "sessions", theme::HDR_TITLE);
                                ui.add_space(24.0);
                                stat_cell(ui, &fmt_cost(Some(total_cost)), "total cost", theme::ACCENT_GREEN);
                                ui.add_space(24.0);
                                stat_cell(ui, &fmt_tokens(total_tokens), "tokens (in+out)", theme::BLUE);
                            });
                        }
                    });

                // By-model table.
                if !model_rows.is_empty() {
                    summary_header(ui, "BY MODEL", model_rows.len(), theme::SECTION);
                    for (name, count, input, output) in &model_rows {
                        egui::Frame::default()
                            .fill(theme::CARD_BG)
                            .inner_margin(egui::Margin::symmetric(9, 7))
                            .corner_radius(CornerRadius::same(8))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        ui.label(
                                            RichText::new(fmt_tokens(input + output))
                                                .color(theme::BLUE)
                                                .size(sz(13.0))
                                                .strong(),
                                        );
                                        dot_sep(ui);
                                        ui.label(
                                            RichText::new(format!("{} sess", count))
                                                .color(theme::SUB)
                                                .size(sz(11.0)),
                                        );
                                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                            ui.add(
                                                egui::Label::new(
                                                    RichText::new(name)
                                                        .color(theme::SESS_NAME)
                                                        .size(sz(13.0))
                                                        .strong(),
                                                )
                                                .truncate(),
                                            );
                                        });
                                    });
                                });
                            });
                        ui.add_space(4.0);
                    }
                }

                // Top-5 most expensive sessions.
                if !costly.is_empty() {
                    summary_header(ui, "MOST EXPENSIVE", costly.len(), theme::DOT_WAIT);
                    let costly_cards = costly.clone();
                    for s in &costly_cards {
                        self.summary_session_card(ui, s, prov_filter.is_none());
                    }
                }
            });
    }

    /// A non-pin, non-terminal card for the summary "most expensive" list. Clicking
    /// jumps to the session's provider tab and opens its prompt-history view.
    fn summary_session_card(&mut self, ui: &mut egui::Ui, s: &Session, show_provider: bool) {
        let path = s.path.to_string_lossy().to_string();
        let id = ui.id().with(("sumcard", &path));
        let bg = ui.painter().add(egui::Shape::Noop);

        let inner = egui::Frame::default()
            .inner_margin(egui::Margin::symmetric(9, 7))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                // line 1: [provider] title ........................ $cost
                ui.horizontal(|ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new(fmt_cost(Some(s.cost)))
                                .color(theme::ACCENT_GREEN)
                                .size(sz(14.0))
                                .strong(),
                        );
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                            if show_provider {
                                ui.label(
                                    RichText::new(format!(" {} ", s.provider.display_name()))
                                        .size(sz(10.0))
                                        .color(Color32::from_rgb(16, 18, 24))
                                        .background_color(theme::provider_accent(s.provider)),
                                );
                                ui.add_space(5.0);
                            }
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&s.title)
                                        .color(theme::SESS_NAME)
                                        .size(sz(14.0))
                                        .strong(),
                                )
                                .truncate(),
                            );
                        });
                    });
                });
                // line 2: project · ago ..................... <tokens> tokens
                ui.horizontal(|ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new(fmt_tokens(s.tokens.input + s.tokens.output))
                                .color(theme::BLUE)
                                .size(sz(12.0)),
                        );
                        dot_sep(ui);
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(format!("{} · {}", s.project, fmt_ago(s.mtime)))
                                        .color(theme::SUB)
                                        .size(sz(12.0)),
                                )
                                .truncate(),
                            );
                        });
                    });
                });
            });

        let rect = inner.response.rect;
        let resp = ui.interact(rect, id, Sense::click());
        let hovered = resp.hovered();
        ui.painter().set(
            bg,
            egui::Shape::rect_filled(
                rect,
                CornerRadius::same(8),
                if hovered { theme::ROW_HOVER } else { theme::CARD_BG },
            ),
        );
        if hovered {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if resp.clicked() {
            let target_tab = match s.provider {
                Provider::Claude => Tab::Claude,
                Provider::Codex => Tab::Codex,
                _ => Tab::Opencode,
            };
            self.tab = target_tab;
            if target_tab == Tab::Opencode && self.oc_sessions.is_empty() {
                self.reload_opencode();
            }
            self.open_history(&path);
        }
        ui.add_space(4.0);
    }

    /// A small pill toggle used by the opencode All/GLM/MiniMax filter row.
    fn chip(&self, ui: &mut egui::Ui, label: &str, active: bool, accent: Color32) -> egui::Response {
        let (bg, fg) = if active {
            (accent, Color32::from_rgb(16, 18, 24))
        } else {
            (Color32::from_rgba_unmultiplied(255, 255, 255, 16), accent)
        };
        let resp = egui::Frame::default()
            .fill(bg)
            .inner_margin(egui::Margin::symmetric(10, 4))
            .corner_radius(CornerRadius::same(8))
            .show(ui, |ui| {
                ui.label(RichText::new(label).size(sz(12.0)).strong().color(fg));
            })
            .response;
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        resp.interact(Sense::click())
    }

    /// Shared session list: PINNED / ACTIVE / RECENT sections + footer.
    /// `terminal` enables the open/focus button and working/waiting legend
    /// (Claude/Codex); opencode rows are history-only.
    fn session_list(&mut self, ui: &mut egui::Ui, results: &[Session], terminal: bool, empty: &str) {
        let pins = self.state.pinned.clone();
        let is_pin = |s: &Session| pins.iter().any(|p| p == &s.path.to_string_lossy());

        let pinned: Vec<Session> = results.iter().filter(|s| is_pin(s)).cloned().collect();
        let mut active: Vec<Session> =
            results.iter().filter(|s| s.open && !is_pin(s)).cloned().collect();
        active.sort_by_key(|s| if s.working { 0 } else if s.waiting { 1 } else { 2 });
        let rest: Vec<Session> = results.iter().filter(|s| !s.open && !is_pin(s)).cloned().collect();
        let count = results.len();

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            if results.is_empty() {
                ui.add_space(12.0);
                ui.label(RichText::new(empty).color(theme::SUB).size(sz(14.0)));
                return;
            }
            self.section(ui, "PINNED", &pinned, terminal, theme::DOT_WAIT);
            self.section(ui, "ACTIVE", &active, terminal, theme::LIVE_GREEN);
            let recent = if self.search.is_empty() { "RECENT" } else { "MATCHES" };
            self.section(ui, recent, &rest, terminal, theme::SECTION);
        });

        ui.add_space(6.0);
        let footer = if self.today_only {
            let t = today_totals(results);
            format!("today: {} prompts · {} tok · {}", t.prompts, fmt_tokens(t.tokens), fmt_cost(Some(t.cost)))
        } else if !self.footer_hint.is_empty() {
            self.footer_hint.clone()
        } else if terminal {
            format!("{} sessions · click a card to view · open to jump to the terminal", count)
        } else {
            format!("{} sessions · click a card to view history", count)
        };
        ui.label(RichText::new(footer).color(theme::SUB).size(sz(11.0)));
    }

    fn section(&mut self, ui: &mut egui::Ui, label: &str, items: &[Session], terminal: bool, accent: Color32) {
        if items.is_empty() {
            return;
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(label).color(accent).size(sz(11.0)).strong());
            ui.add_space(6.0);
            ui.label(RichText::new(items.len().to_string()).color(theme::SUB).size(sz(11.0)));
            ui.add_space(8.0);
            // hairline rule filling the rest of the row, vertically centered
            let (r, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), Sense::hover());
            let y = r.center().y;
            ui.painter().line_segment(
                [egui::pos2(r.left(), y), egui::pos2(r.right(), y)],
                Stroke::new(1.0, theme::DOT_OFF),
            );
        });
        ui.add_space(2.0);
        for s in items {
            self.session_row(ui, s, terminal);
        }
    }

    fn session_row(&mut self, ui: &mut egui::Ui, s: &Session, terminal: bool) {
        let path = s.path.to_string_lossy().to_string();
        let pinned = self.state.is_pinned(&path);
        let id = ui.id().with(("scard", &path));

        // Reserve a background shape so we can paint a hover fill *behind* the content.
        let bg = ui.painter().add(egui::Shape::Noop);
        // Sub-rects for the pin / open affordances; clicks are routed by pointer
        // position against these (the whole card is a single click target, so inner
        // buttons can't steal — and the empty card area is clickable too).
        let mut pin_rect = Rect::NOTHING;
        let mut open_rect = Rect::NOTHING;

        let inner = egui::Frame::default()
            .inner_margin(egui::Margin::symmetric(9, 7))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                // line 1: status · title ............... open/focus · ☆ pin
                // The right-side affordances are allocated FIRST (right-to-left), so the
                // title (nested left-to-right) truncates within the leftover space and
                // can never paint over the buttons.
                ui.horizontal(|ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let (pr, _) = ui.allocate_exact_size(egui::vec2(sz(22.0), sz(20.0)), Sense::hover());
                        pin_rect = pr;
                        draw_star(ui.painter(), pr, pinned, ui.rect_contains_pointer(pr));
                        if terminal {
                            let label = if s.open { "focus" } else { "open" };
                            let galley = ui.painter().layout_no_wrap(label.to_owned(), egui::FontId::proportional(sz(12.0)), theme::ACCENT_GREEN);
                            let icon = sz(12.0);
                            let w = sz(9.0) + icon + sz(5.0) + galley.size().x + sz(9.0);
                            let (r, _) = ui.allocate_exact_size(egui::vec2(w, sz(23.0)), Sense::hover());
                            open_rect = r;
                            // persistent green-tint pill so the button is always visible,
                            // brighter on hover.
                            let hov = ui.rect_contains_pointer(r);
                            let bg = Color32::from_rgba_unmultiplied(154, 230, 180, if hov { 46 } else { 26 });
                            ui.painter().rect_filled(r, CornerRadius::same(7), bg);
                            let ic = Rect::from_min_size(egui::pos2(r.left() + sz(9.0), r.center().y - icon / 2.0), egui::vec2(icon, icon));
                            draw_arrow_icon(ui.painter(), ic, theme::ACCENT_GREEN);
                            ui.painter().galley(egui::pos2(ic.right() + sz(5.0), r.center().y - galley.size().y / 2.0), galley, theme::ACCENT_GREEN);
                            ui.add_space(7.0);
                        }
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                            status_widget(ui, s);
                            ui.add_space(4.0);
                            ui.add(egui::Label::new(RichText::new(&s.title).color(theme::SESS_NAME).size(sz(14.0)).strong()).truncate());
                        });
                    });
                });
                // line 2: [chip] project · ⎇branch · model · recency
                ui.horizontal(|ui| {
                    if let Some((txt, col)) = provider_chip(s) {
                        ui.label(RichText::new(format!(" {txt} ")).size(sz(10.0)).color(Color32::from_rgb(16, 18, 24)).background_color(col));
                    }
                    ui.add(egui::Label::new(RichText::new(&s.project).color(theme::SUB).size(sz(12.0))).truncate());
                    if let Some(b) = &s.branch {
                        ui.add_space(5.0);
                        let (br, _) = ui.allocate_exact_size(egui::vec2(11.0, 12.0), Sense::hover());
                        draw_branch_icon(ui.painter(), br, theme::SUB);
                        ui.add_space(2.0);
                        ui.add(egui::Label::new(RichText::new(b).color(theme::SUB).size(sz(12.0))).truncate());
                    }
                    if let Some(m) = model_label(s) {
                        dot_sep(ui);
                        ui.label(RichText::new(m).color(theme::SUB).size(sz(12.0)));
                    }
                    dot_sep(ui);
                    ui.label(RichText::new(fmt_ago(s.mtime)).color(theme::SUB).size(sz(12.0)));
                });
                // line 3: N prompts ............ <tokens> tokens · $cost
                // Tokens + cost are the headline metrics, kept together on the right
                // (emphasized) so they're easy to compare; prompts is muted on the left.
                ui.horizontal(|ui| {
                    let prompts = if s.total_prompts == 1 { "1 prompt".to_string() } else { format!("{} prompts", s.total_prompts) };
                    ui.label(RichText::new(prompts).color(theme::SUB).size(sz(11.0)));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if s.provider != Provider::Codex {
                            ui.label(RichText::new(fmt_cost(Some(s.cost))).color(theme::ACCENT_GREEN).size(sz(14.0)).strong());
                            dot_sep(ui);
                        }
                        ui.label(RichText::new("tokens").color(theme::SUB).size(sz(11.0)));
                        ui.label(RichText::new(fmt_tokens(s.total_tokens)).color(theme::BLUE).size(sz(14.0)).strong());
                    });
                });
            });

        let rect = inner.response.rect;
        let resp = ui.interact(rect, id, Sense::click());
        // Always paint the faint card surface so adjacent sessions read as distinct
        // cards at rest; brighten it on hover.
        let fill = if resp.hovered() { theme::ROW_HOVER } else { theme::CARD_BG };
        ui.painter().set(bg, egui::Shape::rect_filled(rect, CornerRadius::same(8), fill));
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if resp.clicked() {
            match resp.interact_pointer_pos() {
                Some(pos) if pin_rect.contains(pos) => {
                    self.state.toggle_pin(&path);
                    self.state.save();
                }
                Some(pos) if open_rect.contains(pos) => self.open_action(s),
                _ => self.open_history(&path),
            }
        }
        ui.add_space(4.0);
    }

    fn open_history(&mut self, path: &str) {
        self.clear_attention(path);
        self.show_history(path);
        self.state.last_session = Some(path.to_string());
        self.state.save();
    }

    fn open_action(&mut self, s: &Session) {
        self.clear_attention(&s.path.to_string_lossy());
        let tmpl = self.resume_cmd(s.provider);
        let ok = petcore::terminal::open_session(self.term.as_ref(), s, &tmpl);
        let verb = if s.open { "focused" } else { "opened" };
        self.footer_hint = if ok {
            format!("{} “{}” in your terminal…", verb, s.project)
        } else {
            "couldn't reach the terminal — is it running?".to_string()
        };
    }

    fn new_session_action(&mut self, cwd: &str, provider: Provider) {
        let cmd = self.new_cmd(provider);
        let ok = petcore::terminal::new_session(self.term.as_ref(), std::path::Path::new(cwd), &cmd);
        self.footer_hint = if ok {
            format!("started new {} session in {}", provider.label(), cwd)
        } else {
            "couldn't reach the terminal".to_string()
        };
    }

    // ---- history view ---------------------------------------------------
    fn ui_history(&mut self, ui: &mut egui::Ui) {
        let path = match &self.panel_session {
            Some(p) => p.clone(),
            None => return,
        };
        let session = petcore::parse_session_any(std::path::Path::new(&path));

        let Some(s) = session else {
            ui.horizontal(|ui| {
                if icon_button(ui, IconKind::Back).clicked() {
                    self.panel_session = None;
                }
            });
            ui.add_space(6.0);
            ui.label(RichText::new("No prompts in this session yet.").color(theme::SUB));
            return;
        };

        // --- identity band: back · title, then repo / branch / model / recency ---
        let accent = theme::provider_accent(s.provider);
        egui::Frame::default()
            .inner_margin(egui::Margin { left: 0, right: 0, top: 0, bottom: 6 })
            .show(ui, |ui| {
                // line 1: back button + session name
                ui.horizontal(|ui| {
                    if icon_button(ui, IconKind::Back).clicked() {
                        self.panel_session = None;
                    }
                    ui.add_space(2.0);
                    ui.add(egui::Label::new(RichText::new(&s.title).color(theme::HDR_TITLE).size(sz(16.0)).strong()).truncate());
                });
                ui.add_space(7.0);
                // line 2: metadata chips, indented to sit under the title
                ui.horizontal(|ui| {
                    ui.add_space(30.0);
                    // repo
                    let (fr, _) = ui.allocate_exact_size(egui::vec2(sz(14.0), sz(14.0)), Sense::hover());
                    draw_folder_icon(ui.painter(), fr, theme::SUB);
                    ui.add_space(4.0);
                    ui.add(egui::Label::new(RichText::new(&s.project).color(theme::PTITLE).size(sz(12.5))).truncate());
                    // branch
                    if let Some(b) = &s.branch {
                        ui.add_space(11.0);
                        let (br, _) = ui.allocate_exact_size(egui::vec2(sz(13.0), sz(13.0)), Sense::hover());
                        draw_branch_icon(ui.painter(), br, theme::SUB);
                        ui.add_space(3.0);
                        ui.add(egui::Label::new(RichText::new(b).color(theme::SUB).size(sz(12.5))).truncate());
                    }
                    // model (provider-accent chip)
                    if let Some(m) = model_label(&s) {
                        ui.add_space(11.0);
                        model_chip(ui, &m, accent);
                    }
                    // recency
                    ui.add_space(11.0);
                    let (cr, _) = ui.allocate_exact_size(egui::vec2(sz(13.0), sz(13.0)), Sense::hover());
                    draw_clock_icon(ui.painter(), cr, theme::SUB);
                    ui.add_space(4.0);
                    ui.label(RichText::new(fmt_ago(s.mtime)).color(theme::SUB).size(sz(12.5)));
                });
            });
        // hairline separating the band from the prompt list
        let (hr, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), Sense::hover());
        let y = hr.center().y;
        ui.painter().line_segment([egui::pos2(hr.left(), y), egui::pos2(hr.right(), y)], Stroke::new(1.0, theme::DOT_OFF));
        ui.add_space(8.0);

        if s.prompts.is_empty() {
            ui.label(RichText::new("No prompts in this session yet.").color(theme::SUB));
            return;
        }
        let max_out = s.prompts.iter().map(|p| p.out_tokens).max().unwrap_or(0);

        // gutter width = index chip + the space between it and the title; line 2 (meta)
        // indents by this much so it aligns under the prompt text.
        let gutter = sz(22.0) + sz(8.0);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for p in &s.prompts {
                let full = if p.full_text.is_empty() { p.title.clone() } else { p.full_text.clone() };
                let expandable = full.contains('\n') || full.chars().count() > 90;
                let expanded = expandable && self.expanded.contains(&p.index);

                // Reserve a background shape so the hover/rest card fill paints behind.
                let bg = ui.painter().add(egui::Shape::Noop);
                let inner = egui::Frame::default()
                    .inner_margin(egui::Margin::symmetric(9, 7))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        // line 1: [#] title ........................... <tokens> tokens
                        ui.horizontal_top(|ui| {
                            ui.with_layout(Layout::right_to_left(Align::TOP), |ui| {
                                if p.running {
                                    let d = sz(9.0);
                                    let (dr, _) = ui.allocate_exact_size(egui::vec2(d, d), Sense::hover());
                                    ui.painter().circle_filled(dr.center(), d * 0.32, theme::LIVE_GREEN);
                                    ui.add_space(2.0);
                                    ui.label(RichText::new("running").color(theme::LIVE_GREEN).size(sz(12.0)).strong());
                                } else {
                                    let c = theme::heat_color(p.out_tokens, max_out);
                                    ui.label(RichText::new("tokens").color(theme::SUB).size(sz(11.0)));
                                    ui.add_space(3.0);
                                    ui.label(RichText::new(fmt_tokens(p.out_tokens)).color(c).size(sz(14.0)).strong());
                                }
                                ui.add_space(8.0);
                                ui.with_layout(Layout::left_to_right(Align::TOP), |ui| {
                                    index_chip(ui, p.index, p.running);
                                    ui.add_space(8.0);
                                    let text = if expanded { full.clone() } else { p.title.clone() };
                                    let col = if p.running { theme::HDR_TITLE } else { theme::PTITLE };
                                    let lbl = egui::Label::new(RichText::new(text).color(col).size(sz(14.0)));
                                    ui.add(if expanded { lbl.wrap() } else { lbl.truncate() });
                                });
                            });
                        });
                        // line 2: <elapsed · ago> ......................... ›/⌄
                        ui.horizontal(|ui| {
                            ui.add_space(gutter);
                            let meta = match p.ts {
                                Some(ts) => format!("{} · {}", fmt_elapsed(p.elapsed), fmt_ago(ts)),
                                None => fmt_elapsed(p.elapsed),
                            };
                            ui.label(RichText::new(meta).color(theme::SUB).size(sz(11.0)));
                            // which model handled this prompt (accent-colored, subtle)
                            if let Some(m) = prompt_model_label(&s, p) {
                                dot_sep(ui);
                                let mc = Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 205);
                                ui.label(RichText::new(m).color(mc).size(sz(11.0)));
                            }
                            if expandable {
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    let (cr, _) = ui.allocate_exact_size(egui::vec2(sz(12.0), sz(12.0)), Sense::hover());
                                    draw_chevron(ui.painter(), cr, theme::SUB, expanded);
                                });
                            }
                        });
                    });

                let rect = inner.response.rect;
                let resp = ui.interact(rect, ui.id().with(("prow", p.index)), Sense::click());
                let fill = if resp.hovered() { theme::ROW_HOVER } else { theme::CARD_BG };
                ui.painter().set(bg, egui::Shape::rect_filled(rect, CornerRadius::same(8), fill));
                if expandable {
                    if resp.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    if resp.clicked() {
                        if expanded {
                            self.expanded.remove(&p.index);
                        } else {
                            self.expanded.insert(p.index);
                        }
                    }
                }
                ui.add_space(4.0);
            }
        });

        ui.add_space(6.0);
        // headline summary bar — click to expand the detailed stats card. The key
        // metrics carry their identity colors (tokens BLUE, cost green) like the list.
        let bar = egui::Frame::default()
            .fill(theme::CARD_BG)
            .inner_margin(egui::Margin::symmetric(14, 9))
            .corner_radius(egui::CornerRadius::same(10))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let prompts = if s.total_prompts == 1 { "1 prompt".to_string() } else { format!("{} prompts", s.total_prompts) };
                    ui.label(RichText::new(prompts).color(theme::SUB).size(sz(12.0)));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let (cr, _) = ui.allocate_exact_size(egui::vec2(sz(12.0), sz(12.0)), Sense::hover());
                        draw_chevron(ui.painter(), cr, theme::SUB, self.history_stats_open);
                        ui.add_space(2.0);
                        if s.provider != Provider::Codex {
                            ui.label(RichText::new(fmt_cost(Some(s.cost))).color(theme::ACCENT_GREEN).size(sz(14.0)).strong());
                            dot_sep(ui);
                        }
                        ui.label(RichText::new("tokens").color(theme::SUB).size(sz(11.0)));
                        ui.label(RichText::new(fmt_tokens(s.total_tokens)).color(theme::BLUE).size(sz(14.0)).strong());
                    });
                });
            });
        let bar_resp = ui.interact(bar.response.rect, ui.id().with("hist-stats"), Sense::click());
        if bar_resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if bar_resp.clicked() {
            self.history_stats_open = !self.history_stats_open;
        }

        if self.history_stats_open {
            ui.add_space(4.0);
            let avg = if s.total_prompts > 0 { s.total_tokens / s.total_prompts as u64 } else { 0 };
            egui::Frame::default()
                .fill(theme::CARD_BG)
                .inner_margin(egui::Margin::symmetric(14, 11))
                .corner_radius(egui::CornerRadius::same(10))
                .show(ui, |ui| {
                    let stat = |ui: &mut egui::Ui, num: String, lbl: &str, col: Color32| {
                        ui.vertical(|ui| {
                            ui.label(RichText::new(num).color(col).size(sz(15.0)).strong());
                            ui.label(RichText::new(lbl).color(theme::SUB).size(sz(11.0)));
                        });
                        ui.add_space(18.0);
                    };
                    ui.horizontal(|ui| {
                        stat(ui, fmt_tokens(s.tokens.input), "input", theme::HDR_TITLE);
                        stat(ui, fmt_tokens(s.total_tokens), "output", theme::BLUE);
                        stat(ui, fmt_tokens(s.tokens.cache_read), "cache read", theme::HDR_TITLE);
                    });
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        stat(ui, fmt_cost(Some(s.cost)), "cost", theme::ACCENT_GREEN);
                        stat(ui, fmt_tokens(avg), "avg / prompt", theme::HDR_TITLE);
                        stat(ui, fmt_elapsed(s.wall_seconds), "wall-clock", theme::HDR_TITLE);
                    });
                });
        }
    }

    // ---- quota / usage view ---------------------------------------------
    fn ui_quota(&mut self, ui: &mut egui::Ui) {
        if self.quota.sections.is_empty() {
            ui.label(RichText::new("Fetching usage…").color(theme::SUB).size(sz(13.0)));
            return;
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        let sections = self.quota.sections.clone();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            // A Frame as the literal first child of a ScrollArea drops its top content
            // in egui; a leading spacer anchors the layout so the first card renders.
            ui.add_space(2.0);
            for sec in &sections {
                self.quota_card(ui, sec, now);
                ui.add_space(10.0);
            }
        });
    }

    /// One rounded card per provider, with a brand accent stripe + logo/chip header.
    fn quota_card(&self, ui: &mut egui::Ui, sec: &petcore::quota::ProviderSection, now: i64) {
        let accent = theme::provider_accent(sec.provider);
        let stale = sec.windows.iter().any(|w| w.reset) || sec.note.is_some();
        let inner = egui::Frame::default()
            .fill(Color32::from_rgba_unmultiplied(255, 255, 255, 8))
            .inner_margin(egui::Margin { left: 16, right: 14, top: 12, bottom: 12 })
            .corner_radius(egui::CornerRadius::same(12))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                // header: icon + name (+badge) ......... note
                ui.horizontal(|ui| {
                    self.provider_icon(ui, sec.provider, accent, 22.0);
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(sec.provider.display_name()).color(accent).size(sz(15.0)).strong(),
                    );
                    if let Some(badge) = &sec.badge {
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new(format!(" {} ", badge.to_uppercase()))
                                .size(sz(10.0))
                                .strong()
                                .color(Color32::from_rgb(18, 20, 26))
                                .background_color(accent),
                        );
                    }
                    if let Some(note) = &sec.note {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let col = if stale { theme::DOT_WAIT } else { theme::SUB };
                            ui.add(
                                egui::Label::new(RichText::new(note).color(col).size(sz(10.0))).truncate(),
                            );
                        });
                    }
                });

                if sec.windows.is_empty() && sec.summary.is_none() && sec.children.is_empty() {
                    if sec.note.is_none() {
                        ui.add_space(4.0);
                        ui.label(RichText::new("no data yet").color(theme::SUB).size(sz(12.0)));
                    }
                    return;
                }

                if !sec.windows.is_empty() || sec.summary.is_some() {
                    ui.add_space(8.0);
                    for w in &sec.windows {
                        self.quota_bar(ui, accent, w, now);
                    }
                    if let Some(sum) = &sec.summary {
                        self.quota_summary(ui, sum);
                    }
                }
                for child in &sec.children {
                    self.quota_child(ui, child, now);
                }
            });

        // brand accent stripe down the left edge of the card
        let mut stripe = inner.response.rect;
        stripe.min.y += 2.0;
        stripe.max.y -= 2.0;
        stripe.set_width(4.0);
        ui.painter().rect_filled(stripe, egui::CornerRadius::same(2), accent);
    }

    /// Logo texture if present, else a drawn monogram chip in the brand accent.
    fn provider_icon(&self, ui: &mut egui::Ui, provider: petcore::Provider, accent: Color32, size: f32) {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), Sense::hover());
        if let Some(tex) = self.logos.get(&provider) {
            ui.painter().image(
                tex.id(),
                rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        } else {
            let p = ui.painter();
            p.rect_filled(rect, size * 0.27, accent);
            let init = provider
                .display_name()
                .chars()
                .next()
                .unwrap_or('?')
                .to_ascii_uppercase();
            p.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                init,
                egui::FontId::proportional(size * 0.64),
                Color32::from_rgb(18, 20, 26),
            );
        }
    }

    /// One nested sub-provider block (GLM / MiniMax) inside the opencode card.
    fn quota_child(&self, ui: &mut egui::Ui, child: &petcore::quota::SubUsage, now: i64) {
        let accent = theme::provider_accent(child.provider);
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            self.provider_icon(ui, child.provider, accent, 18.0);
            ui.add_space(7.0);
            ui.label(RichText::new(child.provider.display_name()).color(accent).size(sz(13.0)).strong());
            if let Some(badge) = &child.badge {
                ui.add_space(6.0);
                ui.label(
                    RichText::new(format!(" {} ", badge.to_uppercase()))
                        .size(sz(9.0))
                        .strong()
                        .color(Color32::from_rgb(18, 20, 26))
                        .background_color(accent),
                );
            }
            if let Some(note) = &child.note {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.add(egui::Label::new(RichText::new(note).color(theme::SUB).size(sz(10.0))).truncate());
                });
            }
        });
        ui.add_space(6.0);
        for w in &child.windows {
            self.quota_bar(ui, accent, w, now);
        }
        if let Some(sum) = &child.summary {
            self.quota_summary(ui, sum);
        }
    }

    fn quota_bar(
        &self,
        ui: &mut egui::Ui,
        accent: Color32,
        w: &petcore::quota::QuotaWindow,
        now: i64,
    ) {
        let pct = (w.used_percent / 100.0).clamp(0.0, 1.0);
        let color = theme::pct_color(accent, w.used_percent);
        ui.horizontal(|ui| {
            ui.label(RichText::new(&w.label).color(theme::SESS_NAME).size(sz(13.0)));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let resets = if w.reset {
                    "reset since last run".to_string()
                } else {
                    match w.resets_at {
                        Some(r) if r > now => format!("resets in {}", fmt_elapsed(Some((r - now) as f64))),
                        Some(_) => "resets soon".to_string(),
                        None => String::new(),
                    }
                };
                ui.label(RichText::new(resets).color(theme::SUB).size(sz(11.0)));
                ui.add_space(8.0);
                ui.label(RichText::new(format!("{:.0}%", w.used_percent)).color(color).size(sz(13.0)).strong());
            });
        });
        // bar track + fill
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 8.0), Sense::hover());
        let painter = ui.painter();
        painter.rect_filled(rect, 4.0, Color32::from_rgba_unmultiplied(255, 255, 255, 20));
        let mut fill = rect;
        fill.set_width(rect.width() * pct.max(if w.used_percent > 0.0 { 0.02 } else { 0.0 }));
        painter.rect_filled(fill, 4.0, color);
        ui.add_space(8.0);
    }

    /// Token-totals row for providers without a quota window (opencode).
    fn quota_summary(&self, ui: &mut egui::Ui, sum: &petcore::quota::TokenSummary) {
        ui.horizontal(|ui| {
            let cells = [
                (fmt_tokens(sum.tokens_input), "input"),
                (fmt_tokens(sum.tokens_output), "output"),
                (fmt_cost(Some(sum.cost)), "cost"),
            ];
            for (num, lbl) in cells {
                ui.vertical(|ui| {
                    ui.label(RichText::new(num).color(theme::HDR_TITLE).size(sz(15.0)).strong());
                    ui.label(RichText::new(lbl).color(theme::SUB).size(sz(11.0)));
                });
                ui.add_space(18.0);
            }
        });
    }
}
