// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The browser shell's paper: PDF export and print preview, in a wasm module of its own.
//!
//! `grind-print` is the font stack and a PDF writer, and in the browser that is about four
//! megabytes nobody should download to open a spreadsheet. So `ui_web` loads this module the
//! first time somebody exports or previews (`doc/pdf-export.md` §7), and hands it **the document
//! as flat ODF bytes** — the boundary every other client already uses (`App::save_bytes`,
//! `App::open_bytes`), so nothing about the document has to be shared between two modules — and
//! the bundled fonts the page fetched.
//!
//! The functions with a host-side twin are tested here with no browser; the `#[wasm_bindgen]`
//! wrappers only convert.

use grind_print::{Fonts, Options};
use grind_text::App;
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
    pub fn pdf(&self) -> Vec<u8> {
        self.pdf.clone()
    }

    #[wasm_bindgen(getter)]
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
    pub fn width(&self) -> u32 {
        self.width
    }

    #[wasm_bindgen(getter)]
    pub fn height(&self) -> u32 {
        self.height
    }

    #[wasm_bindgen(getter)]
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
}
