//! Stamp SVGs: parsing, date-stamp generation, preview rasterisation.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, LazyLock};

use eframe::egui;
use kurbo::Point;
use resvg::tiny_skia;
use resvg::usvg::{self, fontdb};

const FONT: &[u8] = include_bytes!("../assets/DejaVuSans.ttf");
const FONT_FAMILY: &str = "DejaVu Sans";
const DATE_FONT_SIZE: f64 = 11.0;

/// Default width of a freshly placed signature, in points.
pub const SIGNATURE_WIDTH: f64 = 150.0;

static FONT_DB: LazyLock<Arc<fontdb::Database>> = LazyLock::new(|| {
    let mut db = fontdb::Database::new();
    db.load_font_data(FONT.to_vec());
    // Every generic family resolves to the one bundled font, so output never depends on the OS.
    db.set_sans_serif_family(FONT_FAMILY);
    db.set_serif_family(FONT_FAMILY);
    db.set_monospace_family(FONT_FAMILY);
    db.set_cursive_family(FONT_FAMILY);
    db.set_fantasy_family(FONT_FAMILY);
    Arc::new(db)
});

fn options() -> usvg::Options<'static> {
    usvg::Options {
        font_family: FONT_FAMILY.to_owned(),
        fontdb: FONT_DB.clone(),
        ..Default::default()
    }
}

/// A parsed stamp SVG. Shared between all stamps that show it.
pub struct Svg {
    pub source: String,
    pub tree: usvg::Tree,
    preview: RefCell<Option<([usize; 2], egui::TextureHandle)>>,
}

impl Svg {
    pub fn parse(source: String) -> Result<Rc<Self>, String> {
        let tree = usvg::Tree::from_str(&source, &options()).map_err(|e| e.to_string())?;
        let size = tree.size();
        if size.width() <= 0.0 || size.height() <= 0.0 {
            return Err("SVG has zero size".to_owned());
        }
        Ok(Rc::new(Self {
            source,
            tree,
            preview: RefCell::new(None),
        }))
    }

    /// Width / height.
    pub fn aspect(&self) -> f64 {
        let size = self.tree.size();
        f64::from(size.width()) / f64::from(size.height())
    }

    /// Natural width in points (1 SVG user unit = 1 pt).
    pub fn natural_width(&self) -> f64 {
        f64::from(self.tree.size().width())
    }

    /// Texture of this SVG rasterised at `size` pixels, re-rendered only when the size changes.
    pub fn preview(&self, ctx: &egui::Context, size: [usize; 2]) -> Option<egui::TextureHandle> {
        let size = [size[0].clamp(1, 4096), size[1].clamp(1, 4096)];
        let mut cached = self.preview.borrow_mut();
        if let Some((s, tex)) = cached.as_ref()
            && *s == size
        {
            return Some(tex.clone());
        }
        let mut pixmap = tiny_skia::Pixmap::new(size[0] as u32, size[1] as u32)?;
        let tree_size = self.tree.size();
        let transform = tiny_skia::Transform::from_scale(
            size[0] as f32 / tree_size.width(),
            size[1] as f32 / tree_size.height(),
        );
        resvg::render(&self.tree, transform, &mut pixmap.as_mut());
        let image = egui::ColorImage::from_rgba_premultiplied(size, pixmap.data());
        let tex = ctx.load_texture("stamp", image, egui::TextureOptions::LINEAR);
        *cached = Some((size, tex.clone()));
        Some(tex)
    }
}

/// `2026-10-31` or `Place, 2026-10-31`.
pub fn date_text(place: &str, date: jiff::civil::Date) -> String {
    let place = place.trim();
    if place.is_empty() {
        date.to_string()
    } else {
        format!("{place}, {date}")
    }
}

pub fn today() -> jiff::civil::Date {
    jiff::Zoned::now().date()
}

/// An SVG showing `text`, with its viewBox fitted tightly around the glyphs.
pub fn date_svg(text: &str) -> Result<Rc<Svg>, String> {
    let text = escape_xml(text);
    let make = |view_box: &str, w: f64, h: f64| {
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="{view_box}" width="{w}" height="{h}"><text x="0" y="{DATE_FONT_SIZE}" font-family="{FONT_FAMILY}" font-size="{DATE_FONT_SIZE}" fill="black">{text}</text></svg>"#
        )
    };
    let probe = usvg::Tree::from_str(&make("0 0 10000 100", 10000.0, 100.0), &options())
        .map_err(|e| e.to_string())?;
    let bbox = probe.root().abs_bounding_box();
    let pad = 1.0;
    let (x, y) = (f64::from(bbox.x()) - pad, f64::from(bbox.y()) - pad);
    let (w, h) = (
        f64::from(bbox.width()) + 2.0 * pad,
        f64::from(bbox.height()) + 2.0 * pad,
    );
    Svg::parse(make(&format!("{x} {y} {w} {h}"), w, h))
}

fn escape_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

/// A stamp placed on a page, in PDF user space.
#[derive(Clone)]
pub struct Stamp {
    pub svg: Rc<Svg>,
    pub page: usize,
    pub center: Point,
    /// Visual width in points; height follows from the SVG's aspect ratio.
    pub width: f64,
}

impl Stamp {
    pub fn height(&self) -> f64 {
        self.width / self.svg.aspect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_svg_is_tight() {
        let svg = date_svg("Zürich & <Co>, 2026-10-31").unwrap();
        // 11 pt text: a couple of pt taller than the font size, well over 100 pt wide.
        let size = svg.tree.size();
        assert!((10.0..20.0).contains(&size.height()), "{size:?}");
        assert!((100.0..300.0).contains(&size.width()), "{size:?}");
    }

    #[test]
    fn date_text_formats() {
        let d = jiff::civil::date(2026, 10, 31);
        assert_eq!(date_text("", d), "2026-10-31");
        assert_eq!(date_text(" Zurich ", d), "Zurich, 2026-10-31");
    }
}
