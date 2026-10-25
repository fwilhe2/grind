// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Font metrics through GDI — this shell's whole contribution to layout (decision 3). Line
//! breaking is `grind_core::layout`'s; a shell supplies [`grind_core::layout::Metrics`]. Win32 has
//! no Pango: GDI maps glyphs, it does not shape them.
//!
//! ## The measuring engine and the drawing engine are the same one
//!
//! `GetTextExtentExPointW` does not shape, so a combining mark gets an advance of its own and the
//! core thinks the text a mark wider than it looks. That is survivable **only if drawing agrees**:
//! `Face::draw_run` measures with the same call and font and hands the result to `ExtTextOutW` as
//! explicit advances, so the mark is drawn in a box of its own — wrong-looking, never
//! inconsistent, and every caret stays right. Measuring with DirectWrite while drawing with
//! `ExtTextOutW` would disagree, which is why the upgrade is a whole stack.
//!
//! The trigger, written down so it is not a judgement call later: **the first time a caret lands
//! in the wrong place because characters of one cluster were measured as separate boxes**, this
//! file moves to DirectWrite — measuring *and* drawing — and nothing above it changes.
//!
//! ## Two halves
//!
//! [`Spec`] and [`spec_for`] (which face a block is set in) and [`fold_advances`] are portable:
//! GDI answers per **UTF-16 code unit** and the trait asks per `char`, so each character takes the
//! entry for its *last* unit, and a character outside the BMP is one advance. `windows_impl`
//! measures on a memory DC from `CreateCompatibleDC(None)`, so it needs no window — which lets
//! `--render-to` lay a document out on a headless runner.

use grind_text::BlockKind;
use grind_text::look::Role;

/// The face prose is set in. Segoe UI is the shell font on every Windows this shell targets, and
/// GDI substitutes when it is absent — which is what happens under Wine.
pub const BODY_FACE: &str = "Segoe UI";

/// What `grind_text::markdown::MONOSPACE` resolves to. `monospace` is a generic family, which
/// fontconfig and a browser resolve and GDI does not — `CreateFontIndirectW` silently substitutes,
/// a proportional face for a code block. So the generic is named here: Consolas, shipped with
/// Windows since Vista.
pub const MONO_FACE: &str = "Consolas";

/// How wide a `text:tab` is, in spaces — a width, **not a tab stop**: this pane does not apply a
/// paragraph's `style:tab-stops`, and snapping to an invented stop would be inventing the
/// document's layout. Four, as the terminal and the browser use.
pub const TAB_SPACES: usize = 4;

/// Which face a block is set in, resolved from what the document says, with no Windows in it.
/// **Chosen per block by the shell**: a heading's or a `Title`'s face is the block's, by kind and
/// style name (`grind_text::look`), so the shell hands the core a provider already set to the
/// block's font, or every caret would be off.
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
    /// The face `role` is set in here: its scale and weight are `grind_text::look`'s, and the
    /// monospace one is [`MONO_FACE`], since GDI resolves no generic family.
    pub fn of(role: Role) -> Spec {
        Spec {
            scale: role.scale(),
            bold: role.bold(),
            italic: role.italic(),
            family: role.mono().then_some(MONO_FACE),
        }
    }
}

/// The [`Spec`] for one block — [`Role::of`], which decides which name wins over which kind and
/// what a heading deeper than six levels is set in, for every shell that draws a page.
pub fn spec_for(kind: &BlockKind, style: Option<&str>) -> Spec {
    Spec::of(Role::of(kind, style))
}

/// Every face a document can be set in, body first — [`Role::ALL`], the list a window builds
/// fonts for, and the reason [`Spec`] equality is enough to look one up. The test at the bottom
/// of this file holds it to [`spec_for`] on any host.
pub fn faces() -> Vec<Spec> {
    Role::ALL.into_iter().map(Spec::of).collect()
}

/// GDI's per-**code-unit** answer folded into the per-`char` one [`grind_core::layout::Metrics`]
/// asks for, appended to `out`; returns the cumulative advance after the string. `dx` is
/// `GetTextExtentExPointW`'s `lpnDx`, each unit's cumulative width; each character takes its
/// **last** unit's entry, so one outside the BMP is one advance. A short `dx` repeats the last
/// known advance, so the array stays one entry per character and non-decreasing — the trait's
/// contract.
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

