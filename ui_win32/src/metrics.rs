// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Font metrics, through GDI — this shell's whole contribution to layout, and
//! `doc/windows-shell.md`'s decision 3 made real.
//!
//! `doc/text-layout.md` closed on Path C: line breaking lives in `grind_core::layout` and a
//! shell supplies only [`grind_core::layout::Metrics`] — how wide is this text, how tall is a
//! line of it. `ui_tui` answers in terminal cells, `ui_text_gtk` asks Pango, `ui_web` asks a
//! canvas. **This is the fourth implementation and the first one the toolkit did not hand over**,
//! because Win32 has no Pango: GDI maps glyphs, it does not shape them.
//!
//! ## The measuring engine and the drawing engine are the same one
//!
//! That is the whole of decision 3 and it is load-bearing rather than tidy. `GetTextExtentExPointW`
//! does not shape, so a combining mark gets an advance of its own instead of folding onto its
//! base — the core then believes the text is one mark wider than it looks. That is survivable
//! **only if drawing agrees**: [`Face::draw_run`] measures with the same call, with the same font,
//! and hands the result to `ExtTextOutW` as explicit advances, so the mark is drawn floating in a
//! box of its own — which looks wrong and is not *inconsistent*, and every caret operation stays
//! right. Measure with DirectWrite and draw with plain `ExtTextOutW` and the two disagree, which
//! is why the upgrade named in decision 3 is a whole stack rather than an API.
//!
//! The named trigger for taking it, written down there and repeated here so it is not a judgement
//! call later: **the first time a caret lands in the wrong place because characters that belong
//! in one cluster were measured as separate boxes**, this file moves to DirectWrite —
//! measurement *and* drawing together — and nothing above it changes.
//!
//! ## Two halves, as everywhere in this crate
//!
//! The decisions are portable and tested on Linux: [`Spec`] and [`spec_for`] are which face a
//! block is set in, and [`fold_advances`] is the mechanical part decision 3 calls cheap — GDI
//! answers per **UTF-16 code unit** and [`grind_core::layout::Metrics`] asks per `char`, so the
//! units are walked and each character takes the entry for its *last* one. A character outside
//! the basic multilingual plane therefore comes out as one advance, correctly.
//!
//! Only [`windows_impl`] needs a window — or rather does not: it holds a memory DC from
//! `CreateCompatibleDC(None)`, so measuring works with no `HWND` anywhere, which is what lets
//! `--render-to` lay a document out on a headless runner.

use grind_text::BlockKind;

/// How much bigger than the body text each heading level is.
///
/// The same six numbers `ui_text_gtk` uses, and deliberately so: two shells that scaled headings
/// differently would break lines in different places, and a document is one document. Flat after
/// level four, because a level-6 heading barely larger than the paragraph under it is exactly
/// what a level-6 heading should look like.
pub const HEADING_SCALE: [f64; 6] = [1.8, 1.5, 1.3, 1.15, 1.05, 1.0];

/// `Title` and `Subtitle` — the two named paragraph styles LibreOffice's own blank document
/// offers, and the only two this shell gives a face of their own. Everything else in
/// `office:styles` is a name this build keeps and does not interpret (`doc/text-core.md`).
pub const TITLE_SCALE: f64 = 2.4;
pub const SUBTITLE_SCALE: f64 = 1.3;

/// The face prose is set in. Segoe UI is the shell font on every Windows this shell targets, and
/// GDI substitutes when it is absent — which is what happens under Wine.
pub const BODY_FACE: &str = "Segoe UI";

/// What `grind_text::markdown::MONOSPACE` resolves to here.
///
/// The document says `monospace`, which is a *generic* family: fontconfig resolves one and so
/// does a browser, and GDI does not — `CreateFontIndirectW` with an unknown face name silently
/// substitutes whatever it likes, which for a code block means proportional text. So the generic
/// is named once, here, and Consolas is the answer because it has shipped with Windows since
/// Vista and this binary depends on nothing Windows does not ship.
pub const MONO_FACE: &str = "Consolas";

/// How wide a `text:tab` is, in spaces.
///
/// A width and **not a tab stop**: a stop is a paragraph property (`style:tab-stops`) that this
/// build does not read, so a pane that snapped to one would be inventing a document's layout.
/// Four is what the terminal shell uses and what every editor defaults to.
pub const TAB_SPACES: usize = 4;

