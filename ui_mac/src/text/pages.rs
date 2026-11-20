// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A frame of the page view as [`Op`]s (`doc/page-model.md`): the document's printed pages,
//! each its raster from `grind_print`, on a desk, and over them what is not on paper — the
//! selection, spelling and grammar dots, bookmark names, an input method's composition and the
//! caret. Where any of it is comes from `grind_print::view`; no CoreGraphics here, so the frame
//! is tested on Linux.

use std::rc::Rc;

use grind_core::color::Rgb;
use grind_print::view::Shown;
use grind_text::{App, Caret};

use super::face;
use super::geom::CARET_W;
use super::state::Page;
use crate::metrics::{Family, Font};
use crate::ops::Op;
use crate::sheet::geom::Rect;

/// The gap a bookmark's name keeps from the end of its line.
pub const NAME_GAP: f64 = 6.0;

/// A misspelt word's mark: a row of dots this wide, one dot's width apart, along the bottom of
/// its line — the dotted red underline every Mac text view draws under one (`doc/spelling.md`).
pub const SPELL_DOT: f64 = 2.0;

/// How thick a composition's underline is.
pub const MARKED: f64 = 1.0;

/// The colours a page is drawn in, already resolved — from `NSColor`'s semantic colours in the
/// view's appearance at draw time, or [`Palette::LIGHT`] and [`Palette::DARK`] for a render.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    /// The paper — `textBackgroundColor`.
    pub page: Rgb,
    /// Text the document gave no colour — `textColor`.
    pub ink: Rgb,
    /// The selection's wash and the caret — `controlAccentColor`, the user's own.
    pub accent: Rgb,
    /// Whether the page is dark, which lifts a document's own colours along their hue.
    pub dark: bool,
    /// A misspelt word's dots — `systemRedColor`.
    pub misspelt: Rgb,
    /// A grammar finding's dots — `systemGreenColor`, AppKit's own mark for grammar.
    pub grammar: Rgb,
}

impl Palette {
    pub const LIGHT: Palette = Palette {
        page: (0xff, 0xff, 0xff),
        ink: (0x1d, 0x1d, 0x1f),
        accent: (0x00, 0x7a, 0xff),
        dark: false,
        misspelt: (0xff, 0x3b, 0x30),
        grammar: (0x34, 0xc7, 0x59),
    };
    pub const DARK: Palette = Palette {
        page: (0x1e, 0x1e, 0x1e),
        ink: (0xf5, 0xf5, 0xf7),
        accent: (0x0a, 0x84, 0xff),
        dark: true,
        misspelt: (0xff, 0x45, 0x3a),
        grammar: (0x30, 0xd1, 0x58),
    };

    /// The ink most of the way to the page — a label that is present and not read.
    pub fn muted(&self) -> Rgb {
        grind_core::color::mix(self.page, self.ink, 0.45)
    }
}

/// A page's raster: premultiplied RGBA, top row first.
#[derive(Clone, Debug)]
pub struct Raster {
    pub width: u32,
    pub height: u32,
    pub rgba: Rc<[u8]>,
}

/// Everything one frame of the page view needs.
pub struct Frame<'a> {
    pub app: &'a App,
    pub shown: &'a Shown,
    /// The part of the view to draw, in the strip's coordinates — which are the view's own.
    pub view: Rect,
    /// Page `n`'s raster at the scale the view is seen at, drawn or kept.
    pub raster: &'a dyn Fn(usize) -> Option<Raster>,
    pub state: &'a Page,
    /// Whether the caret is drawn — off while the view is not the key view.
    pub caret: bool,
    /// View ▸ Names: each bookmark named past the end of its line.
    pub names: bool,
    pub palette: &'a Palette,
    /// The grammar rules the person ignores, which are not drawn (`doc/grammar.md`).
    pub ignored: &'a [String],
}

/// The desk round the pages, and the hairline round each: white paper in either appearance
/// (`doc/page-model.md` decision 5) needs a ground that is not white.
fn desk(palette: &Palette) -> (grind_core::color::Rgb, grind_core::color::Rgb) {
    if palette.dark {
        ((0x2a, 0x2a, 0x2c), (0x48, 0x48, 0x4a))
    } else {
        ((0xe8, 0xe8, 0xea), (0xc6, 0xc6, 0xc8))
    }
}

