// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A frame of the page as a list of [`Op`]s — every decision about what is drawn where, with no
//! CoreGraphics in it (M6). `sheet/paint.rs`'s twin, and the reason it is portable is the same:
//! where a run sits, which part of a line a selection washes, where the caret is inside a
//! composition, all tested here against [`grind_text::Fixed`].
//!
//! What a line is made of is `grind_text::paint`'s — pieces cut at every run, a tab and a line
//! break measured and never drawn, the selection band, the bullet — so this file is only the
//! walk: the blocks the flow says are in view, each laid out through the page's own `Faces`
//! (the same answer the caret's motions got), and each piece set in the
//! [`Font`](crate::metrics::Font) its block and its run resolve to (`text/face.rs`), which is
//! also what measured it.
//!
//! **An input method's composition is drawn inline**, in the line it will land in:
//! `App::layout_composing` lays the caret's block out as if the marked text were typed, and the
//! marked text is underlined, its selected clause more heavily — the Mac convention, and ahead of
//! the Windows pane, whose composition is the system's floating box.
//!
//! **What it does not draw**, each a named gap in `doc/macos-shell.md`: a picture (its block is
//! outlined where the picture would go), the name overlay (M8), and paragraph styles beyond the
//! faces `grind_text::look` names.

use grind_core::color::{self, Rgb};
use grind_text::flow::Flow;
use grind_text::look::Role;
use grind_text::paint::{band, bullet, covered, drawable, pieces};
use grind_text::style::CharStyle;
use grind_text::{App, BlockKind, Caret, Faces, RunView};

use super::face;
use super::geom::{CARET_W, RULE, bullet_x};
use super::state::Page;
use crate::ops::Op;
use crate::sheet::geom::Rect;

/// How far a table's rule moves the page towards the ink — the other two windows' 28%, so a
/// table is one weight in every window that draws one.
pub const RULE_INK: f64 = 0.28;

/// How thick a composition's underline is, and its selected clause's.
pub const MARKED: f64 = 1.0;
pub const MARKED_CLAUSE: f64 = 2.0;

/// The colours a page is drawn in, already resolved — from `NSColor`'s semantic colours in the
/// view's appearance at draw time, or [`Palette::LIGHT`] and [`Palette::DARK`] for a render.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    /// The paper — `textBackgroundColor`.
    pub page: Rgb,
    /// Text the document gave no colour — `textColor`.
    pub ink: Rgb,
    /// The selection's wash and the caret — `controlAccentColor`, the user's own.
    pub accent: Rgb,
    /// Whether the page is dark, which lifts a document's own colours along their hue.
    pub dark: bool,
}

impl Palette {
    pub const LIGHT: Palette = Palette {
        page: (0xff, 0xff, 0xff),
        ink: (0x1d, 0x1d, 0x1f),
        accent: (0x00, 0x7a, 0xff),
        dark: false,
    };
    pub const DARK: Palette = Palette {
        page: (0x1e, 0x1e, 0x1e),
        ink: (0xf5, 0xf5, 0xf7),
        accent: (0x0a, 0x84, 0xff),
        dark: true,
    };

    /// A table's rule: the ink, [`RULE_INK`] of the way over the page.
    pub fn rule(&self) -> Rgb {
        let mix = |page: u8, ink: u8| {
            (f64::from(page) + (f64::from(ink) - f64::from(page)) * RULE_INK).round() as u8
        };
        (
            mix(self.page.0, self.ink.0),
            mix(self.page.1, self.ink.1),
            mix(self.page.2, self.ink.2),
        )
    }
}

/// Everything one frame of the page needs.
pub struct Frame<'a> {
    pub app: &'a App,
    pub flow: &'a Flow,
    /// The page's `Faces` — the same one the caret's motions measure through.
    pub faces: &'a dyn Faces,
    /// Where the text column starts, in the page's own coordinates.
    pub column_x: f64,
    /// The part of the page to draw, in its own coordinates — a view's dirty rectangle, or the
    /// whole of a render.
    pub view: Rect,
    pub state: &'a Page,
    /// Whether the caret is drawn — off while the view is not the key view, as every Mac text
    /// view's is.
    pub caret: bool,
    pub palette: &'a Palette,
}