/// Which face a block is set in — resolved from what the document says, with no Windows in it.
///
/// **The face is chosen per block, by the shell**, and that is not an oversight in the core: a
/// run's *direct* formatting reaches it, but a block's does not, because a heading and a `Title`
/// are a paragraph **style** and style definitions are not read (`doc/text-core.md`). A heading
/// laid out in the body font and drawn in a larger one would put every caret in the wrong place,
/// so the shell hands the core a provider already set to the block's font.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spec {
    /// Multiplier on the body size.
    pub scale: f64,
    pub bold: bool,
    pub italic: bool,
    /// `None` for the body face; `Some` for a block set in a family of its own.
    pub family: Option<&'static str>,
}

impl Spec {
    /// The body face at its own size, which is what a plain paragraph gets.
    pub const BODY: Spec = Spec {
        scale: 1.0,
        bold: false,
        italic: false,
        family: None,
    };
}

/// The [`Spec`] for one block.
///
/// A named style wins over the block's own kind: `Title` and `Subtitle` are `BlockKind::Paragraph`
/// whose only signal is the name. A heading deeper than the six levels there are scales for is
/// set as the last of them rather than refused — the reader is *tolerant* (R5), so a level-9
/// heading loads, and a shell that panicked on one would undo that.
pub fn spec_for(kind: &BlockKind, style: Option<&str>) -> Spec {
    match style {
        Some("Title") => {
            return Spec {
                scale: TITLE_SCALE,
                bold: true,
                ..Spec::BODY
            };
        }
        Some("Subtitle") => {
            return Spec {
                scale: SUBTITLE_SCALE,
                italic: true,
                ..Spec::BODY
            };
        }
        // A fence (```) is a paragraph *style* and nothing else — `grind_text::markdown` names
        // it, LibreOffice writes it, and this is where a window makes it visible. A face rather
        // than a drawing trick, because the same object measures the block: a monospace
        // paragraph drawn in one font and measured in another breaks in the wrong places.
        Some(grind_text::markdown::PREFORMATTED) => {
            return Spec {
                family: Some(MONO_FACE),
                ..Spec::BODY
            };
        }
        _ => {}
    }
    match kind {
        BlockKind::Heading { level } => {
            let index = (*level).max(1) as usize - 1;
            let scale = HEADING_SCALE
                .get(index)
                .copied()
                .unwrap_or(HEADING_SCALE[HEADING_SCALE.len() - 1]);
            Spec {
                scale,
                bold: true,
                ..Spec::BODY
            }
        }
        _ => Spec::BODY,
    }
}

/// Every face a document can be set in, body first.
///
/// The list a window builds fonts for, and the reason [`Spec`] equality is enough to look one up:
/// this and [`spec_for`] are the same decisions said twice, and the test at the bottom of this
/// file holds them to each other on any host. A block whose spec is not in here would be drawn in
/// the body face, which is a bug this makes visible rather than one it hides.
pub fn faces() -> Vec<Spec> {
    let mut all = vec![Spec::BODY];
    all.extend(HEADING_SCALE.iter().map(|scale| Spec {
        scale: *scale,
        bold: true,
        ..Spec::BODY
    }));
    all.push(Spec {
        scale: TITLE_SCALE,
        bold: true,
        ..Spec::BODY
    });
    all.push(Spec {
        scale: SUBTITLE_SCALE,
        italic: true,
        ..Spec::BODY
    });
    all.push(Spec {
        family: Some(MONO_FACE),
        ..Spec::BODY
    });
    all
}

/// GDI's per-**code-unit** answer, folded into the per-`char` one [`grind_core::layout::Metrics`]
/// asks for, appended to `out`; the return value is the cumulative advance after the whole string.
///
/// `dx` is `GetTextExtentExPointW`'s `lpnDx`: the width of the string up to and including each
/// UTF-16 code unit. A `char` may be two of those, so the units are counted as the characters are
/// walked and each one takes the entry for its **last** unit — which is what makes a character
/// outside the basic multilingual plane come out as one advance rather than two half ones.
///
/// A short `dx` (GDI declining, or a string it measured differently) does not lose the caller its
/// caret: whatever the last known advance was is repeated, so the array is still one entry per
/// character and still non-decreasing, which is [`grind_core::layout::Metrics`]' whole contract.
pub fn fold_advances(text: &str, dx: &[i32], base: f32, out: &mut Vec<f32>) -> f32 {
    let mut units = 0usize;
    let mut last = base;
    for c in text.chars() {
        units += c.len_utf16();
        if let Some(width) = dx.get(units - 1) {
            last = base + *width as f32;
        }
        out.push(last);
    }
    last
}

