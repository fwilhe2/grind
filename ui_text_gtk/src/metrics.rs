// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Font metrics, through Pango — this shell's whole contribution to layout.
//!
//! `doc/text-layout.md` decided (Path C) that line breaking lives in `grind_core::layout` and
//! that a shell supplies only [`Metrics`]: how wide is this text, and how tall is a line of
//! it. `ui_tui/src/text/mod.rs` answers in terminal cells in about twenty lines; this answers
//! in pixels through a real shaping engine, and that is the pair the decision was made for.
//! **If the two disagreed about anything but the numbers, the abstraction would be wrong.**
//!
//! Two things are worth knowing about the implementation:
//!
//! * **Cumulative advances come from `pango::LayoutLine::index_to_x`**, not from measuring
//!   prefixes. Pango shapes the whole string once and can then answer the trailing edge of
//!   every cluster from that one pass, which is exactly the contract [`Metrics::advances`]
//!   asks for — and the reason it asks for it: `advance("a") + advance("b")` is not
//!   `advance("ab")` once kerning is involved.
//! * **A line break inside a fragment is measured as nothing.** `Run::Break` is a character
//!   in the model and reaches here as `\n`; handing it to Pango would make a second Pango
//!   line whose x coordinates start over. The core breaks the line there anyway, so the
//!   caret sitting on it belongs at the end of the text before it.
//! * **A fragment is measured in its own formatting.** `grind_text::lay_out` projects each
//!   run's `CharStyle` into the [`TextStyle`] it hands the provider per fragment, so the
//!   family, the weight and the slant arrive here — and are applied to the measuring layout
//!   as Pango attributes rather than ignored. [`run_attributes`] puts the same three on the
//!   *drawing* layout, from the same runs. That pairing is the point: a family drawn but not
//!   measured drifts the caret through the run, which is what a `` `code` `` run did before
//!   either half existed.
//!
//! **The face is chosen per block, by the shell.** A run's *direct* formatting reaches the
//! core, but a block's does not: a heading and a `Title` are a paragraph **style**, and style
//! definitions are not read (`doc/text-core.md`) — so a heading laid out in the body font and
//! drawn in a larger one would put every caret in the wrong place. The shell knows the block's
//! kind and its style name, so it hands the core a provider already set to that block's font.
//! The core neither knows nor needs to: it does arithmetic in whatever unit it is answered in.
//!
//! **What a fragment's style still does not change is its `fo:font-size`.** A line's height is
//! [`Metrics::line_height`]'s answer per fragment and `layout::wrap` takes the tallest, so a
//! size honoured in the width and not in the height would measure a big word wide on a line
//! too short to hold it. Both halves and the drawing move together or none of them does; the
//! notation this file exists for sets a family and no size, so this is where the work stops
//! and is written down rather than half done.

use libadwaita::gtk;

use grind_core::style::TextStyle;
use grind_text::style::CharStyle;
use grind_text::{BlockKind, Metrics, RunView};
use gtk::pango;

/// How much bigger than the body text each heading level is.
///
/// Six levels because that is where `doc/text-core.md` stops authoring, and flat after level
/// four because a level-6 heading that is barely larger than the paragraph under it is
/// exactly what a level-6 heading should look like.
const HEADING_SCALE: [f64; 6] = [1.8, 1.5, 1.3, 1.15, 1.05, 1.0];

/// `Title` and `Subtitle` are the two named paragraph styles LibreOffice's own template offers
/// on a blank document, and the only two this shell gives their own face — everything else in
/// `office:styles` is a name this build keeps and does not interpret (`doc/text-core.md`).
/// Larger than any heading, because a document's title sits above its outline rather than in
/// it, and unlike a heading a `Title`/`Subtitle` block carries no `text:outline-level` for
/// `HEADING_SCALE` to key off — the name on the block is the only signal there is.
const TITLE_SCALE: f64 = 2.4;
const SUBTITLE_SCALE: f64 = 1.3;

