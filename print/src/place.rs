// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Where a caret is on a page, and which caret a point on a page means (`doc/page-model.md`
//! PM2). Lookups over the [`LineBox`]es typesetting leaves on each [`Page`]: no font, no layout.
//! In points from the page's top-left corner, as the display list is.

use grind_text::Caret;

use crate::ops::{Document, Page};

/// One line of a block as it was placed on a page: the caret offsets it holds and the x of
/// each. A picture is one line as tall as itself.
#[derive(Clone, Debug, PartialEq)]
pub struct LineBox {
    pub block: usize,
    /// First caret offset on the line.
    pub start: usize,
    /// The last, which a soft-broken line shares with the start of the next.
    pub end: usize,
    pub top: f32,
    pub height: f32,
    pub baseline: f32,
    /// The x of every caret offset `start..=end`.
    pub xs: Vec<f32>,
    /// Whether this is the block's last line.
    pub last: bool,
}

impl LineBox {
    /// The x of a caret offset on this line; one outside it is its nearer end.
    #[must_use]
    pub fn x(&self, offset: usize) -> f32 {
        let at = offset.clamp(self.start, self.end) - self.start;
        self.xs.get(at).copied().unwrap_or(0.0)
    }

    /// The caret offset nearest `x`, the first of two equally near.
    #[must_use]
    pub fn offset_at(&self, x: f32) -> usize {
        let nearest = self
            .xs
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| (*a - x).abs().total_cmp(&(*b - x).abs()))
            .map_or(0, |(at, _)| at);
        self.start + nearest
    }

    fn holds(&self, at: Caret) -> bool {
        self.block == at.block && (self.start..=self.end).contains(&at.offset)
    }

    /// How far `(x, y)` is from the line, down and then across; zero inside it.
    fn distance(&self, x: f32, y: f32) -> (f32, f32) {
        let outside = |value: f32, from: f32, to: f32| (from - value).max(value - to).max(0.0);
        let (left, right) = (self.x(self.start), self.x(self.end));
        (
            outside(y, self.top, self.top + self.height),
            outside(x, left.min(right), left.max(right)),
        )
    }
}

/// Where a caret is drawn: a page, 0-based, and the vertical stroke on it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spot {
    pub page: usize,
    pub x: f32,
    pub top: f32,
    pub height: f32,
}

/// A rectangle on a page.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Whether `page` can hold `at`, going by the text it starts and ends with. A page of table
/// cells may hold blocks out of order, so a miss here is checked again without it.
fn may_hold(page: &Page, at: Caret) -> bool {
    matches!((page.start, page.end), (Some(start), Some(end)) if start <= at && at <= end)
}

/// Where `at` is. At a soft break it is the start of the later line, as
/// [`grind_core::layout::Layout::line_at`] has it; `None` for a caret in text that is on no page.
#[must_use]
pub fn locate(doc: &Document, at: Caret) -> Option<Spot> {
    let on = |number: usize, page: &Page| {
        let mut holding = page.lines.iter().filter(|line| line.holds(at));
        let first = holding.next()?;
        // The line the offset is inside, else the last that ends on it.
        let line = if first.end == at.offset && !first.last {
            holding.next().unwrap_or(first)
        } else {
            first
        };
        Some((
            Spot {
                page: number,
                x: line.x(at.offset),
                top: line.top,
                height: line.height,
            },
            line.end == at.offset && !line.last,
        ))
    };
    let pages = || doc.pages.iter().enumerate();
    let mut found = pages()
        .filter(|(_, page)| may_hold(page, at))
        .filter_map(|(number, page)| on(number, page));
    let mut fallback = pages().filter_map(|(number, page)| on(number, page));
    let (spot, shared) = found.next().or_else(|| fallback.next())?;
    if !shared {
        return Some(spot);
    }
    // The line ended a page on the shared offset: the caret is at the top of the next.
    let next = doc.pages.get(spot.page + 1).and_then(|page| {
        let line = page.lines.iter().find(|line| line.holds(at))?;
        Some(Spot {
            page: spot.page + 1,
            x: line.x(at.offset),
            top: line.top,
            height: line.height,
        })
    });
    Some(next.unwrap_or(spot))
}

/// The caret a point on `page` means: on the nearest line, nearest first down and then across —
/// a click in a margin, under the last line or between two paragraphs is the text beside it.
/// `None` for a page with no text or no such page.
#[must_use]
pub fn hit(doc: &Document, page: usize, x: f32, y: f32) -> Option<Caret> {
    let line = doc.pages.get(page)?.lines.iter().min_by(|a, b| {
        let (a, b) = (a.distance(x, y), b.distance(x, y));
        a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1))
    })?;
    Some(Caret {
        block: line.block,
        offset: line.offset_at(x),
    })
}

