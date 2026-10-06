// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Breaking styled text into lines. **\[GENERIC\]**
//!
//! `doc/text-layout.md` is normative here, and the argument that put this file in `grind-core`
//! rather than in a shell is worth repeating, because it is the project's own architecture rule
//! meeting the one thing the core cannot know:
//!
//! > Down-arrow, Home, End, Page Down, click-to-caret and selection extents are every one of
//! > them defined in terms of **a line**. A line is not a thing in the ODF document — it is an
//! > output of layout. So if layout lives in the shell, so does that half of the editing model,
//! > three times over, in three shells that will disagree about where the cursor goes. That is
//! > not a rendering difference. It is the program behaving differently depending on which
//! > window you opened it in.
//!
//! So the core owns everything about layout that is **not a font question**: where a line may
//! break (UAX #14), how lines are filled, and every caret operation that mentions one. It owns
//! none of the font question, and asks the shell through [`Metrics`].
//!
//! **Two applications, one engine.** The input is a flat sequence of `(text, TextStyle)`
//! [`Fragment`]s, which mentions no document type's vocabulary — a paragraph's runs produce
//! one, and so does a wrapped spreadsheet cell. That is what keeps this R8-clean and what makes
//! the abstraction real rather than invented for a single caller.
//!
//! **What this is not.** There is no page here: no page box, no widows or orphans, no headers,
//! no footnote placement. Pagination is gated in `doc/not-doing.md` §2 behind loop D, and
//! nothing in this module moves that line. And layout is **left-to-right only** — bidi is an
//! explicit exclusion with its own gate, not an oversight (`doc/text-layout.md`, decision 1).

use unicode_linebreak::{BreakOpportunity, linebreaks};

use crate::style::TextStyle;

/// How wide is a piece of text, and how tall is a line of it?
///
/// The two things the core cannot know, and therefore the entire surface between a layout and
/// the shell drawing it. Implementations: Pango in GTK, the browser in the web shell,
/// character cells in the terminal, [`Fixed`] in the CLI and in every test.
///
/// **The unit is the caller's own, and the core never converts.** Cells, Pango units, CSS
/// pixels — it does not matter, as long as the `width` handed to [`wrap`] is in the same one.
/// A core that invented a unit would need to know a DPI, which is a display's business.
pub trait Metrics {
    /// The cumulative advance after each character of `text`, appended to `out`.
    ///
    /// Exactly `text.chars().count()` values, each the width of `text` up to and including that
    /// character, so the last is the width of the whole string. Never negative, never
    /// decreasing.
    ///
    /// **Cumulative, and in one call, on purpose.** `advance("a") + advance("b")` is not
    /// `advance("ab")` once kerning and ligatures are involved, so a per-character trait would
    /// be quietly wrong and a prefix-measuring one quietly slow. Handing the provider the whole
    /// string lets it shape once and answer everything, and it leaves the resulting [`Layout`]
    /// **metric-free**: every caret x is already in it, so hit-testing and caret movement are
    /// array lookups with no font in sight.
    ///
    /// What that still cannot see is kerning *across* two fragments — which is a boundary
    /// between two different character styles, where kerning is arguably wrong anyway.
    fn advances(&self, text: &str, style: &TextStyle, out: &mut Vec<f32>);

    /// The height of one line set in `style`, in the same unit.
    fn line_height(&self, style: &TextStyle) -> f32;

    /// How far below a line's top its baseline sits, for text set in `style`.
    ///
    /// Only something that places glyphs itself needs this — the PDF (`doc/pdf-export.md`),
    /// whose text is positioned on a baseline. A toolkit that draws a run from the top of its
    /// box works its baseline out on its own, so the shells keep the default: four fifths of
    /// the line, which is close for most Latin faces and never matters to them.
    fn ascent(&self, style: &TextStyle) -> f32 {
        self.line_height(style) * 0.8
    }
}

/// One character wide per character, one unit tall.
///
/// What the CLI measures with, so that `grind text view --width 72` is a real answer and every
/// line operation is reachable without a display (`doc/plan.md` rule 4). Also what every test
/// in the workspace uses, because a synthetic provider makes line breaking *exactly* assertable
/// — a test can say "this breaks after 12 characters" and mean it.
///
/// **Not good enough for a terminal**, and deliberately not trying to be: a CJK ideograph
/// occupies two cells and a combining mark none, which is `unicode-width`'s job. A terminal
/// shell implements [`Metrics`] itself and the core stays free of that table.
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
///
/// The unit of input, and the seam that keeps this module generic: a text document builds these
/// from a paragraph's runs, a spreadsheet from a cell's display text, and neither vocabulary
/// appears here.
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
}

impl Line {
    pub fn len(&self) -> usize {
        self.end - self.start
    }

    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    /// Whether `offset` is one of the caret positions on this line — `start..=end`.
    pub fn holds(&self, offset: usize) -> bool {
        (self.start..=self.end).contains(&offset)
    }
}

