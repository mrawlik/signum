//! The egui application: toolbar, page view, stamp interaction, saving.

use std::collections::HashMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};

use eframe::egui::{self, Color32, Key, Modifiers, Pos2, Rect, Sense, Vec2};

use crate::render::Renderer;
use crate::stamp::{self, Stamp, Svg};
use crate::view::{self, PageGeom};
use crate::write::{self, Placement};

const KEY_SIGNATURE: &str = "signature_svg";
const KEY_PLACE: &str = "place";
const PAGE_GAP: f32 = 12.0;
/// Resize factor per wheel notch / arrow key press.
const RESIZE_STEP: f64 = 1.1;
/// egui reports roughly this many points of scroll per wheel notch.
const POINTS_PER_NOTCH: f32 = 50.0;

pub struct SignumApp {
    doc: Option<OpenDoc>,
    signature: Option<Rc<Svg>>,
    place: String,
    /// Free text to stamp.
    text: String,
    /// The last generated date stamp, reused while its text is unchanged.
    date: Option<(String, Rc<Svg>)>,
    mode: Mode,
    hovered: Option<usize>,
    /// Whether the placing preview was over a page last frame (so the wheel resizes it).
    preview_shown: bool,
    drag: Option<Drag>,
    error: Option<String>,
    /// A dropped PDF waiting for the user to confirm discarding unsaved stamps.
    pending_open: Option<PathBuf>,
    /// An open file dialog, running on its own thread.
    picker: Option<Receiver<Picked>>,
}

enum Picked {
    Pdf(PathBuf),
    Signature(PathBuf),
}

enum Mode {
    Idle,
    /// A stamp follows the pointer; `width` (points) is adjustable before the click.
    Placing { svg: Rc<Svg>, width: f64 },
}

struct Drag {
    stamp: usize,
    /// Pointer position minus the stamp's screen centre at drag start.
    grab: Vec2,
}

struct OpenDoc {
    path: PathBuf,
    bytes: Arc<Vec<u8>>,
    pages: Vec<PageGeom>,
    stamps: Vec<Stamp>,
    renderer: Renderer,
    /// Rendered page textures with the pixel scale they were rendered at.
    textures: HashMap<usize, (f32, egui::TextureHandle)>,
    /// Pixel scale of the outstanding render request per page.
    pending: HashMap<usize, f32>,
    /// Why this file cannot be saved, if it cannot.
    read_only: Option<String>,
}

#[derive(Clone, Copy)]
enum SaveTarget {
    InPlace,
    Copy,
}

impl SignumApp {
    pub fn new(cc: &eframe::CreationContext<'_>, path: Option<PathBuf>) -> Self {
        let storage = cc.storage;
        let signature = storage
            .and_then(|s| s.get_string(KEY_SIGNATURE))
            .and_then(|src| Svg::parse(src).ok());
        let place = storage
            .and_then(|s| s.get_string(KEY_PLACE))
            .unwrap_or_default();
        let mut app = Self {
            doc: None,
            signature,
            place,
            text: String::new(),
            date: None,
            mode: Mode::Idle,
            hovered: None,
            preview_shown: false,
            drag: None,
            error: None,
            pending_open: None,
            picker: None,
        };
        if let Some(path) = path {
            app.open(&cc.egui_ctx, path);
        }
        app
    }

    fn open(&mut self, ctx: &egui::Context, path: PathBuf) {
        match std::fs::read(&path) {
            Ok(bytes) => self.open_bytes(ctx, path, bytes),
            Err(e) => self.error = Some(format!("Cannot read {}: {e}", path.display())),
        }
    }

