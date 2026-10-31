//! Background page rendering with hayro.

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use eframe::egui;

pub struct Request {
    pub page: usize,
    pub scale: f32,
}

pub struct Rendered {
    pub page: usize,
    pub scale: f32,
    pub image: egui::ColorImage,
}

/// Owns a worker thread with its own `hayro::Pdf`; the thread exits when this is dropped.
pub struct Renderer {
    requests: Sender<Request>,
    results: Receiver<Rendered>,
}

impl Renderer {
    pub fn new(bytes: Arc<Vec<u8>>, ctx: egui::Context) -> Self {
        let (requests, request_rx) = mpsc::channel::<Request>();
        let (result_tx, results) = mpsc::channel();
        thread::spawn(move || {
            let Ok(pdf) = hayro::hayro_syntax::Pdf::new(bytes) else {
                return;
            };
            let pages = pdf.pages();
            let cache = hayro::RenderCache::new();
            while let Ok(first) = request_rx.recv() {
                // Serve the newest requests first: they are for what is on screen now.
                let mut batch = vec![first];
                batch.extend(request_rx.try_iter());
                for req in batch.into_iter().rev() {
                    let Some(page) = pages.get(req.page) else {
                        continue;
                    };
                    let pixmap = hayro::render(
                        page,
                        &cache,
                        &Default::default(),
                        &Default::default(),
                        &hayro::PixmapSettings {
                            x_scale: req.scale,
                            y_scale: req.scale,
                            bg_color: hayro::vello_cpu::color::palette::css::WHITE,
                        },
                    );
                    let pixels = pixmap
                        .data()
                        .iter()
                        .map(|p| egui::Color32::from_rgba_premultiplied(p.r, p.g, p.b, p.a))
                        .collect();
                    let size = [pixmap.width() as usize, pixmap.height() as usize];
                    let image = egui::ColorImage::new(size, pixels);
                    let rendered = Rendered { page: req.page, scale: req.scale, image };
                    if result_tx.send(rendered).is_err() {
                        return;
                    }
                    ctx.request_repaint();
                }
            }
        });
        Self { requests, results }
    }

    pub fn request(&self, page: usize, scale: f32) {
        let _ = self.requests.send(Request { page, scale });
    }

    pub fn poll(&self) -> impl Iterator<Item = Rendered> + '_ {
        self.results.try_iter()
    }
}