/// Every line of the document in reading order, with its page.
fn lines(doc: &Document) -> impl Iterator<Item = (usize, &LineBox)> {
    doc.pages
        .iter()
        .enumerate()
        .flat_map(|(number, page)| page.lines.iter().map(move |line| (number, line)))
}

/// Which line of [`lines`] a caret is on — at a soft break the later one, as [`locate`] has it.
fn line_of(doc: &Document, at: Caret) -> Option<usize> {
    let mut found = None;
    for (index, (_, line)) in lines(doc).enumerate() {
        if !line.holds(at) {
            continue;
        }
        found = Some(index);
        if at.offset < line.end || line.last {
            break;
        }
    }
    found
}

/// Move a caret `delta` lines — Down and Up, and a screenful with a bigger number — keeping as
/// near `goal_x` as each line allows, across paragraphs, table cells and pages. It stops at
/// the document's first and last line. `goal_x` is a page x, a [`Spot::x`] the caller keeps
/// until the caret moves sideways.
#[must_use]
pub fn step(doc: &Document, at: Caret, delta: isize, goal_x: f32) -> Caret {
    let Some(from) = line_of(doc, at) else {
        return at;
    };
    let to = from.saturating_add_signed(delta);
    match lines(doc).nth(to).or_else(|| lines(doc).last()) {
        Some((_, line)) if to != from => Caret {
            block: line.block,
            offset: line.offset_at(goal_x),
        },
        _ => at,
    }
}

/// The two ends of the caret's own line — Home and End.
#[must_use]
pub fn line_bounds(doc: &Document, at: Caret) -> Option<(Caret, Caret)> {
    let (_, line) = lines(doc).nth(line_of(doc, at)?)?;
    let caret = |offset| Caret {
        block: line.block,
        offset,
    };
    Some((caret(line.start), caret(line.end)))
}

/// The rectangles covering the text between two carets, in either order: one per line, on its
/// page. What a selection, a find hit or a misspelt word's underline is drawn from.
#[must_use]
pub fn selection(doc: &Document, from: Caret, to: Caret) -> Vec<(usize, Rect)> {
    selection_on(doc, 0..doc.pages.len(), from, to)
}