/// One block-level face: a font, and a Pango layout kept to measure with.
///
/// The layout is reused rather than created per call — one per face for the life of the
/// widget's style — because `pango::Layout::new` allocates and `advances` is called once per
/// run per repaint.
pub struct Face {
    layout: pango::Layout,
    /// A second layout, for drawing.
    ///
    /// Deliberately not the measuring one: a paint sets text on it once per visible line
    /// while the same face is being asked to measure the next block, and one layout serving
    /// both would have its text replaced underneath the caller.
    drawing: pango::Layout,
    /// Ascent plus descent, in pixels. Measured once: it is a property of the font, and
    /// asking Pango per line would be a font-metrics lookup per line of the document.
    height: f64,
}

impl Face {
    pub fn new(context: &pango::Context, font: pango::FontDescription) -> Self {
        let layout = pango::Layout::new(context);
        layout.set_font_description(Some(&font));
        let drawing = pango::Layout::new(context);
        drawing.set_font_description(Some(&font));
        let metrics = context.metrics(Some(&font), None);
        let scale = f64::from(pango::SCALE);
        let height = f64::from(metrics.ascent() + metrics.descent()) / scale;
        Face {
            layout,
            drawing,
            height: height.max(1.0),
        }
    }

    pub fn height(&self) -> f64 {
        self.height
    }

    /// A layout holding `text`, ready to be drawn — in the same font the caret arithmetic
    /// was answered in, which is the whole reason a face is one object rather than two.
    pub fn draw(&self, text: &str) -> &pango::Layout {
        self.drawing.set_text(text);
        // Clear whatever a previous line's `draw_styled` left on the same shared layout —
        // the bullet drawn after a bold heading must not come out bold with it.
        self.drawing.set_attributes(None::<&pango::AttrList>);
        // And whatever `draw_wrapped` left the width at — every other caller already knows
        // its own line's width from the core's own layout and draws one line at a time.
        self.drawing.set_width(-1);
        &self.drawing
    }

    /// The same layout, with per-character Pango attributes over it — what makes a bold or
    /// italic run actually look like one rather than only measuring like one. `attrs`'
    /// indices are byte offsets into `text`, which is [`run_attributes`]'s job to produce.
    pub fn draw_styled(&self, text: &str, attrs: &pango::AttrList) -> &pango::Layout {
        self.drawing.set_text(text);
        self.drawing.set_attributes(Some(attrs));
        self.drawing.set_width(-1);
        &self.drawing
    }

    /// A layout wrapped to `width` pixels, word by word — the one caller that is not drawing a
    /// line the core's own layout already broke for it: an image's caption, which this shell
    /// lays out itself the same way it draws a list's bullet outside the model.
    pub fn draw_wrapped(&self, text: &str, width: f64) -> &pango::Layout {
        self.drawing.set_text(text);
        self.drawing.set_attributes(None::<&pango::AttrList>);
        self.drawing
            .set_width((width * f64::from(pango::SCALE)) as i32);
        self.drawing.set_wrap(pango::WrapMode::WordChar);
        &self.drawing
    }

    /// The cumulative advance after each character of one Pango line, appended to `out`.
    ///
    /// `attrs` is the fragment's own formatting, and is set on the measuring layout for this
    /// call alone — it must be cleared for the next fragment, or a bold word would go on
    /// measuring bold to the end of the paragraph.
    fn measure(
        &self,
        text: &str,
        base: f64,
        attrs: Option<&pango::AttrList>,
        out: &mut Vec<f32>,
    ) -> f64 {
        self.layout.set_text(text);
        self.layout.set_attributes(attrs);
        let Some(line) = self.layout.line(0) else {
            // No line at all means no text: nothing to append, and the base does not move.
            return base;
        };
        let scale = f64::from(pango::SCALE);
        let mut last = base;
        for (byte, _) in text.char_indices() {
            // `trailing` asks for the far edge of the cluster at that index, which *is* the
            // cumulative advance — the leading edge would be off by one character.
            last = base + f64::from(line.index_to_x(byte as i32, true)) / scale;
            out.push(last as f32);
        }
        last
    }
}

