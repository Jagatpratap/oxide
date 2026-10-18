//! Oxide Hub (Unity Hub): installs Oxide editor versions from the release list, lists your
//! projects with the version each one uses, creates new projects and opens existing ones.
//!
//! ```text
//! oxide-hub                 the Hub window
//! oxide-hub --version       prints the Hub's version
//! ```
//! Environment: `OXIDE_RELEASES_URL` (release list URL or file), `OXIDE_EDITORS_DIR`
//! (where editors are installed).

// No console window behind the Hub on Windows.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod installs;
mod manifest;
mod net;
mod projects;

fn icon() -> Option<egui::IconData> {
    let img = image::load_from_memory(include_bytes!("../assets/icon.png")).ok()?;
    let img = img
        .resize(256, 256, image::imageops::FilterType::Lanczos3)
        .to_rgba8();
    Some(egui::IconData {
        width: img.width(),
        height: img.height(),
        rgba: img.into_raw(),
    })
}

fn main() -> std::process::ExitCode {
    if let Some(a) = std::env::args().nth(1) {
        if matches!(a.as_str(), "-V" | "--version") {
            println!("oxide-hub {}", env!("CARGO_PKG_VERSION"));
            return std::process::ExitCode::SUCCESS;
        }
        eprintln!("usage: oxide-hub [--version]");
        return std::process::ExitCode::from(2);
    }
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Oxide Hub")
        .with_inner_size([1100.0, 680.0])
        .with_min_inner_size([820.0, 500.0]);
    if let Some(i) = icon() {
        viewport = viewport.with_icon(i);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    match eframe::run_native(
        "Oxide Hub",
        options,
        Box::new(|cc| Ok(Box::new(app::HubApp::new(&cc.egui_ctx)))),
    ) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("oxide-hub: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