    fn open_bytes(&mut self, ctx: &egui::Context, path: PathBuf, bytes: Vec<u8>) {
        let bytes = Arc::new(bytes);
        let pages = match hayro::hayro_syntax::Pdf::new(bytes.clone()) {
            Ok(pdf) => view::page_geoms(&pdf),
            Err(e) => {
                self.error = Some(format!("Cannot open {}: {e:?}", path.display()));
                return;
            }
        };
        if pages.is_empty() {
            self.error = Some(format!("{} has no pages", path.display()));
            return;
        }
        let read_only = match lopdf::Document::load_mem(&bytes) {
            Ok(d) if d.is_encrypted() => Some("encrypted PDFs cannot be saved".to_owned()),
            Ok(_) => None,
            Err(e) => Some(format!("this PDF can be viewed but not saved ({e})")),
        };
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!("{name} — signum")));
        self.doc = Some(OpenDoc {
            renderer: Renderer::new(bytes.clone(), ctx.clone()),
            path,
            bytes,
            pages,
            stamps: Vec::new(),
            textures: HashMap::new(),
            pending: HashMap::new(),
            read_only,
        });
        self.hovered = None;
        self.drag = None;
        self.mode = Mode::Idle;
    }

    fn set_signature(&mut self, path: &Path) {
        let svg = std::fs::read_to_string(path)
            .map_err(|e| e.to_string())
            .and_then(Svg::parse);
        match svg {
            Ok(svg) => self.signature = Some(svg),
            Err(e) => self.error = Some(format!("Cannot use {} as signature: {e}", path.display())),
        }
    }

    fn date_svg(&mut self) -> Option<Rc<Svg>> {
        let text = stamp::date_text(&self.place, stamp::today());
        if let Some((t, svg)) = &self.date
            && *t == text
        {
            return Some(svg.clone());
        }
        match stamp::text_svg(&text) {
            Ok(svg) => {
                self.date = Some((text, svg.clone()));
                Some(svg)
            }
            Err(e) => {
                self.error = Some(format!("Cannot make date stamp: {e}"));
                None
            }
        }
    }

    fn save(&mut self, ctx: &egui::Context, target: SaveTarget) {
        let Some(doc) = &self.doc else { return };
        if let Some(why) = &doc.read_only {
            self.error = Some(format!("Cannot save: {why}"));
            return;
        }
        if doc.stamps.is_empty() && matches!(target, SaveTarget::InPlace) {
            return;
        }
        let placements: Vec<_> = doc
            .stamps
            .iter()
            .map(|s| {
                let geom = &doc.pages[s.page];
                Placement {
                    page: s.page,
                    svg: &s.svg,
                    matrix: geom.stamp_matrix(geom.stamp_rect(s)),
                }
            })
            .collect();
        let path = match target {
            SaveTarget::InPlace => doc.path.clone(),
            SaveTarget::Copy => signed_path(&doc.path),
        };
        let result = write::write_stamps(&doc.bytes, &placements)
            .and_then(|out| atomic_write(&path, &out).map(|()| out));
        match result {
            // Saved stamps are now page content; show the file as it is on disk.
            Ok(out) => self.open_bytes(ctx, path, out),
            Err(e) => self.error = Some(format!("Cannot save {}: {e}", path.display())),
        }
    }

    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        for path in dropped.iter().map(|f| f.path().to_path_buf()) {
            let ext = path
                .extension()
                .map(|e| e.to_ascii_lowercase())
                .unwrap_or_default();
            if ext == "pdf" {
                self.open_or_confirm(ctx, path);
            } else if ext == "svg" {
                self.set_signature(&path);
            } else {
                self.error = Some(format!("Drop a .pdf to open or an .svg signature, not {}", path.display()));
            }
        }
    }

    fn open_or_confirm(&mut self, ctx: &egui::Context, path: PathBuf) {
        if self.doc.as_ref().is_some_and(|d| !d.stamps.is_empty()) {
            self.pending_open = Some(path);
        } else {
            self.open(ctx, path);
        }
    }

    /// Shows a native file dialog without blocking the UI.
    fn pick(&mut self, ctx: &egui::Context, signature: bool) {
        if self.picker.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let (title, name, ext) = if signature {
                ("Choose signature", "SVG image", "svg")
            } else {
                ("Open PDF", "PDF document", "pdf")
            };
            let picked = rfd::FileDialog::new().set_title(title).add_filter(name, &[ext]).pick_file();
            if let Some(path) = picked {
                let _ = tx.send(if signature { Picked::Signature(path) } else { Picked::Pdf(path) });
            }
            // Wake the UI even on cancel, so it notices the dialog is gone.
            drop(tx);
            ctx.request_repaint();
        });
        self.picker = Some(rx);
    }

    fn poll_picker(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.picker else { return };
        match rx.try_recv() {
            Ok(Picked::Pdf(path)) => {
                self.picker = None;
                self.open_or_confirm(ctx, path);
            }
            Ok(Picked::Signature(path)) => {
                self.picker = None;
                self.set_signature(&path);
            }
            Err(mpsc::TryRecvError::Disconnected) => self.picker = None,
            Err(mpsc::TryRecvError::Empty) => {}
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let idle = self.picker.is_none();
            if ui
                .add_enabled(idle, egui::Button::new("🗁"))
                .on_hover_text("Open a PDF")
                .clicked()
            {
                self.pick(ui.ctx(), false);
            }
            let has_doc = self.doc.is_some();
            // "Sign" and its ⚙ are drawn as one split button: no gap, inner corners square.
            let (sig, set_sig) = ui
                .scope(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    let r = ui.visuals().widgets.inactive.corner_radius;
                    let sig = ui
                        .add_enabled(
                            has_doc && self.signature.is_some(),
                            egui::Button::new("Sign")
                                .corner_radius(egui::CornerRadius { ne: 0, se: 0, ..r }),
                        )
                        .on_disabled_hover_text("Set your signature first");
                    let set_sig = ui
                        .add_enabled(
                            idle,
                            egui::Button::new("⚙")
                                .corner_radius(egui::CornerRadius { nw: 0, sw: 0, ..r }),
                        )
                        .on_hover_text("Set signature: choose an .svg file (or drop it on the window)");
                    (sig, set_sig)
                })
                .inner;
            if sig.clicked()
                && let Some(svg) = &self.signature
            {
                self.mode = Mode::Placing { svg: svg.clone(), width: stamp::SIGNATURE_WIDTH };
            }
            if set_sig.clicked() {
                self.pick(ui.ctx(), true);
            }
            let date = input_with_button(ui, &mut self.place, "Place", 70.0, has_doc, "Date");
            if date.clicked()
                && let Some(svg) = self.date_svg()
            {
                let width = svg.natural_width();
                self.mode = Mode::Placing { svg, width };
            }
            for (label, src, hover) in [
                ("X", stamp::CROSS_SVG, "Place a cross"),
                ("✔", stamp::CHECK_SVG, "Place a checkmark"),
            ] {
                if ui
                    .add_enabled(has_doc, egui::Button::new(label))
                    .on_hover_text(hover)
                    .clicked()
                    && let Ok(svg) = Svg::parse(src.to_owned())
                {
                    let width = svg.natural_width();
                    self.mode = Mode::Placing { svg, width };
                }
            }
            let has_text = !self.text.trim().is_empty();
            let place_text =
                input_with_button(ui, &mut self.text, "Text", 140.0, has_doc && has_text, "🗛")
                    .on_hover_text("Place this text");
            if place_text.clicked() {
                match stamp::text_svg(self.text.trim()) {
                    Ok(svg) => {
                        let width = svg.natural_width();
                        self.mode = Mode::Placing { svg, width };
                    }
                    Err(e) => self.error = Some(format!("Cannot make text stamp: {e}")),
                }
            }
            let can_save = self.doc.as_ref().is_some_and(|d| d.read_only.is_none());
            if ui.add_enabled(can_save, egui::Button::new("Save copy")).clicked() {
                self.save(ui.ctx(), SaveTarget::Copy);
            }
            if let Some(doc) = &self.doc {
                ui.separator();
                let name = doc.path.file_name().unwrap_or_default().to_string_lossy();
                ui.label(name);
                if !doc.stamps.is_empty() {
                    ui.label(egui::RichText::new("●").color(ui.visuals().warn_fg_color))
                        .on_hover_text("Unsaved stamps — Ctrl+S to save");
                }
                if let Some(why) = &doc.read_only {
                    ui.label(egui::RichText::new("read-only").weak()).on_hover_text(why);
                }
            }
        });
    }

    fn banners(&mut self, ui: &mut egui::Ui) {
        if let Some(path) = self.pending_open.clone() {
            let n = self.doc.as_ref().map_or(0, |d| d.stamps.len());
            ui.horizontal(|ui| {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    format!("Discard {n} unsaved stamp(s) and open {}?", path.display()),
                );
                if ui.button("Discard").clicked() {
                    self.pending_open = None;
                    self.open(ui.ctx(), path);
                }
                if ui.button("Cancel").clicked() {
                    self.pending_open = None;
                }
            });
        }
        if let Some(error) = &self.error {
            let mut dismiss = false;
            ui.horizontal(|ui| {
                ui.colored_label(ui.visuals().error_fg_color, error);
                dismiss = ui.small_button("✕").clicked();
            });
            if dismiss {
                self.error = None;
            }
        }
    }

    /// Wheel and keys on the hovered stamp or the placing preview, read before the scroll
    /// area can consume them.
    fn stamp_resize_input(&mut self, ctx: &egui::Context) {
        let Some(doc) = &mut self.doc else { return };
        match &mut self.mode {
            Mode::Placing { width, .. } => {
                if self.preview_shown {
                    *width = resized(*width, take_resize_notches(ctx));
                }
            }
            Mode::Idle => {
                let Some(i) = self.hovered.filter(|&i| i < doc.stamps.len()) else {
                    return;
                };
                let delete = ctx.input_mut(|inp| {
                    inp.consume_key(Modifiers::NONE, Key::Delete)
                        | inp.consume_key(Modifiers::NONE, Key::Backspace)
                });
                if delete {
                    doc.stamps.remove(i);
                    self.hovered = None;
                    return;
                }
                let s = &mut doc.stamps[i];
                s.width = resized(s.width, take_resize_notches(ctx));
            }
        }
    }

    fn pages_ui(&mut self, ui: &mut egui::Ui) {
        let Some(doc) = &mut self.doc else { return };
        let ctx = ui.ctx().clone();

        for r in doc.renderer.poll() {
            if doc.pending.get(&r.page) == Some(&r.scale) {
                doc.pending.remove(&r.page);
            }
            let tex = ctx.load_texture(format!("page{}", r.page), r.image, egui::TextureOptions::LINEAR);
            doc.textures.insert(r.page, (r.scale, tex));
        }

        // Layout: pages stacked vertically, all scaled so the widest fits the width.
        let width = ui.available_width();
        let widest = doc.pages.iter().map(|g| g.size.width).fold(1.0, f64::max) as f32;
        let zoom = ((width - 2.0 * PAGE_GAP) / widest).max(0.05);
        let page_rects: Vec<Rect> = {
            let mut y = PAGE_GAP;
            doc.pages
                .iter()
                .map(|g| {
                    let size = Vec2::new(g.size.width as f32, g.size.height as f32) * zoom;
                    let r = Rect::from_min_size(Pos2::new((width - size.x) / 2.0, y), size);
                    y += size.y + PAGE_GAP;
                    r
                })
                .collect()
        };
        let height = page_rects.last().map_or(0.0, |r| r.max.y) + PAGE_GAP;
        let (area, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click_and_drag());
        let page_rects: Vec<Rect> = page_rects.iter().map(|r| r.translate(area.min.to_vec2())).collect();
        let to_screen = |page: usize, p: kurbo::Point| {
            page_rects[page].min + Vec2::new(p.x as f32, p.y as f32) * zoom
        };
        let to_view = |page: usize, pos: Pos2| {
            let v = (pos - page_rects[page].min) / zoom;
            kurbo::Point::new(f64::from(v.x), f64::from(v.y))
        };
        let page_at = |pos: Pos2| page_rects.iter().position(|r| r.contains(pos));
        let stamp_screen_rect = |s: &Stamp| {
            let r = doc.pages[s.page].stamp_rect(s);
            Rect::from_min_max(
                to_screen(s.page, r.origin()),
                to_screen(s.page, kurbo::Point::new(r.x1, r.y1)),
            )
        };

        // Pages: draw the visible ones, request renders, evict far-away textures.
        let painter = ui.painter_at(area);
        let clip = ui.clip_rect();
        let keep = clip.expand2(Vec2::new(0.0, clip.height() * 5.0));
        let near = clip.expand2(Vec2::new(0.0, clip.height()));
        let target_scale = zoom * ctx.pixels_per_point();
        for (i, r) in page_rects.iter().enumerate() {
            if !keep.intersects(*r) {
                doc.textures.remove(&i);
                continue;
            }
            if !near.intersects(*r) {
                continue;
            }
            painter.rect_filled(*r, 0.0, Color32::WHITE);
            let fresh = |s: f32| (s / target_scale - 1.0).abs() < 0.1;
            if let Some((_, tex)) = doc.textures.get(&i) {
                let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
                painter.image(tex.id(), *r, uv, Color32::WHITE);
            }
            let have = doc.textures.get(&i).is_some_and(|(s, _)| fresh(*s));
            let asked = doc.pending.get(&i).is_some_and(|s| fresh(*s));
            if !have && !asked {
                doc.renderer.request(i, target_scale);
                doc.pending.insert(i, target_scale);
            }
            painter.rect_stroke(*r, 0.0, (1.0, Color32::from_gray(160)), egui::StrokeKind::Outside);
        }

        // Interaction.
        let pointer = response.hover_pos().or(ctx.pointer_latest_pos().filter(|_| self.drag.is_some()));
        self.preview_shown = false;
        match &self.mode {
            Mode::Placing { svg, width } => {
                ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
                self.hovered = None;
                if let Some(pos) = pointer
                    && let Some(page) = page_at(pos)
                {
                    self.preview_shown = true;
                    let stamp = Stamp {
                        svg: svg.clone(),
                        page,
                        center: doc.pages[page].to_user(to_view(page, pos)),
                        width: *width,
                    };
                    if response.clicked() {
                        doc.stamps.push(stamp);
                        if !ui.input(|i| i.modifiers.shift) {
                            self.mode = Mode::Idle;
                        }
                    } else {
                        draw_stamp(&painter, &ctx, &stamp, stamp_screen_rect(&stamp), 0.6);
                    }
                }
                if response.secondary_clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                    self.mode = Mode::Idle;
                }
            }
            Mode::Idle => {
                if let Some(drag) = &self.drag {
                    ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
                    if let Some(pos) = pointer {
                        let center = pos - drag.grab;
                        if let Some(page) = page_at(center) {
                            let s = &mut doc.stamps[drag.stamp];
                            s.page = page;
                            s.center = doc.pages[page].to_user(to_view(page, center));
                        }
                    }
                    if !response.dragged() {
                        self.drag = None;
                    }
                } else {
                    self.hovered = pointer.and_then(|pos| {
                        doc.stamps.iter().rposition(|s| stamp_screen_rect(s).contains(pos))
                    });
                    if let Some(i) = self.hovered {
                        ctx.set_cursor_icon(egui::CursorIcon::Grab);
                        if response.drag_started()
                            && let Some(pos) = response.interact_pointer_pos()
                        {
                            let grab = pos - stamp_screen_rect(&doc.stamps[i]).center();
                            self.drag = Some(Drag { stamp: i, grab });
                        } else if response.secondary_clicked() {
                            doc.stamps.remove(i);
                            self.hovered = None;
                        }
                    }
                }
            }
        }

        let active = self.drag.as_ref().map(|d| d.stamp).or(self.hovered);
        for (i, s) in doc.stamps.iter().enumerate() {
            let rect = stamp_screen_rect(s);
            if !near.intersects(rect) {
                continue;
            }
            draw_stamp(&painter, &ctx, s, rect, 1.0);
            if active == Some(i) {
                let accent = ui.visuals().selection.stroke.color;
                painter.rect_stroke(rect.expand(2.0), 2.0, (1.0, accent), egui::StrokeKind::Outside);
            }
        }
    }
}

