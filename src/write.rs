//! Appends stamps to a PDF as an incremental update. The original bytes are never modified.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;

use lopdf::{Dictionary, Document, IncrementalDocument, Object, ObjectId, Stream};
use pdf_writer::{Pdf, Ref};
use resvg::usvg;

use crate::stamp::Svg;

/// One stamp to draw: `matrix` maps the SVG's 1x1 pt unit box into page user space.
pub struct Placement<'a> {
    pub page: usize,
    pub svg: &'a Svg,
    pub matrix: [f64; 6],
}

pub fn write_stamps(bytes: &[u8], placements: &[Placement]) -> Result<Vec<u8>, String> {
    let prev = Document::load_mem(bytes).map_err(|e| format!("cannot parse PDF: {e}"))?;
    if prev.is_encrypted() {
        return Err("encrypted PDFs are not supported".to_owned());
    }
    let pages = prev.get_pages();
    let mut inc = IncrementalDocument::create_from(bytes.to_vec(), prev);

    // Each distinct SVG becomes one Form XObject, shared by all its placements.
    let mut xobjects: HashMap<*const Svg, ObjectId> = HashMap::new();
    let mut by_page: BTreeMap<usize, Vec<(ObjectId, [f64; 6])>> = BTreeMap::new();
    for p in placements {
        let id = match xobjects.get(&std::ptr::from_ref(p.svg)) {
            Some(&id) => id,
            None => {
                let id = import_svg(&mut inc.new_document, &p.svg.tree)?;
                xobjects.insert(std::ptr::from_ref(p.svg), id);
                id
            }
        };
        by_page.entry(p.page).or_default().push((id, p.matrix));
    }

    for (page, items) in by_page {
        let page_id = *pages
            .get(&(page as u32 + 1))
            .ok_or_else(|| format!("page {} does not exist", page + 1))?;
        stamp_page(&mut inc, page_id, &items).map_err(|e| format!("page {}: {e}", page + 1))?;
    }

    let mut out = Vec::with_capacity(bytes.len() + 16 * 1024);
    inc.save_to(&mut out).map_err(|e| format!("cannot write PDF: {e}"))?;
    Ok(out)
}

/// Converts `tree` with svg2pdf and copies the resulting objects into `doc`.
/// Returns the id of the root Form XObject.
fn import_svg(doc: &mut Document, tree: &usvg::Tree) -> Result<ObjectId, String> {
    let (chunk, root) = svg2pdf::to_chunk(tree, svg2pdf::ConversionOptions::default())
        .map_err(|e| format!("cannot convert SVG: {e}"))?;

    // Move the chunk's ids above everything already in the document.
    let offset = doc.max_id as i32;
    let mut max = offset;
    let chunk = chunk.renumber(|r| {
        let r = Ref::new(r.get() + offset);
        max = max.max(r.get());
        r
    });

    // lopdf parses whole files, not loose objects, so wrap the chunk in a minimal PDF.
    let catalog = Ref::new(max + 1);
    let page_tree = Ref::new(max + 2);
    let mut pdf = Pdf::new();
    pdf.catalog(catalog).pages(page_tree);
    pdf.pages(page_tree).kids(std::iter::empty()).count(0);
    pdf.extend(&chunk);
    let mini = Document::load_mem(&pdf.finish()).map_err(|e| format!("cannot import SVG: {e}"))?;

    for (id, object) in mini.objects {
        if (offset + 1..=max).contains(&(id.0 as i32)) {
            doc.objects.insert(id, object);
        }
    }
    doc.max_id = max as u32;
    Ok(((root.get() + offset) as u32, 0))
}

/// Replaces the page in the update: resources gain the XObjects, contents gain the stamps.
fn stamp_page(
    inc: &mut IncrementalDocument,
    page_id: ObjectId,
    items: &[(ObjectId, [f64; 6])],
) -> lopdf::Result<()> {
    let prev = inc.get_prev_documents();
    let mut page = prev.get_dictionary(page_id)?.clone();

    // Resources may be inherited from the page tree; copy them onto the page so adding
    // our XObjects does not shadow the inherited fonts and images.
    let mut resources = match inherited(prev, &page, b"Resources") {
        Some(o) => resolve_dict(prev, o)?,
        None => Dictionary::new(),
    };
    let mut xobject_dict = match resources.get(b"XObject") {
        Ok(o) => resolve_dict(prev, o)?,
        Err(_) => Dictionary::new(),
    };

    let mut ops = String::from("Q\n");
    let mut n = 0;
    for (xobject, m) in items {
        let name = loop {
            let name = format!("SgN{n}");
            n += 1;
            if !xobject_dict.has(name.as_bytes()) {
                break name;
            }
        };
        xobject_dict.set(name.as_bytes(), Object::Reference(*xobject));
        let [a, b, c, d, e, f] = m.map(num);
        writeln!(ops, "q {a} {b} {c} {d} {e} {f} cm /{name} Do Q").unwrap();
    }
    resources.set("XObject", xobject_dict);
    page.set("Resources", resources);

    // Wrap the original content in q ... Q so whatever graphics state it leaves behind
    // (an unbalanced cm, a clip) does not apply to the stamps.
    let mut contents = match page.get(b"Contents") {
        Ok(o) => content_streams(prev, o)?,
        Err(_) => Vec::new(),
    };
    let open = inc
        .new_document
        .add_object(Stream::new(Dictionary::new(), b"q\n".to_vec()));
    let stamps = inc
        .new_document
        .add_object(Stream::new(Dictionary::new(), ops.into_bytes()));
    contents.insert(0, Object::Reference(open));
    contents.push(Object::Reference(stamps));
    page.set("Contents", contents);

    inc.new_document.set_object(page_id, page);
    Ok(())
}