impl Metrics for Face {
    fn advances(&self, text: &str, style: &TextStyle, out: &mut Vec<f32>) {
        let attrs = fragment_attributes(style);
        let mut base = 0.0;
        for (index, segment) in text.split('\n').enumerate() {
            if index > 0 {
                // The `\n` itself. One advance, because the contract is one per character,
                // and no width, because the line ends there.
                out.push(base as f32);
            }
            base = self.measure(segment, base, attrs.as_ref(), out);
        }
    }

    /// The block face's own height, whatever the fragment is set in.
    ///
    /// A monospace run in a paragraph of prose is measured in a monospace face and drawn in
    /// one, and sits on a line as tall as the paragraph's — the module documentation says why
    /// a per-fragment height is the same change as a per-fragment size and waits for it.
    fn line_height(&self, _style: &TextStyle) -> f32 {
        self.height as f32
    }
}

/// One fragment's formatting as Pango attributes over the whole of it, or `None` where it has
/// none — which is most fragments, and the reason this allocates nothing for them.
///
/// The mirror image of [`run_attributes`]: that one builds the attributes a line is *drawn*
/// with from the block's runs, this one the attributes a fragment is *measured* with from the
/// [`TextStyle`] the core projected out of the same run. Three properties, both sides.
fn fragment_attributes(style: &TextStyle) -> Option<pango::AttrList> {
    // Whether `fo:font-weight: 600` reads as bold is a question `CharStyle` already answers,
    // and it is asked rather than restated: two readings of ODF's own vocabulary in one
    // program is one too many, whichever of them is right.
    let props = CharStyle {
        font_weight: style.font_weight.clone(),
        font_style: style.font_style.clone(),
        ..CharStyle::default()
    };
    let family = style.font_family.as_deref();
    if family.is_none() && !props.is_bold() && !props.is_italic() {
        return None;
    }
    // A fresh attribute covers the whole text it is set on, which is exactly one fragment
    // here — so none of these needs a start or an end index.
    let attrs = pango::AttrList::new();
    let add = |attr: pango::Attribute| attrs.insert(attr);
    if let Some(family) = family {
        add(pango::AttrString::new_family(family).into());
    }
    if props.is_bold() {
        add(pango::AttrInt::new_weight(pango::Weight::Bold).into());
    }
    if props.is_italic() {
        add(pango::AttrInt::new_style(pango::Style::Italic).into());
    }
    Some(attrs)
}

/// The faces a document is set in: one for body text, one per heading level.
///
/// Rebuilt whenever the widget's style changes, which is what makes a theme's font size
/// change re-wrap the document instead of clipping it.
pub struct Faces {
    body: Face,
    headings: Vec<Face>,
    title: Face,
    subtitle: Face,
    /// A fenced code block (`grind_text::markdown::PREFORMATTED`). A *face* and not a drawing
    /// trick, because the same object is what measures the block: a monospace paragraph drawn
    /// in one font and measured in another breaks its lines in the wrong places. `ui_web`'s
    /// `Faces` carries the same field for the same reason.
    code: Face,
}

impl Faces {
    pub fn new(context: &pango::Context) -> Self {
        let base = context.font_description().unwrap_or_default();
        let size = match base.size() {
            0 => 11 * pango::SCALE,
            size => size,
        };
        let scaled = |scale: f64, bold: bool, italic: bool| {
            let mut font = base.clone();
            font.set_size((f64::from(size) * scale) as i32);
            if bold {
                font.set_weight(pango::Weight::Bold);
            }
            if italic {
                font.set_style(pango::Style::Italic);
            }
            Face::new(context, font)
        };
        let body = scaled(1.0, false, false);
        let headings = HEADING_SCALE
            .iter()
            .map(|scale| scaled(*scale, true, false))
            .collect();
        let title = scaled(TITLE_SCALE, true, false);
        let subtitle = scaled(SUBTITLE_SCALE, false, true);
        // The *generic* family, spelled the way the document spells it
        // (`grind_text::markdown::MONOSPACE`) — which monospace face a reader has is theirs to
        // know, and Pango resolves the generic through fontconfig exactly as the document
        // intends. At the body's own size, because a code block is a paragraph and not a
        // heading.
        let code = {
            let mut font = base.clone();
            font.set_size(size);
            font.set_family(grind_text::markdown::MONOSPACE);
            Face::new(context, font)
        };
        Faces {
            body,
            headings,
            title,
            subtitle,
            code,
        }
    }

