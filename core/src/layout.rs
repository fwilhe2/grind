// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Breaking styled text into lines. **\[GENERIC\]**
//!
//! `doc/text-layout.md` is normative. Down-arrow, Home, End, click-to-caret and selection extents
//! are all defined in terms of a line, so line layout lives here rather than in each shell: where
//! a line may break (UAX #14), how lines are filled, and every caret operation on a line. Font
//! measurement is the shell's, through [`Metrics`].
//!
//! The input is a flat sequence of [`Fragment`]s, which a paragraph's runs and a wrapped
//! spreadsheet cell both produce. There are no pages here, and layout is left-to-right only
//! (`doc/text-layout.md`, decision 1).

use unicode_linebreak::{BreakOpportunity, linebreaks};

use crate::style::TextStyle;

/// How wide is a piece of text, and how tall is a line of it? Implemented with Pango, the
/// browser's canvas, terminal cells, GDI, CoreText, and [`Fixed`].
///
/// The unit is the caller's own and the core never converts: the `width` handed to [`wrap`] just
/// has to be in the same one.
pub trait Metrics {
    /// The cumulative advance after each character of `text`, appended to `out`: exactly
    /// `text.chars().count()` values, never decreasing, the last being the width of the whole
    /// string.
    ///
    /// Cumulative and in one call because `advance("a") + advance("b")` is not `advance("ab")`
    /// once kerning and ligatures are involved. It also leaves the resulting [`Layout`]
    /// metric-free: hit-testing and caret movement are array lookups.
    fn advances(&self, text: &str, style: &TextStyle, out: &mut Vec<f32>);

    /// The height of one line set in `style`, in the same unit.
    fn line_height(&self, style: &TextStyle) -> f32;

    /// How far below a line's top its baseline sits, for text set in `style`. Only something that
    /// places glyphs itself needs it (the PDF); the default is four fifths of the line.
    fn ascent(&self, style: &TextStyle) -> f32 {
        self.line_height(style) * 0.8
    }
}

/// One character wide per character, one unit tall: what the CLI measures with (`grind text view
/// --width 72`) and what tests use, since it makes line breaking exactly assertable.
///
/// Not right for a terminal, where an ideograph takes two cells and a combining mark none; a
/// terminal shell implements [`Metrics`] itself.
#[derive(Clone, Copy, Debug, Default)]
pub struct Fixed;

impl Metrics for Fixed {
    fn advances(&self, text: &str, _style: &TextStyle, out: &mut Vec<f32>) {
        for (i, _) in text.chars().enumerate() {
            out.push((i + 1) as f32);
        }
    }

    fn line_height(&self, _style: &TextStyle) -> f32 {
        1.0
    }
}

/// A run of text that shares one set of metrics.
#[derive(Clone, Copy, Debug)]
pub struct Fragment<'a> {
    pub text: &'a str,
    pub style: &'a TextStyle,
}

/// One laid-out line: a range of character offsets into the concatenated fragments.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Line {
    /// First character offset on the line.
    pub start: usize,
    /// One past the last, **including any trailing spaces** that the break ate. A caret may sit
    /// at `end`, which is what makes End meaningful on a wrapped line.
    pub end: usize,
    /// Width up to `end`, trailing spaces included.
    pub width: f32,
    pub height: f32,
    /// Distance from the top of the whole layout to the top of this line.
    pub top: f32,
    /// How far right of the measure's left edge alignment starts the line ([`Align`]).
    pub shift: f32,
    /// How much wider justification sets each space between the line's first and last visible
    /// characters; [`Layout::x_at`] already counts it.
    pub stretch: f32,
}

impl Line {
    #[must_use]
    pub fn len(&self) -> usize {
        self.end - self.start
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    /// Whether `offset` is one of the caret positions on this line — `start..=end`.
    #[must_use]
    pub fn holds(&self, offset: usize) -> bool {
        (self.start..=self.end).contains(&offset)
    }
}

/// The result of breaking one paragraph's worth of fragments at a width. It carries the x of every
/// caret position, so nothing below needs [`Metrics`] again.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    lines: Vec<Line>,
    /// Cumulative advance at every caret offset `0..=len`, measured from the start of the whole
    /// text rather than of its line. Line-relative x is a subtraction (see [`Layout::x_at`]).
    xs: Vec<f32>,
    /// Distance from each line's top to its baseline: the largest ascent of any fragment, by
    /// the same rule that makes every line as tall as the tallest one ([`wrap`]).
    baseline: f32,
    /// How far the first line starts in from the others ([`Setting::first`]); negative hangs.
    first: f32,
}

impl Layout {
    /// How far below the top of each line its baseline sits ([`Metrics::ascent`]).
    #[must_use]
    pub fn baseline(&self) -> f32 {
        self.baseline
    }

    #[must_use]
    pub fn lines(&self) -> &[Line] {
        &self.lines
    }

