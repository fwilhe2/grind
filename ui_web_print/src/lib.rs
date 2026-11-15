// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The browser shell's paper: PDF export and print preview, in a wasm module of its own —
//! `grind-print`'s font stack and PDF writer are about four megabytes nobody should download to
//! open a spreadsheet. `ui_web` loads this the first time somebody exports or previews, and hands
//! it the document as flat ODF bytes (the `save_bytes`/`open_bytes` boundary every client uses)
//! with the fonts the page fetched. Functions with a host-side twin are tested without a browser;
//! the `#[wasm_bindgen]` wrappers only convert.
//!
//! [`Pages`] is the text pane's page view (`doc/page-model.md` PM7): the document kept typeset,
//! its pages placed in a strip, and every question about where text is answered in the strip's
//! CSS pixels, so the main module positions elements and knows no font.

use std::rc::Rc;

use grind_print::raster::Rasteriser;
use grind_print::view::Shown;
use grind_print::{Fonts, Options, Typesetter, place};
use grind_text::{App, Caret};
use wasm_bindgen::prelude::*;

/// The document's bytes, opened — refused by name when they are not a text document, since the
/// reader is tolerant and would otherwise hand back an empty one (`grind_core::kind`).
fn open(document: &[u8]) -> Result<App, String> {
    if grind_core::kind(document) != Some(grind_core::DocumentKind::Text) {
        return Err("those bytes are not a text document".to_owned());
    }
    let app = App::new();
    app.open_bytes("document.fodt", document)
        .map_err(|error| error.to_string())?;
    Ok(app)
}

/// The faces, from the files the page fetched.
fn fonts(files: Vec<Vec<u8>>) -> Fonts {
    let mut fonts = Fonts::default();
    for bytes in files {
        fonts.add(bytes);
    }
    fonts
}

/// The document as a PDF, and the export's one-sentence report.
pub fn export_bytes(
    document: &[u8],
    title: &str,
    files: Vec<Vec<u8>>,
) -> Result<(Vec<u8>, String), String> {
    let app = open(document)?;
    let options = Options {
        paper: None,
        title: Some(title.to_owned()).filter(|t| !t.is_empty()),
    };
    let (pdf, report) = grind_print::export(&app, fonts(files), &options)?;
    Ok((pdf, report.summary()))
}

/// Every page rasterised at `scale` pixels per point: width, height, premultiplied RGBA.
pub fn preview_pages(
    document: &[u8],
    files: Vec<Vec<u8>>,
    scale: f32,
) -> Result<Vec<(u32, u32, Vec<u8>)>, String> {
    let app = open(document)?;
    let (doc, setter) = grind_print::typeset(&app, fonts(files), &Options::default());
    Ok(doc
        .pages
        .iter()
        .map(|page| {
            let raster = grind_print::raster::render(page, setter.fonts(), scale);
            (raster.width, raster.height, raster.rgba)
        })
        .collect())
}

fn files(fonts: &js_sys::Array) -> Vec<Vec<u8>> {
    fonts
        .iter()
        .map(|file| js_sys::Uint8Array::new(&file).to_vec())
        .collect()
}

/// An export, as the page receives it.
#[wasm_bindgen]
pub struct Printed {
    pdf: Vec<u8>,
    summary: String,
}

#[wasm_bindgen]
impl Printed {
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn pdf(&self) -> Vec<u8> {
        self.pdf.clone()
    }

    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn summary(&self) -> String {
        self.summary.clone()
    }
}

