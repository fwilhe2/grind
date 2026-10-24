// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Font metrics, through Pango — this shell's whole contribution to layout. Line breaking is
//! `grind_core::layout`'s; a shell supplies [`Metrics`] — how wide is this text, how tall a line
//! of it (`doc/text-layout.md`).
//!
//! * **Cumulative advances come from `pango::LayoutLine::index_to_x`**, not from measuring
//!   prefixes: Pango shapes the string once and answers every cluster's trailing edge from that
//!   pass, and `advance("a") + advance("b")` is not `advance("ab")` once kerning applies.
//! * **A line break inside a fragment measures as nothing.** `Run::Break` reaches here as `\n`,
//!   and handing it to Pango would start a second line whose x restarts; the core breaks there
//!   anyway.
//! * **A fragment is measured in its own formatting**: the family, weight, slant and size
//!   `grind_text::lay_out` projects into its [`TextStyle`] become Pango attributes on the
//!   measuring layout, and [`run_attributes`] puts the same ones on the drawing layout from the
//!   same runs, so the caret goes where the ink does. A size moves width, height
//!   ([`Metrics::line_height`]) and drawing together through [`size_units`]; one it cannot read
//!   (`cm`) is left alone.
//!
//! **The face is chosen per block, by the shell**: a heading's or a `Title`'s look is the block's,
//! decided by kind and style name (`grind_text::look`), so the shell hands the core a provider
//! already set to that block's font.

use libadwaita::gtk;

use grind_core::style::TextStyle;
use grind_text::look::{self, Role};
use grind_text::style::CharStyle;
use grind_text::{BlockKind, Metrics, RunView};
use gtk::pango;

/// One block-level face: a font, and a Pango layout kept to measure with — reused, since
/// `pango::Layout::new` allocates and `advances` runs once per run per repaint.
pub struct Face {
    layout: pango::Layout,
    /// A second layout, for drawing: a paint sets text on it per line while the same face measures
    /// the next block, and one layout serving both would have its text replaced underneath the
    /// caller.
    drawing: pango::Layout,
    /// Ascent plus descent, in pixels. Measured once: it is a property of the font, and
    /// asking Pango per line would be a font-metrics lookup per line of the document.
    height: f64,
    /// The face's own size, in Pango units — what a run's `fo:font-size` is resolved against
    /// when the document spells it as a percentage, and what its height is scaled from.
    size: i32,
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
            size: font.size().max(1),
        }
    }

    pub fn height(&self) -> f64 {
        self.height
    }

    /// This face's own font size, in Pango units.
    pub fn size(&self) -> i32 {
        self.size
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

    /// The cumulative advance after each character of one Pango line, appended to `out`. `attrs`
    /// is set on the measuring layout for this call alone and cleared after, or a bold word would
    /// measure bold to the end of the paragraph.
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
        let attrs = fragment_attributes(style, self.size);
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

    /// The block face's own height, scaled by the fragment's size. A family barely changes a
    /// line's height; a **size** does, and `layout::wrap` takes the tallest fragment's answer, so
    /// a 24pt word makes room for itself. Scaled from the face's measured height, since the
    /// ascent-to-descent proportion of one family does not change.
    fn line_height(&self, style: &TextStyle) -> f32 {
        match size_units(style.font_size.as_deref(), self.size) {
            Some(size) => (self.height * f64::from(size) / f64::from(self.size)) as f32,
            None => self.height as f32,
        }
    }
}

/// A run's `fo:font-size` in Pango units, against the face it sits in: a point length (`14pt`) or
/// a percentage of the block's face (`120%`). **Anything else is `None`** — a `cm` size has no
/// resolution here, and guessing would move the caret from the ink.
pub fn size_units(font_size: Option<&str>, base: i32) -> Option<i32> {
    let value = font_size?.trim();
    if let Some(points) = value
        .strip_suffix("pt")
        .and_then(|v| v.trim().parse::<f64>().ok())
    {
        return (points > 0.0).then(|| (points * f64::from(pango::SCALE)) as i32);
    }
    let percent = value
        .strip_suffix('%')
        .and_then(|v| v.trim().parse::<f64>().ok())?;
    (percent > 0.0).then(|| (f64::from(base) * percent / 100.0) as i32)
}

/// The page a run is drawn on and the theme's ink for it — what a document's colour is read
/// against. Nothing decided here is written; the arithmetic is [`grind_core::color`]'s.
///
/// - **No colour and no highlight**: nothing emitted; the line's own ink, the theme's.
/// - **No colour, on a highlight**: ODF's automatic colour — the theme's ink where it reads on the
///   highlight, else black or white.
/// - **A colour on the page, in a dark theme**: lifted along its hue until it reads
///   ([`grind_core::color::legible`]), so a navy word stays visible.
/// - **Anything else** is the document's decision about its own paper, drawn as it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Paper {
    pub ink: grind_core::color::Rgb,
    pub page: grind_core::color::Rgb,
    /// What a link is drawn in when it has no colour of its own — the theme's accent.
    pub link: grind_core::color::Rgb,
}

