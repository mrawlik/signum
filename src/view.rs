//! Page geometry and the user space <-> view space mapping.
//!
//! View space: points, y down, origin at the top-left of the visible (cropped, rotated) page.

use hayro::hayro_interpret::util::TransformExt;
use kurbo::{Affine, Point, Rect, Size};

use crate::stamp::Stamp;

#[derive(Clone, Copy, Debug)]
pub struct PageGeom {
    /// Visible page size in points, after CropBox and `/Rotate`.
    pub size: Size,
    /// User space -> view space.
    pub to_view: Affine,
}

impl PageGeom {
    pub fn of(page: &hayro::hayro_syntax::page::Page<'_>) -> Self {
        let (w, h) = page.render_dimensions();
        Self {
            size: Size::new(f64::from(w), f64::from(h)),
            to_view: page.initial_transform(true).to_kurbo(),
        }
    }

    pub fn to_user(self, view: Point) -> Point {
        self.to_view.inverse() * view
    }

    /// Upright box of the stamp in view space.
    pub fn stamp_rect(&self, stamp: &Stamp) -> Rect {
        Rect::from_center_size(
            self.to_view * stamp.center,
            Size::new(stamp.width, stamp.height()),
        )
    }

    /// `cm` matrix that draws a 1x1 pt (y-up) Form XObject into `view_rect`.
    pub fn stamp_matrix(&self, view_rect: Rect) -> [f64; 6] {
        let unit_to_view = Affine::new([
            view_rect.width(),
            0.0,
            0.0,
            -view_rect.height(),
            view_rect.x0,
            view_rect.y1,
        ]);
        (self.to_view.inverse() * unit_to_view).as_coeffs()
    }
}

/// Geometry of every page of a PDF.
pub fn page_geoms(pdf: &hayro::hayro_syntax::Pdf) -> Vec<PageGeom> {
    pdf.pages().iter().map(PageGeom::of).collect()
}