/// One previewed page, as the page receives it.
#[wasm_bindgen]
pub struct Sheet {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

#[wasm_bindgen]
impl Sheet {
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn width(&self) -> u32 {
        self.width
    }

    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn height(&self) -> u32 {
        self.height
    }

    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn rgba(&self) -> Vec<u8> {
        self.rgba.clone()
    }
}

/// [`export_bytes`], for the page: `fonts` is an array of `Uint8Array`, one per bundled file.
#[wasm_bindgen]
pub fn export_pdf(document: &[u8], title: &str, fonts: &js_sys::Array) -> Result<Printed, JsValue> {
    export_bytes(document, title, files(fonts))
        .map(|(pdf, summary)| Printed { pdf, summary })
        .map_err(|error| JsValue::from_str(&error))
}

/// [`preview_pages`], for the page.
#[wasm_bindgen]
pub fn preview(document: &[u8], fonts: &js_sys::Array, scale: f32) -> Result<Vec<Sheet>, JsValue> {
    preview_pages(document, files(fonts), scale)
        .map(|pages| {
            pages
                .into_iter()
                .map(|(width, height, rgba)| Sheet {
                    width,
                    height,
                    rgba,
                })
                .collect()
        })
        .map_err(|error| JsValue::from_str(&error))
}

/// The document's pages as the text pane shows them: set from its bytes when it changes, placed
/// in a strip at a zoom and a width, and asked about in the strip's pixels.
pub struct PageView {
    setter: Typesetter,
    shown: Shown,
    rasteriser: Rasteriser,
}

impl PageView {
    #[must_use]
    pub fn new(files: Vec<Vec<u8>>) -> Self {
        let setter = Typesetter::new(fonts(files));
        let set = grind_print::text::typeset(
            &App::new(),
            &setter,
            &grind_print::text::Options::default(),
        );
        PageView {
            setter,
            shown: Shown::new(Rc::new(set), 1.0, 0.0),
            rasteriser: Rasteriser::default(),
        }
    }

    /// Set the document again from its bytes. Answers the pages that differ from before, new
    /// ones included — what has to be drawn again.
    pub fn set(&mut self, document: &[u8]) -> Result<Vec<u32>, String> {
        let app = open(document)?;
        let set =
            grind_print::text::typeset(&app, &self.setter, &grind_print::text::Options::default());
        let damaged = set
            .pages
            .iter()
            .enumerate()
            .filter(|(at, page)| self.shown.set.pages.get(*at) != Some(page))
            .map(|(at, _)| at as u32)
            .collect();
        self.shown = Shown::new(
            Rc::new(set),
            self.shown.strip.scale(),
            self.shown.strip.size().0,
        );
        Ok(damaged)
    }

    /// Place the pages at `scale` pixels per point in a view `width` pixels wide.
    pub fn place(&mut self, scale: f64, width: f64) {
        self.shown = Shown::new(self.shown.set.clone(), scale, width);
    }

    #[must_use]
    pub fn count(&self) -> usize {
        self.shown.set.pages.len()
    }

    /// The strip's width and height.
    #[must_use]
    pub fn size(&self) -> [f64; 2] {
        let (width, height) = self.shown.strip.size();
        [width, height]
    }

    /// Page `page`'s rectangle in the strip: x, y, width, height.
    #[must_use]
    pub fn sheet(&self, page: usize) -> Option<[f64; 4]> {
        self.shown.sheet(page)
    }

    /// The pages any part of which is between `top` and `bottom`.
    #[must_use]
    pub fn visible(&self, top: f64, bottom: f64) -> [usize; 2] {
        let range = self.shown.visible(top, bottom);
        [range.start, range.end]
    }

    /// Page `page` drawn at `scale` pixels per point: width, height and premultiplied RGBA.
    pub fn render(&mut self, page: usize, scale: f32) -> Option<(u32, u32, Vec<u8>)> {
        let page = self.shown.set.pages.get(page)?;
        let raster = self.rasteriser.render(page, self.setter.fonts(), scale);
        Some((raster.width, raster.height, raster.rgba))
    }

    /// The caret's stroke in the strip: x, top, height — and its x on its page, in points,
    /// which is what a run of Down presses aims for.
    #[must_use]
    pub fn spot(&self, at: Caret) -> Option<[f64; 4]> {
        let stroke = self.shown.stroke(at)?;
        Some([stroke.x, stroke.y, stroke.height, f64::from(stroke.page_x)])
    }