/// Wheel notches (up = positive) plus Up/Down key presses, consumed from this frame's input.
/// A text field and a button drawn as one control: no gap, the facing corners square.
/// Returns the button's response.
fn input_with_button(
    ui: &mut egui::Ui,
    text: &mut String,
    hint: &str,
    width: f32,
    enabled: bool,
    label: &str,
) -> egui::Response {
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let r = ui.visuals().widgets.inactive.corner_radius;
        let w = &mut ui.visuals_mut().widgets;
        for v in [&mut w.inactive, &mut w.hovered, &mut w.active] {
            v.corner_radius.ne = 0;
            v.corner_radius.se = 0;
        }
        ui.add(egui::TextEdit::singleline(text).hint_text(hint).desired_width(width));
        ui.add_enabled(
            enabled,
            egui::Button::new(label).corner_radius(egui::CornerRadius { nw: 0, sw: 0, ..r }),
        )
    })
    .inner
}

fn take_resize_notches(ctx: &egui::Context) -> f64 {
    let (scroll, up, down) = ctx.input_mut(|inp| {
        (
            std::mem::take(&mut inp.smooth_scroll_delta).y,
            inp.consume_key(Modifiers::NONE, Key::ArrowUp),
            inp.consume_key(Modifiers::NONE, Key::ArrowDown),
        )
    });
    f64::from(scroll / POINTS_PER_NOTCH) + f64::from(up) - f64::from(down)
}