impl Paper {
    /// Black on white — what a test that is not about colour draws on.
    pub const LIGHT: Paper = Paper {
        ink: (0, 0, 0),
        page: (255, 255, 255),
        link: (0x1c, 0x71, 0xd8),
    };

    fn dark(self) -> bool {
        grind_core::color::luminance(self.page) < 0.2
    }

    /// The foreground a run is drawn in, or `None` for the line's own ink.
    pub fn ink(self, color: Option<&str>, fill: Option<&str>) -> Option<(u16, u16, u16)> {
        use grind_core::color as core;
        let rgb = |value: Option<&str>| {
            channels(value).map(|(r, g, b)| ((r >> 8) as u8, (g >> 8) as u8, (b >> 8) as u8))
        };
        let wide =
            |(r, g, b): core::Rgb| (u16::from(r) * 257, u16::from(g) * 257, u16::from(b) * 257);
        match (rgb(color), rgb(fill)) {
            (None, None) => None,
            (None, Some(fill)) => Some(wide(core::automatic_ink(fill, self.ink))),
            (Some(own), None) if self.dark() => {
                Some(wide(core::legible(own, self.page, core::TEXT)))
            }
            (Some(own), _) => Some(wide(own)),
        }
    }
}

/// An ODF colour — `#rrggbb`, or a name GDK knows — as Pango's three channels. `transparent`
/// answers `None`, which leaves a highlight of it undrawn rather than black.
fn channels(value: Option<&str>) -> Option<(u16, u16, u16)> {
    let value = value?;
    if value.eq_ignore_ascii_case("transparent") {
        return None;
    }
    let rgba = gtk::gdk::RGBA::parse(value).ok()?;
    let channel = |v: f32| (v.clamp(0.0, 1.0) * f32::from(u16::MAX)) as u16;
    Some((
        channel(rgba.red()),
        channel(rgba.green()),
        channel(rgba.blue()),
    ))
}

/// One fragment's formatting as Pango attributes over the whole of it, or `None` — most fragments,
/// so nothing is allocated for them. [`run_attributes`]'s mirror: the four properties that change
/// how wide text is, which `TextStyle` carries.
fn fragment_attributes(style: &TextStyle, base: i32) -> Option<pango::AttrList> {
    // Whether `fo:font-weight: 600` reads as bold is a question `CharStyle` already answers,
    // and it is asked rather than restated: two readings of ODF's own vocabulary in one
    // program is one too many, whichever of them is right.
    let props = CharStyle {
        font_weight: style.font_weight.clone(),
        font_style: style.font_style.clone(),
        ..CharStyle::default()
    };
    let family = style.font_family.as_deref();
    let size = size_units(style.font_size.as_deref(), base);
    if family.is_none() && size.is_none() && !props.is_bold() && !props.is_italic() {
        return None;
    }
    // A fresh attribute covers the whole text it is set on, which is exactly one fragment
    // here — so none of these needs a start or an end index.
    let attrs = pango::AttrList::new();
    let add = |attr: pango::Attribute| attrs.insert(attr);
    if let Some(family) = family {
        add(pango::AttrString::new_family(family).into());
    }
    if let Some(size) = size {
        add(pango::AttrSize::new(size).into());
    }
    if props.is_bold() {
        add(pango::AttrInt::new_weight(pango::Weight::Bold).into());
    }
    if props.is_italic() {
        add(pango::AttrInt::new_style(pango::Style::Italic).into());
    }
    Some(attrs)
}

/// The faces a document is set in: one for body text, one per heading level — rebuilt on a style
/// change, so a theme's font size re-wraps the document.
pub struct Faces {
    body: Face,
    headings: Vec<Face>,
    title: Face,
    subtitle: Face,
    /// A fenced code block's face (`grind_text::markdown::PREFORMATTED`) — a face, not a drawing
    /// trick, since the same object measures the block.
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
        // Every size and weight is `grind_text::look`'s, so a document has one shape in every
        // window that draws a page.
        let face = |role: Role| scaled(role.scale(), role.bold(), role.italic());
        let body = face(Role::Body);
        let headings = (1..=look::HEADING_SCALE.len() as u8)
            .map(|level| face(Role::Heading(level)))
            .collect();
        let title = face(Role::Title);
        let subtitle = face(Role::Subtitle);
        // The generic family as the document spells it (`grind_text::markdown::MONOSPACE`),
        // resolved by fontconfig to the reader's monospace face, at the body's size.
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

    /// The face a block is set in — [`Role::of`], which decides which name wins over which kind
    /// and what a heading deeper than six levels is set in, for every shell that draws a page.
    pub fn of(&self, kind: &BlockKind, style: Option<&str>) -> &Face {
        match Role::of(kind, style) {
            Role::Body => &self.body,
            Role::Heading(level) => &self.headings[usize::from(level) - 1],
            Role::Title => &self.title,
            Role::Subtitle => &self.subtitle,
            Role::Code => &self.code,
        }
    }

    pub fn body(&self) -> &Face {
        &self.body
    }
}