/// The body face's size in points: [`crate::theme::text::PROSE`] pixels at 96 DPI.
const BODY_PT: f64 = crate::theme::text::PROSE * 72.0 / 96.0;

/// A run's `fo:font-size` in pixels, in a block whose face is `block_px`, the body being `body_px`:
/// a point length (`14pt`) or a share of the block's face (`120%`). Anything else, a `cm` size
/// included, is the block's own.
pub fn size_px(font_size: Option<&str>, block_px: f64, body_px: f64) -> f64 {
    let Some(value) = font_size.map(str::trim) else {
        return block_px;
    };
    if let Some(points) = value
        .strip_suffix("pt")
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|points| *points > 0.0)
    {
        return points * body_px / BODY_PT;
    }
    value
        .strip_suffix('%')
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|percent| *percent > 0.0)
        .map_or(block_px, |percent| block_px * percent / 100.0)
}

#[cfg(windows)]
pub use windows_impl::{Faces, Fonts};

#[cfg(windows)]
mod windows_impl {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use windows::Win32::Foundation::{COLORREF, RECT, SIZE};
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleDC, DT_CALCRECT, DT_LEFT, DT_NOPREFIX, DT_WORDBREAK, DeleteDC, DrawTextW,
        ETO_OPAQUE, ExtTextOutW, GetTextExtentExPointW, GetTextMetricsW, HDC, HFONT, HGDIOBJ,
        SelectObject, SetBkColor, SetTextColor, TEXTMETRICW,
    };
    use windows::core::PCWSTR;

    use grind_core::layout::Metrics;
    use grind_core::style::TextStyle;
    use grind_text::BlockKind;
    use grind_text::style::CharStyle;

    use crate::gdi::Font;
    use crate::text::geom::{Across, Spacing};
    use crate::theme::Rgb;

    use super::{BODY_FACE, Spec, fold_advances, spec_for};

    /// One entry in the font cache: everything `CreateFontIndirectW` is given — underline and
    /// strikethrough included, since GDI draws both from the `LOGFONT`.
    #[derive(Clone, PartialEq, Eq, Hash)]
    struct Key {
        face: String,
        height: i32,
        bold: bool,
        italic: bool,
        underline: bool,
        strike: bool,
    }

    /// The measuring surface and the fonts on it. The DC is `CreateCompatibleDC(None)` — **no
    /// window required**, so measuring works under `--render-to`. The map is a `RefCell` because
    /// the trait takes `&self`, and a cache because a `CreateFontIndirectW` per run per repaint
    /// would show on every keystroke. Every handle is deleted on drop; the window rebuilds this on
    /// `WM_DPICHANGED`.
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

        /// The measuring surface itself. A picture's caption is measured and drawn with
        /// `DrawTextW` rather than the core's layout (see `Face::wrapped_height`), which needs a
        /// DC.
        pub fn dc(&self) -> HDC {
            self.dc
        }

        /// The handle for one exact font, made once and kept — a handle rather than a `&Font`, so
        /// the `RefCell` borrow ends here; an `HFONT` stays valid as long as the map holds its
        /// `Font`, which is this object's whole life.
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

        /// The font a fragment of `spec`'s block, formatted with `props`, is set in: the run's
        /// formatting over the block's, so `**bold**` in a heading is bold *and* a heading, and a
        /// run's own size over the block's ([`super::size_px`]).
        fn resolve(&self, spec: &Spec, props: &CharStyle) -> HFONT {
            let face = props
                .font_family
                .as_deref()
                .map(mapped_family)
                .or(spec.family)
                .unwrap_or(BODY_FACE);
            self.font(Key {
                face: face.to_owned(),
                height: super::size_px(
                    props.font_size.as_deref(),
                    self.body_px * spec.scale,
                    self.body_px,
                )
                .round()
                .max(1.0) as i32,
                bold: spec.bold || props.is_bold(),
                italic: spec.italic || props.is_italic(),
                underline: props.is_underlined(),
                strike: props.is_struck(),
            })
        }

        /// Measure `text` in `font` on `dc`, appending one cumulative advance per `char`. Two
        /// guards: an **empty string** is refused before `as_ptr`, since an empty `Vec<u16>` has
        /// no allocation and its pointer dangles (an access violation in the drawing code once),
        /// and the advance array is sized in code units, which GDI fills.
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

        /// How wide one `text:tab` is in `font` — [`super::TAB_SPACES`] spaces of it.
        fn tab_width(&self, font: HFONT) -> f32 {
            let spaces = " ".repeat(super::TAB_SPACES);
            let mut out = Vec::with_capacity(super::TAB_SPACES);
            self.measure(self.dc, font, &spaces, 0.0, &mut out);
            out.last().copied().unwrap_or(0.0)
        }

        /// How tall a line set in `font` is: ascent, descent and the leading the font itself asks
        /// for between one line and the next.
        fn height(&self, font: HFONT) -> f32 {
            let tm = self.text_metrics(font);
            ((tm.tmHeight + tm.tmExternalLeading) as f32).max(1.0)
        }

        /// How far below a line's top the baseline of text in `font` sits.
        fn ascent(&self, font: HFONT) -> f32 {
            let tm = self.text_metrics(font);
            (tm.tmAscent + tm.tmExternalLeading) as f32
        }

        fn text_metrics(&self, font: HFONT) -> TEXTMETRICW {
            let mut tm = TEXTMETRICW::default();
            // SAFETY: `tm` is a live local; the font is restored before the call returns.
            unsafe {
                let previous = SelectObject(self.dc, HGDIOBJ(font.0));
                let _ = GetTextMetricsW(self.dc, &mut tm);
                SelectObject(self.dc, previous);
            }
            tm
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

    /// One block-level face: a [`Spec`], the fonts it resolves against, and its line height.
    /// Borrowed, since [`grind_text::Faces`] hands out `&dyn Metrics` that must outlive the call:
    /// the cache lives in the window and a `Faces` is built round it per operation.
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

        /// The font a fragment of this block in `style` is set in, for its height and ascent.
        fn sized(&self, style: &TextStyle) -> HFONT {
            let props = CharStyle {
                font_size: style.font_size.clone(),
                font_weight: style.font_weight.clone(),
                font_style: style.font_style.clone(),
                font_family: style.font_family.clone(),
                ..CharStyle::default()
            };
            self.fonts.resolve(&self.spec, &props)
        }

        /// Draw one run at `x`, sitting on `baseline`, in the font it was **measured** in:
        /// `ExtTextOutW` with that measurement's advance array (decision 3), not `DrawTextW`, which
        /// would place glyphs where the caret arithmetic does not. The advances are per code unit,
        /// which `ExtTextOutW` wants, so this is the one place they are not folded to characters.
        /// Returns how far the pen moved.
        pub fn draw_run(
            &self,
            dc: HDC,
            x: f64,
            baseline: f64,
            text: &str,
            props: &CharStyle,
            ink: Rgb,
        ) -> f64 {
            let units: Vec<u16> = text.encode_utf16().collect();
            if units.is_empty() {
                return 0.0;
            }
            let font = self.fonts.resolve(&self.spec, props);
            let top = baseline - f64::from(self.fonts.ascent(font));
            let height = self.fonts.height(font);
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
                    bottom: (top + f64::from(height)).round() as i32,
                };
                if let Some(colour) = highlight {
                    SetBkColor(dc, COLORREF(colour.colorref()));
                }
                // `ink` is already the run's own colour where it has one, made to read on the
                // ground it lands on (`theme::document_ink`) — the caller's decision, not this
                // measuring call's.
                SetTextColor(dc, COLORREF(ink.colorref()));
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

        /// How tall `text` comes out wrapped to `width` in this face — a picture's caption,
        /// wrapped by `DrawTextW` rather than `grind_core::layout`. The one text outside decision
        /// 3's rule, on purpose: no caret goes in a caption, so nothing can disagree with it.
        pub fn wrapped_height(&self, dc: HDC, text: &str, width: f64) -> f64 {
            f64::from(self.wrapped_rect(dc, text, width).bottom)
        }

        /// Draw `text` wrapped to `width`, top-left at `x, y` — the caption's own ink.
        pub fn draw_wrapped(&self, dc: HDC, text: &str, x: f64, y: f64, width: f64, ink: Rgb) {
            let mut wide: Vec<u16> = text.encode_utf16().collect();
            if wide.is_empty() {
                return;
            }
            let mut rect = RECT {
                left: x.round() as i32,
                top: y.round() as i32,
                right: (x + width).round() as i32,
                bottom: i32::MAX,
            };
            let font = self.fonts.resolve(&self.spec, &CharStyle::default());
            // SAFETY: `wide` and `rect` are live locals for the length of the call; the DC's
            // previous font is restored before returning.
            unsafe {
                let previous = SelectObject(dc, HGDIOBJ(font.0));
                SetTextColor(dc, COLORREF(ink.colorref()));
                let _ = DrawTextW(
                    dc,
                    &mut wide[..],
                    &mut rect,
                    DT_WORDBREAK | DT_NOPREFIX | DT_LEFT,
                );
                SelectObject(dc, previous);
            }
        }

        /// [`Self::wrapped_height`]'s own measurement, factored out so drawing never asks GDI a
        /// second question the height did not already answer.
        fn wrapped_rect(&self, dc: HDC, text: &str, width: f64) -> RECT {
            let mut wide: Vec<u16> = text.encode_utf16().collect();
            if wide.is_empty() {
                return RECT::default();
            }
            let mut rect = RECT {
                left: 0,
                top: 0,
                right: width.max(1.0).round() as i32,
                bottom: 0,
            };
            let font = self.fonts.resolve(&self.spec, &CharStyle::default());
            // SAFETY: `wide` and `rect` are live locals for the length of the call; the DC's
            // previous font is restored before returning.
            unsafe {
                let previous = SelectObject(dc, HGDIOBJ(font.0));
                let _ = DrawTextW(
                    dc,
                    &mut wide[..],
                    &mut rect,
                    DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX | DT_LEFT,
                );
                SelectObject(dc, previous);
            }
            rect
        }
    }

    impl Metrics for Face<'_> {
        /// One cumulative advance per `char`, in pixels. **A line break in a fragment measures as
        /// nothing**: `Run::Break` arrives as `\n`, which GDI would measure as whatever glyph the
        /// font has; the core breaks there, so the caret on it belongs at the end of the text
        /// before.
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
                // **A tab is measured, never drawn** (`text/draw.rs`), and measured here, since
                // `ExtTextOutW` draws Segoe UI's box for U+0009: one fixed advance, like any
                // character in the model.
                for (piece, run) in segment.split('\t').enumerate() {
                    if piece > 0 {
                        base += tab;
                        out.push(base);
                    }
                    base = self.fonts.measure(self.fonts.dc, font, run, base, out);
                }
            }
        }

        /// The height of the font the fragment is set in; `layout::wrap` takes the tallest, so a
        /// 24pt word makes room for itself.
        fn line_height(&self, style: &TextStyle) -> f32 {
            match style.font_size {
                None => self.height,
                Some(_) => self.fonts.height(self.sized(style)),
            }
        }

        fn ascent(&self, style: &TextStyle) -> f32 {
            self.fonts.ascent(self.sized(style))
        }
    }

    /// The faces a document is set in and the measure it is set to — [`grind_text::Faces`], which
    /// metrics *this* block wants, built per operation round the window's [`Fonts`]. Not
    /// `Uniform`: Down-arrow out of a heading must measure the paragraph below in its own font.
    pub struct Faces<'a> {
        /// The text column's width, before any indent comes out of it.
        column: f64,
        /// The pane's numbers at this monitor's scaling. **The indent comes out of the column**,
        /// through [`Spacing::measure`] — the function the flow places text with — or a wrapped
        /// list item would break in one place and draw in another.
        spacing: Spacing,
        /// Every face a block can be set in, body first. A flat list because the lookup is
        /// [`Spec`] equality — [`spec_for`] decides which face a block wants and this finds the
        /// one built for it, so the decision is written down once.
        all: Vec<Face<'a>>,
        /// The measure of every block in a table cell — `text::geom::across`, built per reflow
        /// beside the flow — so a Down-arrow inside a cell is measured where the ink is. Read by
        /// index, never by asking the document, since `grind_text::Faces::of` runs under `App`'s
        /// read lock.
        cells: &'a HashMap<usize, Across>,
    }

    impl<'a> Faces<'a> {
        pub fn new(
            fonts: &'a Fonts,
            column: f64,
            spacing: Spacing,
            cells: &'a HashMap<usize, Across>,
        ) -> Self {
            Faces {
                column,
                spacing,
                cells,
                all: super::faces()
                    .into_iter()
                    .map(|spec| Face::new(fonts, spec))
                    .collect(),
            }
        }

        /// The face a block is set in — [`spec_for`]'s answer looked up, so a paint measuring
        /// three hundred blocks creates no fonts. A spec with no face falls back to the body's:
        /// the lists come from the same constants, and a document is no place to discover a
        /// mismatch.
        pub fn face(&self, kind: &BlockKind, style: Option<&str>) -> &Face<'a> {
            let spec = spec_for(kind, style);
            self.all
                .iter()
                .find(|face| face.spec == spec)
                .unwrap_or(&self.all[0])
        }
    }

    impl grind_text::Faces for Faces<'_> {
        fn of(&self, index: usize, kind: &BlockKind, style: Option<&str>) -> (f32, &dyn Metrics) {
            let width = match self.cells.get(&index) {
                Some(cell) => cell.width,
                None => self.spacing.measure(kind, self.column),
            };
            ((width as f32).max(1.0), self.face(kind, style))
        }
    }

    /// A document's family name as a face name GDI will find: the one substitution is the generic
    /// family of a fenced code block; everything else passes through as the document spells it.
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
        assert_eq!(body, Spec::of(Role::Body));
        assert!(h1.scale > body.scale && h1.bold);
        assert!(title.scale > h1.scale);
        assert!(spec_for(&BlockKind::Paragraph, Some("Subtitle")).italic);
    }

    /// A level-9 heading loads (R5), and indexing the scale table with it must not panic.
    #[test]
    fn a_heading_deeper_than_the_faces_go_is_set_as_the_last_of_them() {
        let deep = spec_for(&BlockKind::Heading { level: 9 }, None);
        assert_eq!(deep, Spec::of(Role::Heading(6)));
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
            // A name kept and not interpreted lands on the body face.
            spec_for(&BlockKind::Paragraph, Some("Quotations")),
        ];
        asked.extend((1..=9).map(|level| spec_for(&BlockKind::Heading { level }, None)));
        for spec in asked {
            assert!(built.contains(&spec), "no face built for {spec:?}");
        }
    }

    /// GDI answers per UTF-16 code unit and the trait asks per `char`, so each character takes its
    /// **last** unit's entry.
    #[test]
    fn advances_are_per_character_where_gdi_answers_per_code_unit() {
        let mut out = Vec::new();
        // "ab" — one unit each.
        let last = fold_advances("ab", &[7, 15], 0.0, &mut out);
        assert_eq!(out, vec![7.0, 15.0]);
        assert_eq!(last, 15.0);
    }

    /// A run's size: points against the body's, a percentage against its block's face, anything
    /// else the block's own.
    #[test]
    fn a_runs_size_is_points_or_a_share_of_its_block() {
        let body = crate::theme::text::PROSE;
        assert_eq!(size_px(None, 30.0, body), 30.0);
        assert_eq!(size_px(Some("11.25pt"), 30.0, body), body);
        assert_eq!(size_px(Some("22.5pt"), 30.0, body), 2.0 * body);
        assert_eq!(size_px(Some("150%"), 30.0, body), 45.0);
        assert_eq!(size_px(Some("1cm"), 30.0, body), 30.0);
        assert_eq!(size_px(Some("-3pt"), 30.0, body), 30.0);
    }

    /// A character outside the basic multilingual plane is two code units and **one** advance.
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