/// The result of breaking one paragraph's worth of fragments at a width.
///
/// A **plain value**: it carries the x of every caret position, so nothing below needs
/// [`Metrics`] again. A shell can hold one, query it while painting and throw it away, which is
/// the same contract `App::get_viewport` offers for content.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    lines: Vec<Line>,
    /// Cumulative advance at every caret offset `0..=len`, measured from the start of the whole
    /// text rather than of its line. Line-relative x is a subtraction (see [`Layout::x_at`]).
    xs: Vec<f32>,
    /// Distance from each line's top to its baseline: the largest ascent of any fragment, by
    /// the same rule that makes every line as tall as the tallest one ([`wrap`]).
    baseline: f32,
    /// How far the first line starts in from the others ([`wrap_indented`]); negative hangs.
    first: f32,
}

impl Layout {
    /// How far below the top of each line its baseline sits ([`Metrics::ascent`]).
    pub fn baseline(&self) -> f32 {
        self.baseline
    }

    pub fn lines(&self) -> &[Line] {
        &self.lines
    }

    /// How many characters were laid out.
    pub fn len(&self) -> usize {
        self.xs.len() - 1
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Total height — what a shell advances by before drawing the next block.
    pub fn height(&self) -> f32 {
        self.lines.last().map_or(0.0, |l| l.top + l.height)
    }

    /// Which line a caret offset is on.
    ///
    /// At a soft break the offset is ambiguous — it is both the end of one line and the start
    /// of the next — and this resolves it to **the later line**, which is where a person
    /// watching a cursor walk off the end of a wrapped line expects it to appear. The one
    /// exception is the very last offset, which has no later line to go to.
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
    pub fn line_at_y(&self, y: f32) -> usize {
        self.lines
            .iter()
            .position(|line| y < line.top + line.height)
            .unwrap_or(self.lines.len().saturating_sub(1))
    }

    /// The x of a caret offset, **relative to the start of its line**.
    pub fn x_at(&self, offset: usize) -> f32 {
        let offset = offset.min(self.len());
        let line = &self.lines[self.line_at(offset)];
        self.xs[offset] - self.xs[line.start] + self.indent_of(line.start)
    }

    /// The first-line indent for the line starting at `start`, and nothing for any other.
    fn indent_of(&self, start: usize) -> f32 {
        match start {
            0 => self.first,
            _ => 0.0,
        }
    }

    /// The caret offset nearest to `x` on `line` — hit-testing a click.
    ///
    /// Nearest rather than "the character containing x", because a caret goes *between*
    /// characters: clicking the right half of a letter puts the cursor after it, which is the
    /// behaviour every editor has and the reason this rounds rather than truncates.
    pub fn offset_at(&self, line: usize, x: f32) -> usize {
        let Some(line) = self.lines.get(line) else {
            return self.len();
        };
        let origin = self.xs[line.start];
        let x = x - self.indent_of(line.start);
        let mut best = line.start;
        let mut best_d = f32::INFINITY;
        for offset in line.start..=line.end {
            let d = (self.xs[offset] - origin - x).abs();
            if d < best_d {
                best_d = d;
                best = offset;
            }
        }
        best
    }

    /// Move a caret `delta` lines, keeping as close to `goal_x` as the target line allows.
    ///
    /// `None` when the move would leave the layout — the caller's cue to carry on into the
    /// previous or next block, which is what makes Down-arrow work across a paragraph boundary.
    ///
    /// `goal_x` is the caller's to remember. Walking down through a short line and out the
    /// other side should return to the column you started in, and that is a property of a *run*
    /// of keystrokes rather than of the document — so it is passed in rather than stored, and
    /// a shell keeps it until the caret moves horizontally.
    pub fn step(&self, offset: usize, delta: isize, goal_x: f32) -> Option<usize> {
        let from = self.line_at(offset) as isize;
        let to = from.checked_add(delta)?;
        if to < 0 || to as usize >= self.lines.len() {
            return None;
        }
        Some(self.offset_at(to as usize, goal_x))
    }
}

/// Break `fragments` into lines no wider than `width`.
///
/// Greedy: fill a line until the next break opportunity would overflow it, then start another.
/// Not Knuth–Plass — even paragraph-at-once justification is a typesetting refinement, and this
/// is what a text editor does because it is what makes the line under the cursor stop moving
/// while you type in it.
///
/// A `width` of zero or less means **do not wrap**: one line per mandatory break, which is what
/// a CLI printing a document without `--width` wants and what a shell measuring intrinsic width
/// asks for.
///
/// Break opportunities come from UAX #14 (`unicode-linebreak`), so this splits at the places
/// Unicode says a line may end rather than at ASCII spaces — which is the difference between
/// wrapping prose and wrapping English prose.
pub fn wrap(fragments: &[Fragment<'_>], width: f32, metrics: &dyn Metrics) -> Layout {
    wrap_indented(fragments, width, metrics, 0.0)
}

/// [`wrap`], with the first line starting `first` further in than the others and breaking that
/// much shorter — `fo:text-indent` (`doc/odt-format.md` §5c, fact 11). Negative hangs: the first
/// line starts further out and has that much more room. Only a printed page asks for it today.
pub fn wrap_indented(
    fragments: &[Fragment<'_>],
    width: f32,
    metrics: &dyn Metrics,
    first: f32,
) -> Layout {
    wrap_tabbed(fragments, width, metrics, first, &Tabs::default())
}

/// How a tab character's text lines up against its stop (`style:type`, rng:13921).
#[derive(Clone, Copy, Debug, PartialEq)]
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
    pub fn is_empty(&self) -> bool {
        self.stops.is_empty() && self.interval <= 0.0
    }

    /// The first stop past `x`: one of the paragraph's own, else the next whole interval past
    /// both `x` and the last of its own. `None` when there is neither.
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
        })
    }
}

