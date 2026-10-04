// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The real [`Metrics`]: text measured by shaping it in the font file the PDF will embed.
//!
//! **One engine measures and draws.** `doc/windows-shell.md` W5a learned it from bugs: measure
//! with one engine and draw with another, and the caret lands where the ink is not. Here the
//! "caret" is every glyph on the page, so [`Typesetter::shape`] is what both
//! [`Metrics::advances`] and the PDF call, with the same face, the same size and the same text.
//!
//! The unit is the **PostScript point**, a PDF's own, so a page's geometry and a line's width are
//! in one unit with no conversion in between.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::ops::Range;

use grind_core::layout::Metrics;
use grind_core::style::{TextStyle, length_mm};

use crate::fonts::{FaceId, Fonts, Match, Resolved};

/// The size a run gets when it states none: Writer's default body text.
pub const DEFAULT_SIZE: f32 = 12.0;

/// One shaped glyph, in points.
#[derive(Clone, Debug, PartialEq)]
pub struct Glyph {
    pub id: u32,
    pub x_advance: f32,
    pub x_offset: f32,
    pub y_offset: f32,
    /// The bytes of the shaped text this glyph came from — its cluster — which is what a PDF's
    /// `ToUnicode` needs to make the glyph copy as the text it stands for.
    pub text: Range<usize>,
}

/// A piece of text set in one face at one size.
#[derive(Clone, Debug, PartialEq)]
pub struct Shaped {
    pub face: FaceId,
    pub size: f32,
    pub glyphs: Vec<Glyph>,
}

/// A family that was not set in the face it named — a line of the export's report.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Substitution {
    /// What the document asked for; `None` when a run named no family at all is not a
    /// substitution, so this is always a name.
    pub asked: String,
    /// The family it was set in.
    pub used: String,
    /// Whether the two share their metrics, so lines break where they would have.
    pub compatible: bool,
}

/// A family as a run asks for it: the name, bold, italic.
type Asked = (Option<String>, bool, bool);

/// Shapes text in the faces of a [`Fonts`], and remembers what it had to substitute.
pub struct Typesetter {
    fonts: Fonts,
    shapers: RefCell<HashMap<FaceId, harfrust::ShaperData>>,
    resolved: RefCell<BTreeMap<Asked, Resolved>>,
}

impl Typesetter {
    pub fn new(fonts: Fonts) -> Self {
        Typesetter {
            fonts,
            shapers: RefCell::new(HashMap::new()),
            resolved: RefCell::new(BTreeMap::new()),
        }
    }

    pub fn fonts(&self) -> &Fonts {
        &self.fonts
    }

    /// Which face and size `style` sets text in. Remembered, so the report can say afterwards
    /// what was substituted.
    pub fn face_of(&self, style: &TextStyle) -> (FaceId, f32) {
        let bold = style.font_weight.as_deref().is_some_and(is_bold);
        let italic = matches!(style.font_style.as_deref(), Some("italic" | "oblique"));
        let key = (style.font_family.clone(), bold, italic);
        let face = self
            .resolved
            .borrow_mut()
            .entry(key)
            .or_insert_with(|| {
                self.fonts
                    .resolve(style.font_family.as_deref(), bold, italic)
            })
            .face;
        (face, size_of(style))
    }

    /// Shape `text` as `style` sets it.
    pub fn shape(&self, text: &str, style: &TextStyle) -> Shaped {
        let (face, size) = self.face_of(style);
        let mut glyphs = Vec::new();
        let bytes = self.fonts.face(face).bytes();
        let index = self.fonts.face(face).index;
        if let Ok(font) = harfrust::FontRef::from_index(bytes, index)
            && !text.is_empty()
        {
            let mut shapers = self.shapers.borrow_mut();
            let data = shapers
                .entry(face)
                .or_insert_with(|| harfrust::ShaperData::new(&font));
            let shaper = data.shaper(&font).build();
            let mut buffer = harfrust::UnicodeBuffer::new();
            buffer.push_str(text);
            buffer.guess_segment_properties();
            let out = shaper.shape(buffer, harfrust::ShapeOptions::new());
            let scale = size / f32::from(shaper.units_per_em().max(1) as u16);
            let infos = out.glyph_infos();
            // A cluster runs from its own byte to the next *larger* cluster's — clusters arrive
            // in text order for the left-to-right text this build lays out.
            let mut starts: Vec<usize> = infos.iter().map(|info| info.cluster as usize).collect();
            starts.sort_unstable();
            starts.dedup();
            for (info, position) in infos.iter().zip(out.glyph_positions()) {
                let start = info.cluster as usize;
                let next = starts.partition_point(|&at| at <= start);
                let end = starts.get(next).copied().unwrap_or(text.len());
                glyphs.push(Glyph {
                    id: info.glyph_id,
                    x_advance: position.x_advance as f32 * scale,
                    x_offset: position.x_offset as f32 * scale,
                    y_offset: position.y_offset as f32 * scale,
                    text: start..end,
                });
            }
        }
        Shaped { face, size, glyphs }
    }