    /// The caret a point of the strip means: on the page under it or the nearer of two, and on
    /// it the nearest line.
    #[must_use]
    pub fn hit(&self, x: f64, y: f64) -> Option<Caret> {
        self.shown.hit(x, y)
    }

    /// `delta` lines from `at`, as near the page x `goal` as each line allows.
    #[must_use]
    pub fn step(&self, at: Caret, delta: isize, goal: f32) -> Caret {
        place::step(&self.shown.set, at, delta, goal)
    }

    /// The two ends of the caret's own line.
    #[must_use]
    pub fn bounds(&self, at: Caret) -> Option<(Caret, Caret)> {
        place::line_bounds(&self.shown.set, at)
    }

    /// The text between two carets on the pages between `top` and `bottom` of the strip, one
    /// rectangle per line: x, y, width, height after each other.
    #[must_use]
    pub fn bands(&self, from: Caret, to: Caret, top: f64, bottom: f64) -> Vec<f64> {
        let pages = self.shown.visible(top, bottom);
        self.shown
            .bands(from, to, pages)
            .into_iter()
            .flatten()
            .collect()
    }

    /// The blocks with a line on the pages between `top` and `bottom` of the strip.
    #[must_use]
    pub fn blocks(&self, top: f64, bottom: f64) -> Option<[usize; 2]> {
        let range = self.shown.blocks(self.shown.visible(top, bottom))?;
        Some([range.start, range.end])
    }
}

/// [`PageView`], for the page. Carets cross as `[block, offset]`.
#[wasm_bindgen]
pub struct Pages(PageView);

fn caret(block: u32, offset: u32) -> Caret {
    Caret {
        block: block as usize,
        offset: offset as usize,
    }
}

fn pair(at: Caret) -> Vec<u32> {
    vec![at.block as u32, at.offset as u32]
}

#[wasm_bindgen]
impl Pages {
    #[wasm_bindgen(constructor)]
    #[must_use]
    pub fn new(fonts: &js_sys::Array) -> Pages {
        Pages(PageView::new(files(fonts)))
    }

    /// The pages that changed, which the page draws again.
    pub fn set(&mut self, document: &[u8]) -> Result<Vec<u32>, JsValue> {
        self.0
            .set(document)
            .map_err(|error| JsValue::from_str(&error))
    }

    pub fn place(&mut self, scale: f64, width: f64) {
        self.0.place(scale, width);
    }

    #[must_use]
    pub fn count(&self) -> u32 {
        self.0.count() as u32
    }

    #[must_use]
    pub fn size(&self) -> Vec<f64> {
        self.0.size().to_vec()
    }

    #[must_use]
    pub fn sheet(&self, page: u32) -> Option<Vec<f64>> {
        self.0.sheet(page as usize).map(|sheet| sheet.to_vec())
    }

    #[must_use]
    pub fn visible(&self, top: f64, bottom: f64) -> Vec<u32> {
        self.0.visible(top, bottom).map(|at| at as u32).to_vec()
    }

    pub fn render(&mut self, page: u32, scale: f32) -> Option<Sheet> {
        let (width, height, rgba) = self.0.render(page as usize, scale)?;
        Some(Sheet {
            width,
            height,
            rgba,
        })
    }

    #[must_use]
    pub fn spot(&self, block: u32, offset: u32) -> Option<Vec<f64>> {
        self.0.spot(caret(block, offset)).map(|spot| spot.to_vec())
    }

    #[must_use]
    pub fn hit(&self, x: f64, y: f64) -> Option<Vec<u32>> {
        self.0.hit(x, y).map(pair)
    }

    #[must_use]
    pub fn step(&self, block: u32, offset: u32, delta: i32, goal: f32) -> Vec<u32> {
        pair(self.0.step(caret(block, offset), delta as isize, goal))
    }

    #[must_use]
    pub fn bounds(&self, block: u32, offset: u32) -> Option<Vec<u32>> {
        let (start, end) = self.0.bounds(caret(block, offset))?;
        Some([pair(start), pair(end)].concat())
    }

