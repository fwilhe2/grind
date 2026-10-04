// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Pages, PDF and print preview for the word processor. `doc/pdf-export.md` is normative here.
//!
//! The font stack lives in this crate and nowhere else: `grind-core` stays font-free
//! (`doc/text-layout.md`, decision 2), and so does any build that leaves this crate out.

pub mod align;
pub mod faces;
pub mod fonts;
pub mod metrics;
pub mod ops;
pub mod pdf;
pub mod raster;
pub mod svg;
pub mod text;

use grind_core::page::PageGeometry;
use grind_text::App;

pub use fonts::Fonts;
pub use metrics::{Substitution, Typesetter};

/// What the caller decides about an export.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// Print on this paper instead of the page the document states ([`text::Options::paper`]).
    pub paper: Option<PageGeometry>,
    /// The PDF's title, shown in a viewer's title bar.
    pub title: Option<String>,
}

/// What an export did — part of the output, not an afterthought, as the xlsx filter's report is.
#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    pub pages: usize,
    /// The paper it was printed on.
    pub page: PageGeometry,
    /// Every family set in another face than the one it named.
    pub substitutions: Vec<Substitution>,
    /// Characters no face had a glyph for, drawn as the face's empty box.
    pub missing_glyphs: usize,
}

impl Report {
    /// The one sentence every client shows after an export.
    pub fn summary(&self) -> String {
        let paper = self.page.iso_name().unwrap_or_else(|| {
            let mm = |v: f64| (v * 10.0).round() / 10.0;
            format!("{} × {} mm", mm(self.page.width), mm(self.page.height))
        });
        let mut out = format!(
            "{} page{}, {paper}.",
            self.pages,
            if self.pages == 1 { "" } else { "s" }
        );
        for sub in &self.substitutions {
            out.push_str(&format!(
                " \u{201c}{}\u{201d} set in {}{}.",
                sub.asked,
                sub.used,
                if sub.compatible {
                    " (same metrics)"
                } else {
                    ""
                }
            ));
        }
        if self.missing_glyphs > 0 {
            out.push_str(&format!(
                " {} character{} had no glyph in any font.",
                self.missing_glyphs,
                if self.missing_glyphs == 1 { "" } else { "s" }
            ));
        }
        out
    }
}

/// Every font family the document's runs name, once each, in the order they first appear. The
/// markdown marker for code (`monospace`) is a generic, not a family to look for.
pub fn families(app: &App) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let viewport = app.get_viewport(0..app.block_count());
    for run in viewport.iter().flat_map(|block| &block.runs) {
        let Some(family) = run.props.font_family.as_deref().map(str::trim) else {
            continue;
        };
        if family.is_empty()
            || family == grind_text::markdown::MONOSPACE
            || out.iter().any(|seen| seen.eq_ignore_ascii_case(family))
        {
            continue;
        }
        out.push(family.to_owned());
    }
    out
}

/// The fonts to set this document in: the bundled faces, and — where the build can look —
/// every installed face of a family the document names or of its metric-compatible twin.
pub fn fonts_for(app: &App) -> Fonts {
    let fonts = Fonts::bundled();
    #[cfg(feature = "system-fonts")]
    let fonts = {
        let mut fonts = fonts;
        let named = families(app);
        let wanted = fonts::wanted(named.iter().map(String::as_str));
        if !wanted.is_empty() {
            fonts.add_families(&Fonts::system_database(), &wanted);
        }
        fonts
    };
    #[cfg(not(feature = "system-fonts"))]
    let _ = app;
    fonts
}

/// Where each page begins and ends: the first character on it and one past the last, `None`
/// for a page with no text. Typeset exactly as [`export`] typesets, so these are the PDF's own
/// page breaks — what `grind text pages` prints and loop G will compare with LibreOffice's.
pub fn pages(
    app: &App,
    fonts: Fonts,
    options: &Options,
) -> Vec<(Option<grind_text::Caret>, Option<grind_text::Caret>)> {
    let setter = Typesetter::new(fonts);
    let doc = text::typeset(
        app,
        &setter,
        &text::Options {
            paper: options.paper,
        },
    );
    doc.pages
        .iter()
        .map(|page| (page.start, page.end))
        .collect()
}

/// The document typeset, every page as a display list, for a client that shows pages — a
/// preview window draws [`raster::render`] of whichever page it is on, and the typesetting is
/// done once rather than per page turned.
pub fn typeset(app: &App, fonts: Fonts, options: &Options) -> (ops::Document, Typesetter) {
    let setter = Typesetter::new(fonts);
    let doc = text::typeset(
        app,
        &setter,
        &text::Options {
            paper: options.paper,
        },
    );
    (doc, setter)
}