    /// Every family set in another face than the one it named, in name order.
    pub fn substitutions(&self) -> Vec<Substitution> {
        let mut out: Vec<Substitution> = self
            .resolved
            .borrow()
            .iter()
            .filter_map(|((asked, _, _), resolved)| {
                let asked = asked.clone()?;
                let compatible = match resolved.how {
                    Match::Exact => return None,
                    Match::Compatible => true,
                    // A generic family *names* no face, so setting it in one substitutes nothing.
                    Match::Generic => return None,
                    Match::Fallback => false,
                };
                Some(Substitution {
                    asked,
                    used: self.fonts.face(resolved.face).family.clone(),
                    compatible,
                })
            })
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// How far below the top of a line set in `style` its baseline is, and how tall the line:
    /// the face's own ascent, and ascent plus descent plus its line gap.
    fn vertical(&self, style: &TextStyle) -> (f32, f32) {
        use skrifa::MetadataProvider;
        let (face, size) = self.face_of(style);
        let face = self.fonts.face(face);
        match skrifa::FontRef::from_index(face.bytes(), face.index) {
            Ok(font) => {
                let m = font.metrics(
                    skrifa::instance::Size::new(size),
                    skrifa::instance::LocationRef::default(),
                );
                (m.ascent, m.ascent - m.descent + m.leading)
            }
            Err(_) => (size * 0.8, size * 1.15),
        }
    }
}

/// `fo:font-weight`: `bold`, or a number from 600 up.
fn is_bold(weight: &str) -> bool {
    weight == "bold" || weight.parse::<f32>().is_ok_and(|w| w >= 600.0)
}

/// `fo:font-size` in points, when it is an absolute length. A percentage is relative to a parent
/// style this build does not read, so it is the default like no size at all.
fn size_of(style: &TextStyle) -> f32 {
    style
        .font_size
        .as_deref()
        .and_then(length_mm)
        .map(|mm| grind_core::page::pt(mm) as f32)
        .filter(|pt| *pt > 0.0)
        .unwrap_or(DEFAULT_SIZE)
}

impl Metrics for Typesetter {
    /// The shaped advances, cumulative per **character**: a cluster's advance (a ligature, a
    /// base with its marks) is shared evenly between the characters it covers, so a caret
    /// inside `fi` lands halfway across it rather than nowhere.
    fn advances(&self, text: &str, style: &TextStyle, out: &mut Vec<f32>) {
        if text.is_empty() {
            return;
        }
        let shaped = self.shape(text, style);
        let mut per_byte = vec![0.0_f32; text.len() + 1];
        let mut ends: BTreeMap<usize, usize> = BTreeMap::new();
        for glyph in &shaped.glyphs {
            per_byte[glyph.text.start] += glyph.x_advance;
            ends.insert(glyph.text.start, glyph.text.end);
        }
        // Each character belongs to the cluster starting at or before it; one pass to count
        // the characters per cluster, one to hand each its share.
        let cluster_of: Vec<Option<usize>> = text
            .char_indices()
            .map(|(at, _)| ends.range(..=at).next_back().map(|(&start, _)| start))
            .collect();
        let mut count: HashMap<usize, usize> = HashMap::new();
        for start in cluster_of.iter().flatten() {
            *count.entry(*start).or_default() += 1;
        }
        let each = cluster_of.iter().map(|cluster| match cluster {
            Some(start) => per_byte[*start] / count[start] as f32,
            None => 0.0,
        });
        let mut sum = 0.0;
        for advance in each {
            sum += advance.max(0.0);
            out.push(sum);
        }
    }

    fn line_height(&self, style: &TextStyle) -> f32 {
        self.vertical(style).1
    }

