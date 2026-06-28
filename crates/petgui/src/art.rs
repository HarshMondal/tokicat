//! Sprite loading: GIF/PNG/JPEG/WebP -> scaled, edge-de-matted egui frames.
//! Ports `find_art`, `_fit`, and `dematte` from pet.py.

use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

use image::{AnimationDecoder, ImageBuffer, Rgba};

pub struct Frame {
    pub image: egui::ColorImage,
    pub delay_ms: u32,
}

const TRIM_DARK: u8 = 70;
const TRIM_ITERS: usize = 2;

/// Locate a pet image in assets/, searching dev and installed layouts.
pub fn find_art(max: u32) -> Vec<Frame> {
    let Some(path) = locate() else {
        return Vec::new();
    };
    let is_gif = path.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case("gif")).unwrap_or(false);
    let frames = if is_gif { load_gif(&path, max) } else { load_static(&path, max) };
    frames.unwrap_or_default()
}

/// Candidate `assets/` directories, searching dev and installed layouts.
pub fn asset_roots() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(env) = std::env::var("CC_PET_ASSETS") {
        roots.push(PathBuf::from(env));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            roots.push(dir.join("assets"));
            // target/<profile>/cc-pet -> repo root
            roots.push(dir.join("../../assets"));
        }
    }
    // compile-time repo location (dev convenience)
    roots.push(PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets")));
    roots.push(PathBuf::from("assets"));
    roots
}

fn locate() -> Option<PathBuf> {
    for root in asset_roots() {
        for name in ["pet.gif", "pet.png", "pet.jpg", "pet.jpeg", "pet.webp"] {
            let p = root.join(name);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

fn load_gif(path: &PathBuf, max: u32) -> Option<Vec<Frame>> {
    let file = File::open(path).ok()?;
    let decoder = image::codecs::gif::GifDecoder::new(BufReader::new(file)).ok()?;
    let frames = decoder.into_frames().collect_frames().ok()?;
    let mut out = Vec::new();
    for frame in frames {
        let (num, den) = frame.delay().numer_denom_ms();
        let delay_ms = if den == 0 { 100 } else { (num / den).max(20) };
        let buf = frame.into_buffer();
        out.push(Frame { image: process(buf, max), delay_ms });
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn load_static(path: &PathBuf, max: u32) -> Option<Vec<Frame>> {
    let img = image::open(path).ok()?.to_rgba8();
    Some(vec![Frame { image: process(img, max), delay_ms: 1000 }])
}

/// Scale to <= max on the long edge, de-matte the dark edge halo, to ColorImage.
fn process(buf: ImageBuffer<Rgba<u8>, Vec<u8>>, max: u32) -> egui::ColorImage {
    let (w, h) = (buf.width(), buf.height());
    let buf = if w.max(h) > max {
        let scale = max as f32 / w.max(h) as f32;
        let nw = (w as f32 * scale).round().max(1.0) as u32;
        let nh = (h as f32 * scale).round().max(1.0) as u32;
        image::imageops::resize(&buf, nw, nh, image::imageops::FilterType::Triangle)
    } else {
        buf
    };
    let (w, h) = (buf.width() as usize, buf.height() as usize);
    let mut px = buf.into_raw(); // RGBA8
    dematte(&mut px, w, h);
    egui::ColorImage::from_rgba_unmultiplied([w, h], &px)
}

/// Erode the dark edge halo: dark pixels touching transparency become transparent,
/// `TRIM_ITERS` layers deep, without holing the interior. Port of `dematte`.
fn dematte(px: &mut [u8], w: usize, h: usize) {
    let alpha = |px: &[u8], x: isize, y: isize| -> u8 {
        if x >= 0 && (x as usize) < w && y >= 0 && (y as usize) < h {
            px[(y as usize * w + x as usize) * 4 + 3]
        } else {
            0
        }
    };
    for _ in 0..TRIM_ITERS {
        let mut clear: Vec<usize> = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) * 4;
                if px[i + 3] == 0 {
                    continue;
                }
                if px[i] < TRIM_DARK && px[i + 1] < TRIM_DARK && px[i + 2] < TRIM_DARK {
                    let (xi, yi) = (x as isize, y as isize);
                    let touches = alpha(px, xi - 1, yi) < 128
                        || alpha(px, xi + 1, yi) < 128
                        || alpha(px, xi, yi - 1) < 128
                        || alpha(px, xi, yi + 1) < 128
                        || alpha(px, xi - 1, yi - 1) < 128
                        || alpha(px, xi + 1, yi - 1) < 128
                        || alpha(px, xi - 1, yi + 1) < 128
                        || alpha(px, xi + 1, yi + 1) < 128;
                    if touches {
                        clear.push(i + 3);
                    }
                }
            }
        }
        if clear.is_empty() {
            break;
        }
        for i in clear {
            px[i] = 0;
        }
    }
}
