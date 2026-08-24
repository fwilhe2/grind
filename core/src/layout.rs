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
}

impl Layout {
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

    /// The x of a caret offset, **relative to the start of its line**.
    pub fn x_at(&self, offset: usize) -> f32 {
        let offset = offset.min(self.len());
        let line = &self.lines[self.line_at(offset)];
        self.xs[offset] - self.xs[line.start]
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
    // One `advances` call per fragment, concatenated into a single cumulative array over the
    // whole text. `xs[i]` is the x of caret offset `i`, before any line breaking.
    let mut xs = Vec::with_capacity(64);
    xs.push(0.0);
    let mut text = String::new();
    let mut heights: Vec<f32> = Vec::new();
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
    }
    let len = xs.len() - 1;
    // A line is as tall as the tallest thing that could be on it. Per-line height would need
    // the breaks first, and mixed font sizes inside one paragraph are rare enough that the
    // simpler rule is the honest trade — named here rather than discovered.
    let height = heights.iter().copied().fold(0.0_f32, f32::max).max(1.0);

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

    let fits = |from: usize, to: usize| width <= 0.0 || xs[to] - xs[from] <= width;

    for (byte, opportunity) in linebreaks(&text) {
        let at = char_of_byte[byte];
        if at <= start {
            continue;
        }
        // Overflowed the line in hand. Close it at the last opportunity that fitted — and if
        // none did, this run is wider than the whole line, so it gets a line of its own further
        // down. A mandatory break is checked here too: the end of a paragraph is still allowed
        // to be the moment a line turns out not to fit.
        if !fits(start, at)
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
        } else if fits(start, at) {
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

    Layout { lines, xs }
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

    #[test]
    fn text_breaks_at_the_last_opportunity_that_fits() {
        let text = "the cat sat on the mat";
        let layout = at(text, 10.0);
        // "the cat " is 8 and fits; "the cat sat" is 11 and does not.
        assert_eq!(
            rendered(text, &layout),
            vec!["the cat ", "sat on ", "the mat"]
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
}
