//! Monitor geometry. On X11 we enumerate all monitors via `xrandr` (so the pet
//! can wander across screens and the panel opens on the right one). Falls back to
//! egui's current-monitor size when xrandr is unavailable.

use std::process::Command;

#[derive(Clone, Copy, Debug)]
pub struct MonRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl MonRect {
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
    pub fn right(&self) -> f32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
}

/// All monitors. Tries xrandr first; falls back to a single rect at the origin
/// sized from egui's reported monitor size.
pub fn all(ctx: &egui::Context) -> Vec<MonRect> {
    let xr = from_xrandr();
    if !xr.is_empty() {
        return xr;
    }
    let size = ctx.input(|i| i.viewport().monitor_size).unwrap_or(egui::vec2(1920.0, 1080.0));
    vec![MonRect { x: 0.0, y: 0.0, w: size.x, h: size.y }]
}

/// Parse `xrandr --query` lines like `DP-1 connected 2560x1440+1920+0 ...`.
fn from_xrandr() -> Vec<MonRect> {
    let out = match Command::new("xrandr").arg("--query").output() {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut rects = Vec::new();
    for line in text.lines() {
        if !line.contains(" connected") {
            continue;
        }
        for tok in line.split_whitespace() {
            if let Some(r) = parse_geom(tok) {
                rects.push(r);
                break;
            }
        }
    }
    rects
}

/// Parse a `WxH+X+Y` geometry token.
fn parse_geom(tok: &str) -> Option<MonRect> {
    // expected form: 2560x1440+1920+0
    let (wh, rest) = tok.split_once('x')?;
    let w: f32 = wh.parse().ok()?;
    let plus = rest.find('+')?;
    let h: f32 = rest[..plus].parse().ok()?;
    let coords = &rest[plus + 1..]; // "X+Y"
    let (xs, ys) = coords.split_once('+')?;
    let x: f32 = xs.parse().ok()?;
    let y: f32 = ys.parse().ok()?;
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    Some(MonRect { x, y, w, h })
}

/// The monitor whose rect contains the point, else the nearest by center, else
/// the first. Used to clamp the pet and place the panel.
pub fn monitor_at(mons: &[MonRect], px: f32, py: f32) -> MonRect {
    if let Some(m) = mons.iter().find(|m| m.contains(px, py)) {
        return *m;
    }
    mons.iter()
        .min_by(|a, b| {
            let da = (a.x + a.w / 2.0 - px).powi(2) + (a.y + a.h / 2.0 - py).powi(2);
            let db = (b.x + b.w / 2.0 - px).powi(2) + (b.y + b.h / 2.0 - py).powi(2);
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
        })
        .copied()
        .unwrap_or(MonRect { x: 0.0, y: 0.0, w: 1920.0, h: 1080.0 })
}