/// One page, 0-based, rasterised at `scale` pixels per point — or `Err` naming how many pages
/// there are when there is no such page.
pub fn preview(
    app: &App,
    fonts: Fonts,
    options: &Options,
    page: usize,
    scale: f32,
) -> Result<raster::Raster, String> {
    let (doc, setter) = typeset(app, fonts, options);
    let count = doc.pages.len();
    let page = doc.pages.get(page).ok_or_else(|| {
        format!(
            "there is no page {}: the document is {count} page{}",
            page + 1,
            if count == 1 { "" } else { "s" }
        )
    })?;
    Ok(raster::render(page, setter.fonts(), scale))
}

/// Typeset `app`'s document and write it as a PDF, in `fonts`.
pub fn export(app: &App, fonts: Fonts, options: &Options) -> Result<(Vec<u8>, Report), String> {
    let setter = Typesetter::new(fonts);
    let page = options.paper.or(app.page()).unwrap_or_default();
    let doc = text::typeset(app, &setter, &text::Options { paper: Some(page) });
    let missing_glyphs = doc
        .pages
        .iter()
        .flat_map(|page| &page.ops)
        .map(|op| match op {
            // Glyph 0 is `.notdef` in every OpenType face: the box a face draws for a character
            // it has no glyph for.
            ops::Op::Text { glyphs, .. } => glyphs.iter().filter(|g| g.id == 0).count(),
            _ => 0,
        })
        .sum();
    let metadata = pdf::Metadata {
        title: options.title.clone(),
    };
    let bytes = pdf::write(&doc, setter.fonts(), &metadata)?;
    let report = Report {
        pages: doc.pages.len(),
        page,
        substitutions: setter.substitutions(),
        missing_glyphs,
    };
    Ok((bytes, report))
}

#[cfg(test)]
mod tests {
    use super::*;
    use grind_text::{BlockKind, Caret, CharStyle};

    #[test]
    fn an_export_reports_its_pages_and_its_paper() {
        let app = App::new();
        app.insert(0, BlockKind::Paragraph, "Hello").unwrap();
        let (bytes, report) = export(&app, Fonts::bundled(), &Options::default()).unwrap();
        assert!(bytes.starts_with(b"%PDF-"));
        assert_eq!(report.pages, 1);
        assert_eq!(report.page, PageGeometry::a4());
        assert_eq!(report.substitutions, vec![]);
        assert_eq!(report.missing_glyphs, 0);
        assert_eq!(report.summary(), "1 page, A4.");
    }

    #[test]
    fn a_substitution_and_a_missing_glyph_are_both_in_the_summary() {
        let app = App::new();
        app.insert(0, BlockKind::Paragraph, "Arial and 漢字")
            .unwrap();
        let arial = CharStyle {
            font_family: Some("Arial".into()),
            ..CharStyle::default()
        };
        app.set_char_style(
            Caret {
                block: 0,
                offset: 0,
            },
            Caret {
                block: 0,
                offset: 5,
            },
            &arial,
        )
        .unwrap();
        let options = Options {
            paper: PageGeometry::paper("a5"),
            ..Options::default()
        };
        let (_, report) = export(&app, Fonts::bundled(), &options).unwrap();
        assert_eq!(report.missing_glyphs, 2);
        assert_eq!(
            report.summary(),
            "1 page, A5. \u{201c}Arial\u{201d} set in Liberation Sans (same metrics). \
             2 characters had no glyph in any font."
        );
    }

    #[test]
    fn the_families_a_document_names_are_listed_once_in_order() {
        let app = App::new();
        app.insert(0, BlockKind::Paragraph, "one two three four")
            .unwrap();
        let family = |name: &str| CharStyle {
            font_family: Some(name.into()),
            ..CharStyle::default()
        };
        let at = |offset| Caret { block: 0, offset };
        app.set_char_style(at(0), at(3), &family("Georgia"))
            .unwrap();
        app.set_char_style(at(4), at(7), &family("monospace"))
            .unwrap();
        app.set_char_style(at(8), at(13), &family("Calibri"))
            .unwrap();
        app.set_char_style(at(14), at(18), &family("Georgia"))
            .unwrap();
        assert_eq!(families(&app), vec!["Georgia", "Calibri"]);
    }

    #[test]
    fn a_page_of_no_iso_size_is_given_in_millimetres() {
        let report = Report {
            pages: 3,
            page: PageGeometry {
                width: 215.9,
                height: 279.4,
                ..PageGeometry::a4()
            },
            substitutions: vec![Substitution {
                asked: "Fancy".into(),
                used: "Liberation Serif".into(),
                compatible: false,
            }],
            missing_glyphs: 1,
        };
        assert_eq!(
            report.summary(),
            "3 pages, 215.9 × 279.4 mm. \u{201c}Fancy\u{201d} set in Liberation Serif. \
             1 character had no glyph in any font."
        );
    }
}
