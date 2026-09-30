// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Text measured by CoreText — `grind_core::layout::Metrics`, answered by a shaping engine
//! (decision 4).
//!
//! Two halves. [`fold`] is portable: CoreText answers "where is the caret at this UTF-16 index",
//! the trait asks "how far after each `char`", and turning one into the other is arithmetic that
//! is tested on any host. `CoreText` is the Mac's, and it is **also what draws**: `render.rs`
//! sets each line with the very `CTLine` this measured, so the caret and the ink cannot disagree
//! (`doc/windows-shell.md` decision 3, which GDI met with its own advances and CoreText meets by
//! construction).

/// The cumulative advance after each character of `text`, appended to `out`, from `offset_at` —
/// the caret's offset at a UTF-16 index, which is what `CTLineGetOffsetForStringIndex` answers.
///
/// A `char` is one UTF-16 unit or two, and its advance is the offset after its **last** unit, so
/// a character outside the Basic Multilingual Plane is one advance rather than two half ones. A
/// combining mark or the inside of a ligature can answer an offset *before* the one ahead of it —
/// CoreText gives a caret stop per cluster, not per character — and the trait promises a
/// non-decreasing sequence, so each answer is at least the last. That is what makes a decomposed
/// `é` measure as wide as a precomposed one: the mark adds nothing.
pub fn fold(text: &str, offset_at: impl Fn(usize) -> f64, out: &mut Vec<f32>) {
    let mut units = 0;
    let mut last = 0.0f32;
    for c in text.chars() {
        units += c.len_utf16();
        last = last.max(offset_at(units) as f32);
        out.push(last);
    }
}

/// The size a cell or a paragraph with no size of its own is drawn at, in points: the system
/// font a little larger than `NSFont.smallSystemFontSize`, which reads at a spreadsheet's row
/// height. A document's own size is a multiple of it (`grind_sheet::look::font_scale`).
pub const BASE_PT: f64 = 12.0;

/// Which family a [`Font`] is set in.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Family {
    /// The system's own face — San Francisco — which is what a cell and a page's prose are set
    /// in when the document names nothing.
    System,
    /// The user's fixed-pitch face: a code fence, and the generic `monospace` a `` `code` `` run
    /// carries. Named by the system rather than by us, since which monospace face a reader has
    /// is theirs to choose.
    Mono,
    /// A family the document names, verbatim; CoreText finds the best match it has.
    Named(String),
}

/// A font, resolved: everything CoreText is asked for, and nothing it has to interpret.
///
/// The one currency between measuring and drawing — a cell's `TextStyle` and a page's block face
/// and run both come down to one of these, `CoreText` measures with it and `render.rs` sets the
/// line in it, so the caret and the ink cannot come from two different fonts (decision 4).
#[derive(Clone, Debug, PartialEq)]
pub struct Font {
    /// In points.
    pub size: f64,
    pub bold: bool,
    pub italic: bool,
    pub family: Family,
}

impl Font {
    /// A cell's font: the system face at `base` times the document's own multiple of its default
    /// size (`grind_sheet::look::font_scale`), bold and italic as the style says. The grid's
    /// family is always the system's.
    pub fn cell(base: f64, style: &grind_core::style::TextStyle) -> Font {
        use grind_sheet::look;
        Font {
            size: base * look::font_scale(style.font_size.as_deref()).unwrap_or(1.0),
            bold: look::bold_weight(style.font_weight.as_deref()),
            italic: look::italic_style(style.font_style.as_deref()),
            family: Family::System,
        }
    }
}

#[cfg(target_os = "macos")]
pub use mac::{CoreText, Face};

#[cfg(target_os = "macos")]
mod mac {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::ptr::null;

    use grind_core::layout::Metrics;
    use grind_core::style::TextStyle;
    use grind_text::look::Role;

    use super::{Family, Font};
    use crate::text::face;
    use objc2_core_foundation::{
        CFAttributedString, CFBoolean, CFDictionary, CFIndex, CFRetained, CFString, CFType, CGFloat,
    };
    use objc2_core_text::{
        CTFont, CTFontSymbolicTraits, CTFontUIFontType, CTLine, kCTFontAttributeName,
        kCTForegroundColorFromContextAttributeName,
    };

    /// What a made font is kept under: its size's bits, bold, italic and family.
    type FontKey = (u64, bool, bool, Family);

    /// Fonts by size and face, and the lines set in them.
    ///
    /// The cache is by the whole [`Font`], because a paint measures hundreds of cells and lines
    /// in a handful of faces, and creating a `CTFont` per piece would be most of the paint.
    pub struct CoreText {
        base: CGFloat,
        fonts: RefCell<HashMap<FontKey, CFRetained<CTFont>>>,
    }

    impl CoreText {
        pub fn new(base: CGFloat) -> Self {
            CoreText {
                base,
                fonts: RefCell::new(HashMap::new()),
            }
        }

