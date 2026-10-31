#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod render;
mod stamp;
mod view;
mod write;

use std::path::PathBuf;

use eframe::egui;

fn main() -> eframe::Result {
    let path = std::env::args_os().nth(1).map(PathBuf::from);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("signum")
            .with_inner_size([900.0, 1000.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        "signum",
        options,
        Box::new(|cc| Ok(Box::new(app::SignumApp::new(cc, path)))),
    )
}
