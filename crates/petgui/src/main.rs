//! cc-pet — desktop pet GUI (egui/eframe). Successor to pet.py.

mod app;
mod art;
mod monitors;
mod panel;
mod pet;
mod theme;

use app::App;

fn main() -> eframe::Result<()> {
    // Headless self-check of the sprite pipeline (GIF decode + de-matte).
    if std::env::args().any(|a| a == "--art-check") {
        let frames = art::find_art(110);
        if frames.is_empty() {
            println!("art-check: no art found (placeholder mascot would be drawn)");
        } else {
            let s = frames[0].image.size;
            let opaque = frames[0]
                .image
                .pixels
                .iter()
                .filter(|p| p.a() > 0)
                .count();
            println!(
                "art-check: {} frame(s), first {}x{} px, {} opaque px, delay {}ms",
                frames.len(),
                s[0],
                s[1],
                opaque,
                frames[0].delay_ms
            );
        }
        return Ok(());
    }

    if std::env::var("DISPLAY").is_err() && std::env::var("WAYLAND_DISPLAY").is_err() {
        eprintln!("cc-pet: no display found (need X11/Wayland).");
        std::process::exit(1);
    }
    if std::env::var("WAYLAND_DISPLAY").is_ok() && std::env::var("DISPLAY").is_err() {
        eprintln!("cc-pet: warning — Wayland restricts window positioning; the pet is designed for X11.");
    }

    let viewport = egui::ViewportBuilder::default()
        .with_title("cc-pet")
        .with_decorations(false)
        .with_transparent(true)
        .with_always_on_top()
        .with_resizable(false)
        .with_taskbar(false)
        .with_inner_size([110.0, 110.0])
        .with_position([1500.0, 60.0]);

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native("cc-pet", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}
