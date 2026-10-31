#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod render;
mod stamp;
mod view;
mod write;

use std::path::PathBuf;

use eframe::egui;

/// Matches the `.desktop` file and icon names, so desktops can pair the window with them.
const APP_ID: &str = "io.github.mrawlik.signum";

fn main() -> eframe::Result {
    let path = std::env::args_os().nth(1).map(PathBuf::from);
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../packaging/linux/icons/256.png"))
        .expect("bundled icon is a valid PNG");
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("signum")
            .with_app_id(APP_ID)
            .with_icon(icon)
            .with_inner_size([900.0, 1000.0])
            .with_drag_and_drop(true),
        // eframe would otherwise name the settings folder after APP_ID; keep the old one.
        persistence_path: eframe::storage_dir("signum").map(|dir| dir.join("app.ron")),
        event_loop_builder: Some(Box::new(|_builder| {
            // winit cannot receive file drops on native Wayland; XWayland can.
            #[cfg(target_os = "linux")]
            if std::env::var_os("DISPLAY").is_some() {
                use winit::platform::x11::EventLoopBuilderExtX11;
                _builder.with_x11();
            }
        })),
        ..Default::default()
    };
    eframe::run_native(
        "signum",
        options,
        Box::new(|cc| Ok(Box::new(app::SignumApp::new(cc, path)))),
    )
}