/// Every drawable property of a block's runs as Pango attributes over one line of `text` — the
/// drawing half of the pair [`fragment_attributes`] measures with. That one carries the four
/// metric properties; this all eight, since underline, strikethrough, colour and highlight change
/// the look but not the width.
///
/// `base` is the block face's size in Pango units, which a percentage `fo:font-size` resolves
/// against, as [`Metrics::line_height`] does. `line_start`/`line_end` are character offsets into
/// the block ([`grind_core::layout::Line`], [`RunView`]); `text` is that range, which Pango's byte
/// offsets count from.
pub fn run_attributes(
    runs: &[RunView],
    line_start: usize,
    line_end: usize,
    text: &str,
    base: i32,
    paper: Paper,
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
        // The family, which `` `code` `` sets — drawn here and measured by `fragment_attributes`
        // from the same run, so the caret goes where the ink does.
        if let Some(family) = run.shown.font_family.as_deref() {
            mark(pango::AttrString::new_family(family).into());
        }
        // The size, on the same terms and for the same reason: `fragment_attributes` put it on
        // the measuring layout and `line_height` made the line tall enough to hold it, so this
        // is the third of three and none of them is optional.
        if let Some(size) = size_units(run.shown.font_size.as_deref(), base) {
            mark(pango::AttrSize::new(size).into());
        }
        // The two colours, neither changing width. The ink is [`Paper::ink`]'s, so a navy word
        // stays readable on a dark page and a yellow highlight does not carry the dark theme's
        // white text.
        let own = paper.ink(run.shown.color.as_deref(), run.shown.background.as_deref());
        // A link looks like one: underlined, and in the accent unless it chose a colour. The
        // underline is the cue that survives colour blindness and documents whose links are all
        // coloured.
        let link = match (&run.href, own) {
            (Some(_), None) => {
                let wide = |c: u8| u16::from(c) * 257;
                let (r, g, b) = paper.link;
                Some((wide(r), wide(g), wide(b)))
            }
            _ => None,
        };
        if let Some((r, g, b)) = own.or(link) {
            mark(pango::AttrColor::new_foreground(r, g, b).into());
        }
        if let Some((r, g, b)) = channels(run.shown.background.as_deref()) {
            mark(pango::AttrColor::new_background(r, g, b).into());
        }
        if run.shown.is_bold() {
            mark(pango::AttrInt::new_weight(pango::Weight::Bold).into());
        }
        if run.shown.is_italic() {
            mark(pango::AttrInt::new_style(pango::Style::Italic).into());
        }
        if run.shown.is_underlined() || run.href.is_some() {
            mark(pango::AttrInt::new_underline(pango::Underline::Single).into());
        }
        if run.shown.is_struck() {
            mark(pango::AttrInt::new_strikethrough(true).into());
        }
    }
    attrs
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The parse that had to exist before a size could be honoured anywhere: it feeds the
    /// measuring attribute, the line's height and the drawing attribute, so all three agree by
    /// construction rather than by three matching edits.
    #[test]
    fn a_size_is_read_in_points_or_per_cent_and_otherwise_left_alone() {
        let base = 11 * pango::SCALE;
        assert_eq!(size_units(Some("14pt"), base), Some(14 * pango::SCALE));
        assert_eq!(size_units(Some(" 14pt "), base), Some(14 * pango::SCALE));
        assert_eq!(size_units(Some("200%"), base), Some(2 * base));
        // Nothing set, and the two shapes this shell cannot resolve: a length in a unit it has
        // no resolution for, and a size of zero.
        assert_eq!(size_units(None, base), None);
        assert_eq!(size_units(Some("5cm"), base), None);
        assert_eq!(size_units(Some("0pt"), base), None);
    }

    /// The dark-mode screenshot, as a test: navy on the dark page is lifted, navy on white is
    /// left alone, and an uncoloured word on a yellow highlight is not drawn in white.
    #[test]
    fn a_documents_colour_reads_on_the_page_it_is_drawn_on() {
        let dark = Paper {
            ink: (255, 255, 255),
            page: (30, 30, 30),
            link: (120, 174, 237),
        };
        let navy = Some("#001f3f");
        let (r, g, b) = dark.ink(navy, None).expect("a colour of its own is drawn");
        let lifted = ((r >> 8) as u8, (g >> 8) as u8, (b >> 8) as u8);
        assert!(grind_core::color::contrast(lifted, dark.page) >= grind_core::color::TEXT);
        assert_eq!(
            Paper::LIGHT.ink(navy, None),
            Some((0, 0x1f * 257, 0x3f * 257))
        );
        let on_yellow = dark
            .ink(None, Some("#ffdc00"))
            .expect("automatic ink on a highlight");
        assert_eq!(on_yellow, (0, 0, 0));
        assert_eq!(dark.ink(None, None), None);
    }

    /// `transparent` is ODF's spelling for "no highlight", and it is a value rather than an
    /// absence — drawing it as a colour would paint every un-highlighted run black.
    #[test]
    fn a_colour_is_parsed_and_transparent_is_not_one() {
        assert_eq!(channels(Some("#ff0000")), Some((u16::MAX, 0, 0)));
        assert_eq!(channels(Some("transparent")), None);
        assert_eq!(channels(None), None);
        assert_eq!(channels(Some("not a colour")), None);
    }
}
