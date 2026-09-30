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

#[cfg(target_os = "macos")]
pub use mac::CoreText;

#[cfg(target_os = "macos")]
mod mac {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::ptr::null;

    use grind_core::layout::Metrics;
    use grind_core::style::TextStyle;
    use grind_sheet::look;
    use objc2_core_foundation::{
        CFAttributedString, CFBoolean, CFDictionary, CFIndex, CFRetained, CFString, CFType, CGFloat,
    };
    use objc2_core_text::{
        CTFont, CTFontSymbolicTraits, CTFontUIFontType, CTLine, kCTFontAttributeName,
        kCTForegroundColorFromContextAttributeName,
    };

    /// Fonts by size and face, and the lines set in them.
    ///
    /// The cache is by the three things a `TextStyle` changes about a font here — its size, and
    /// whether it is bold or italic — because a paint measures hundreds of cells in a handful of
    /// faces, and creating a `CTFont` per cell would be most of the paint.
    pub struct CoreText {
        base: CGFloat,
        fonts: RefCell<HashMap<(u64, bool, bool), CFRetained<CTFont>>>,
    }

    impl CoreText {
        pub fn new(base: CGFloat) -> Self {
            CoreText {
                base,
                fonts: RefCell::new(HashMap::new()),
            }
        }

        /// The font `style` is set in: the system font, at the base size times the document's
        /// own multiple, bold and italic as it says.
        pub fn font(&self, style: &TextStyle) -> CFRetained<CTFont> {
            let size = self.base * look::font_scale(style.font_size.as_deref()).unwrap_or(1.0);
            let bold = look::bold_weight(style.font_weight.as_deref());
            let italic = look::italic_style(style.font_style.as_deref());
            let key = (size.to_bits(), bold, italic);
            if let Some(font) = self.fonts.borrow().get(&key) {
                return font.clone();
            }
            let kind = match bold {
                true => CTFontUIFontType::EmphasizedSystem,
                false => CTFontUIFontType::System,
            };
            // SAFETY: a null language means the user's own, which is what the UI font follows.
            let upright = unsafe { CTFont::new_ui_font_for_language(kind, size, None) }
                .expect("the system UI font exists on every macOS");
            let font = match italic {
                // SAFETY: a size of zero keeps the font's own, and a null matrix is the identity;
                // both are documented. A face with no italic answers `None`, and the upright one
                // is then the honest answer.
                true => unsafe {
                    upright.copy_with_symbolic_traits(
                        0.0,
                        null(),
                        CTFontSymbolicTraits::ItalicTrait,
                        CTFontSymbolicTraits::ItalicTrait,
                    )
                }
                .unwrap_or(upright),
                false => upright,
            };
            self.fonts.borrow_mut().insert(key, font.clone());
            font
        }

        /// One line of `text` set in `style`'s font, drawn in whatever fill colour the context
        /// has — the line [`Metrics::advances`] measured and `render.rs` draws.
        pub fn line(&self, text: &str, style: &TextStyle) -> CFRetained<CTLine> {
            let font = self.font(style);
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

        /// How far below a line's top its baseline is, in `style`'s font.
        pub fn ascent(&self, style: &TextStyle) -> CGFloat {
            // SAFETY: an ordinary read of a live font's metrics.
            unsafe { self.font(style).ascent() }
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
            let font = self.font(style);
            // SAFETY: ordinary reads of a live font's metrics.
            let height = unsafe { font.ascent() + font.descent() + font.leading() };
            height.ceil() as f32
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