        /// The CoreText font for `font`, made once and kept.
        pub fn font_of(&self, font: &Font) -> CFRetained<CTFont> {
            let key = (
                font.size.to_bits(),
                font.bold,
                font.italic,
                font.family.clone(),
            );
            if let Some(made) = self.fonts.borrow().get(&key) {
                return made.clone();
            }
            let made = make(font);
            self.fonts.borrow_mut().insert(key, made.clone());
            made
        }

        /// One line of `text` set in `font`, drawn in whatever fill colour the context has —
        /// the line a [`Metrics`] implementation measured and `render.rs` draws.
        pub fn line_of(&self, text: &str, font: &Font) -> CFRetained<CTLine> {
            let font = self.font_of(font);
            // SAFETY: both keys are constants CoreText exports for exactly these attributes.
            let (font_key, from_context): (&CFString, &CFString) = unsafe {
                (
                    kCTFontAttributeName,
                    kCTForegroundColorFromContextAttributeName,
                )
            };
            let yes = CFBoolean::new(true);
            let values: [&CFType; 2] = [font.as_ref(), yes.as_ref()];
            let attributes =
                CFDictionary::<CFString, CFType>::from_slices(&[font_key, from_context], &values);
            let string = CFString::from_str(text);
            // SAFETY: the dictionary's keys are strings and its values the font and a boolean,
            // the shape CoreText documents for an attributed string's attributes.
            let attributed = unsafe {
                CFAttributedString::new(None, Some(&string), Some(attributes.as_opaque()))
            }
            .expect("an attributed string from a string and two attributes");
            // SAFETY: the attributed string is valid for the duration of the call.
            unsafe { CTLine::with_attributed_string(&attributed) }
        }

        /// One line of a cell's `text`, set in `style` as the grid sets it.
        pub fn line(&self, text: &str, style: &TextStyle) -> CFRetained<CTLine> {
            self.line_of(text, &Font::cell(self.base, style))
        }

        /// How far below a line's top its baseline is, in `font`.
        pub fn ascent_of(&self, font: &Font) -> CGFloat {
            // SAFETY: an ordinary read of a live font's metrics.
            unsafe { self.font_of(font).ascent() }
        }

        /// How far below a line's top its baseline is, in a cell's `style`.
        pub fn ascent(&self, style: &TextStyle) -> CGFloat {
            self.ascent_of(&Font::cell(self.base, style))
        }

        /// Where `font` draws an underline — its offset below the baseline, and how thick — the
        /// font's own answer, so an underline sits where its designer put it.
        pub fn underline_of(&self, font: &Font) -> (CGFloat, CGFloat) {
            let font = self.font_of(font);
            // SAFETY: ordinary reads of a live font's metrics. CoreText's position is up from
            // the baseline, so a line below it is negative; this answers the distance down.
            let (position, thickness) =
                unsafe { (font.underline_position(), font.underline_thickness()) };
            (-position, thickness.max(0.5))
        }

        /// How tall `font`'s lower-case letters are — where a strike goes through them.
        pub fn x_height_of(&self, font: &Font) -> CGFloat {
            // SAFETY: an ordinary read of a live font's metrics.
            unsafe { self.font_of(font).x_height() }
        }

        /// How tall a line set in `font` is: ascent, descent and leading, rounded up to a point.
        pub fn height_of(&self, font: &Font) -> f32 {
            let font = self.font_of(font);
            // SAFETY: ordinary reads of a live font's metrics.
            let height = unsafe { font.ascent() + font.descent() + font.leading() };
            height.ceil() as f32
        }
    }

    /// A CoreText font for `font`: the family first, then the weight and slant over it.
    fn make(font: &Font) -> CFRetained<CTFont> {
        let size = font.size as CGFloat;
        // SAFETY: a null language means the user's own, which is what the UI fonts follow, and a
        // null matrix is the identity; both are documented.
        let base = unsafe {
            match &font.family {
                // The system face's bold is its own emphasized type rather than a trait applied
                // afterwards, which is how AppKit asks for it too.
                Family::System => CTFont::new_ui_font_for_language(
                    match font.bold {
                        true => CTFontUIFontType::EmphasizedSystem,
                        false => CTFontUIFontType::System,
                    },
                    size,
                    None,
                ),
                Family::Mono => {
                    CTFont::new_ui_font_for_language(CTFontUIFontType::UserFixedPitch, size, None)
                }
                Family::Named(name) => {
                    Some(CTFont::with_name(&CFString::from_str(name), size, null()))
                }
            }
        }
        .expect("the system and fixed-pitch UI fonts exist on every macOS");
        let bold = font.bold && font.family != Family::System;
        let mut traits = CTFontSymbolicTraits::empty();
        if bold {
            traits |= CTFontSymbolicTraits::BoldTrait;
        }
        if font.italic {
            traits |= CTFontSymbolicTraits::ItalicTrait;
        }
        if traits.is_empty() {
            return base;
        }
        // SAFETY: a size of zero keeps the font's own, and a null matrix is the identity; both
        // are documented. A face with no bold or no italic answers `None`, and the face as it is
        // is then the honest answer.
        unsafe { base.copy_with_symbolic_traits(0.0, null(), traits, traits) }.unwrap_or(base)
    }