/// Looks `key` up on the page, then on its `/Pages` ancestors.
fn inherited<'a>(doc: &'a Document, page: &'a Dictionary, key: &[u8]) -> Option<&'a Object> {
    let mut dict = page;
    for _ in 0..64 {
        if let Ok(value) = dict.get(key) {
            return Some(value);
        }
        let parent = dict.get(b"Parent").ok()?.as_reference().ok()?;
        dict = doc.get_dictionary(parent).ok()?;
    }
    None
}

fn resolve_dict(doc: &Document, object: &Object) -> lopdf::Result<Dictionary> {
    match object {
        Object::Reference(id) => resolve_dict(doc, doc.get_object(*id)?),
        o => Ok(o.as_dict()?.clone()),
    }
}

/// `/Contents` as a list of stream references (it may be one stream or an array of them).
fn content_streams(doc: &Document, contents: &Object) -> lopdf::Result<Vec<Object>> {
    match contents {
        Object::Array(items) => Ok(items.clone()),
        Object::Reference(id) => match doc.get_object(*id)? {
            Object::Array(items) => Ok(items.clone()),
            _ => Ok(vec![contents.clone()]),
        },
        _ => Ok(Vec::new()),
    }
}

/// Compact PDF number: at most 4 decimals, no trailing zeros.
fn num(x: f64) -> String {
    let s = format!("{x:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".to_owned() } else { s.to_owned() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::PageGeom;
    use kurbo::Rect;
    use pdf_writer::{Content, Name};

    const RED_SQUARE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10" width="10" height="10"><rect width="10" height="10" fill="red"/></svg>"#;

    /// A page whose geometry exercises everything that can go wrong.
    struct Fixture {
        rotate: i32,
        crop: Option<[f32; 4]>,
        inherit_resources: bool,
        /// Content that leaves the CTM scaled without restoring it.
        dirty_ctm: bool,
    }

    fn make_pdf(f: &Fixture) -> Vec<u8> {
        let (catalog, tree, page, content, font) =
            (Ref::new(1), Ref::new(2), Ref::new(3), Ref::new(4), Ref::new(5));
        let mut pdf = Pdf::new();
        pdf.catalog(catalog).pages(tree);
        {
            let mut pages = pdf.pages(tree);
            pages.kids([page]).count(1);
            if f.inherit_resources {
                pages.resources().fonts().pair(Name(b"F1"), font);
            }
        }
        {
            let mut p = pdf.page(page);
            p.parent(tree)
                .media_box(pdf_writer::Rect::new(0.0, 0.0, 400.0, 600.0))
                .contents(content)
                .rotate(f.rotate);
            if let Some([a, b, c, d]) = f.crop {
                p.crop_box(pdf_writer::Rect::new(a, b, c, d));
            }
            if !f.inherit_resources {
                p.resources().fonts().pair(Name(b"F1"), font);
            }
        }
        pdf.type1_font(font).base_font(Name(b"Helvetica"));
        let mut c = Content::new();
        if f.dirty_ctm {
            c.transform([0.5, 0.0, 0.0, 0.5, 50.0, 50.0]);
        }
        c.begin_text().set_font(Name(b"F1"), 12.0).next_line(20.0, 20.0);
        c.show(pdf_writer::Str(b"hello")).end_text();
        pdf.stream(content, &c.finish());
        pdf.finish()
    }

    fn render(bytes: &[u8]) -> (PageGeom, hayro::vello_cpu::Pixmap) {
        let pdf = hayro::hayro_syntax::Pdf::new(bytes.to_vec()).unwrap();
        let page = &pdf.pages()[0];
        let pixmap = hayro::render(
            page,
            &hayro::RenderCache::new(),
            &Default::default(),
            &Default::default(),
            &hayro::PixmapSettings {
                bg_color: hayro::vello_cpu::color::palette::css::WHITE,
                ..Default::default()
            },
        );
        (PageGeom::of(page), pixmap)
    }

    fn pixel(pm: &hayro::vello_cpu::Pixmap, x: f64, y: f64) -> [u8; 4] {
        let p = pm.data()[y as usize * pm.width() as usize + x as usize];
        [p.r, p.g, p.b, p.a]
    }

    fn is_red(px: [u8; 4]) -> bool {
        px[0] > 200 && px[1] < 60 && px[2] < 60
    }

    /// Stamps a red square at a view-space rect and checks it renders exactly there.
    fn check(f: Fixture) {
        check_bytes(&make_pdf(&f));
    }

    fn check_bytes(original: &[u8]) {
        let (geom, _) = render(original);
        let rect = Rect::new(60.0, 80.0, 120.0, 140.0);
        let svg = Svg::parse(RED_SQUARE.to_owned()).unwrap();
        let out = write_stamps(
            original,
            &[Placement { page: 0, svg: &svg, matrix: geom.stamp_matrix(rect) }],
        )
        .unwrap();
        // For checking the output with external tools (qpdf --check, mutool, Acrobat).
        if let Ok(dir) = std::env::var("SIGNUM_DUMP_DIR") {
            let name = format!("{:016x}.pdf", fxhash(&out));
            std::fs::write(std::path::Path::new(&dir).join(name), &out).unwrap();
        }

        assert_eq!(&out[..original.len()], original, "original bytes modified");
        Document::load_mem(&out).expect("lopdf re-parses output");

        let (geom2, pm) = render(&out);
        assert_eq!(geom2.size, geom.size);
        assert!(is_red(pixel(&pm, rect.center().x, rect.center().y)));
        assert!(is_red(pixel(&pm, rect.x0 + 2.0, rect.y0 + 2.0)));
        assert!(!is_red(pixel(&pm, rect.x0 - 3.0, rect.y0 - 3.0)));
        assert!(!is_red(pixel(&pm, rect.x1 + 3.0, rect.y1 + 3.0)));
    }

    #[test]
    fn plain_page() {
        check(Fixture { rotate: 0, crop: None, inherit_resources: false, dirty_ctm: false });
    }

    #[test]
    fn rotated_page() {
        for rotate in [90, 180, 270] {
            check(Fixture { rotate, crop: None, inherit_resources: false, dirty_ctm: false });
        }
    }

    #[test]
    fn cropped_rotated_page() {
        check(Fixture {
            rotate: 90,
            crop: Some([30.0, 40.0, 380.0, 500.0]),
            inherit_resources: false,
            dirty_ctm: false,
        });
    }

    #[test]
    fn dirty_ctm() {
        check(Fixture { rotate: 0, crop: None, inherit_resources: false, dirty_ctm: true });
    }

    #[test]
    fn xref_stream_input() {
        let plain = make_pdf(&Fixture { rotate: 90, crop: None, inherit_resources: true, dirty_ctm: false });
        let mut modern = Vec::new();
        Document::load_mem(&plain).unwrap().save_modern(&mut modern).unwrap();
        assert!(modern.windows(5).any(|w| w == b"/XRef"));
        check_bytes(&modern);
    }

    fn fxhash(bytes: &[u8]) -> u64 {
        bytes.iter().fold(0xcbf29ce484222325, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x100000001b3))
    }

    #[test]
    fn inherited_resources_survive() {
        let f = Fixture { rotate: 0, crop: None, inherit_resources: true, dirty_ctm: false };
        let original = make_pdf(&f);
        let svg = Svg::parse(RED_SQUARE.to_owned()).unwrap();
        let out = write_stamps(
            &original,
            &[Placement { page: 0, svg: &svg, matrix: [10.0, 0.0, 0.0, 10.0, 0.0, 0.0] }],
        )
        .unwrap();
        let doc = Document::load_mem(&out).unwrap();
        let page = doc.get_dictionary(doc.get_pages()[&1]).unwrap();
        let resources = page.get(b"Resources").unwrap().as_dict().unwrap();
        assert!(resources.get(b"Font").unwrap().as_dict().unwrap().has(b"F1"));
        assert!(resources.get(b"XObject").unwrap().as_dict().unwrap().has(b"SgN0"));
    }

    #[test]
    fn shared_svg_embedded_once_and_text_survives() {
        let original = make_pdf(&Fixture {
            rotate: 0,
            crop: None,
            inherit_resources: false,
            dirty_ctm: false,
        });
        let date = crate::stamp::text_svg("Zurich, 2026-10-31").unwrap();
        let m = [100.0, 0.0, 0.0, 12.0, 50.0, 300.0];
        let placements: Vec<_> =
            (0..3).map(|_| Placement { page: 0, svg: &date, matrix: m }).collect();
        let once = write_stamps(&original, &placements[..1]).unwrap();
        let thrice = write_stamps(&original, &placements).unwrap();
        // Two extra `Do` lines, not two extra copies of the font.
        assert!(thrice.len() - once.len() < 200, "{} vs {}", thrice.len(), once.len());
    }

    #[test]
    fn num_format() {
        assert_eq!(num(1.0), "1");
        assert_eq!(num(-0.00001), "0");
        assert_eq!(num(12.345678), "12.3457");
        assert_eq!(num(-3.5), "-3.5");
    }
}
