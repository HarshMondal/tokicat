//! Pet viewport: sprite/placeholder drawing, bob/wiggle/bounce, hearts, petting
//! banner, attention badge, and click-vs-drag input. Methods on `App`.

use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Vec2};

use crate::app::{App, PetState};
use crate::theme;

impl App {
    fn ensure_texture(&mut self, ctx: &egui::Context, idx: usize) -> Option<egui::TextureId> {
        if idx >= self.frames.len() {
            return None;
        }
        if self.textures[idx].is_none() {
            let img = self.frames[idx].image.clone();
            let tex = ctx.load_texture(format!("pet-{idx}"), img, egui::TextureOptions::LINEAR);
            self.textures[idx] = Some(tex);
        }
        self.textures[idx].as_ref().map(|t| t.id())
    }

    pub fn ui_pet(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let t = ctx.input(|i| i.time);
        let rect = ui.max_rect();
        let resp = ui.interact(rect, ui.id().with("petbody"), Sense::click_and_drag());
        self.handle_input(&ctx, &resp);

        let bob = 3.0 * (t * 2.5).sin() as f32 - self.bounce * (t * 15.0).sin().abs() as f32;
        let wig = self.wiggle * (t * 22.5).sin() as f32;
        let center = rect.center() + Vec2::new(wig, bob * self.scale);

        // sprite or placeholder
        let idx = self.frame_idx;
        if let Some(tex) = self.ensure_texture(&ctx, idx) {
            let sz = self.frames[idx].image.size;
            let w = sz[0] as f32 * self.scale;
            let h = sz[1] as f32 * self.scale;
            let r = Rect::from_center_size(center, Vec2::new(w, h));
            ui.painter().image(
                tex,
                r,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        } else {
            self.draw_placeholder(ui.painter(), center, self.size * 0.40);
        }

        if matches!(self.pet_state, PetState::Petting | PetState::Celebrate) {
            self.draw_banner(ui.painter(), rect, t);
        }
        self.draw_hearts(ui.painter(), rect.min);
        self.draw_badge(ui.painter(), rect);
    }

    fn handle_input(&mut self, ctx: &egui::Context, resp: &egui::Response) {
        match self.pet_state {
            PetState::Petting => {
                if resp.clicked() {
                    self.on_pet();
                }
            }
            PetState::Celebrate => {}
            PetState::Normal => {
                if resp.drag_started() {
                    self.cancel_animation();
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                if resp.drag_stopped() {
                    self.state.pet_x = Some(self.pos.x.round() as i32);
                    self.state.pet_y = Some(self.pos.y.round() as i32);
                    self.state.save();
                }
                if resp.clicked() {
                    self.toggle_panel(ctx);
                }
            }
        }
    }

    fn draw_placeholder(&self, painter: &egui::Painter, c: Pos2, r: f32) {
        painter.circle_filled(c, r, theme::COIN_BODY);
        painter.circle_stroke(c, r, egui::Stroke::new(3.0, theme::COIN_EDGE));
        let eye = r * 0.12;
        painter.circle_filled(c + Vec2::new(-r * 0.35, -r * 0.12), eye, theme::COIN_FACE);
        painter.circle_filled(c + Vec2::new(r * 0.35, -r * 0.12), eye, theme::COIN_FACE);
        // smile: a short arc approximated by a few segments
        let mut pts = Vec::new();
        let steps = 12;
        for i in 0..=steps {
            let a = std::f32::consts::PI * (0.15 + 0.70 * i as f32 / steps as f32);
            pts.push(c + Vec2::new(a.cos() * r * 0.45, a.sin() * r * 0.45 + r * 0.05));
        }
        painter.add(egui::Shape::line(pts, egui::Stroke::new(2.5, theme::COIN_FACE)));
    }

    fn draw_banner(&self, painter: &egui::Painter, rect: Rect, t: f64) {
        let remaining = self.cfg.pet.pets_required.saturating_sub(self.pets);
        let text = if self.pet_state == PetState::Celebrate {
            "yay! ♥".to_string()
        } else if remaining < self.cfg.pet.pets_required {
            format!("more! {}", remaining)
        } else {
            "pet me!".to_string()
        };
        let pulse = (t * 3.0).sin().abs() as f32;
        let font = FontId::proportional(15.0);
        let pos = Pos2::new(rect.center().x, rect.min.y + 14.0);
        let galley = painter.layout_no_wrap(text.clone(), font.clone(), theme::HEART);
        let pill = Rect::from_center_size(pos, Vec2::new(galley.size().x + 20.0, 24.0));
        painter.rect_filled(
            pill,
            12.0,
            Color32::from_rgba_unmultiplied(15, 18, 23, (140.0 + 64.0 * pulse) as u8),
        );
        painter.text(
            pos,
            Align2::CENTER_CENTER,
            text,
            font,
            Color32::from_rgb(255, 115, 158),
        );
    }

    fn draw_hearts(&self, painter: &egui::Painter, origin: Pos2) {
        for h in &self.hearts {
            let a = h.life.clamp(0.0, 1.0);
            let col = Color32::from_rgba_unmultiplied(255, 92, 133, (a * 255.0) as u8);
            let c = origin + Vec2::new(h.x, h.y);
            let r = h.r;
            // two lobes + a point below
            painter.circle_filled(c + Vec2::new(-r * 0.5, -r * 0.2), r * 0.55, col);
            painter.circle_filled(c + Vec2::new(r * 0.5, -r * 0.2), r * 0.55, col);
            painter.add(egui::Shape::convex_polygon(
                vec![
                    c + Vec2::new(-r, -r * 0.05),
                    c + Vec2::new(r, -r * 0.05),
                    c + Vec2::new(0.0, r * 1.1),
                ],
                col,
                egui::Stroke::NONE,
            ));
        }
    }

    fn draw_badge(&self, painter: &egui::Painter, rect: Rect) {
        if !self.attention.is_empty() {
            let n = self.attention.len();
            let c = Pos2::new(rect.max.x - 12.0, rect.min.y + 12.0);
            painter.circle_filled(c, 10.0, theme::ATTENTION);
            let label = if n == 1 { "!".to_string() } else { n.to_string() };
            painter.text(c, Align2::CENTER_CENTER, label, FontId::proportional(14.0), Color32::WHITE);
        } else if self.scale < 1.2 && self.any_live() {
            let c = Pos2::new(rect.max.x - 10.0, rect.min.y + 10.0);
            painter.circle_filled(c, 5.0, theme::LIVE_GREEN);
        }
    }
}