    /// A page's block face — the [`Metrics`] one block is laid out with. Each run's own
    /// formatting is layered over the block's role by `text::face::font`, which is also what
    /// `text/paint.rs` hands the renderer, so a run is measured and drawn in one font.
    pub struct Face<'a> {
        pub text: &'a CoreText,
        pub role: Role,
    }

    impl Metrics for Face<'_> {
        fn advances(&self, text: &str, style: &TextStyle, out: &mut Vec<f32>) {
            let line = self.text.line_of(text, &face::font(self.role, style));
            super::fold(
                text,
                // SAFETY: a null secondary offset is documented as allowed.
                |units| unsafe {
                    line.offset_for_string_index(units as CFIndex, std::ptr::null_mut())
                },
                out,
            );
        }

        fn line_height(&self, style: &TextStyle) -> f32 {
            self.text.height_of(&face::font(self.role, style))
        }
    }

    impl Metrics for CoreText {
        fn advances(&self, text: &str, style: &TextStyle, out: &mut Vec<f32>) {
            let line = self.line(text, style);
            super::fold(
                text,
                // SAFETY: a null secondary offset is documented as allowed.
                |units| unsafe {
                    line.offset_for_string_index(units as CFIndex, std::ptr::null_mut())
                },
                out,
            );
        }

        fn line_height(&self, style: &TextStyle) -> f32 {
            self.height_of(&Font::cell(self.base, style))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Offsets as CoreText gives them for `a😀b`: the emoji is two UTF-16 units and one caret
    /// stop, and the index *between* its units answers the stop before it.
    #[test]
    fn a_character_outside_the_bmp_is_one_advance() {
        let offsets = [0.0, 10.0, 10.0, 34.0, 44.0];
        let mut out = Vec::new();
        fold("a\u{1f600}b", |units| offsets[units], &mut out);
        assert_eq!(out, [10.0, 34.0, 44.0]);
    }

    /// A combining mark is its base's cluster: CoreText may answer the cluster's leading edge for
    /// the index after the base, and the mark must not make the width go backwards.
    #[test]
    fn a_combining_mark_adds_nothing_and_nothing_goes_backwards() {
        // `e` then U+0301: the stop after `e` alone is inside the cluster.
        let offsets = [0.0, 0.0, 9.0];
        let mut out = Vec::new();
        fold("e\u{301}", |units| offsets[units], &mut out);
        assert_eq!(out, [0.0, 9.0]);
        assert!(out.windows(2).all(|pair| pair[0] <= pair[1]));
    }

    #[test]
    fn nothing_is_measured_for_no_text_and_out_is_appended_to() {
        let mut out = vec![99.0];
        fold("", |_| unreachable!(), &mut out);
        assert_eq!(out, [99.0]);
        fold("ab", |units| units as f64, &mut out);
        assert_eq!(out, [99.0, 1.0, 2.0]);
    }
}

/// Decision 4's claims, measured through the page's own `Metrics` rather than by the probe —
/// which measured them once, on 2026-09-30, on macOS 15 and 26 alike (*Evidence*). These run on
/// the runner, where CoreText is, and are type-checked here.
#[cfg(all(test, target_os = "macos"))]
mod coretext {
    use grind_core::layout::Metrics;
    use grind_core::style::TextStyle;
    use grind_text::look::Role;

    use super::{BASE_PT, CoreText, Face};

    fn advances(text: &str) -> Vec<f32> {
        let fonts = CoreText::new(BASE_PT);
        let face = Face {
            text: &fonts,
            role: Role::Body,
        };
        let mut out = Vec::new();
        face.advances(text, &TextStyle::default(), &mut out);
        assert_eq!(out.len(), text.chars().count(), "one advance per char");
        assert!(out.windows(2).all(|pair| pair[0] <= pair[1]), "{out:?}");
        out
    }

    /// A cluster of several characters is **one caret stop**: nothing inside it has a width of
    /// its own, and the whole of it lands on its last character.
    fn one_stop(text: &str) {
        let out = advances(text);
        let (last, inside) = out.split_last().expect("some text");
        assert!(*last > 0.0, "{text}: {out:?}");
        assert!(inside.iter().all(|x| *x == 0.0), "{text}: {out:?}");
    }

    #[test]
    fn a_decomposed_e_is_as_wide_as_a_precomposed_one() {
        let precomposed = advances("\u{e9}");
        let decomposed = advances("e\u{301}");
        assert!(
            (precomposed[0] - decomposed[1]).abs() < 0.01,
            "{precomposed:?} against {decomposed:?}"
        );
        one_stop("e\u{301}");
    }

    #[test]
    fn a_zwj_family_is_one_caret_stop() {
        one_stop("\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}");
    }

    #[test]
    fn a_devanagari_conjunct_is_one_caret_stop() {
        one_stop("\u{915}\u{94d}\u{937}\u{93f}");
    }

    /// Latin has a stop per letter, or the tests above would pass for any text.
    #[test]
    fn a_latin_word_has_a_stop_per_letter() {
        let out = advances("Hello");
        assert!(out.windows(2).all(|pair| pair[0] < pair[1]), "{out:?}");
    }
}