#[cfg(windows)]
pub use windows_impl::{Faces, Fonts};

#[cfg(windows)]
mod windows_impl {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use windows::Win32::Foundation::{COLORREF, RECT, SIZE};
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleDC, DeleteDC, ETO_OPAQUE, ExtTextOutW, GetTextExtentExPointW,
        GetTextMetricsW, HDC, HFONT, HGDIOBJ, SelectObject, SetBkColor, SetTextColor, TEXTMETRICW,
    };
    use windows::core::PCWSTR;

    use grind_core::layout::Metrics;
    use grind_core::style::TextStyle;
    use grind_text::BlockKind;
    use grind_text::style::CharStyle;

    use crate::gdi::Font;
    use crate::theme::Rgb;

    use super::{BODY_FACE, Spec, fold_advances, spec_for};

    /// One entry in the font cache: everything `CreateFontIndirectW` is given.
    ///
    /// Underline and strikethrough are in the key because GDI draws both from the `LOGFONT`
    /// rather than from a separate call — which is why this shell gets them for free where the
    /// GTK window needed Pango attributes.
    #[derive(Clone, PartialEq, Eq, Hash)]
    struct Key {
        face: String,
        height: i32,
        bold: bool,
        italic: bool,
        underline: bool,
        strike: bool,
    }

    /// The measuring surface and the fonts on it.
    ///
    /// A [`Metrics`] implementation needs a device context and a font, and this owns both. The DC
    /// is `CreateCompatibleDC(None)` — **no window required**, which is what makes measuring work
    /// under `--render-to` with no `HWND`, no compositor and no display.
    ///
    /// The map is `RefCell` because the trait's methods take `&self`, and it is a *cache* because
    /// creating a font per call would be a `CreateFontIndirectW` per run per repaint — visible on
    /// every keystroke. Every handle is deleted when this is dropped, and the window drops it and
    /// builds another on `WM_DPICHANGED`.
    pub struct Fonts {
        dc: HDC,
        /// The body size in pixels, already scaled for this monitor.
        body_px: f64,
        cache: RefCell<HashMap<Key, Font>>,
    }

    impl Fonts {
        pub fn new(body_px: f64) -> Option<Self> {
            // SAFETY: a memory DC derived from no window; deleted in `Drop`.
            let dc = unsafe { CreateCompatibleDC(None) };
            if dc.is_invalid() {
                return None;
            }
            Some(Fonts {
                dc,
                body_px,
                cache: RefCell::new(HashMap::new()),
            })
        }

        pub fn body_px(&self) -> f64 {
            self.body_px
        }

        /// The handle for one exact font, made once and kept.
        ///
        /// A handle rather than a `&Font`, and that is what keeps the `RefCell` honest: the
        /// borrow ends inside this function, and an `HFONT` stays valid for as long as the map
        /// holds the `Font` that owns it — which is this object's whole life.
        fn font(&self, key: Key) -> HFONT {
            let mut cache = self.cache.borrow_mut();
            cache
                .entry(key.clone())
                .or_insert_with(|| {
                    Font::styled(
                        &key.face,
                        key.height,
                        key.bold,
                        key.italic,
                        key.underline,
                        key.strike,
                    )
                })
                .handle()
        }

        /// The font a fragment of `spec`'s block, formatted with `props`, is set in.
        ///
        /// The run's own formatting is layered over the block's: `**bold**` inside a heading is
        /// bold *and* a heading. `fo:font-size` is deliberately **not** honoured — see
        /// [`Face::line_height`], which is where that stops and why.
        fn resolve(&self, spec: &Spec, props: &CharStyle) -> HFONT {
            let face = props
                .font_family
                .as_deref()
                .map(mapped_family)
                .or(spec.family)
                .unwrap_or(BODY_FACE);
            self.font(Key {
                face: face.to_owned(),
                height: (self.body_px * spec.scale).round().max(1.0) as i32,
                bold: spec.bold || props.is_bold(),
                italic: spec.italic || props.is_italic(),
                underline: props.is_underlined(),
                strike: props.is_struck(),
            })
        }

        /// Measure `text` in `font` on `dc`, appending one cumulative advance per `char`.
        ///
        /// The one call this whole file is built on, and the two guards in front of it are both
        /// scars. An **empty string** is refused before `as_ptr` is taken: an empty `Vec<u16>`
        /// has no allocation, so the pointer is dangling — that is W3's access violation reading
        /// address 2, in the drawing code, and it is the same mistake here. And the advance array
        /// is sized in *code units*, not characters, because that is what GDI fills in.
        fn measure(&self, dc: HDC, font: HFONT, text: &str, base: f32, out: &mut Vec<f32>) -> f32 {
            let units: Vec<u16> = text.encode_utf16().collect();
            if units.is_empty() {
                return base;
            }
            let mut dx = vec![0i32; units.len()];
            let mut size = SIZE::default();
            // SAFETY: the buffers are live locals of exactly the lengths given, the count is the
            // slice's own, and the font is restored before the call returns.
            unsafe {
                let previous = SelectObject(dc, HGDIOBJ(font.0));
                let _ = GetTextExtentExPointW(
                    dc,
                    PCWSTR(units.as_ptr()),
                    units.len() as i32,
                    0,
                    None,
                    Some(dx.as_mut_ptr()),
                    &mut size,
                );
                SelectObject(dc, previous);
            }
            fold_advances(text, &dx, base, out)
        }

        /// How wide one `text:tab` is in `font` — [`TAB_SPACES`] spaces of it.
        fn tab_width(&self, font: HFONT) -> f32 {
            let spaces = " ".repeat(super::TAB_SPACES);
            let mut out = Vec::with_capacity(super::TAB_SPACES);
            self.measure(self.dc, font, &spaces, 0.0, &mut out);
            out.last().copied().unwrap_or(0.0)
        }

        /// How tall a line set in `font` is: ascent, descent and the leading the font itself asks
        /// for between one line and the next.
        fn height(&self, font: HFONT) -> f32 {
            let mut tm = TEXTMETRICW::default();
            // SAFETY: `tm` is a live local; the font is restored before the call returns.
            unsafe {
                let previous = SelectObject(self.dc, HGDIOBJ(font.0));
                let _ = GetTextMetricsW(self.dc, &mut tm);
                SelectObject(self.dc, previous);
            }
            ((tm.tmHeight + tm.tmExternalLeading) as f32).max(1.0)
        }
    }

    impl Drop for Fonts {
        fn drop(&mut self) {
            // The `Font`s go first: `DeleteObject` on a handle still selected into a DC silently
            // does nothing, and `measure` restores the previous object on every path, so nothing
            // of ours is selected here. The DC is deleted last.
            self.cache.borrow_mut().clear();
            // SAFETY: the DC is ours, made in `new`, and nothing refers to it afterwards.
            unsafe {
                let _ = DeleteDC(self.dc);
            }
        }
    }

    /// One block-level face: a [`Spec`], the fonts it resolves against, and the line height it
    /// answers with.
    ///
    /// Borrowed rather than owning, because [`grind_text::Faces`] hands out `&dyn Metrics` and a
    /// face therefore has to outlive the call that asked for it — so the cache lives in the
    /// window and a `Faces` is built around it per operation.
    pub struct Face<'a> {
        fonts: &'a Fonts,
        /// Which block face this is. Compared rather than remembered by position, which is how
        /// [`Faces::face`] finds one without a second copy of [`spec_for`]'s decisions.
        spec: Spec,
        height: f32,
    }

    impl<'a> Face<'a> {
        pub fn new(fonts: &'a Fonts, spec: Spec) -> Self {
            let height = fonts.height(fonts.resolve(&spec, &CharStyle::default()));
            Face {
                fonts,
                spec,
                height,
            }
        }

        /// Draw one run at `x`, with its top at `top`, in the font it was **measured** in.
        ///
        /// `ExtTextOutW` with the advance array that same measurement produced, which is decision
        /// 3's rule and the reason this function exists rather than a `DrawTextW` call: GDI
        /// placing glyphs freely and the core placing carets by its own arithmetic is exactly the
        /// disagreement the caret would show. The advances are per code unit, which is the unit
        /// `ExtTextOutW` wants — so this is the one place the fold back to characters is *not*
        /// done.
        ///
        /// Returns how far the pen moved, so a caller drawing a line run by run needs no second
        /// measurement of its own.
        pub fn draw_run(
            &self,
            dc: HDC,
            x: f64,
            top: f64,
            text: &str,
            props: &CharStyle,
            ink: Rgb,
        ) -> f64 {
            let units: Vec<u16> = text.encode_utf16().collect();
            if units.is_empty() {
                return 0.0;
            }
            let font = self.fonts.resolve(&self.spec, props);
            let mut dx = vec![0i32; units.len()];
            let mut size = SIZE::default();
            // SAFETY: every buffer is a live local of exactly the length given; the DC's previous
            // font is restored before returning, so nothing of the caller's is disturbed and no
            // handle is left selected where `Fonts::drop` would fail to delete it.
            unsafe {
                let previous = SelectObject(dc, HGDIOBJ(font.0));
                let _ = GetTextExtentExPointW(
                    dc,
                    PCWSTR(units.as_ptr()),
                    units.len() as i32,
                    0,
                    None,
                    Some(dx.as_mut_ptr()),
                    &mut size,
                );
                // Per-unit widths from the cumulative ones — the difference between neighbours,
                // which is what `lpDx` is.
                let mut widths = Vec::with_capacity(dx.len());
                let mut previous_x = 0;
                for cumulative in &dx {
                    widths.push(cumulative - previous_x);
                    previous_x = *cumulative;
                }
                // Highlighting is `fo:background-color`, and `ETO_OPAQUE` with the background
                // colour set is GDI's own way to paint it under exactly the run's own extent —
                // which is why it is here and not a rectangle computed somewhere else.
                let highlight = props
                    .background
                    .as_deref()
                    .filter(|value| *value != "transparent")
                    .and_then(Rgb::parse);
                let rect = RECT {
                    left: x.round() as i32,
                    top: top.round() as i32,
                    right: (x + f64::from(size.cx)).round() as i32,
                    bottom: (top + f64::from(self.height)).round() as i32,
                };
                if let Some(colour) = highlight {
                    SetBkColor(dc, COLORREF(colour.colorref()));
                }
                SetTextColor(
                    dc,
                    COLORREF(
                        props
                            .color
                            .as_deref()
                            .and_then(Rgb::parse)
                            .unwrap_or(ink)
                            .colorref(),
                    ),
                );
                let _ = ExtTextOutW(
                    dc,
                    rect.left,
                    rect.top,
                    match highlight {
                        Some(_) => ETO_OPAQUE,
                        None => Default::default(),
                    },
                    Some(&rect),
                    PCWSTR(units.as_ptr()),
                    units.len() as u32,
                    Some(widths.as_ptr()),
                );
                SelectObject(dc, previous);
            }
            f64::from(size.cx)
        }
    }

    impl Metrics for Face<'_> {
        /// One cumulative advance per `char`, in pixels.
        ///
        /// **A line break inside a fragment is measured as nothing.** `Run::Break` is a character
        /// in the model and arrives here as `\n`; handing it to GDI would measure whatever glyph
        /// the font has for it. The core breaks the line there anyway, so the caret sitting on it
        /// belongs at the end of the text before it — the same answer `ui_text_gtk` gives, for
        /// the same reason.
        fn advances(&self, text: &str, style: &TextStyle, out: &mut Vec<f32>) {
            // The fragment's own formatting, as the core projected it out of the run. Only the
            // four properties that change how *wide* text is are in a `TextStyle`, which is
            // exactly the set that reaches the font here.
            let props = CharStyle {
                font_family: style.font_family.clone(),
                font_size: style.font_size.clone(),
                font_weight: style.font_weight.clone(),
                font_style: style.font_style.clone(),
                ..CharStyle::default()
            };
            let font = self.fonts.resolve(&self.spec, &props);
            let tab = self.fonts.tab_width(font);
            let mut base = 0.0;
            for (line, segment) in text.split('\n').enumerate() {
                if line > 0 {
                    // The `\n` itself. One advance, because the contract is one per character,
                    // and no width, because the line ends there.
                    out.push(base);
                }
                // **A tab is measured, never drawn** (`text/draw.rs`), and it is measured here
                // rather than handed to GDI: `ExtTextOutW` draws whatever glyph the font has for
                // U+0009, which for Segoe UI is a box. A `text:tab` is a character in the model,
                // so it gets exactly one advance like every other character — a fixed width and
                // not a tab *stop*, because a stop is a paragraph property this build does not
                // read (`doc/text-core.md`) and inventing one would put the caret where no
                // document asked for it.
                for (piece, run) in segment.split('\t').enumerate() {
                    if piece > 0 {
                        base += tab;
                        out.push(base);
                    }
                    base = self.fonts.measure(self.fonts.dc, font, run, base, out);
                }
            }
        }

        /// The block face's own height, whatever the fragment is set in.
        ///
        /// **`fo:font-size` on a run is not honoured**, here or in `ui_text_gtk`, and the two
        /// halves have to move together or neither does: a size honoured in the width and not in
        /// the height would measure a big word wide on a line too short to hold it. The notation
        /// this shell draws sets a family and no size, so this is where the work stops and is
        /// written down rather than half done.
        fn line_height(&self, _style: &TextStyle) -> f32 {
            self.height
        }
    }

    /// The faces a document is set in, and the measure it is set to — [`grind_text::Faces`], which
    /// is *which* metrics this block wants rather than a single provider for all of them.
    ///
    /// Built per operation around the window's [`Fonts`], and the reason it exists at all is a
    /// bug found in the core while `ui_text_gtk` was being built: Down-arrow out of a heading
    /// measured the paragraph below it with the heading's font. This shell inherits the fix and
    /// must not re-introduce it by reaching for `Uniform`.
    pub struct Faces<'a> {
        width: f32,
        /// How much narrower each nesting level of a list is. **The indent comes out of the
        /// column**, so a list item is measured to less than the measure — and it has to be the
        /// same subtraction the flow makes when it places the text, or a wrapped list item would
        /// break its lines in one place and draw them in another.
        indent: f32,
        /// Every face a block can be set in, body first. A flat list because the lookup is
        /// [`Spec`] equality — [`spec_for`] decides which face a block wants and this finds the
        /// one built for it, so the decision is written down once.
        all: Vec<Face<'a>>,
    }

    impl<'a> Faces<'a> {
        pub fn new(fonts: &'a Fonts, width: f64, indent: f64) -> Self {
            Faces {
                width: width as f32,
                indent: indent as f32,
                all: super::faces()
                    .into_iter()
                    .map(|spec| Face::new(fonts, spec))
                    .collect(),
            }
        }

        /// The face a block is set in — [`spec_for`]'s answer, looked *up* rather than rebuilt, so
        /// that a paint measuring three hundred blocks creates no fonts at all.
        ///
        /// A spec with no face falls back to the body's rather than panicking: the two lists are
        /// built from the same constants and a mismatch would be a bug, but a document is not the
        /// place to discover one.
        pub fn face(&self, kind: &BlockKind, style: Option<&str>) -> &Face<'a> {
            let spec = spec_for(kind, style);
            self.all
                .iter()
                .find(|face| face.spec == spec)
                .unwrap_or(&self.all[0])
        }
    }

    impl grind_text::Faces for Faces<'_> {
        fn of(&self, _index: usize, kind: &BlockKind, style: Option<&str>) -> (f32, &dyn Metrics) {
            let indent = match kind {
                BlockKind::ListItem { depth } => *depth as f32 * self.indent,
                _ => 0.0,
            };
            ((self.width - indent).max(1.0), self.face(kind, style))
        }
    }

    /// A document's family name as a face name GDI will actually find.
    ///
    /// The one substitution, and it is the generic family a fenced code block carries: everything
    /// else is passed through exactly as the document spells it, because `fo:font-family` is a
    /// real face name in every document that sets one.
    fn mapped_family(family: &str) -> &str {
        match family {
            grind_text::markdown::MONOSPACE => super::MONO_FACE,
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_heading_is_bigger_than_a_paragraph_and_a_title_bigger_still() {
        let body = spec_for(&BlockKind::Paragraph, None);
        let h1 = spec_for(&BlockKind::Heading { level: 1 }, None);
        let title = spec_for(&BlockKind::Paragraph, Some("Title"));
        assert_eq!(body, Spec::BODY);
        assert!(h1.scale > body.scale && h1.bold);
        assert!(title.scale > h1.scale);
        assert!(spec_for(&BlockKind::Paragraph, Some("Subtitle")).italic);
    }

    /// R5 is tolerance on the way in, and a level-9 heading is a document this build loads. A
    /// shell that indexed the scale table with it would panic on one.
    #[test]
    fn a_heading_deeper_than_the_faces_go_is_set_as_the_last_of_them() {
        let deep = spec_for(&BlockKind::Heading { level: 9 }, None);
        assert_eq!(deep.scale, HEADING_SCALE[HEADING_SCALE.len() - 1]);
        assert!(deep.bold);
    }

    /// The generic family a fence carries is resolved by fontconfig and by a browser, and *not*
    /// by GDI — so it is named here rather than handed to `CreateFontIndirectW`.
    #[test]
    fn a_fenced_block_is_set_in_a_real_monospace_face() {
        let fence = spec_for(
            &BlockKind::Paragraph,
            Some(grind_text::markdown::PREFORMATTED),
        );
        assert_eq!(fence.family, Some(MONO_FACE));
        assert_ne!(MONO_FACE, grind_text::markdown::MONOSPACE);
        assert_eq!(
            fence.scale, 1.0,
            "a code block is a paragraph, not a heading"
        );
    }

    /// Every face a block can ask for is a face the window built, which is what makes [`Spec`]
    /// equality a safe way to look one up. A new kind of block with a face of its own fails here
    /// until [`faces`] knows about it.
    #[test]
    fn every_block_has_a_face_built_for_it() {
        let built = faces();
        let mut asked = vec![
            spec_for(&BlockKind::Paragraph, None),
            spec_for(&BlockKind::Paragraph, Some("Title")),
            spec_for(&BlockKind::Paragraph, Some("Subtitle")),
            spec_for(
                &BlockKind::Paragraph,
                Some(grind_text::markdown::PREFORMATTED),
            ),
            spec_for(&BlockKind::ListItem { depth: 2 }, None),
            // A name this build keeps and does not interpret, which must land on the body face.
            spec_for(&BlockKind::Paragraph, Some("Quotations")),
        ];
        asked.extend((1..=9).map(|level| spec_for(&BlockKind::Heading { level }, None)));
        for spec in asked {
            assert!(built.contains(&spec), "no face built for {spec:?}");
        }
    }

    /// The mechanical half of decision 3: GDI answers per UTF-16 code unit and the trait asks per
    /// `char`, so each character takes the entry for its **last** unit.
    #[test]
    fn advances_are_per_character_where_gdi_answers_per_code_unit() {
        let mut out = Vec::new();
        // "ab" — one unit each.
        let last = fold_advances("ab", &[7, 15], 0.0, &mut out);
        assert_eq!(out, vec![7.0, 15.0]);
        assert_eq!(last, 15.0);
    }

    /// A character outside the basic multilingual plane is two code units and **one** advance —
    /// which is more than `ui_tui`'s cell counting manages.
    #[test]
    fn a_character_outside_the_bmp_is_one_advance() {
        let mut out = Vec::new();
        // "a𝄞b": 1 unit, 2 units, 1 unit — four entries from GDI, three characters out.
        fold_advances("a\u{1D11E}b", &[6, 12, 20, 26], 0.0, &mut out);
        assert_eq!(out, vec![6.0, 20.0, 26.0], "the surrogate pair is one step");
    }

    /// The base is where a fragment starts, because a line is measured fragment by fragment and
    /// the offsets are into the whole of it.
    #[test]
    fn a_fragment_is_measured_from_where_the_last_one_ended() {
        let mut out = Vec::new();
        let last = fold_advances("xy", &[4, 9], 100.0, &mut out);
        assert_eq!(out, vec![104.0, 109.0]);
        assert_eq!(last, 109.0);
    }

    /// GDI declining must not lose the caller its caret: the contract is one entry per character,
    /// never decreasing, and a short answer still has to keep it.
    #[test]
    fn a_short_answer_still_gives_one_advance_per_character() {
        let mut out = Vec::new();
        fold_advances("abc", &[5], 0.0, &mut out);
        assert_eq!(out, vec![5.0, 5.0, 5.0]);
        let mut none = Vec::new();
        fold_advances("ab", &[], 3.0, &mut none);
        assert_eq!(none, vec![3.0, 3.0]);
        assert!(none.windows(2).all(|w| w[1] >= w[0]));
    }

    #[test]
    fn an_empty_string_measures_to_nothing() {
        let mut out = Vec::new();
        assert_eq!(fold_advances("", &[], 12.0, &mut out), 12.0);
        assert!(out.is_empty());
    }
}
