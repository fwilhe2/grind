// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The real [`Metrics`]: text measured by shaping it in the font file the PDF embeds.
//!
//! **One engine measures and draws**, or the ink lands where the layout did not put it:
//! [`Typesetter::shape`] is what both [`Metrics::advances`] and the PDF call, with the same face,
//! size and text. The unit is the PostScript point, the PDF's own.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use grind_core::layout::Metrics;
use grind_core::style::{TextStyle, length_mm};

use crate::fonts::{Data, FaceId, Fonts, Match, Resolved};

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
    /// The face it is drawn from when that is not its run's: a character the run's face has no
    /// glyph for, set in a face that has one ([`Fonts::fallback_for`]).
    pub face: Option<FaceId>,
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
    /// What kind of face each family the document declares is (`style:font-family-generic`), for
    /// falling back on a family that is not here ([`Fonts::resolve_hinted`]).
    generics: RefCell<HashMap<String, String>>,
    /// Each face as harfrust reads it, with its layout tables cached; `None` when it cannot.
    shapers: RefCell<HashMap<FaceId, Option<harfrust::Font>>>,
    resolved: RefCell<BTreeMap<Asked, Resolved>>,
    measured: RefCell<Measured>,
    /// What [`Typesetter::shape`] returned, by face, size and text: a page drawn again after
    /// an edit is all but one paragraph the glyphs it was.
    shaped: RefCell<Shapes>,
    /// The face each character a run's face lacks is drawn from, by weight and slant.
    fallbacks: RefCell<HashMap<(char, bool, bool), Option<FaceId>>>,
    /// Which typesetter this is, of all the process has made ([`Typesetter::id`]).
    id: u64,
}

/// Cumulative advances by the text they are of.
type Advances = HashMap<Box<str>, Rc<[f32]>>;

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// A size, as bits, and how the letters at it are set: [`Letters::set`].
type Set = (u32, u32, bool);

/// How a style sets its letters past what its face does (`doc/odt-format.md` §5c facts 51 and
/// 52): the space after every character, in points, and whether the face's kerning applies.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Letters {
    spacing: f32,
    kern: bool,
}

impl Letters {
    fn of(style: &TextStyle) -> Self {
        let spacing = style
            .letter_spacing
            .as_deref()
            .filter(|value| value.trim() != "normal")
            .and_then(grind_core::style::length_mm)
            .map_or(0.0, |mm| (mm * 72.0 / 25.4) as f32);
        Letters {
            spacing,
            kern: style.letter_kerning.as_deref() != Some("false"),
        }
    }

    fn set(self, size: f32) -> Set {
        (size.to_bits(), self.spacing.to_bits(), self.kern)
    }
}

/// The advances of text already shaped, by face and size. A document is measured several times
/// over — to stack its blocks, to cut its pages, to draw them — and again after every edit,
/// when all but one paragraph is the text it was.
#[derive(Default)]
struct Measured {
    by_face: HashMap<(FaceId, Set), Advances>,
    /// How many advances are held, across every face.
    held: usize,
}

impl Measured {
    // ponytail: past the ceiling everything is dropped and measured again as it is asked for,
    // a stall of one layout on a document of several million characters. Evict the least
    // recently used when one is edited at that size.
    const CEILING: usize = 8_000_000;

    fn get(&self, face: FaceId, set: Set, text: &str) -> Option<Rc<[f32]>> {
        self.by_face.get(&(face, set))?.get(text).cloned()
    }

    fn keep(&mut self, face: FaceId, set: Set, text: &str, advances: Rc<[f32]>) {
        if self.held > Self::CEILING {
            self.by_face.clear();
            self.held = 0;
        }
        self.held += advances.len();
        self.by_face
            .entry((face, set))
            .or_default()
            .insert(text.into(), advances);
    }
}

/// Shaped glyphs by face, size (as bits) and text, bounded as [`Measured`] is.
#[derive(Default)]
struct Shapes {
    by_text: HashMap<(FaceId, Set, Box<str>), Rc<[Glyph]>>,
    held: usize,
}

impl Shapes {
    const CEILING: usize = 4_000_000;

    fn keep(&mut self, key: (FaceId, Set, Box<str>), glyphs: Rc<[Glyph]>) {
        if self.held > Self::CEILING {
            self.by_text.clear();
            self.held = 0;
        }
        self.held += glyphs.len() + 1;
        self.by_text.insert(key, glyphs);
    }
}

impl Typesetter {
    #[must_use]
    pub fn new(fonts: Fonts) -> Self {
        Typesetter {
            fonts,
            generics: RefCell::new(HashMap::new()),
            shapers: RefCell::new(HashMap::new()),
            resolved: RefCell::new(BTreeMap::new()),
            measured: RefCell::default(),
            shaped: RefCell::default(),
            fallbacks: RefCell::default(),
            id: NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        }
    }