/// [`wrap_indented`], with every tab character advanced to the stop `tabs` gives it on its own
/// line — which depends on where the line starts, so tabs are placed *while* breaking rather than
/// measured once beforehand.
pub fn wrap_tabbed(
    fragments: &[Fragment<'_>],
    width: f32,
    metrics: &dyn Metrics,
    first: f32,
    tabs: &Tabs,
) -> Layout {
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
    // A line is as tall as the tallest thing that could be on it. Per-line height would need
    // the breaks first, and mixed font sizes inside one paragraph are rare enough that the
    // simpler rule is the honest trade — named here rather than discovered.
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
        });
        *top += height;
    };

    // The characters, and each one's own advance, for placing tabs — only when there is a tab
    // to place, so a paragraph without one is measured exactly as it always was.
    let chars: Vec<char> = text.chars().collect();
    let text_chars = &chars;
    let tabbed = !tabs.is_empty() && chars.contains(&'\t');
    let advance: Vec<f32> = match tabbed {
        true => xs.windows(2).map(|pair| pair[1] - pair[0]).collect(),
        false => Vec::new(),
    };
    // The cumulative x of each character from `start` to `end`, on a line that begins `origin`
    // in from the paragraph's left edge, with every tab on it at its stop.
    //
    // A tab whose stop is past the end of the line costs nothing there: the text after it
    // breaks onto the next line rather than taking the text before the tab with it.
    let place = |start: usize, end: usize, origin: f32, out: &mut Vec<f32>| {
        let room = if width > 0.0 {
            width - origin
        } else {
            f32::INFINITY
        };
        out.clear();
        out.push(0.0);
        let mut x = 0.0_f32;
        for i in start..end {
            let step = match chars[i] {
                '\t' => match tabs.next(origin + x) {
                    None => advance[i],
                    Some(stop) if stop.position - origin > room + 1e-3 => 0.0,
                    Some(stop) => {
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
                        (stop.position - origin - lead - x).max(0.0)
                    }
                },
                _ => advance[i],
            };
            x += step;
            out.push(x);
        }
    };
    let scratch = std::cell::RefCell::new(Vec::new());
    let span = |from: usize, to: usize, first_line: bool| match tabbed {
        false => xs[to] - xs[from],
        true => {
            let mut out = scratch.borrow_mut();
            place(from, to, if first_line { first } else { 0.0 }, &mut out);
            // Text right-aligned at a stop on the margin ends *at* the margin, which the
            // arithmetic of getting there may overshoot by a rounding error.
            out.last().copied().unwrap_or(0.0) - 1e-3
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
            // `last_fit` is deliberately not cleared here: every branch below assigns it, and
            // clearing it first would be a store nothing reads.
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
    let xs = match tabbed {
        false => xs,
        true => {
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
        }
    };

    Layout {
        lines,
        xs,
        baseline,
        first,
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
            Fixed.advances(text, style, out)
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
        let indented = wrap_indented(&fragments, 10.0, &Fixed, 3.0);
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
        let hanging = wrap_indented(&fragments, 10.0, &Fixed, -2.0);
        assert_eq!(hanging.lines()[0].end, 10, "twelve wide: \"aaaa bbbb \"");
        assert_eq!(
            wrap_indented(&fragments, 10.0, &Fixed, 0.0),
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
                top: 0.0
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

    /// Bidi is an explicit exclusion (`doc/text-layout.md`, decision 1). This test does not
    /// assert that RTL looks right — it asserts that it does not *crash* or lose characters,
    /// which is the whole of what is promised.
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
        wrap_tabbed(&fragments, width, &Fixed, first, tabs)
    }

    fn stop(position: f32, align: TabAlign) -> TabStop {
        TabStop { position, align }
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