    /// The face a block is set in.
    ///
    /// A named style wins over the block's own kind — `Title` and `Subtitle` are paragraphs
    /// (`BlockKind::Paragraph`) whose only signal is the name in `style`, so that is checked
    /// first. A heading deeper than the six levels this shell has faces for is drawn as the
    /// last of them rather than refused: the reader is *tolerant* (R5), so a level-9 heading
    /// loads, and a shell that panicked on one would undo that.
    pub fn of(&self, kind: &BlockKind, style: Option<&str>) -> &Face {
        match style {
            Some("Title") => return &self.title,
            Some("Subtitle") => return &self.subtitle,
            // A fence (```) is a paragraph *style* and nothing else — `grind_text::markdown`
            // names it, LibreOffice writes it, and this is where a window makes it visible.
            Some(grind_text::markdown::PREFORMATTED) => return &self.code,
            _ => {}
        }
        match kind {
            BlockKind::Heading { level } => {
                let index = (*level).max(1) as usize - 1;
                self.headings
                    .get(index)
                    .unwrap_or_else(|| self.headings.last().unwrap_or(&self.body))
            }
            _ => &self.body,
        }
    }

    pub fn body(&self) -> &Face {
        &self.body
    }
}

/// The family/bold/italic/underline/strikethrough Pango attributes for one line of `text`,
/// from the block's own runs — the toolbar's other half, and the drawing half of the pair
/// [`fragment_attributes`] measures with. `App::layout_block` hands each run's
/// [`grind_text::CharStyle`] to the provider as a [`TextStyle`] (`lay_out`), that function
/// turns three of those properties into the attributes the run is *measured* with, and this
/// one turns five of them into the attributes it is *drawn* with. Underline and strikethrough
/// are in this list and not in that one because they change how text looks and not how wide
/// it is, which is the same split `TextStyle` itself makes.
///
/// `line_start`/`line_end` are character offsets into the whole block, matching
/// [`grind_core::layout::Line::start`]/`end` and [`RunView::start`]/`end`; `text` is that same
/// range already sliced out, which is what a Pango attribute's byte offsets are measured
/// against.
pub fn run_attributes(
    runs: &[RunView],
    line_start: usize,
    line_end: usize,
    text: &str,
) -> pango::AttrList {
    let attrs = pango::AttrList::new();
    // A char-offset-to-byte-offset table for this line alone — built once rather than once
    // per run, since a line can carry several.
    let bytes: Vec<u32> = text
        .char_indices()
        .map(|(byte, _)| byte as u32)
        .chain(std::iter::once(text.len() as u32))
        .collect();
    let byte_at = |chars: usize| bytes.get(chars).copied().unwrap_or(text.len() as u32);

    for run in runs {
        let start = run.start.max(line_start);
        let end = run.end().min(line_end);
        if start >= end {
            continue;
        }
        let (s, e) = (byte_at(start - line_start), byte_at(end - line_start));
        let mark = |mut attr: pango::Attribute| {
            attr.set_start_index(s);
            attr.set_end_index(e);
            attrs.insert(attr);
        };
        // The family, which is what `` `code` `` sets. Safe to draw because it is now also
        // *measured*: `fragment_attributes` puts the same family on the measuring layout from
        // the same run's `TextStyle`, so the caret goes where the ink does. Drawing one
        // without the other is the drift this pair exists to prevent.
        if let Some(family) = run.props.font_family.as_deref() {
            mark(pango::AttrString::new_family(family).into());
        }
        if run.props.is_bold() {
            mark(pango::AttrInt::new_weight(pango::Weight::Bold).into());
        }
        if run.props.is_italic() {
            mark(pango::AttrInt::new_style(pango::Style::Italic).into());
        }
        if run.props.is_underlined() {
            mark(pango::AttrInt::new_underline(pango::Underline::Single).into());
        }
        if run.props.is_struck() {
            mark(pango::AttrInt::new_strikethrough(true).into());
        }
    }
    attrs
}