/// A row of dots along the bottom of `band` — the Mac's mark under a misspelt word.
fn dots(ops: &mut Vec<Op>, [x, y, w, h]: [f64; 4], color: grind_core::color::Rgb) {
    let mut dot = x;
    while dot < x + w {
        ops.push(Op::Fill {
            rect: Rect::new(dot, y + h - SPELL_DOT, SPELL_DOT, SPELL_DOT),
            color,
        });
        dot += 2.0 * SPELL_DOT;
    }
}

pub fn frame(frame: &Frame) -> Vec<Op> {
    let Frame {
        app,
        shown,
        view,
        state,
        palette,
        ..
    } = *frame;
    let (ground, hairline) = desk(palette);
    let mut ops = vec![Op::Fill {
        rect: view,
        color: ground,
    }];
    let pages = shown.visible(view.y, view.y + view.h);
    for page in pages.clone() {
        let Some([x, y, w, h]) = shown.sheet(page) else {
            continue;
        };
        ops.push(Op::Fill {
            rect: Rect::new(x - 1.0, y - 1.0, w + 2.0, h + 2.0),
            color: hairline,
        });
        ops.push(Op::Fill {
            rect: Rect::new(x, y, w, h),
            color: (0xff, 0xff, 0xff),
        });
        if let Some(raster) = (frame.raster)(page) {
            ops.push(Op::Pixels {
                rect: Rect::new(x, y, w, h),
                width: raster.width,
                height: raster.height,
                rgba: raster.rgba,
            });
        }
    }
    let selection = state.selection();
    if let Some((from, to)) = selection {
        for [x, y, w, h] in shown.bands(from, to, pages.clone()) {
            ops.push(Op::Wash {
                rect: Rect::new(x, y, w, h),
                color: palette.accent,
            });
        }
    }
    // The composition, underlined until it is committed.
    if let Some(composing) = &state.composing {
        let end = Caret {
            offset: state.caret.offset + composing.text.chars().count(),
            ..state.caret
        };
        for [x, y, w, h] in shown.bands(state.caret, end, pages.clone()) {
            ops.push(Op::Fill {
                rect: Rect::new(x, y + h - MARKED, w, MARKED),
                color: (0, 0, 0),
            });
        }
    }
    let at = |block: usize, offset: usize| Caret { block, offset };
    let caret = state.shown_caret();
    if let Some(blocks) = shown.blocks(pages.clone()) {
        // The word still being typed is not marked: the caret at its end with nothing selected is
        // somebody halfway through it.
        for wrong in app.misspellings(blocks.clone()) {
            if selection.is_none() && wrong.being_typed(caret) {
                continue;
            }
            let (from, to) = (
                at(wrong.block, wrong.offset),
                at(wrong.block, wrong.offset + wrong.len),
            );
            for band in shown.bands(from, to, pages.clone()) {
                dots(&mut ops, band, palette.misspelt);
            }
        }
        // A grammar finding's dots in green — muted for typography and style; one covering
        // nothing (a missing comma) dots the character before it.
        for flag in app.grammar(blocks.clone()) {
            let range = &flag.finding.range;
            if frame.ignored.contains(&flag.finding.rule)
                || (selection.is_none() && flag.block == caret.block && range.end == caret.offset)
            {
                continue;
            }
            let start = range.start.saturating_sub(usize::from(range.is_empty()));
            let color = match flag.finding.kind {
                grind_text::grammar::Kind::Typography | grind_text::grammar::Kind::Style => {
                    palette.muted()
                }
                _ => palette.grammar,
            };
            for band in shown.bands(
                at(flag.block, start),
                at(flag.block, range.end),
                pages.clone(),
            ) {
                dots(&mut ops, band, color);
            }
        }
        // A bookmark contributes no characters (`doc/view-modes.md` §3.6): a tick where it
        // anchors, and its name past the end of its line, muted — a label, not a control.
        if frame.names {
            let viewport = app.get_viewport(blocks.clone());
            for index in blocks {
                let Some(block) = viewport.get(index) else {
                    continue;
                };
                for (offset, name) in &block.marks {
                    let anchor = at(index, *offset);
                    let Some(tick) = shown.stroke(anchor) else {
                        continue;
                    };
                    ops.push(Op::Fill {
                        rect: Rect::new(tick.x - 0.5, tick.y, 1.0, tick.height),
                        color: palette.muted(),
                    });
                    let end = grind_print::place::line_bounds(&shown.set, anchor)
                        .and_then(|(_, end)| shown.stroke(end))
                        .map_or(tick.x, |stroke| stroke.x);
                    ops.push(Op::Run {
                        x: end + NAME_GAP,
                        top: tick.y,
                        text: format!("\u{2039}{name}\u{203a}"),
                        font: Font {
                            size: face::BODY_PT * 0.85,
                            bold: false,
                            italic: false,
                            family: Family::System,
                            tabular: false,
                        },
                        color: palette.muted(),
                        underline: false,
                        strike: false,
                        clip: Rect::new(end, tick.y, view.x + view.w - end, tick.height),
                    });
                }
            }
        }
    }
    let Some(stroke) = shown.stroke(caret) else {
        return ops;
    };
    if state.composing.is_some() {
        ops.push(Op::Fill {
            rect: Rect::new(stroke.x, stroke.y, CARET_W, stroke.height),
            color: palette.accent,
        });
        return ops;
    }
    if frame.caret && selection.is_none() {
        ops.push(Op::Fill {
            rect: Rect::new(stroke.x, stroke.y, CARET_W, stroke.height),
            color: palette.accent,
        });
    }
    ops
}

