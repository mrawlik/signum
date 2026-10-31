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