    /// How many characters were laid out.
    #[must_use]
    pub fn len(&self) -> usize {
        self.xs.len() - 1
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Total height — what a shell advances by before drawing the next block.
    #[must_use]
    pub fn height(&self) -> f32 {
        self.lines.last().map_or(0.0, |l| l.top + l.height)
    }

    /// Which line a caret offset is on.
    ///
    /// At a soft break the offset is ambiguous — it is both the end of one line and the start
    /// of the next — and this resolves it to **the later line**, which is where a person
    /// watching a cursor walk off the end of a wrapped line expects it to appear. The one
    /// exception is the very last offset, which has no later line to go to.
    #[must_use]
    pub fn line_at(&self, offset: usize) -> usize {
        let offset = offset.min(self.len());
        self.lines
            .iter()
            .position(|line| offset < line.end)
            .unwrap_or(self.lines.len().saturating_sub(1))
    }

    /// Which line a height `y` below the layout's top is on — the vertical half of hit-testing a
    /// click, [`Layout::offset_at`] being the horizontal one.
    ///
    /// **Nearest, never nothing**: above the first line is the first line and below the last is
    /// the last, since a click in the space around a paragraph means the line nearest it.
    #[must_use]
    pub fn line_at_y(&self, y: f32) -> usize {
        self.lines
            .iter()
            .position(|line| y < line.top + line.height)
            .unwrap_or(self.lines.len().saturating_sub(1))
    }

    /// The x of a caret offset, **relative to the start of its line**.
    #[must_use]
    pub fn x_at(&self, offset: usize) -> f32 {
        let offset = offset.min(self.len());
        let line = &self.lines[self.line_at(offset)];
        self.xs[offset] - self.xs[line.start] + self.indent_of(line.start) + line.shift
    }

    /// The x of a caret offset on `line`, which must hold it. Unlike [`Layout::x_at`] this can
    /// say where a soft-broken line *ends*: the offset it shares with the next line's start.
    #[must_use]
    pub fn x_on(&self, line: usize, offset: usize) -> f32 {
        let Some(line) = self.lines.get(line) else {
            return 0.0;
        };
        let offset = offset.clamp(line.start, line.end);
        self.xs[offset] - self.xs[line.start] + self.indent_of(line.start) + line.shift
    }

    /// The first-line indent for the line starting at `start`, and nothing for any other.
    fn indent_of(&self, start: usize) -> f32 {
        match start {
            0 => self.first,
            _ => 0.0,
        }
    }

    /// The caret offset nearest to `x` on `line` — hit-testing a click. Rounds rather than
    /// truncates: a click on the right half of a letter puts the caret after it.
    #[must_use]
    pub fn offset_at(&self, line: usize, x: f32) -> usize {
        let Some(line) = self.lines.get(line) else {
            return self.len();
        };
        let origin = self.xs[line.start];
        let x = x - self.indent_of(line.start) - line.shift;
        // The first of equally near offsets, which `min_by` keeps.
        (line.start..=line.end)
            .min_by(|&a, &b| {
                let distance = |offset: usize| (self.xs[offset] - origin - x).abs();
                distance(a).total_cmp(&distance(b))
            })
            .unwrap_or(line.start)
    }

    /// Move a caret `delta` lines, keeping as close to `goal_x` as the target line allows. `None`
    /// when the move would leave the layout, the caller's cue to carry on into the neighbouring
    /// block.
    ///
    /// `goal_x` is the caller's to remember, so that walking down through a short line returns to
    /// the starting column; a shell keeps it until the caret moves horizontally.
    #[must_use]
    pub fn step(&self, offset: usize, delta: isize, goal_x: f32) -> Option<usize> {
        let from = self.line_at(offset) as isize;
        let to = from.checked_add(delta)?;
        if to < 0 || to as usize >= self.lines.len() {
            return None;
        }
        Some(self.offset_at(to as usize, goal_x))
    }
}

/// Break `fragments` into lines no wider than `width`, greedily, at UAX #14 break opportunities. A
/// `width` of zero or less means do not wrap: one line per mandatory break.
pub fn wrap(fragments: &[Fragment<'_>], width: f32, metrics: &dyn Metrics) -> Layout {
    wrap_set(fragments, width, metrics, &Setting::default())
}

/// Where a line sits across its measure (`fo:text-align`, `doc/odt-format.md` §5c fact 8), left
/// to right only.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Align {
    #[default]
    Start,
    Center,
    End,
    Justify,
}

impl Align {
    /// ODF's spelling: `start`/`left`, `center`, `end`/`right`, `justify`; anything else, or
    /// nothing, is the start.
    #[must_use]
    pub fn parse(value: Option<&str>) -> Align {
        match value.map(str::trim) {
            Some("center") => Align::Center,
            Some("end" | "right") => Align::End,
            Some("justify") => Align::Justify,
            _ => Align::Start,
        }
    }

    /// How far right a line whose text is `content` wide starts in a measure `available` wide,
    /// and how much wider each of its `spaces` interior spaces is set; `last` is the paragraph's
    /// last line, which justification leaves alone.
    #[must_use]
    pub fn fit(self, available: f32, content: f32, spaces: usize, last: bool) -> (f32, f32) {
        let slack = (available - content).max(0.0);
        match self {
            Align::Start => (0.0, 0.0),
            Align::Center => (slack / 2.0, 0.0),
            Align::End => (slack, 0.0),
            Align::Justify if last || spaces == 0 => (0.0, 0.0),
            Align::Justify => (0.0, slack / spaces as f32),
        }
    }
}

/// How a paragraph is set, beyond its text and its measure. The default is what a plain screen
/// draws: no indent, a tab as wide as the provider says, every line at the start.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Setting {
    /// How far the first line starts in from the others, breaking that much shorter
    /// (`fo:text-indent`, `doc/odt-format.md` §5c fact 11). A negative indent hangs.
    pub first: f32,
    pub tabs: Tabs,
    pub align: Align,
}

/// How a tab character's text lines up against its stop (`style:type`, rng:13921).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabAlign {
    /// The text after the tab starts at the stop.
    Left,
    /// It ends at the stop.
    Right,
    /// It is centred on the stop.
    Center,
    /// Its first occurrence of this character sits at the stop — a column of figures.
    Decimal(char),
}

/// One tab stop: where, in the caller's unit from the paragraph's left edge, and how text meets it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TabStop {
    pub position: f32,
    pub align: TabAlign,
    /// What fills the gap the tab leaves — `style:leader-text`, the dots of a table of
    /// contents. Drawing it is the caller's; it changes nothing about where text goes.
    pub leader: Option<char>,
}