/// A block's runs with a composition spliced in at `at`, in `props` — the pieces
/// `App::layout_composing` laid out, so the painter walks the same characters the layout
/// measured.
fn composed(runs: &[RunView], at: usize, text: &str, props: &CharStyle) -> Vec<RunView> {
    let len = text.chars().count();
    let mut out = Vec::with_capacity(runs.len() + 2);
    let mut inserted = false;
    let composition = || RunView {
        start: at,
        text: text.to_owned(),
        props: props.clone(),
        style: None,
        href: None,
        image: None,
    };
    for run in runs {
        let end = run.end();
        if !inserted && run.start < at && at < end {
            // The composition lands inside this run: cut it in two around the marked text.
            let split = run
                .text
                .char_indices()
                .nth(at - run.start)
                .map_or(run.text.len(), |(byte, _)| byte);
            out.push(RunView {
                text: run.text[..split].to_owned(),
                ..run.clone()
            });
            out.push(composition());
            out.push(RunView {
                start: at + len,
                text: run.text[split..].to_owned(),
                ..run.clone()
            });
            inserted = true;
            continue;
        }
        if !inserted && run.start >= at {
            out.push(composition());
            inserted = true;
        }
        let shift = if inserted && run.start >= at { len } else { 0 };
        out.push(RunView {
            start: run.start + shift,
            ..run.clone()
        });
    }
    if !inserted {
        out.push(composition());
    }
    out
}

/// A colour a document wrote, if it is one — `transparent` is not.
fn document_color(value: Option<&str>) -> Option<Rgb> {
    value
        .filter(|value| *value != "transparent")
        .and_then(color::parse)
}