    fn ascent(&self, style: &TextStyle) -> f32 {
        self.vertical(style).0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setter() -> Typesetter {
        Typesetter::new(Fonts::bundled())
    }

    fn style(family: &str, size: &str, bold: bool) -> TextStyle {
        TextStyle {
            font_family: Some(family.to_owned()),
            font_size: Some(size.to_owned()),
            font_weight: bold.then(|| "bold".to_owned()),
            font_style: None,
        }
    }

    fn width(t: &Typesetter, text: &str, style: &TextStyle) -> f32 {
        let mut out = Vec::new();
        t.advances(text, style, &mut out);
        out.last().copied().unwrap_or(0.0)
    }

    fn serif() -> TextStyle {
        style("Liberation Serif", "12pt", false)
    }

    #[test]
    fn one_cumulative_advance_per_character_never_decreasing() {
        let t = setter();
        let mut out = Vec::new();
        t.advances("Grüße, Œuvre — fi", &serif(), &mut out);
        assert_eq!(out.len(), "Grüße, Œuvre — fi".chars().count());
        assert!(out.windows(2).all(|w| w[1] >= w[0]), "{out:?}");
        assert!(out[0] > 0.0);
    }

    #[test]
    fn a_width_is_the_fonts_own_advance_at_the_size() {
        use skrifa::MetadataProvider;
        let t = setter();
        let bytes = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/fonts/LiberationSerif-Regular.ttf"
        ))
        .unwrap();
        let font = skrifa::FontRef::new(&bytes).unwrap();
        let glyph = font.charmap().map('M').unwrap();
        let expected = font
            .glyph_metrics(
                skrifa::instance::Size::new(12.0),
                skrifa::instance::LocationRef::default(),
            )
            .advance_width(glyph)
            .unwrap();
        assert!((width(&t, "M", &serif()) - expected).abs() < 1e-3);
        assert!(
            (width(&t, "M", &style("Liberation Serif", "24pt", false)) - 2.0 * expected).abs()
                < 1e-3
        );
    }

    #[test]
    fn kerning_is_applied_because_the_text_is_shaped_whole() {
        let t = setter();
        let s = serif();
        assert!(width(&t, "AV", &s) < width(&t, "A", &s) + width(&t, "V", &s));
    }

    #[test]
    fn a_ligature_shares_its_advance_between_its_characters() {
        let t = setter();
        let shaped = t.shape("fi", &serif());
        let mut out = Vec::new();
        t.advances("fi", &serif(), &mut out);
        assert_eq!(out.len(), 2);
        assert!(out[0] > 0.0 && out[1] > out[0]);
        let total: f32 = shaped.glyphs.iter().map(|g| g.x_advance).sum();
        assert!((out[1] - total).abs() < 1e-4);
    }

    #[test]
    fn bold_is_set_in_the_bold_face() {
        let t = setter();
        let regular = t.shape("Office", &serif());
        let bold = t.shape("Office", &style("Liberation Serif", "12pt", true));
        assert_ne!(regular.face, bold.face);
        assert!(t.fonts().face(bold.face).bold);
        assert!(
            width(&t, "Office", &style("Liberation Serif", "12pt", true))
                > width(&t, "Office", &serif())
        );
    }

    #[test]
    fn a_run_with_no_size_is_twelve_points() {
        let t = setter();
        let shaped = t.shape("x", &TextStyle::default());
        assert_eq!(shaped.size, DEFAULT_SIZE);
        assert_eq!(t.fonts().face(shaped.face).family, "Liberation Serif");
    }

    #[test]
    fn a_line_is_a_little_taller_than_its_size_and_sits_on_its_ascent() {
        let t = setter();
        let height = t.line_height(&serif());
        let ascent = t.ascent(&serif());
        assert!((13.0..15.0).contains(&height), "{height}");
        assert!(ascent > 0.0 && ascent < height, "{ascent}");
    }

    #[test]
    fn a_substituted_family_is_reported_once_and_an_exact_one_never() {
        let t = setter();
        width(&t, "a", &style("Arial", "12pt", false));
        width(&t, "b", &style("Arial", "12pt", true));
        width(&t, "c", &style("Wingdings Deluxe", "12pt", false));
        width(&t, "d", &serif());
        width(&t, "e", &TextStyle::default());
        assert_eq!(
            t.substitutions(),
            vec![
                Substitution {
                    asked: "Arial".into(),
                    used: "Liberation Sans".into(),
                    compatible: true
                },
                Substitution {
                    asked: "Wingdings Deluxe".into(),
                    used: "Liberation Serif".into(),
                    compatible: false
                },
            ]
        );
    }

    #[test]
    fn nothing_to_measure_appends_nothing() {
        let t = setter();
        let mut out = vec![1.0];
        t.advances("", &serif(), &mut out);
        assert_eq!(out, vec![1.0]);
    }
}
