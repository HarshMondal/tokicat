//! Colors + visuals, mirroring the CSS block in pet.py.

use egui::Color32;

pub const PANEL_BG: Color32 = Color32::from_rgba_premultiplied(20, 22, 28, 247);
pub const HDR_TITLE: Color32 = Color32::from_rgb(0xF1, 0xF3, 0xF5);
pub const SUB: Color32 = Color32::from_rgb(0x82, 0x8A, 0x93);
pub const SESS_NAME: Color32 = Color32::from_rgb(0xEC, 0xEF, 0xF2);
pub const ACCENT_GREEN: Color32 = Color32::from_rgb(0x9A, 0xE6, 0xB4);
pub const LIVE_GREEN: Color32 = Color32::from_rgb(0x7C, 0xE3, 0x8B);
pub const DOT_WAIT: Color32 = Color32::from_rgb(0xFF, 0xD1, 0x66);
pub const DOT_OFF: Color32 = Color32::from_rgb(0x4A, 0x4F, 0x58);
pub const BLUE: Color32 = Color32::from_rgb(0x8A, 0xB4, 0xF8);
pub const PTITLE: Color32 = Color32::from_rgb(0xDC, 0xE0, 0xE5);

pub const TOK_HEAVY: Color32 = Color32::from_rgb(255, 107, 107);
pub const TOK_MID: Color32 = Color32::from_rgb(255, 209, 102);
pub const TOK_LIGHT: Color32 = Color32::from_rgb(138, 180, 248);

pub const HEART: Color32 = Color32::from_rgb(255, 92, 133);
pub const ATTENTION: Color32 = Color32::from_rgb(255, 77, 82);

pub const COIN_BODY: Color32 = Color32::from_rgb(255, 209, 77);
pub const COIN_EDGE: Color32 = Color32::from_rgb(217, 158, 26);
pub const COIN_FACE: Color32 = Color32::from_rgb(38, 31, 13);

/// Token-heat color for a prompt row. Port of `heat_class`.
pub fn heat_color(out_tokens: u64, max_out: u64) -> Color32 {
    if max_out > 0 && out_tokens >= (max_out as f64 * 0.66) as u64 && out_tokens > 0 {
        TOK_HEAVY
    } else if max_out > 0 && out_tokens >= (max_out as f64 * 0.33) as u64 && out_tokens > 0 {
        TOK_MID
    } else {
        TOK_LIGHT
    }
}

/// Apply a dark, rounded visual theme to a context.
pub fn apply(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = Color32::TRANSPARENT;
    visuals.window_fill = PANEL_BG;
    visuals.override_text_color = Some(SESS_NAME);
    ctx.set_visuals(visuals);
}