/// Where a paragraph's tabs go (`doc/odt-format.md` §5c, fact 13): its own stops in order, then a
/// stop every `interval` past the last of them — `style:tab-stop-distance`, measured from the
/// paragraph's left edge as the explicit stops are. The default, no stops and no interval, leaves
/// a tab as wide as the provider says it is, which is what every screen draws.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tabs {
    pub stops: Vec<TabStop>,
    pub interval: f32,
}

impl Tabs {
    /// Whether this places a tab at all, rather than leaving it the provider's width.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.stops.is_empty() && self.interval <= 0.0
    }

    /// The first stop past `x`: one of the paragraph's own, else the next whole interval past
    /// both `x` and the last of its own. `None` when there is neither.
    #[must_use]
    pub fn next(&self, x: f32) -> Option<TabStop> {
        const EPS: f32 = 1e-3;
        if let Some(stop) = self.stops.iter().find(|stop| stop.position > x + EPS) {
            return Some(*stop);
        }
        if self.interval <= 0.0 {
            return None;
        }
        let from = self.stops.last().map_or(x, |stop| stop.position.max(x));
        let n = ((from + EPS) / self.interval).floor() + 1.0;
        Some(TabStop {
            position: n * self.interval,
            align: TabAlign::Left,
            leader: None,
        })
    }
}

/// How far past the end of a line a right tab stop is still kept, in points (`doc/odt-format.md`
/// §5c fact 32).
const RIGHT_STOP_REACH: f32 = 16.8;

