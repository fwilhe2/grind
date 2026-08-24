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
//!
//! **The face is chosen per block, by the shell.** `grind_text` measures every run with the
//! default character style, because a run's style is a *name* and style definitions are not
//! read yet (`doc/text-core.md`) — so a heading laid out in the body font and drawn in a
//! larger one would put every caret in the wrong place. The shell knows the block's kind, so
//! it hands the core a provider already set to that block's font. The core neither knows nor
//! needs to: it does arithmetic in whatever unit it is answered in.

use libadwaita::gtk;

use grind_core::style::TextStyle;
use grind_text::{BlockKind, Metrics};
use gtk::pango;

/// How much bigger than the body text each heading level is.
///
/// Six levels because that is where `doc/text-core.md` stops authoring, and flat after level
/// four because a level-6 heading that is barely larger than the paragraph under it is
/// exactly what a level-6 heading should look like.
const HEADING_SCALE: [f64; 6] = [1.8, 1.5, 1.3, 1.15, 1.05, 1.0];

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
        &self.drawing
    }

    /// The cumulative advance after each character of one Pango line, appended to `out`.
    fn measure(&self, text: &str, base: f64, out: &mut Vec<f32>) -> f64 {
        self.layout.set_text(text);
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
    fn advances(&self, text: &str, _style: &TextStyle, out: &mut Vec<f32>) {
        let mut base = 0.0;
        for (index, segment) in text.split('\n').enumerate() {
            if index > 0 {
                // The `\n` itself. One advance, because the contract is one per character,
                // and no width, because the line ends there.
                out.push(base as f32);
            }
            base = self.measure(segment, base, out);
        }
    }

    fn line_height(&self, _style: &TextStyle) -> f32 {
        self.height as f32
    }
}

/// The faces a document is set in: one for body text, one per heading level.
///
/// Rebuilt whenever the widget's style changes, which is what makes a theme's font size
/// change re-wrap the document instead of clipping it.
pub struct Faces {
    body: Face,
    headings: Vec<Face>,
}

impl Faces {
    pub fn new(context: &pango::Context) -> Self {
        let base = context.font_description().unwrap_or_default();
        let size = match base.size() {
            0 => 11 * pango::SCALE,
            size => size,
        };
        let body = {
            let mut font = base.clone();
            font.set_size(size);
            Face::new(context, font)
        };
        let headings = HEADING_SCALE
            .iter()
            .map(|scale| {
                let mut font = base.clone();
                font.set_size((f64::from(size) * scale) as i32);
                font.set_weight(pango::Weight::Bold);
                Face::new(context, font)
            })
            .collect();
        Faces { body, headings }
    }

    /// The face a block is set in.
    ///
    /// A heading deeper than the six levels this shell has faces for is drawn as the last of
    /// them rather than refused: the reader is *tolerant* (R5), so a level-9 heading loads,
    /// and a shell that panicked on one would undo that.
    pub fn of(&self, kind: &BlockKind) -> &Face {
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
