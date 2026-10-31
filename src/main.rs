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
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "signum",
        options,
        Box::new(|cc| {
            #[cfg(target_os = "linux")]
            name_x11_window(cc);
            Ok(Box::new(app::SignumApp::new(cc, path)))
        }),
    )
}

/// Tells the window manager which desktop file this window belongs to.
///
/// egui-winit sets WM_CLASS to ("", app id). KWin finds a window's desktop file through
/// _KDE_NET_WM_DESKTOP_FILE, or else by matching the WM_CLASS instance against StartupWMClass.
/// With neither, KWin cannot tell that the window is the app it is launching, and its launch
/// feedback keeps bouncing until it times out. eframe keeps the window unmapped until the
/// first frame, so these properties are in place before the window manager first reads them.
#[cfg(target_os = "linux")]
fn name_x11_window(cc: &eframe::CreationContext<'_>) {
    use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _, PropMode};
    use x11rb::wrapper::ConnectionExt as _;

    let Ok(handle) = cc.window_handle() else { return };
    let window = match handle.as_raw() {
        RawWindowHandle::Xlib(h) => h.window as u32,
        RawWindowHandle::Xcb(h) => h.window.get(),
        _ => return,
    };
    // Best effort: without these the app still works, only the launch feedback lingers.
    let _ = (|| -> Result<(), Box<dyn std::error::Error>> {
        let (conn, _) = x11rb::connect(None)?;
        let desktop_file = conn.intern_atom(false, b"_KDE_NET_WM_DESKTOP_FILE")?.reply()?.atom;
        let utf8 = conn.intern_atom(false, b"UTF8_STRING")?.reply()?.atom;
        conn.change_property8(PropMode::REPLACE, window, desktop_file, utf8, APP_ID.as_bytes())?;
        let class = format!("{APP_ID}\0{APP_ID}\0");
        conn.change_property8(
            PropMode::REPLACE,
            window,
            AtomEnum::WM_CLASS,
            AtomEnum::STRING,
            class.as_bytes(),
        )?;
        conn.sync()?;
        Ok(())
    })();
}