    /// A number no other typesetter in the process has: two may hold different fonts, so what
    /// one measured is not what the other would ([`Metrics::identity`]).
    #[must_use]
    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn fonts(&self) -> &Fonts {
        &self.fonts
    }

    /// Tell the typesetter what kind of face each of the document's families is, before it sets
    /// any of them — `grind_text::App::font_generics`.
    pub fn hint(&self, generics: HashMap<String, String>) {
        *self.generics.borrow_mut() = generics;
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
                let family = style.font_family.as_deref();
                let generics = self.generics.borrow();
                let generic = family.and_then(|f| generics.get(f)).map(String::as_str);
                self.fonts.resolve_hinted(family, bold, italic, generic)
            })
            .face;
        (face, size_of(style))
    }

    /// Shape `text` as `style` sets it.
    pub fn shape(&self, text: &str, style: &TextStyle) -> Shaped {
        let (face, size) = self.face_of(style);
        let letters = Letters::of(style);
        let key = (face, letters.set(size), Box::<str>::from(text));
        if let Some(glyphs) = self.shaped.borrow().by_text.get(&key) {
            return Shaped {
                face,
                size,
                glyphs: glyphs.to_vec(),
            };
        }
        let mut shaped = self.shape_in(face, size, letters.kern, text);
        // The spacing follows every character, so a cluster of several takes it once for each.
        if letters.spacing != 0.0 {
            let glyphs = &mut shaped.glyphs;
            for at in 0..glyphs.len() {
                let last = glyphs
                    .get(at + 1)
                    .is_none_or(|next| next.text != glyphs[at].text);
                if last {
                    let chars = text
                        .get(glyphs[at].text.clone())
                        .map_or(1, |c| c.chars().count());
                    glyphs[at].x_advance += letters.spacing * chars as f32;
                }
            }
        }
        self.shaped
            .borrow_mut()
            .keep(key, Rc::from(shaped.glyphs.as_slice()));
        shaped
    }

    fn fallback_for(&self, c: char, bold: bool, italic: bool) -> Option<FaceId> {
        *self
            .fallbacks
            .borrow_mut()
            .entry((c, bold, italic))
            .or_insert_with(|| self.fonts.fallback_for(c, bold, italic))
    }

    /// Shape `text` in `face` alone, `.notdef` where it has no glyph.
    fn shape_once(&self, face: FaceId, size: f32, kern: bool, text: &str) -> Shaped {
        let mut glyphs = Vec::new();
        let mut shapers = self.shapers.borrow_mut();
        let font = shapers.entry(face).or_insert_with(|| {
            let found = self.fonts.face(face);
            match &found.data {
                Data::Static(bytes) => harfrust::Font::new(*bytes, found.index),
                Data::Shared(bytes) => {
                    let bytes: Arc<dyn AsRef<[u8]> + Send + Sync> = bytes.clone();
                    harfrust::Font::new(bytes, found.index)
                }
            }
        });
        if let Some(font) = font.as_ref()
            && !text.is_empty()
        {
            let mut out = harfrust::Buffer::new();
            out.push_str(text);
            out.guess_segment_properties();
            let shaper = harfrust::ShaperFont::new(font);
            let unkerned = [harfrust::Feature::new(harfrust::Tag::new(b"kern"), 0, ..)];
            let features: &[harfrust::Feature] = if kern { &[] } else { &unkerned };
            let options = harfrust::ShapeOptions::new().features(features);
            if harfrust::shape(&shaper, &mut out, options).is_err() {
                return Shaped { face, size, glyphs };
            }
            let scale = size / f32::from(font.units_per_em().max(1));
            let infos = out.glyph_infos();
            // A cluster runs from its own byte to the next larger cluster's — they arrive in text
            // order for left-to-right text.
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
                    face: None,
                });
            }
        }
        Shaped { face, size, glyphs }
    }

    /// [`Typesetter::shape_once`], with each run of characters its face has no glyph for set
    /// again in a face that has one — per cluster, so what is measured is what is drawn.
    fn shape_in(&self, face: FaceId, size: f32, kern: bool, text: &str) -> Shaped {
        let mut shaped = self.shape_once(face, size, kern, text);
        if !shaped.glyphs.iter().any(|glyph| glyph.id == 0) {
            return shaped;
        }
        let (bold, italic) = {
            let face = self.fonts.face(face);
            (face.bold, face.italic)
        };
        let mut out = Vec::with_capacity(shaped.glyphs.len());
        let mut glyphs = std::mem::take(&mut shaped.glyphs).into_iter().peekable();
        while let Some(glyph) = glyphs.next() {
            if glyph.id != 0 {
                out.push(glyph);
                continue;
            }
            // The run of `.notdef` clusters this one starts, as one stretch of the text.
            let mut stretch = glyph.text.clone();
            while let Some(next) = glyphs.next_if(|next| next.id == 0) {
                stretch.start = stretch.start.min(next.text.start);
                stretch.end = stretch.end.max(next.text.end);
            }
            let Some(missing) = text.get(stretch.clone()) else {
                continue;
            };
            // Split where the face that has the character changes; a character none has stays
            // the run face's empty box.
            let mut from = 0;
            let mut chars = missing.char_indices().peekable();
            while let Some((_, c)) = chars.next() {
                let here = self.fallback_for(c, bold, italic);
                let end = chars.peek().map_or(missing.len(), |&(next, _)| next);
                let same = chars
                    .peek()
                    .is_some_and(|&(_, next)| self.fallback_for(next, bold, italic) == here);
                if same {
                    continue;
                }
                let piece = &missing[from..end];
                let offset = stretch.start + from;
                let (drawn, set) = match here {
                    Some(other) if other != face => (
                        Some(other),
                        self.shape_once(other, size, kern, piece).glyphs,
                    ),
                    _ => (None, self.shape_once(face, size, kern, piece).glyphs),
                };
                out.extend(set.into_iter().map(|mut glyph| {
                    glyph.text = glyph.text.start + offset..glyph.text.end + offset;
                    glyph.face = drawn;
                    glyph
                }));
                from = end;
            }
        }
        shaped.glyphs = out;
        shaped
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
                    Match::Compatible => true,
                    // A generic family *names* no face, so setting it in one substitutes nothing.
                    Match::Exact | Match::Generic => return None,
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
/// style that is not read, so it counts as no size.
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
        let (face, size) = self.face_of(style);
        let set = Letters::of(style).set(size);
        if let Some(known) = self.measured.borrow().get(face, set, text) {
            out.extend_from_slice(&known);
            return;
        }
        let from = out.len();
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
        self.measured
            .borrow_mut()
            .keep(face, set, text, out[from..].into());
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
            ..TextStyle::default()
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

    /// `doc/odt-format.md` §5c facts 51 and 52: kerning unless a style turns it off, and letter
    /// spacing after every character, on top of the kerning.
    #[test]
    fn kerning_is_on_unless_turned_off_and_spacing_follows_every_character() {
        let t = setter();
        let at = |kerning: Option<&str>, spacing: Option<&str>| TextStyle {
            letter_kerning: kerning.map(str::to_owned),
            letter_spacing: spacing.map(str::to_owned),
            ..style("Liberation Serif", "24pt", false)
        };
        let kerned = width(&t, "AVATAR", &at(None, None));
        assert_eq!(kerned, width(&t, "AVATAR", &at(Some("true"), None)));
        let plain = width(&t, "AVATAR", &at(Some("false"), None));
        assert!(plain - kerned > 10.0, "{plain} unkerned, {kerned} kerned");
        let spaced = width(&t, "AVATAR", &at(None, Some("0.1cm")));
        assert!(
            (spaced - kerned - 6.0 * 0.1 / 2.54 * 72.0).abs() < 0.01,
            "{spaced}"
        );
        assert_eq!(width(&t, "AVATAR", &at(None, Some("normal"))), kerned);
        // Drawn as measured.
        let shaped = t.shape("AVATAR", &at(None, Some("0.1cm")));
        let drawn: f32 = shaped.glyphs.iter().map(|glyph| glyph.x_advance).sum();
        assert!((drawn - spaced).abs() < 1e-3);
    }

    /// Liberation Serif has no ☿ and the bundled Mono or Sans has: it is drawn from that face,
    /// measured as drawn, and the letters round it stay the run's.
    #[test]
    fn a_character_the_face_lacks_is_drawn_from_one_that_has_it() {
        let t = setter();
        let fonts = t.fonts();
        let serif_face = t.face_of(&serif()).0;
        assert!(!fonts.covers(serif_face, '☿'), "the fixture needs a gap");
        let shaped = t.shape("a☿b", &serif());
        assert_eq!(shaped.glyphs.len(), 3);
        assert!(
            shaped.glyphs.iter().all(|glyph| glyph.id != 0),
            "no empty box"
        );
        let mercury = &shaped.glyphs[1];
        let other = mercury.face.expect("drawn from another face");
        assert!(fonts.covers(other, '☿'));
        assert_eq!(mercury.text, 1..4, "its cluster is its own bytes");
        assert_eq!((shaped.glyphs[0].face, shaped.glyphs[2].face), (None, None));
        let drawn: f32 = shaped.glyphs.iter().map(|glyph| glyph.x_advance).sum();
        assert!((width(&t, "a☿b", &serif()) - drawn).abs() < 1e-3);
    }

    #[test]
    fn one_cumulative_advance_per_character_never_decreasing() {
        let t = setter();
        let mut out = Vec::new();
        t.advances("Grüße, Œuvre — fi", &serif(), &mut out);
        assert_eq!(out.len(), "Grüße, Œuvre — fi".chars().count());
        assert!(
            out.array_windows().all(|[prev, next]| next >= prev),
            "{out:?}"
        );
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