/// Where the caret is drawn, in the view's coordinates — for the input method's candidate window
/// and for scrolling it into view.
pub fn caret_rect(shown: &Shown, state: &Page) -> Option<Rect> {
    let stroke = shown.stroke(state.shown_caret())?;
    Some(Rect::new(stroke.x, stroke.y, CARET_W, stroke.height))
}

#[cfg(test)]
mod tests {
    use super::*;
    use grind_text::BlockKind;

    fn shown(app: &App) -> Shown {
        let session = grind_print::session::Session::new(true);
        Shown::new(session.document(app), 1.0, 800.0)
    }

    fn app() -> App {
        let app = App::new();
        let paragraph = "All work and no play makes a dull document. ".repeat(30);
        for at in 0..4 {
            app.insert(at, BlockKind::Paragraph, &paragraph).unwrap();
        }
        app
    }

    fn ops(app: &App, shown: &Shown, state: &Page, view: Rect) -> Vec<Op> {
        let raster = |_: usize| {
            Some(Raster {
                width: 2,
                height: 2,
                rgba: Rc::from(vec![255u8; 16]),
            })
        };
        frame(&Frame {
            app,
            shown,
            view,
            raster: &raster,
            state,
            caret: true,
            names: false,
            palette: &Palette::LIGHT,
            ignored: &[],
        })
    }

    #[test]
    fn the_pages_in_view_are_drawn_from_their_rasters_and_the_caret_over_them() {
        let app = app();
        let shown = shown(&app);
        let state = Page::default();
        let all = ops(&app, &shown, &state, Rect::new(0.0, 0.0, 800.0, 5000.0));
        let pixels = all
            .iter()
            .filter(|op| matches!(op, Op::Pixels { .. }))
            .count();
        assert_eq!(pixels, shown.set.pages.len());
        let caret = caret_rect(&shown, &state).unwrap();
        assert!(all.contains(&Op::Fill {
            rect: caret,
            color: Palette::LIGHT.accent,
        }));
        // Only the first page shows in a short view at the top.
        let top = ops(&app, &shown, &state, Rect::new(0.0, 0.0, 800.0, 200.0));
        assert_eq!(
            top.iter()
                .filter(|op| matches!(op, Op::Pixels { .. }))
                .count(),
            1
        );
    }

    #[test]
    fn a_selection_is_washed_and_hides_the_caret() {
        let app = app();
        let shown = shown(&app);
        let mut state = Page::default();
        state.place(
            Caret {
                block: 0,
                offset: 200,
            },
            true,
        );
        state.anchor = Caret {
            block: 0,
            offset: 0,
        };
        let drawn = ops(&app, &shown, &state, Rect::new(0.0, 0.0, 800.0, 1200.0));
        assert!(
            drawn
                .iter()
                .filter(|op| matches!(op, Op::Wash { .. }))
                .count()
                >= 2
        );
        let caret = caret_rect(&shown, &state).unwrap();
        assert!(
            !drawn
                .iter()
                .any(|op| matches!(op, Op::Fill { rect, .. } if *rect == caret))
        );
    }
}