/// The page's frame, as the things to draw — in its own coordinates, flipped, y down.
pub fn frame(frame: &Frame) -> Vec<Op> {
    let Frame {
        app,
        flow,
        faces,
        column_x,
        view,
        state,
        palette,
        ..
    } = *frame;
    let mut ops = vec![Op::Fill {
        rect: view,
        color: palette.page,
    }];

    // A table's grid, under its text. Each edge is a band on the box's outer line, where the
    // neighbour's edge is too, so a shared edge stays one rule wide.
    let rule = palette.rule();
    for cell in flow.cells() {
        let (l, t) = (column_x + cell.left, cell.top);
        let (r, b) = (column_x + cell.right_edge(), cell.bottom());
        for rect in [
            Rect::new(l, t, r - l + RULE, RULE),
            Rect::new(l, b, r - l + RULE, RULE),
            Rect::new(l, t, RULE, b - t + RULE),
            Rect::new(r, t, RULE, b - t + RULE),
        ] {
            let rect = rect.intersection(&view);
            if !rect.is_empty() {
                ops.push(Op::Fill { rect, color: rule });
            }
        }
    }

    let visible = flow.visible(view.y, view.bottom());
    let (Some(first), Some(last)) = (visible.first(), visible.last()) else {
        return ops;
    };
    // Exactly the blocks in view reach the viewport — architecture rule 1.
    let viewport = app.get_viewport(first.index..last.index + 1);
    let selection = state.selection();
    let shown = state.shown_caret();
    for slot in visible {
        let Some(block) = viewport.get(slot.index) else {
            continue;
        };
        let x = column_x + slot.indent;
        let role = Role::of(&block.kind, block.style.as_deref());

        if grind_text::picture_of(block).is_some() {
            // ponytail: a picture is outlined where it goes rather than drawn. Decoding one is
            // `CGImageSource`'s, and the flow sizes the block as its placeholder line until the
            // page has a decoder to hand `grind_text::flow::lay_out`'s picture hook.
            let rect = Rect::new(x, slot.top, slot.width, slot.height.max(1.0));
            for edge in [
                Rect::new(rect.x, rect.y, rect.w, RULE),
                Rect::new(rect.x, rect.bottom() - RULE, rect.w, RULE),
                Rect::new(rect.x, rect.y, RULE, rect.h),
                Rect::new(rect.right() - RULE, rect.y, RULE, rect.h),
            ] {
                ops.push(Op::Fill {
                    rect: edge,
                    color: rule,
                });
            }
            continue;
        }

        let (width, metrics) = faces.of(slot.index, &block.kind, block.style.as_deref());
        let composing = state
            .composing
            .as_ref()
            .filter(|_| slot.index == state.caret.block);
        let (layout, runs) = match composing {
            Some(composing) => {
                // The formatting the next character takes: a pending markdown `resume`, or the
                // run at the caret — `App::char_style`'s answer for an empty span, which is the
                // rule `layout_composing` applies.
                let props = match &state.resume {
                    Some(resume) => resume.clone(),
                    None => app.char_style(state.caret, state.caret).unwrap_or_default(),
                };
                let layout = app.layout_composing(
                    state.caret,
                    &composing.text,
                    state.resume.as_ref(),
                    width,
                    metrics,
                );
                (
                    layout,
                    composed(&block.runs, state.caret.offset, &composing.text, &props),
                )
            }
            None => (
                app.layout_block(slot.index, width, metrics),
                block.runs.clone(),
            ),
        };
        let Ok(layout) = layout else {
            continue;
        };

        // The bullet, hung in the indent and drawn in the block's own face — outside the text,
        // and outside the model.
        if let BlockKind::ListItem { depth } = block.kind {
            ops.push(Op::Run {
                x: x + bullet_x(),
                top: slot.top,
                text: bullet(depth).to_owned(),
                font: face::font(role, &Default::default()),
                color: palette.ink,
                underline: false,
                strike: false,
                clip: view,
            });
        }

        let washed = selection.and_then(|(from, to)| covered(slot.index, from, to));
        for (number, line) in layout.lines().iter().enumerate() {
            let top = slot.top + f64::from(line.top);
            let height = f64::from(line.height);
            if top + height < view.y || top > view.bottom() {
                continue;
            }
            // The selection, under the text.
            if let Some((left, right)) =
                washed.and_then(|(start, end)| band(&layout, line, start, end))
            {
                ops.push(Op::Wash {
                    rect: Rect::new(x + f64::from(left), top, f64::from(right - left), height),
                    color: palette.accent,
                });
            }
            for piece in pieces(&runs, line.start, line.end) {
                let fill = document_color(piece.props.background.as_deref());
                let length = piece.text.chars().count();
                if let (Some(fill), Some((left, right))) =
                    (fill, band(&layout, line, piece.start, piece.start + length))
                {
                    ops.push(Op::Fill {
                        rect: Rect::new(x + f64::from(left), top, f64::from(right - left), height),
                        color: fill,
                    });
                }
                let ink = color::document_ink(
                    document_color(piece.props.color.as_deref()),
                    fill,
                    palette.page,
                    palette.ink,
                    palette.dark,
                );
                let font = face::font(role, &piece.props.metrics());
                // Each segment at the x the core measured for its first character — which is
                // what places the text after a tab or a line break, both measured and neither
                // drawn.
                for (start, segment) in drawable(piece.start, piece.text) {
                    ops.push(Op::Run {
                        x: x + f64::from(layout.x_at(start)),
                        top,
                        text: segment.to_owned(),
                        font: font.clone(),
                        color: ink,
                        underline: piece.props.is_underlined(),
                        strike: piece.props.is_struck(),
                        clip: view,
                    });
                }
            }
            // The composition's underline: all of it thin, the clause the input method has
            // selected thick.
            if let Some(composing) = composing {
                let at = state.caret.offset;
                let len = composing.text.chars().count();
                let clause = at + composing.selected.start..at + composing.selected.end;
                for (range, weight) in [(at..at + len, MARKED), (clause, MARKED_CLAUSE)] {
                    if let Some((left, right)) = band(&layout, line, range.start, range.end) {
                        ops.push(Op::Fill {
                            rect: Rect::new(
                                x + f64::from(left),
                                top + height - weight,
                                f64::from(right - left),
                                weight,
                            ),
                            color: palette.ink,
                        });
                    }
                }
            }
            // The caret, over the text it sits in, on the line `Layout` resolved it to — at a
            // soft break the offset is on two lines and only the core knows which it meant.
            if frame.caret
                && selection.is_none()
                && slot.index == shown.block
                && layout.line_at(shown.offset) == number
            {
                ops.push(Op::Fill {
                    rect: Rect::new(
                        x + f64::from(layout.x_at(shown.offset)),
                        top,
                        CARET_W,
                        height,
                    ),
                    color: palette.accent,
                });
            }
        }
    }
    ops
}