/// [`selection`] on `pages` alone — what a window showing a few of them draws.
#[must_use]
pub fn selection_on(
    doc: &Document,
    pages: std::ops::Range<usize>,
    from: Caret,
    to: Caret,
) -> Vec<(usize, Rect)> {
    let (from, to) = (from.min(to), from.max(to));
    let mut out = Vec::new();
    let skip = pages.start;
    let pages = doc.pages.get(pages).unwrap_or_default();
    for (number, page) in pages.iter().enumerate().map(|(at, page)| (at + skip, page)) {
        for line in &page.lines {
            let caret = |offset| Caret {
                block: line.block,
                offset,
            };
            let (lo, hi) = (from.max(caret(line.start)), to.min(caret(line.end)));
            if lo >= hi {
                continue;
            }
            let (left, right) = (line.x(lo.offset), line.x(hi.offset));
            out.push((
                number,
                Rect {
                    x: left.min(right),
                    y: line.top,
                    width: (right - left).abs(),
                    height: line.height,
                },
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Fonts, Typesetter, text};
    use grind_text::App;

    fn set(app: &App) -> Document {
        text::typeset(
            app,
            &Typesetter::new(Fonts::bundled()),
            &text::Options::default(),
        )
    }

    fn fodt(styles: &str, body: &str) -> App {
        let bytes = format!(
            r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:mimetype="application/vnd.oasis.opendocument.text">
            <office:automatic-styles>{styles}</office:automatic-styles>
            <office:body><office:text>{body}</office:text></office:body></office:document>"#
        );
        let app = App::new();
        app.open_bytes("a.fodt", bytes.as_bytes()).unwrap();
        app
    }

    const STYLES: &str = r#"<style:style style:name="J" style:family="paragraph"><style:paragraph-properties fo:text-align="justify" fo:text-indent="1cm"/></style:style>
        <style:style style:name="C" style:family="paragraph"><style:paragraph-properties fo:text-align="center"/></style:style>
        <style:style style:name="B" style:family="text"><style:text-properties fo:font-weight="bold"/></style:style>"#;

    /// Paragraphs enough for three pages: justified with an indent, centred, plain with a bold
    /// word, and a table.
    fn long() -> App {
        let words = "AVATAR To office waffle ".repeat(30);
        let one = format!(
            r#"<text:p text:style-name="J">{words}</text:p><text:p text:style-name="C">middle of it</text:p><text:p>plain <text:span text:style-name="B">bold</text:span> plain {words}</text:p>"#
        );
        let mut body = one.repeat(12);
        body.push_str(
            r#"<table:table table:name="T"><table:table-column table:number-columns-repeated="2"/><table:table-row><table:table-cell><text:p>left cell</text:p></table:table-cell><table:table-cell><text:p>right cell</text:p></table:table-cell></table:table-row></table:table><text:p>after</text:p>"#,
        );
        fodt(STYLES, &body)
    }

    #[test]
    fn a_click_where_a_caret_is_drawn_is_that_caret() {
        let app = long();
        let doc = set(&app);
        assert!(doc.pages.len() >= 3, "{} pages", doc.pages.len());
        let mut checked = 0;
        for view in app.get_viewport(0..app.block_count()).iter() {
            for offset in 0..=view.text.chars().count() {
                let at = Caret {
                    block: view.index,
                    offset,
                };
                let spot = locate(&doc, at).unwrap_or_else(|| panic!("{at:?} is on no page"));
                let back = hit(&doc, spot.page, spot.x, spot.top + spot.height / 2.0);
                // Two offsets can share an x: a space the line broke at hangs at zero width
                // only when it is not there, so the first of them is what a click means.
                let same_x = back.is_some_and(|back| {
                    back.block == at.block
                        && locate(&doc, back).is_some_and(|other| {
                            other.page == spot.page
                                && (other.x - spot.x).abs() < 1e-3
                                && other.top == spot.top
                        })
                });
                assert!(back == Some(at) || same_x, "{at:?} came back as {back:?}");
                checked += 1;
            }
        }
        assert!(checked > 5000, "{checked}");
    }

    #[test]
    fn a_caret_at_a_soft_break_is_at_the_start_of_the_later_line() {
        let app = fodt("", &format!("<text:p>{}</text:p>", "word ".repeat(40)));
        let doc = set(&app);
        let lines = &doc.pages[0].lines;
        assert!(lines.len() >= 2);
        let shared = lines[0].end;
        assert_eq!(shared, lines[1].start);
        let spot = locate(
            &doc,
            Caret {
                block: 0,
                offset: shared,
            },
        )
        .unwrap();
        assert_eq!(spot.top, lines[1].top);
        assert_eq!(spot.x, lines[1].x(shared));
        // The paragraph's very end has no later line.
        let end = lines.last().unwrap();
        let spot = locate(
            &doc,
            Caret {
                block: 0,
                offset: end.end,
            },
        )
        .unwrap();
        assert_eq!((spot.top, spot.x), (end.top, end.x(end.end)));
    }

    #[test]
    fn a_click_off_the_text_is_the_text_beside_it() {
        let app = fodt("", "<text:p>one two</text:p><text:p>three</text:p>");
        let doc = set(&app);
        let (first, second) = (&doc.pages[0].lines[0], &doc.pages[0].lines[1]);
        let middle = first.top + first.height / 2.0;
        assert_eq!(
            hit(&doc, 0, 0.0, middle),
            Some(Caret {
                block: 0,
                offset: 0
            })
        );
        assert_eq!(
            hit(&doc, 0, 590.0, middle),
            Some(Caret {
                block: 0,
                offset: 7
            })
        );
        assert_eq!(
            hit(&doc, 0, 590.0, 800.0),
            Some(Caret {
                block: 1,
                offset: 5
            })
        );
        assert_eq!(
            hit(&doc, 0, 0.0, 0.0),
            Some(Caret {
                block: 0,
                offset: 0
            })
        );
        assert_eq!(
            hit(&doc, 0, second.x(2) + 0.1, second.top + 1.0),
            Some(Caret {
                block: 1,
                offset: 2
            })
        );
        assert_eq!(hit(&doc, 7, 10.0, 10.0), None);
    }

    #[test]
    fn a_click_in_a_table_cell_is_in_that_cell() {
        let app = long();
        let doc = set(&app);
        let views = app.get_viewport(0..app.block_count());
        let right = views.iter().find(|view| view.text == "right cell").unwrap();
        let spot = locate(
            &doc,
            Caret {
                block: right.index,
                offset: 3,
            },
        )
        .unwrap();
        let line = doc.pages[spot.page]
            .lines
            .iter()
            .find(|line| line.block == right.index)
            .unwrap();
        // Past the end of the right cell's text, on its line.
        assert_eq!(
            hit(&doc, spot.page, line.x(line.end) + 40.0, spot.top + 2.0),
            Some(Caret {
                block: right.index,
                offset: 10
            })
        );
    }

    #[test]
    fn down_and_up_walk_the_lines_across_paragraphs_and_pages() {
        let app = long();
        let doc = set(&app);
        let all: Vec<(usize, &LineBox)> = lines(&doc).collect();
        // From the first line down to the last, one line at a time, never skipping one.
        let mut at = Caret {
            block: 0,
            offset: 0,
        };
        let goal = locate(&doc, at).unwrap().x;
        for (index, (_, line)) in all.iter().enumerate().skip(1) {
            at = step(&doc, at, 1, goal);
            assert_eq!(line_of(&doc, at), Some(index), "{at:?}");
            assert_eq!(at.block, line.block);
        }
        assert_eq!(
            step(&doc, at, 1, goal),
            at,
            "the last line is where Down stops"
        );
        // A screenful up, and back to the top however far is asked.
        let up = step(&doc, at, -30, goal);
        assert_eq!(line_of(&doc, up), Some(all.len() - 31));
        let top = step(&doc, at, -100_000, goal);
        assert_eq!(line_of(&doc, top), Some(0));
        assert_eq!(step(&doc, top, -1, goal), top);
    }

    #[test]
    fn down_keeps_its_column_through_a_short_line() {
        let app = fodt(
            "",
            "<text:p>a long line of some text</text:p><text:p>x</text:p><text:p>a long line of some text</text:p>",
        );
        let doc = set(&app);
        let from = Caret {
            block: 0,
            offset: 20,
        };
        let goal = locate(&doc, from).unwrap().x;
        let short = step(&doc, from, 1, goal);
        assert_eq!(
            short,
            Caret {
                block: 1,
                offset: 1
            }
        );
        assert_eq!(
            step(&doc, short, 1, goal),
            Caret {
                block: 2,
                offset: 20
            }
        );
    }

    #[test]
    fn home_and_end_are_the_ends_of_the_carets_own_line() {
        let app = fodt("", &format!("<text:p>{}</text:p>", "word ".repeat(40)));
        let doc = set(&app);
        let lines = &doc.pages[0].lines;
        let second = &lines[1];
        let inside = Caret {
            block: 0,
            offset: second.start + 3,
        };
        let ends = line_bounds(&doc, inside).unwrap();
        assert_eq!((ends.0.offset, ends.1.offset), (second.start, second.end));
        // The offset two lines share is the later line's start.
        let shared = Caret {
            block: 0,
            offset: second.start,
        };
        assert_eq!(line_bounds(&doc, shared).unwrap().0.offset, second.start);
        assert_eq!(
            line_bounds(
                &doc,
                Caret {
                    block: 9,
                    offset: 0
                }
            ),
            None
        );
    }

    #[test]
    fn a_selection_is_one_rectangle_a_line() {
        let app = fodt(
            "",
            &format!(
                "<text:p>{}</text:p><text:p>next</text:p>",
                "word ".repeat(40)
            ),
        );
        let doc = set(&app);
        let lines = &doc.pages[0].lines;
        let from = Caret {
            block: 0,
            offset: 2,
        };
        let to = Caret {
            block: 1,
            offset: 2,
        };
        let rects = selection(&doc, from, to);
        assert_eq!(rects.len(), lines.len(), "every line is touched");
        assert_eq!(rects, selection(&doc, to, from), "in either order");
        let (page, first) = rects[0];
        assert_eq!(page, 0);
        assert_eq!(first.x, lines[0].x(2));
        assert_eq!(first.x + first.width, lines[0].x(lines[0].end));
        let (_, last) = rects[rects.len() - 1];
        let next = lines.last().unwrap();
        assert_eq!((last.x, last.width), (next.x(0), next.x(2) - next.x(0)));
        assert_eq!(selection(&doc, from, from), vec![]);
    }

    #[test]
    fn a_caret_across_a_page_break_is_on_the_page_its_line_is_on() {
        let app = long();
        let doc = set(&app);
        for (number, page) in doc.pages.iter().enumerate().skip(1) {
            let first = &page.lines[0];
            let at = Caret {
                block: first.block,
                offset: first.start,
            };
            let spot = locate(&doc, at).unwrap();
            assert_eq!((spot.page, spot.top), (number, first.top), "{at:?}");
        }
    }
}