fn resized(width: f64, notches: f64) -> f64 {
    (width * RESIZE_STEP.powf(notches)).clamp(5.0, 2000.0)
}

fn draw_stamp(painter: &egui::Painter, ctx: &egui::Context, stamp: &Stamp, rect: Rect, opacity: f32) {
    let ppp = ctx.pixels_per_point();
    let px = [(rect.width() * ppp).round() as usize, (rect.height() * ppp).round() as usize];
    if let Some(tex) = stamp.svg.preview(ctx, px) {
        let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
        painter.image(tex.id(), rect, uv, Color32::WHITE.gamma_multiply(opacity));
    }
}

fn signed_path(path: &Path) -> PathBuf {
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    path.with_file_name(format!("{stem}.signed.pdf"))
}

/// Writes `bytes` next to `path`, then renames over it, so a crash never leaves a half-written file.
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let tmp = path.with_file_name(format!(".{name}.signum-tmp"));
    let result = (|| {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result.map_err(|e| e.to_string())
}

impl eframe::App for SignumApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.handle_dropped_files(&ctx);
        self.poll_picker(&ctx);
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::S)) {
            self.save(&ctx, SaveTarget::InPlace);
        }
        self.stamp_resize_input(&ctx);

        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.add_space(4.0);
            self.toolbar(ui);
            self.banners(ui);
            ui.add_space(2.0);
        });
        egui::CentralPanel::default().show(ui, |ui| {
            if self.doc.is_none() {
                ui.centered_and_justified(|ui| {
                    let sig = if self.signature.is_some() {
                        ""
                    } else {
                        "\nUse ⚙ next to “Sign” to choose your signature .svg."
                    };
                    ui.label(format!("Use 🗁, drop a PDF here, or run `signum file.pdf`.{sig}"));
                });
                return;
            }
            egui::ScrollArea::vertical()
                .auto_shrink(false)
                .show(ui, |ui| self.pages_ui(ui));
        });
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if let Some(svg) = &self.signature {
            storage.set_string(KEY_SIGNATURE, svg.source.clone());
        }
        storage.set_string(KEY_PLACE, self.place.clone());
    }
}