/// [`wrap`], set as `setting` says. Every tab character is advanced to its stop on its own line —
/// which depends on where the line starts, so tabs are placed *while* breaking rather than
/// measured once beforehand — and each line is then aligned, so every caret x is where the ink is.
pub fn wrap_set(
    fragments: &[Fragment<'_>],
    width: f32,
    metrics: &dyn Metrics,
    setting: &Setting,
) -> Layout {
    let (first, tabs) = (setting.first, &setting.tabs);
    // One `advances` call per fragment, concatenated into a single cumulative array over the
    // whole text. `xs[i]` is the x of caret offset `i`, before any line breaking.
    let mut xs = Vec::with_capacity(64);
    xs.push(0.0);
    let mut text = String::new();
    let mut heights: Vec<f32> = Vec::new();
    let mut ascents: Vec<f32> = Vec::new();
    let mut fragment_advances = Vec::new();
    for fragment in fragments {
        let origin = *xs.last().expect("xs starts with one element");
        fragment_advances.clear();
        metrics.advances(fragment.text, fragment.style, &mut fragment_advances);
        for advance in &fragment_advances {
            xs.push(origin + advance);
        }
        text.push_str(fragment.text);
        heights.push(metrics.line_height(fragment.style));
        ascents.push(metrics.ascent(fragment.style));
    }
    let len = xs.len() - 1;
    // A line is as tall as the tallest fragment in the paragraph. Per-line height would need the
    // breaks first, and mixed sizes within one paragraph are rare.
    let height = heights.iter().copied().fold(0.0_f32, f32::max).max(1.0);
    // An empty paragraph has no fragment to ask, and still has a line to sit on.
    let baseline = match ascents.iter().copied().reduce(f32::max) {
        Some(ascent) => ascent,
        None => metrics.ascent(&TextStyle::default()),
    };

    // UAX #14 hands back *byte* indices; everything else here counts characters, so map once.
    let mut char_of_byte = vec![0usize; text.len() + 1];
    for (index, (byte, _)) in text.char_indices().enumerate() {
        char_of_byte[byte] = index;
    }
    char_of_byte[text.len()] = len;

    let mut lines = Vec::new();
    let mut start = 0usize;
    let mut last_fit: Option<usize> = None;
    let mut top = 0.0;
    let push = |start: usize, end: usize, top: &mut f32, lines: &mut Vec<Line>| {
        lines.push(Line {
            start,
            end,
            width: xs[end] - xs[start],
            height,
            top: *top,
            shift: 0.0,
            stretch: 0.0,
        });
        *top += height;
    };

    let chars: Vec<char> = text.chars().collect();
    let text_chars = &chars;
    let tabbed = !tabs.is_empty() && chars.contains(&'\t');
    let advance: Vec<f32> = if tabbed {
        xs.array_windows().map(|[prev, next]| next - prev).collect()
    } else {
        Vec::new()
    };
    // The cumulative x of each character from `start` to `end`, on a line that begins `origin`
    // in from the paragraph's left edge, with every tab on it at its stop.
    //
    // A tab whose stop is past the end of the line costs nothing there: the text after it
    // breaks onto the next line rather than taking the text before the tab with it.
    //
    // A right stop past the end is kept up to `RIGHT_STOP_REACH` past it, and the text set against
    // it hangs there: the answer is how far, which fitting the line leaves out.
    let place = |start: usize, end: usize, origin: f32, out: &mut Vec<f32>| -> f32 {
        let room = if width > 0.0 {
            width - origin
        } else {
            f32::INFINITY
        };
        out.clear();
        out.push(0.0);
        let mut x = 0.0_f32;
        let mut hang = 0.0_f32;
        for i in start..end {
            let step = match chars[i] {
                '\t' => match tabs.next(origin + x) {
                    None => advance[i],
                    Some(stop)
                        if stop.position - origin > room + 1e-3
                            && stop.align != TabAlign::Right =>
                    {
                        0.0
                    }
                    Some(stop) => {
                        let position = (stop.position - origin).min(room + RIGHT_STOP_REACH);
                        hang = (position - room).max(0.0);
                        // What lines up against the stop: the text after the tab, to the next
                        // tab or the line's end, trailing spaces aside — or to the decimal sign.
                        let mut run = 0.0_f32;
                        let mut visible = 0.0_f32;
                        for j in i + 1..end {
                            match (chars[j], stop.align) {
                                ('\t', _) => break,
                                (c, TabAlign::Decimal(sign)) if c == sign => break,
                                _ => {}
                            }
                            run += advance[j];
                            if !chars[j].is_whitespace() {
                                visible = run;
                            }
                        }
                        let lead = match stop.align {
                            TabAlign::Left => 0.0,
                            TabAlign::Center => visible / 2.0,
                            TabAlign::Right | TabAlign::Decimal(_) => visible,
                        };
                        (position - lead - x).max(0.0)
                    }
                },
                _ => advance[i],
            };
            x += step;
            out.push(x);
        }
        hang
    };
    let scratch = std::cell::RefCell::new(Vec::new());
    let span = |from: usize, to: usize, first_line: bool| {
        if tabbed {
            let mut out = scratch.borrow_mut();
            let hang = place(from, to, if first_line { first } else { 0.0 }, &mut out);
            // Text right-aligned at a stop on the margin ends *at* the margin, which the
            // arithmetic of getting there may overshoot by a rounding error.
            out.last().copied().unwrap_or(0.0) - hang - 1e-3
        } else {
            xs[to] - xs[from]
        }
    };

    // The first line has the indent less room; every other line the whole width. The spaces a
    // break would end the line with hang past the margin, as they do in every word processor:
    // they are part of the line and take no room on it.
    let fits = |from: usize, to: usize, first_line: bool| {
        let room = width - if first_line { first } else { 0.0 };
        let visible = (from..to)
            .rev()
            .find(|&i| !text_chars[i].is_whitespace())
            .map_or(from, |i| i + 1);
        width <= 0.0 || span(from, visible, first_line) <= room
    };

    for (byte, opportunity) in linebreaks(&text) {
        let at = char_of_byte[byte];
        if at <= start {
            continue;
        }
        // Overflowed the line in hand. Close it at the last opportunity that fitted — and if
        // none did, this run is wider than the whole line, so it gets a line of its own further
        // down. A mandatory break is checked here too: the end of a paragraph is still allowed
        // to be the moment a line turns out not to fit.
        if !fits(start, at, lines.is_empty())
            && let Some(end) = last_fit
        {
            push(start, end, &mut top, &mut lines);
            start = end;
        }

        if opportunity == BreakOpportunity::Mandatory {
            if at > start {
                push(start, at, &mut top, &mut lines);
            }
            start = at;
            last_fit = None;
        } else if fits(start, at, lines.is_empty()) {
            last_fit = Some(at);
        } else {
            // Still too wide on a line of its own: an unbreakable run has to go somewhere, and
            // letting it overhang is better than dropping it or looping forever.
            push(start, at, &mut top, &mut lines);
            start = at;
            last_fit = None;
        }
    }
    if start < len || lines.is_empty() {
        push(start, len, &mut top, &mut lines);
    }

    // The tabs are placed line by line now the lines are known: every caret x on a line is then
    // measured past its tabs' stops, which is what a caret, a click and the ink all read.
    let xs = if tabbed {
        let mut placed = Vec::with_capacity(xs.len());
        placed.push(0.0);
        let mut out = Vec::new();
        for (i, line) in lines.iter_mut().enumerate() {
            place(
                line.start,
                line.end,
                if i == 0 { first } else { 0.0 },
                &mut out,
            );
            let base = *placed.last().expect("starts with one element");
            placed.extend(out[1..].iter().map(|x| base + x));
            line.width = out.last().copied().unwrap_or(0.0);
        }
        placed
    } else {
        xs
    };

    let mut layout = Layout {
        lines,
        xs,
        baseline,
        first,
    };
    if width > 0.0 && setting.align != Align::Start {
        layout.align(setting.align, width, &chars);
    }
    layout
}

impl Layout {
    /// Place every line across a measure `width` wide. What is aligned is the line up to its
    /// last visible character — a space a line was broken at hangs past the measure — except on
    /// the paragraph's last line, whose trailing spaces Writer counts (`doc/odt-format.md` §5c
    /// fact 19). The first line's indent is part of what is aligned.
    fn align(&mut self, align: Align, width: f32, chars: &[char]) {
        let count = self.lines.len();
        let natural = self.xs.clone();
        // How much wider everything before the line in hand has been set.
        let mut added = 0.0_f32;
        for at in 0..count {
            let line = self.lines[at];
            let last = at + 1 == count;
            let indent = self.indent_of(line.start);
            let visible_end = (line.start..line.end)
                .rev()
                .find(|&i| !chars[i].is_whitespace())
                .map_or(line.start, |i| i + 1);
            let end = if last {
                (visible_end..line.end)
                    .take_while(|&i| chars[i] == ' ')
                    .last()
                    .map_or(visible_end, |i| i + 1)
            } else {
                visible_end
            };
            let content = (natural[end] - natural[line.start] + indent).min(width);
            let spaces = (line.start..visible_end)
                .filter(|&i| chars[i] == ' ')
                .count();
            let (shift, stretch) = align.fit(width, content, spaces, last);
            let mut interior = 0usize;
            for i in line.start..line.end {
                if i < visible_end && chars[i] == ' ' {
                    interior += 1;
                }
                self.xs[i + 1] += added + stretch * interior as f32;
            }
            let wider = stretch * interior as f32;
            added += wider;
            let line = &mut self.lines[at];
            line.shift = shift;
            line.stretch = stretch;
            line.width += wider;
        }
        if let Some(rest) = self
            .xs
            .get_mut(self.lines.last().map_or(0, |l| l.end) + 1..)
        {
            for x in rest {
                *x += added;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(text: &str) -> Layout {
        let style = TextStyle::default();
        let fragments = [Fragment {
            text,
            style: &style,
        }];
        wrap(&fragments, 0.0, &Fixed)
    }

    fn indent(first: f32) -> Setting {
        Setting {
            first,
            ..Setting::default()
        }
    }

    fn aligned(text: &str, width: f32, align: Align, first: f32) -> Layout {
        let style = TextStyle::default();
        let fragments = [Fragment {
            text,
            style: &style,
        }];
        let setting = Setting {
            first,
            tabs: Tabs::default(),
            align,
        };
        wrap_set(&fragments, width, &Fixed, &setting)
    }

    #[test]
    fn odfs_spellings_of_an_alignment_are_read_and_anything_else_is_the_start() {
        assert_eq!(Align::parse(Some("center")), Align::Center);
        assert_eq!(Align::parse(Some("end")), Align::End);
        assert_eq!(Align::parse(Some("right")), Align::End);
        assert_eq!(Align::parse(Some("justify")), Align::Justify);
        assert_eq!(Align::parse(Some("left")), Align::Start);
        assert_eq!(Align::parse(Some("start")), Align::Start);
        assert_eq!(Align::parse(Some("sideways")), Align::Start);
        assert_eq!(Align::parse(None), Align::Start);
    }

    #[test]
    fn centre_and_end_shift_a_line_and_widen_nothing() {
        assert_eq!(Align::Center.fit(100.0, 40.0, 3, false), (30.0, 0.0));
        assert_eq!(Align::End.fit(100.0, 40.0, 3, true), (60.0, 0.0));
        assert_eq!(Align::Start.fit(100.0, 40.0, 3, false), (0.0, 0.0));
        // A line wider than its measure starts at the margin.
        assert_eq!(Align::Center.fit(100.0, 140.0, 2, false), (0.0, 0.0));
    }

    #[test]
    fn justification_shares_the_slack_between_the_spaces_except_on_the_last_line() {
        assert_eq!(Align::Justify.fit(100.0, 70.0, 3, false), (0.0, 10.0));
        assert_eq!(Align::Justify.fit(100.0, 70.0, 3, true), (0.0, 0.0));
        assert_eq!(Align::Justify.fit(100.0, 70.0, 0, false), (0.0, 0.0));
        assert_eq!(Align::Justify.fit(100.0, 140.0, 2, false), (0.0, 0.0));
    }

    #[test]
    fn a_centred_line_puts_its_carets_and_its_clicks_where_the_ink_is() {
        // "aaaa " / "bb": four visible on a measure of six, then two.
        let layout = aligned("aaaa bb", 6.0, Align::Center, 0.0);
        let lines: Vec<(usize, usize)> = layout.lines().iter().map(|l| (l.start, l.end)).collect();
        assert_eq!(lines, vec![(0, 5), (5, 7)]);
        assert_eq!(layout.lines()[0].shift, 1.0);
        assert_eq!(layout.lines()[1].shift, 2.0);
        assert_eq!(layout.x_at(0), 1.0);
        assert_eq!(layout.x_at(2), 3.0);
        assert_eq!(layout.x_at(5), 2.0, "the second line's first caret");
        assert_eq!(layout.x_at(7), 4.0);
        assert_eq!(layout.offset_at(0, 3.2), 2);
        assert_eq!(layout.offset_at(1, 0.0), 5, "left of the line is its start");
        assert_eq!(layout.step(1, 1, layout.x_at(1)), Some(5));
    }

    #[test]
    fn a_soft_broken_lines_end_has_an_x_of_its_own() {
        let layout = at("aaaa bb", 6.0);
        assert_eq!(
            layout.x_at(5),
            0.0,
            "the shared offset reads as the next line's start"
        );
        assert_eq!(layout.x_on(0, 5), 5.0);
        assert_eq!(layout.x_on(1, 5), 0.0);
        assert_eq!(layout.x_on(1, 7), 2.0);
    }

    #[test]
    fn an_end_aligned_last_line_counts_its_trailing_spaces() {
        // `doc/odt-format.md` §5c fact 19.
        assert_eq!(aligned("X", 10.0, Align::End, 0.0).x_at(0), 9.0);
        assert_eq!(aligned("X ", 10.0, Align::End, 0.0).x_at(0), 8.0);
    }

    #[test]
    fn a_justified_line_widens_its_interior_spaces_and_leaves_the_last_line() {
        // "aa bb " is five visible on a measure of nine: the one space takes the four spare.
        let layout = aligned("aa bb cccc", 9.0, Align::Justify, 0.0);
        let lines: Vec<(usize, usize)> = layout.lines().iter().map(|l| (l.start, l.end)).collect();
        assert_eq!(lines, vec![(0, 6), (6, 10)]);
        assert_eq!(layout.lines()[0].stretch, 4.0);
        assert_eq!(layout.x_at(2), 2.0, "before the space");
        assert_eq!(layout.x_at(3), 7.0, "after it");
        assert_eq!(
            layout.x_at(5),
            9.0,
            "the last visible character ends at the measure"
        );
        assert_eq!(layout.offset_at(0, 6.9), 3);
        assert_eq!(layout.offset_at(0, 3.0), 2);
        // The last line is set as it would be unjustified.
        assert_eq!(layout.lines()[1].stretch, 0.0);
        assert_eq!(layout.x_at(6), 0.0);
        assert_eq!(layout.x_at(10), 4.0);
    }

    #[test]
    fn every_justified_line_reaches_the_measure_not_only_the_first() {
        let layout = aligned("aa bb cc dd ee", 7.0, Align::Justify, 0.0);
        let lines: Vec<(usize, usize)> = layout.lines().iter().map(|l| (l.start, l.end)).collect();
        assert_eq!(lines, vec![(0, 6), (6, 12), (12, 14)]);
        assert_eq!(layout.lines()[0].stretch, 2.0);
        assert_eq!(layout.lines()[1].stretch, 2.0);
        assert_eq!(layout.x_at(5), 7.0);
        assert_eq!(layout.x_at(11), 7.0);
        assert_eq!(layout.x_at(6), 0.0);
        assert_eq!(layout.x_at(14), 2.0);
    }

    #[test]
    fn a_first_line_indent_is_part_of_what_is_aligned() {
        let layout = aligned("aaaa", 10.0, Align::End, 2.0);
        assert_eq!(layout.x_at(4), 10.0, "the text ends at the measure");
        assert_eq!(layout.x_at(0), 6.0);
    }

    #[test]
    fn no_alignment_and_no_measure_leave_a_layout_as_it_was() {
        assert_eq!(
            aligned("aaaa bb", 10.0, Align::Start, 0.0),
            at("aaaa bb", 10.0)
        );
        assert_eq!(
            aligned("aaaa bb", 0.0, Align::Center, 0.0),
            plain("aaaa bb")
        );
    }

    fn at(text: &str, width: f32) -> Layout {
        let style = TextStyle::default();
        let fragments = [Fragment {
            text,
            style: &style,
        }];
        wrap(&fragments, width, &Fixed)
    }

    /// The lines as strings, which is what every assertion below is really about.
    fn rendered(text: &str, layout: &Layout) -> Vec<String> {
        let chars: Vec<char> = text.chars().collect();
        layout
            .lines()
            .iter()
            .map(|line| chars[line.start..line.end].iter().collect())
            .collect()
    }

    /// A provider whose text is ten units tall with an ascent of seven, unless the style asks
    /// for a size, in which case both scale with it — enough to tell max-of-ascents from
    /// anything else.
    struct Tall;

    impl Metrics for Tall {
        fn advances(&self, text: &str, style: &TextStyle, out: &mut Vec<f32>) {
            Fixed.advances(text, style, out);
        }
        fn line_height(&self, style: &TextStyle) -> f32 {
            10.0 * scale(style)
        }
        fn ascent(&self, style: &TextStyle) -> f32 {
            7.0 * scale(style)
        }
    }

    fn scale(style: &TextStyle) -> f32 {
        if style.font_size.is_some() { 2.0 } else { 1.0 }
    }

    #[test]
    fn a_line_sits_on_the_largest_ascent_in_it() {
        let small = TextStyle::default();
        let big = TextStyle {
            font_size: Some("24pt".into()),
            ..TextStyle::default()
        };
        let one = wrap(
            &[Fragment {
                text: "ab",
                style: &small,
            }],
            0.0,
            &Tall,
        );
        assert_eq!(one.baseline(), 7.0);
        let mixed = wrap(
            &[
                Fragment {
                    text: "ab",
                    style: &small,
                },
                Fragment {
                    text: "CD",
                    style: &big,
                },
            ],
            0.0,
            &Tall,
        );
        assert_eq!(mixed.baseline(), 14.0);
        assert_eq!(mixed.lines()[0].height, 20.0);
    }

    #[test]
    fn a_provider_that_knows_no_ascent_puts_the_baseline_four_fifths_down() {
        assert_eq!(plain("x").baseline(), 0.8);
        assert_eq!(
            plain("").baseline(),
            0.8,
            "an empty block still has a line to sit on"
        );
    }

    /// `doc/odt-format.md` §5c fact 11: a first-line indent starts the first line further in and
    /// breaks it shorter by as much; every other line is as it was. A negative one hangs.
    #[test]
    fn a_first_line_indent_moves_and_shortens_the_first_line_alone() {
        let style = TextStyle::default();
        let text = "aaaa bbbb cccc dd";
        let fragments = [Fragment {
            text,
            style: &style,
        }];
        let indented = wrap_set(&fragments, 10.0, &Fixed, &indent(3.0));
        let lines: Vec<(usize, usize)> =
            indented.lines().iter().map(|l| (l.start, l.end)).collect();
        // "aaaa " fits the seven left on the first line; "aaaa bbbb" would not.
        assert_eq!(lines, vec![(0, 5), (5, 15), (15, 17)]);
        assert_eq!(indented.x_at(0), 3.0);
        assert_eq!(indented.x_at(2), 5.0);
        assert_eq!(
            indented.x_at(6),
            1.0,
            "the second line starts at the margin"
        );
        assert_eq!(
            indented.offset_at(0, 3.5),
            0,
            "a click at the indent is the first character"
        );
        assert_eq!(indented.offset_at(0, 5.0), 2);
        let hanging = wrap_set(&fragments, 10.0, &Fixed, &indent(-2.0));
        assert_eq!(hanging.lines()[0].end, 10, "twelve wide: \"aaaa bbbb \"");
        assert_eq!(
            wrap_set(&fragments, 10.0, &Fixed, &indent(0.0)),
            wrap(&fragments, 10.0, &Fixed)
        );
    }

    #[test]
    fn a_width_of_zero_means_one_line() {
        let layout = plain("the cat sat on the mat");
        assert_eq!(layout.lines().len(), 1);
        assert_eq!(layout.len(), 22);
        assert_eq!(
            layout.lines()[0],
            Line {
                start: 0,
                end: 22,
                width: 22.0,
                height: 1.0,
                top: 0.0,
                shift: 0.0,
                stretch: 0.0,
            }
        );
    }

    /// A click's height finds its line, and a click above or below the paragraph finds the
    /// nearest one rather than nothing.
    #[test]
    fn a_height_finds_the_nearest_line() {
        let layout = at("the cat sat on the mat", 10.0);
        assert_eq!(layout.lines().len(), 3);
        assert_eq!(layout.line_at_y(0.0), 0);
        assert_eq!(layout.line_at_y(1.5), 1, "one unit per line under Fixed");
        assert_eq!(layout.line_at_y(-50.0), 0, "above the block");
        assert_eq!(layout.line_at_y(500.0), 2, "below it");
        assert_eq!(
            plain("").line_at_y(3.0),
            0,
            "an empty block has its one line"
        );
    }

    #[test]
    fn text_breaks_at_the_last_opportunity_that_fits() {
        let text = "the cat sat on the mat";
        let layout = at(text, 10.0);
        // "the cat " is 8 and fits; "the cat sat" is 11 and does not. "sat on the" is 10 and
        // fits, the space after it hanging past the margin as it does in every word processor.
        assert_eq!(
            rendered(text, &layout),
            vec!["the cat ", "sat on the ", "mat"]
        );
        // Trailing spaces stay on the line they ended, so End puts the caret after them.
        assert_eq!(layout.lines()[0].end, 8);
    }

    #[test]
    fn a_word_wider_than_the_line_breaks_rather_than_overflowing_silently() {
        let text = "a supercalifragilistic b";
        let layout = at(text, 6.0);
        let lines = rendered(text, &layout);
        assert_eq!(lines[0], "a ");
        assert_eq!(
            lines[1], "supercalifragilistic ",
            "an unbreakable run has to go somewhere, and it goes on a line of its own"
        );
        assert_eq!(lines[2], "b");
    }

    /// UAX #14, not "split on spaces" — the difference is the point of taking the dependency.
    #[test]
    fn breaking_follows_unicode_rather_than_ascii_spaces() {
        // A hyphen is a break opportunity and there is no space anywhere in this string.
        let text = "well-known-example";
        let layout = at(text, 12.0);
        assert_eq!(rendered(text, &layout), vec!["well-known-", "example"]);

        // A no-break space is not one, so this cannot be split at all.
        let text = "aaa\u{a0}bbb";
        assert_eq!(at(text, 4.0).lines().len(), 1);
    }

    #[test]
    fn a_mandatory_break_ends_a_line_however_short_it_is() {
        let text = "a\nbb\nccc";
        let layout = at(text, 100.0);
        assert_eq!(layout.lines().len(), 3);
        // The newline stays on the line it ended, exactly as a trailing space does.
        assert_eq!(rendered(text, &layout), vec!["a\n", "bb\n", "ccc"]);
    }

    #[test]
    fn an_empty_text_is_one_empty_line_rather_than_none() {
        // A shell still has to put a caret somewhere and draw a cursor of some height.
        let layout = plain("");
        assert_eq!(layout.lines().len(), 1);
        assert!(layout.is_empty());
        assert_eq!(layout.height(), 1.0);
        assert_eq!(layout.line_at(0), 0);
        assert_eq!(layout.x_at(0), 0.0);
    }

    #[test]
    fn every_caret_offset_has_an_x_relative_to_its_own_line() {
        let text = "the cat sat on the mat";
        let layout = at(text, 10.0);
        // Offset 0 and the start of the second line are both at x 0.
        assert_eq!(layout.x_at(0), 0.0);
        assert_eq!(layout.x_at(8), 0.0, "start of line 2, not 8 units in");
        assert_eq!(layout.x_at(11), 3.0, "\"sat\" is three characters along");
    }

    #[test]
    fn a_caret_at_a_soft_break_belongs_to_the_later_line() {
        let text = "the cat sat on the mat";
        let layout = at(text, 10.0);
        assert_eq!(layout.line_at(7), 0);
        assert_eq!(
            layout.line_at(8),
            1,
            "walking off the end lands on the next line"
        );
        // Except at the very end, which has nowhere later to go.
        assert_eq!(layout.line_at(layout.len()), layout.lines().len() - 1);
    }

    #[test]
    fn hit_testing_rounds_to_the_nearer_caret() {
        let text = "the cat sat on the mat";
        let layout = at(text, 10.0);
        assert_eq!(layout.offset_at(0, 0.0), 0);
        assert_eq!(layout.offset_at(0, 2.4), 2, "left half of a character");
        assert_eq!(layout.offset_at(0, 2.6), 3, "right half puts it after");
        assert_eq!(
            layout.offset_at(0, 999.0),
            8,
            "past the end clamps to the line"
        );
    }

    #[test]
    fn stepping_by_lines_keeps_the_goal_column_and_reports_leaving() {
        let text = "the cat sat on the mat";
        let layout = at(text, 10.0);
        let start = 3; // "the|"
        let goal = layout.x_at(start);
        let down = layout.step(start, 1, goal).expect("there is a line below");
        assert_eq!(layout.line_at(down), 1);
        assert_eq!(layout.x_at(down), goal, "same column");

        // Up from the top and down from the bottom leave the layout, which is the caller's cue
        // to carry into the neighbouring block.
        assert!(layout.step(start, -1, goal).is_none());
        assert!(layout.step(layout.len(), 1, goal).is_none());
    }

    #[test]
    fn fragments_are_measured_separately_and_laid_out_as_one_text() {
        // Two character styles in one paragraph: the offsets run across both, which is what
        // lets a caret address the paragraph rather than a run.
        let a = TextStyle::default();
        let b = TextStyle {
            font_weight: Some("bold".to_owned()),
            ..TextStyle::default()
        };
        let fragments = [
            Fragment {
                text: "the cat ",
                style: &a,
            },
            Fragment {
                text: "sat down",
                style: &b,
            },
        ];
        let layout = wrap(&fragments, 10.0, &Fixed);
        assert_eq!(layout.len(), 16);
        assert_eq!(layout.lines().len(), 2);
        assert_eq!(layout.lines()[0].end, 8);
        assert_eq!(
            layout.x_at(11),
            3.0,
            "three characters into the second line"
        );
    }

    /// Bidi is excluded (`doc/text-layout.md`, decision 1); this asserts only that RTL text does
    /// not crash or lose characters.
    #[test]
    fn right_to_left_text_lays_out_left_to_right_without_losing_anything() {
        let text = "\u{5e9}\u{5dc}\u{5d5}\u{5dd} world";
        let layout = at(text, 5.0);
        let total: usize = layout.lines().iter().map(Line::len).sum();
        assert_eq!(
            total,
            text.chars().count(),
            "every character is on some line"
        );
    }

    fn tabbed(text: &str, width: f32, first: f32, tabs: &Tabs) -> Layout {
        let style = TextStyle::default();
        let fragments = [Fragment {
            text,
            style: &style,
        }];
        let setting = Setting {
            first,
            tabs: tabs.clone(),
            align: Align::Start,
        };
        wrap_set(&fragments, width, &Fixed, &setting)
    }

    fn stop(position: f32, align: TabAlign) -> TabStop {
        TabStop {
            position,
            align,
            leader: None,
        }
    }

    /// `doc/odt-format.md` §5c fact 13: with no stops of its own a paragraph's tabs go to the
    /// next whole interval — `A` and `Longer` reach the same column.
    #[test]
    fn a_tab_goes_to_the_next_interval() {
        let tabs = Tabs {
            stops: vec![],
            interval: 4.0,
        };
        let short = tabbed("A\tB", 0.0, 0.0, &tabs);
        assert_eq!(short.x_at(2), 4.0, "B at the first stop");
        let long = tabbed("Longer\tB", 0.0, 0.0, &tabs);
        assert_eq!(long.x_at(7), 8.0, "past 6, the next stop is 8");
        let exact = tabbed("Four\tB", 0.0, 0.0, &tabs);
        assert_eq!(exact.x_at(5), 8.0, "a stop already reached is no stop");
    }

    #[test]
    fn a_paragraphs_own_stops_come_first_and_the_interval_after_them() {
        let tabs = Tabs {
            stops: vec![stop(10.0, TabAlign::Left)],
            interval: 4.0,
        };
        let layout = tabbed("a\tb\tc", 0.0, 0.0, &tabs);
        assert_eq!(layout.x_at(2), 10.0, "its own stop, not the interval's 4");
        assert_eq!(layout.x_at(4), 12.0, "then the interval past the last stop");
    }

    /// `doc/odt-format.md` §5c fact 32: a right stop past the line's end is kept, up to 16.8 past
    /// it, and what stands against it hangs there without breaking the line.
    #[test]
    fn a_right_stop_past_the_end_of_the_line_is_kept_a_little_way_out() {
        let at = |position| Tabs {
            stops: vec![stop(position, TabAlign::Right)],
            interval: 0.0,
        };
        let near = tabbed("Guidance\t2", 40.0, 0.0, &at(41.0));
        assert_eq!(near.lines().len(), 1);
        assert_eq!(near.x_at(9), 40.0, "ending at the stop");
        let far = tabbed("Guidance\t2", 40.0, 0.0, &at(100.0));
        assert_eq!(far.lines().len(), 1);
        assert!((far.x_at(9) - 55.8).abs() < 1e-3, "{}", far.x_at(9));
    }

    #[test]
    fn right_centre_and_decimal_stops_line_their_text_up_against_the_stop() {
        let at = |align| Tabs {
            stops: vec![stop(20.0, align)],
            interval: 0.0,
        };
        let right = tabbed("x\tPage 3", 0.0, 0.0, &at(TabAlign::Right));
        assert_eq!(right.x_at(2), 14.0, "six characters ending at 20");
        assert_eq!(right.lines()[0].width, 20.0);
        let centre = tabbed("x\tmid", 0.0, 0.0, &at(TabAlign::Center));
        assert_eq!(centre.x_at(2), 18.5);
        let decimal = tabbed("x\t123.45", 0.0, 0.0, &at(TabAlign::Decimal('.')));
        assert_eq!(decimal.x_at(5), 20.0, "the point at the stop");
    }

    /// A stop is measured from the paragraph's left edge, so on a first line set in by its indent
    /// the tab is that much shorter — and on the next line it starts from the edge again.
    #[test]
    fn stops_are_from_the_paragraphs_edge_on_every_line() {
        let tabs = Tabs {
            stops: vec![],
            interval: 4.0,
        };
        let layout = tabbed("a\tb c\td", 6.0, 2.0, &tabs);
        let lines: Vec<_> = layout.lines().iter().map(|l| (l.start, l.end)).collect();
        assert_eq!(lines, [(0, 4), (4, 7)]);
        assert_eq!(
            layout.x_at(2),
            4.0,
            "the first line: 2 in, `a`, the tab to 4"
        );
        assert_eq!(layout.x_at(6), 4.0, "the second: `c` at 0, the tab to 4");
    }

    #[test]
    fn with_no_stops_a_tab_is_as_wide_as_the_provider_says() {
        let plain = tabbed("a\tb", 0.0, 0.0, &Tabs::default());
        assert_eq!(plain.x_at(2), 2.0);
        assert_eq!(
            plain,
            wrap(
                &[Fragment {
                    text: "a\tb",
                    style: &TextStyle::default()
                }],
                0.0,
                &Fixed
            )
        );
    }
}
