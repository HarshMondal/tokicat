//! Optional per-provider brand logos for the usage screen. PNGs live in
//! `assets/logos/<key>.png` (key == `Provider::label()`). Missing files are fine —
//! the panel falls back to a drawn monogram chip, so the screen never depends on
//! shipping logo art.

use std::collections::HashMap;

use petcore::Provider;

use crate::art;

const PROVIDERS: [Provider; 5] = [
    Provider::Claude,
    Provider::Codex,
    Provider::Zai,
    Provider::Minimax,
    Provider::Opencode,
];

/// Load whatever provider logos exist into GPU textures, keyed by provider.
pub fn load(ctx: &egui::Context) -> HashMap<Provider, egui::TextureHandle> {
    let mut out = HashMap::new();
    for p in PROVIDERS {
        if let Some(img) = find_logo(p) {
            let tex = ctx.load_texture(format!("logo-{}", p.label()), img, egui::TextureOptions::LINEAR);
            out.insert(p, tex);
        }
    }
    out
}

fn find_logo(p: Provider) -> Option<egui::ColorImage> {
    let name = format!("{}.png", p.label());
    for root in art::asset_roots() {
        let path = root.join("logos").join(&name);
        if path.is_file() {
            if let Ok(img) = image::open(&path) {
                let rgba = img.to_rgba8();
                let (w, h) = (rgba.width() as usize, rgba.height() as usize);
                return Some(egui::ColorImage::from_rgba_unmultiplied([w, h], rgba.as_raw()));
            }
        }
    }
    None
}