    #[must_use]
    pub fn bands(&self, from: Vec<u32>, to: Vec<u32>, top: f64, bottom: f64) -> Vec<f64> {
        let at = |pair: &[u32]| {
            caret(
                pair.first().copied().unwrap_or(0),
                pair.get(1).copied().unwrap_or(0),
            )
        };
        self.0.bands(at(&from), at(&to), top, bottom)
    }

    #[must_use]
    pub fn blocks(&self, top: f64, bottom: f64) -> Option<Vec<u32>> {
        self.0
            .blocks(top, bottom)
            .map(|range| range.map(|at| at as u32).to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundled() -> Vec<Vec<u8>> {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../print/fonts");
        grind_print::fonts::BUNDLED_NAMES
            .iter()
            .map(|name| std::fs::read(dir.join(name)).unwrap())
            .collect()
    }

    fn document(paragraphs: &[&str]) -> Vec<u8> {
        let app = App::new();
        for (at, text) in paragraphs.iter().enumerate() {
            app.insert(at, grind_text::BlockKind::Paragraph, text)
                .unwrap();
        }
        app.save_bytes(grind_text::Form::Flat).unwrap()
    }

    #[test]
    fn a_document_arrives_as_bytes_and_leaves_as_a_pdf() {
        let (pdf, summary) =
            export_bytes(&document(&["Hello, paper."]), "note", bundled()).unwrap();
        assert!(pdf.starts_with(b"%PDF-"));
        assert_eq!(summary, "1 page, A4.");
    }

    #[test]
    fn the_preview_is_every_page_at_the_scale_asked_for() {
        let long = "words ".repeat(900);
        let pages = preview_pages(&document(&[&long, &long]), bundled(), 0.5).unwrap();
        assert!(pages.len() >= 2, "{} pages", pages.len());
        let (width, height, rgba) = &pages[0];
        assert_eq!((*width, *height), (297, 420));
        assert_eq!(rgba.len(), (width * height * 4) as usize);
    }

    #[test]
    fn bytes_that_are_no_text_document_are_refused_by_name() {
        let error = export_bytes(b"not a document", "x", bundled()).unwrap_err();
        assert!(error.contains("not a text document"), "{error}");
    }

    #[test]
    fn the_page_view_answers_in_the_strips_pixels_and_damages_only_what_changed() {
        let long = "All work and no play makes a dull document. ".repeat(60);
        let mut view = PageView::new(bundled());
        let first = view.set(&document(&[&long, &long, &long])).unwrap();
        assert!(first.len() >= 2, "every page is drawn: {first:?}");
        view.place(96.0 / 72.0, 1000.0);
        let [x, y, w, h] = view.sheet(0).unwrap();
        assert!(x > 0.0 && y > 0.0 && w > 700.0 && h > 1000.0);
        let at = Caret {
            block: 0,
            offset: 30,
        };
        let [cx, cy, ch, _] = view.spot(at).unwrap();
        assert_eq!(view.hit(cx, cy + ch / 2.0), Some(at));
        let below = view.step(at, 1, 0.0);
        assert!(below > at);
        assert!(view.bounds(at).is_some());
        let bands = view.bands(
            at,
            Caret {
                block: 0,
                offset: 300,
            },
            0.0,
            2000.0,
        );
        assert_eq!(bands.len() % 4, 0);
        assert!(bands.len() >= 8);
        assert_eq!(view.blocks(0.0, 100.0).map(|[from, _]| from), Some(0));
        let (width, height, rgba) = view.render(0, 0.25).unwrap();
        assert_eq!(rgba.len(), (width * height * 4) as usize);

        // The last paragraph grows: the first page is not drawn again.
        let again = view
            .set(&document(&[&long, &long, &format!("{long} More.")]))
            .unwrap();
        assert!(!again.contains(&0), "{again:?}");
        assert_ne!(again, Vec::<u32>::new());
    }
}