/// Where the caret is drawn, in the page's own coordinates: its rectangle, for the input
/// method's candidate window and for scrolling it into view — the same arithmetic [`frame`]
/// draws it with. `None` when its block is not laid out.
pub fn caret_rect(
    app: &App,
    flow: &Flow,
    faces: &dyn Faces,
    column_x: f64,
    state: &Page,
) -> Option<Rect> {
    let at: Caret = state.shown_caret();
    let slot = flow.slot(at.block)?;
    let viewport = app.get_viewport(at.block..at.block + 1);
    let block = viewport.get(at.block)?;
    let (width, metrics) = faces.of(at.block, &block.kind, block.style.as_deref());
    let layout = match &state.composing {
        Some(composing) => app.layout_composing(
            state.caret,
            &composing.text,
            state.resume.as_ref(),
            width,
            metrics,
        ),
        None => app.layout_block(at.block, width, metrics),
    }
    .ok()?;
    let line = layout.lines().get(layout.line_at(at.offset))?;
    Some(Rect::new(
        column_x + slot.indent + f64::from(layout.x_at(at.offset)),
        slot.top + f64::from(line.top),
        CARET_W,
        f64::from(line.height),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::face::Column;
    use crate::text::geom::spacing;
    use grind_text::Fixed;
    use std::collections::HashMap;

    struct Setup {
        app: App,
        flow: Flow,
        faces: Vec<Fixed>,
        across: HashMap<usize, grind_text::flow::Across>,
    }

    /// A document at a measure of 20, one unit a character and a line.
    fn setup(blocks: &[&str]) -> Setup {
        let app = App::new();
        for (index, text) in blocks.iter().enumerate() {
            match index {
                0 => app.set_text(0, text).unwrap(),
                _ => app.insert(index, BlockKind::Paragraph, text).unwrap(),
            }
        }
        let faces = vec![Fixed; Role::ALL.len()];
        let across = grind_text::flow::across(&app, 20.0, &spacing());
        let column = Column {
            faces: &faces,
            width: 20.0,
            spacing: spacing(),
            across: &across,
        };
        let flow = grind_text::flow::lay_out(&app, &column, 20.0, &spacing(), &|_, _| None);
        Setup {
            app,
            flow,
            faces,
            across,
        }
    }

    fn draw(setup: &Setup, state: &Page) -> Vec<Op> {
        let column = Column {
            faces: &setup.faces,
            width: 20.0,
            spacing: spacing(),
            across: &setup.across,
        };
        frame(&Frame {
            app: &setup.app,
            flow: &setup.flow,
            faces: &column,
            column_x: 100.0,
            view: Rect::new(0.0, 0.0, 400.0, 400.0),
            state,
            caret: true,
            palette: &Palette::LIGHT,
        })
    }

    fn runs(ops: &[Op]) -> Vec<(String, f64, f64)> {
        ops.iter()
            .filter_map(|op| match op {
                Op::Run { text, x, top, .. } => Some((text.clone(), *x, *top)),
                _ => None,
            })
            .collect()
    }

    fn fills(ops: &[Op], color: Rgb) -> Vec<Rect> {
        ops.iter()
            .filter_map(|op| match op {
                Op::Fill { rect, color: c } if *c == color => Some(*rect),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_block_is_drawn_line_by_line_where_the_flow_put_it() {
        let setup = setup(&["the cat sat on the mat and more"]);
        let ops = draw(&setup, &Page::default());
        let top = spacing().top;
        assert_eq!(
            runs(&ops),
            [
                ("the cat sat on the ".to_owned(), 100.0, top),
                ("mat and more".to_owned(), 100.0, top + 1.0),
            ]
        );
        // The caret at the start, on the first line, in the accent.
        assert_eq!(
            fills(&ops, Palette::LIGHT.accent),
            [Rect::new(100.0, top, CARET_W, 1.0)]
        );
    }

    #[test]
    fn a_selection_washes_its_part_of_each_line_and_hides_the_caret() {
        let setup = setup(&["the cat sat on the mat and more"]);
        let mut state = Page::default();
        state.place(
            Caret {
                block: 0,
                offset: 4,
            },
            false,
        );
        state.place(
            Caret {
                block: 0,
                offset: 23,
            },
            true,
        );
        let ops = draw(&setup, &state);
        let washes: Vec<Rect> = ops
            .iter()
            .filter_map(|op| match op {
                Op::Wash { rect, .. } => Some(*rect),
                _ => None,
            })
            .collect();
        let top = spacing().top;
        assert_eq!(
            washes,
            [
                Rect::new(104.0, top, 15.0, 1.0),
                Rect::new(100.0, top + 1.0, 4.0, 1.0),
            ],
            "to the first line's own width, then into the second"
        );
        assert!(fills(&ops, Palette::LIGHT.accent).is_empty(), "no caret");
    }

    /// Marked text is drawn in the line, underlined, and the caret sits inside it where the input
    /// method put its own — and the document is untouched.
    #[test]
    fn a_composition_is_drawn_inline_and_underlined() {
        let setup = setup(&["caf au lait"]);
        let mut state = Page::default();
        state.place(
            Caret {
                block: 0,
                offset: 3,
            },
            false,
        );
        state.mark(&setup.app, "\u{e9}", 1..1).unwrap();
        let ops = draw(&setup, &state);
        let top = spacing().top;
        assert_eq!(
            runs(&ops),
            [
                ("caf".to_owned(), 100.0, top),
                ("\u{e9}".to_owned(), 103.0, top),
                (" au lait".to_owned(), 104.0, top),
            ],
            "the marked text in the line, and what follows it moved on"
        );
        let ink = fills(&ops, Palette::LIGHT.ink);
        assert_eq!(
            ink,
            [Rect::new(103.0, top + 1.0 - MARKED, 1.0, MARKED)],
            "one character underlined; an empty clause draws nothing"
        );
        assert_eq!(
            fills(&ops, Palette::LIGHT.accent),
            [Rect::new(104.0, top, CARET_W, 1.0)],
            "the caret after the marked text"
        );
        assert_eq!(setup.app.input_text(0).unwrap(), "caf au lait");
    }

    #[test]
    fn a_list_item_hangs_its_bullet_in_its_indent() {
        let setup = setup(&["one"]);
        setup
            .app
            .set_kind(0, BlockKind::ListItem { depth: 2 })
            .unwrap();
        let setup = Setup {
            flow: {
                let column = Column {
                    faces: &setup.faces,
                    width: 20.0,
                    spacing: spacing(),
                    across: &setup.across,
                };
                grind_text::flow::lay_out(&setup.app, &column, 20.0, &spacing(), &|_, _| None)
            },
            ..setup
        };
        let drawn = runs(&draw(&setup, &Page::default()));
        let indent = 2.0 * spacing().indent;
        assert_eq!(
            drawn[0],
            (
                bullet(2).to_owned(),
                100.0 + indent + bullet_x(),
                spacing().top
            )
        );
        assert_eq!(drawn[1].1, 100.0 + indent, "the text at its indent");
    }

    /// A tab is measured and never drawn: the text after it is placed where the core put it.
    #[test]
    fn the_text_after_a_tab_is_where_the_core_measured_it() {
        let setup = setup(&["name\tvalue"]);
        let drawn = runs(&draw(&setup, &Page::default()));
        assert_eq!(
            drawn
                .iter()
                .map(|(t, x, _)| (t.as_str(), *x))
                .collect::<Vec<_>>(),
            [("name", 100.0), ("value", 105.0)]
        );
    }

    #[test]
    fn a_composition_splices_into_the_run_it_lands_in() {
        let run = |start, text: &str| RunView {
            start,
            text: text.to_owned(),
            props: CharStyle::default(),
            style: None,
            href: None,
            image: None,
        };
        let spliced = composed(
            &[run(0, "abc"), run(3, "def")],
            1,
            "XY",
            &CharStyle::default(),
        );
        let got: Vec<(usize, &str)> = spliced.iter().map(|r| (r.start, r.text.as_str())).collect();
        assert_eq!(got, [(0, "a"), (1, "XY"), (3, "bc"), (5, "def")]);
        let at_end = composed(&[run(0, "ab")], 2, "Z", &CharStyle::default());
        assert_eq!(at_end.last().unwrap().start, 2);
        let between = composed(&[run(0, "ab"), run(2, "cd")], 2, "Z", &CharStyle::default());
        let got: Vec<usize> = between.iter().map(|r| r.start).collect();
        assert_eq!(got, [0, 2, 3]);
    }

    #[test]
    fn the_caret_rectangle_is_where_the_frame_draws_it() {
        let setup = setup(&["the cat sat on the mat and more"]);
        let mut state = Page::default();
        state.place(
            Caret {
                block: 0,
                offset: 22,
            },
            false,
        );
        let column = Column {
            faces: &setup.faces,
            width: 20.0,
            spacing: spacing(),
            across: &setup.across,
        };
        let rect = caret_rect(&setup.app, &setup.flow, &column, 100.0, &state).unwrap();
        assert_eq!(fills(&draw(&setup, &state), Palette::LIGHT.accent), [rect]);
        assert_eq!(rect, Rect::new(103.0, spacing().top + 1.0, CARET_W, 1.0));
    }

    #[test]
    fn a_rule_is_quieter_than_the_ink_and_louder_than_the_page() {
        for palette in [Palette::LIGHT, Palette::DARK] {
            let rule = palette.rule();
            assert!(color::contrast(rule, palette.page) > 1.1);
            assert!(
                color::contrast(rule, palette.page) < color::contrast(palette.ink, palette.page)
            );
        }
    }
}
