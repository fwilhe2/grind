// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Screen = PDF (`doc/page-model.md`): every page of every vendored document, as the
//! windows draw it (`raster::render`), against `hayro`'s rendering of the PDF written from the same
//! pages. Two rasterisers antialias differently, and the screen darkens small text
//! (`raster::contrast`), so the claim is that almost every inked pixel agrees, not every one; a
//! picture, which each resamples its own way, is held to its mean colour.

use grind_print::text::{Options, typeset};
use grind_print::{Fonts, Typesetter, raster};
use grind_text::App;

/// How many of a page's inked pixels may be far apart: one in a hundred, and five on a page with
/// hardly any ink, leaves room for antialiasing and none for a glyph, a rule or a picture out
/// of place.
fn agrees(name: &str, app: &App) {
    if let Some(why) = apart(name, app) {
        panic!("{why}");
    }
}

/// The first page of `app` whose screen and PDF disagree, said; `None` when none does.
fn apart(name: &str, app: &App) -> Option<String> {
    let setter = Typesetter::new(Fonts::bundled());
    let doc = typeset(app, &setter, &Options::default());
    let bytes =
        grind_print::pdf::write(&doc, setter.fonts(), &grind_print::pdf::Metadata::default())
            .unwrap();
    let pdf = hayro::hayro_syntax::Pdf::new(bytes).unwrap();
    if pdf.pages().len() != doc.pages.len() {
        return Some(format!(
            "{name}: {} pages in the PDF, {} on screen",
            pdf.pages().len(),
            doc.pages.len()
        ));
    }
    for (number, page) in doc.pages.iter().enumerate() {
        let ours = raster::render(page, setter.fonts(), 1.0);
        let theirs = hayro::render(
            &pdf.pages()[number],
            &hayro::RenderCache::new(),
            &hayro::hayro_interpret::InterpreterSettings::default(),
            // Outline against outline, as the preview draws them.
            &hayro::RenderSettings {
                hinting: false,
                glyph_caching: false,
                ..hayro::RenderSettings::default()
            },
            &hayro::PixmapSettings {
                bg_color: hayro::vello_cpu::color::palette::css::WHITE,
                ..hayro::PixmapSettings::default()
            },
        );
        if (u32::from(theirs.width()), u32::from(theirs.height())) != (ours.width, ours.height) {
            return Some(format!("{name}: page {} is another size", number + 1));
        }
        let theirs = theirs.data_as_u8_slice();
        let (w, h) = (ours.width as usize, ours.height as usize);
        // A picture is resampled by each rasteriser its own way: inside its box, only its mean
        // colour is held to agree — enough to catch one missing or out of place.
        let mut picture = vec![false; w * h];
        for op in &page.ops {
            let grind_print::ops::Op::Image {
                x,
                y,
                width,
                height,
                ..
            } = op
            else {
                continue;
            };
            let (x0, y0) = ((x.max(0.0) as usize).min(w), (y.max(0.0) as usize).min(h));
            let (x1, y1) = (
                ((x + width).ceil().max(0.0) as usize).min(w),
                ((y + height).ceil().max(0.0) as usize).min(h),
            );
            let (mut a, mut b, mut n) = ([0u64; 3], [0u64; 3], 0u64);
            for row in y0..y1 {
                for column in x0..x1 {
                    let i = row * w + column;
                    picture[i] = true;
                    for c in 0..3 {
                        a[c] += u64::from(ours.rgba[i * 4 + c]);
                        b[c] += u64::from(theirs[i * 4 + c]);
                    }
                    n += 1;
                }
            }
            // A box of a few pixels is all edge, where antialiasing is all there is.
            if n >= 64 && (0..3).any(|c| (a[c] / n).abs_diff(b[c] / n) > 24) {
                return Some(format!(
                    "{name}: page {}: a picture at {x}, {y} is another colour",
                    number + 1
                ));
            }
        }
        let pixels = w * h;
        let apart = (0..pixels)
            .filter(|&i| !picture[i])
            .filter(|&i| (0..3).any(|c| ours.rgba[i * 4 + c].abs_diff(theirs[i * 4 + c]) > 96))
            .count();
        let inked = (0..pixels)
            .filter(|&i| !picture[i])
            .filter(|&i| ours.rgba[i * 4..i * 4 + 3].iter().any(|&c| c < 200))
            .count();
        if apart * 100 > inked.max(500) {
            return Some(format!(
                "{name}: page {}: {apart} pixels apart of {inked} inked",
                number + 1
            ));
        }
    }
    None
}

#[test]
fn every_vendored_page_on_screen_is_its_page_in_the_pdf() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../text/tests/data");
    let mut seen = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        let app = App::new();
        if app.open_file(&path).is_ok() {
            agrees(&path.display().to_string(), &app);
            seen += 1;
        }
    }
    assert!(seen >= 3, "the vendored documents are there: {seen}");
}

#[test]
fn every_corpus_page_on_screen_is_its_page_in_the_pdf() {
    let Some(root) = std::env::var_os("GRIND_LO_CORPUS") else {
        eprintln!("GRIND_LO_CORPUS not set: skipping the corpus");
        return;
    };
    let (mut seen, mut failed) = (0, Vec::new());
    let mut stack = vec![std::path::PathBuf::from(root)];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|ext| ext == "odt") {
                let app = App::new();
                if app.open_file(&path).is_ok() {
                    failed.extend(apart(&path.display().to_string(), &app));
                    seen += 1;
                }
            }
        }
    }
    eprintln!("{seen} documents, {} apart", failed.len());
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}
