// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Where a modal goes and what is inside it — Fluent's *ContentDialog*, measured, and **fitted to
//! the screen it opens on**.
//!
//! Every modal `dialog.rs` opens is one shape (`doc/windows-shell.md` decision 12): a surface with
//! no caption of its own, a title set in the type ramp's *Subtitle*, a body, and a **footer** of a
//! quieter ground holding the buttons. That is ContentDialog's anatomy, and its numbers are here
//! rather than in the window code so they can be tested without a window.
//!
//! The half that was broken before this file existed is [`place`]. Each popup used to be asked
//! for at a fixed *outer* size in logical pixels — the print preview at 640 × 860, which is 1075
//! pixels tall at 125% and therefore taller than a 1080p screen's work area — centred on its owner
//! with nothing stopping it at the edge of the monitor, and laid out by guessing how tall the
//! system caption would be. So the rule is now one function: a modal is **as large as its content
//! wants, and never larger than the work area of the monitor its owner is on**; what does not fit
//! is the body's to absorb, by scrolling (a list, a reader) or by scaling (a picture, a page).

use crate::sheet::geom::{Rect, scale};

/// ContentDialog's measurements at 100%, in Fluent's own words where it has them.
pub mod size {
    /// `ContentDialogPadding` — round the content, and round the buttons inside the footer.
    pub const PAD: f64 = 24.0;
    /// The title's font size: the type ramp's *Subtitle*.
    pub const TITLE: f64 = 20.0;
    /// And its line, which is what the body is placed under.
    pub const TITLE_LINE: f64 = 28.0;
    /// Between the title and the content.
    pub const TITLE_GAP: f64 = 12.0;
    /// A button, and a text field: Fluent's standard control height.
    pub const CONTROL_H: f64 = 32.0;
    /// The footer: a control with [`PAD`] above and below it.
    pub const FOOTER: f64 = PAD + CONTROL_H + PAD;
    /// Between two buttons in the footer.
    pub const BUTTON_GAP: f64 = 8.0;
    /// The text inset of a button, each side.
    pub const BUTTON_PAD: f64 = 12.0;
    /// The narrowest a trailing button is drawn, so that *OK* is not a 40-pixel target.
    pub const BUTTON_MIN: f64 = 96.0;
    /// Between a field's header and the field.
    pub const LABEL_GAP: f64 = 8.0;
    /// One row of a list.
    pub const ROW_H: f64 = 32.0;
    /// `ContentDialogMinWidth` and `ContentDialogMaxWidth`. The maximum is a message's: a list or
    /// a picture asks for more, and gets it up to the screen's own edge.
    pub const MIN_W: f64 = 320.0;
    pub const MAX_W: f64 = 548.0;
    /// The least distance a modal keeps from the edge of the work area.
    pub const MARGIN: f64 = 16.0;
}

/// A rectangle in screen pixels, as edges — what `GetWindowRect` and `MONITORINFO::rcWork` give.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Edges {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Edges {
    pub fn width(&self) -> i32 {
        self.right - self.left
    }

    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }

    fn inset(&self, by: i32) -> Self {
        Self {
            left: self.left + by,
            top: self.top + by,
            right: self.right - by,
            bottom: self.bottom - by,
        }
    }
}

/// Where a modal of `want` pixels goes: centred on `owner`, **shrunk to fit** the monitor's `work`
/// area less [`size::MARGIN`], and slid along whichever axis would otherwise leave it hanging off
/// the edge.
///
/// The size is clamped before the position is decided, because a window that is too big cannot be
/// placed anywhere that fits, and the owner's centre is only a preference: an owner pushed half off
/// the screen still gets a modal that is wholly on it. A work area too small for even the margin
/// (a degenerate case, but `MONITORINFO` has been known to report one mid-change) gives the margin
/// up rather than the fit.
pub fn place(want: (i32, i32), owner: Edges, work: Edges, dpi: u32) -> Edges {
    let margin = scale(size::MARGIN, dpi).round() as i32;
    let area = match work.inset(margin) {
        inner if inner.width() > 0 && inner.height() > 0 => inner,
        _ => work,
    };
    let w = want.0.clamp(1, area.width().max(1));
    let h = want.1.clamp(1, area.height().max(1));
    let centre = (
        owner.left + owner.width() / 2,
        owner.top + owner.height() / 2,
    );
    let left = (centre.0 - w / 2).clamp(area.left, (area.right - w).max(area.left));
    let top = (centre.1 - h / 2).clamp(area.top, (area.bottom - h).max(area.top));
    Edges {
        left,
        top,
        right: left + w,
        bottom: top + h,
    }
}

/// The three regions of a modal's client area.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    /// Where the title is drawn; `None` for a modal that has none.
    pub title: Option<Rect>,
    /// Everything between the title and the footer, inside the padding.
    pub body: Rect,
    /// The footer band, edge to edge — its ground is drawn across the whole width.
    pub footer: Rect,
}

/// How a client area of `w` × `h` pixels divides.
pub fn frame(w: f64, h: f64, dpi: u32, titled: bool) -> Frame {
    let s = |v: f64| scale(v, dpi);
    let pad = s(size::PAD);
    let footer_h = s(size::FOOTER).min(h);
    let footer = Rect {
        x: 0.0,
        y: (h - footer_h).max(0.0),
        w,
        h: footer_h,
    };
    let title = titled.then(|| Rect {
        x: pad,
        y: pad,
        w: (w - pad * 2.0).max(0.0),
        h: s(size::TITLE_LINE),
    });
    let top = match title {
        Some(title) => title.y + title.h + s(size::TITLE_GAP),
        None => pad,
    };
    let body = Rect {
        x: pad,
        y: top,
        w: (w - pad * 2.0).max(0.0),
        h: (footer.y - pad - top).max(0.0),
    };
    Frame {
        title,
        body,
        footer,
    }
}

/// The client size a body of `body` pixels asks for — the inverse of [`frame`], so a caller can say
/// how big its *content* is and get back how big the window has to be.
pub fn wanted(body: (f64, f64), dpi: u32, titled: bool) -> (i32, i32) {
    let s = |v: f64| scale(v, dpi);
    let pad = s(size::PAD);
    let title = match titled {
        true => s(size::TITLE_LINE) + s(size::TITLE_GAP),
        false => 0.0,
    };
    let w = (body.0 + pad * 2.0).max(s(size::MIN_W));
    let h = pad + title + body.1 + pad + s(size::FOOTER);
    (w.ceil() as i32, h.ceil() as i32)
}

/// Which end of the footer a button stands at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum End {
    /// The answers — OK, Cancel, Save — at the trailing end, where Windows puts them.
    Trailing,
    /// Something that is not an answer: *Previous*/*Next* in a preview, a chart's kinds, a filter's
    /// *Clear*. At the leading end, so it is never mistaken for one.
    Leading,
}

/// Where each button goes in `footer`, given its text's width in pixels and which end it is at.
///
/// Two rules, chosen by what is in the footer:
///
/// - **Only answers**: ContentDialog's own, the buttons share the footer in equal columns — and at
///   least two columns, so that a lone *Close* takes the trailing half rather than the whole width.
/// - **Anything at the leading end too**: each button as wide as its text (an answer no narrower
///   than [`size::BUTTON_MIN`]), the leading ones from the left, the answers from the right.
///
/// When the second rule would not fit, the first is used for every button, which is what a footer
/// on a very narrow screen needs: every button still on it and still a target.
pub fn buttons(footer: Rect, dpi: u32, specs: &[(f64, End)]) -> Vec<Rect> {
    let s = |v: f64| scale(v, dpi);
    let pad = s(size::PAD);
    let gap = s(size::BUTTON_GAP);
    let h = s(size::CONTROL_H).min(footer.h);
    let y = footer.y + ((footer.h - h) / 2.0).max(0.0);
    let inner = (footer.w - pad * 2.0).max(0.0);
    let columns = |count: usize, from: usize| -> Vec<Rect> {
        let n = count.max(from) as f64;
        let w = ((inner - gap * (n - 1.0)) / n).max(0.0);
        let skip = n as usize - count;
        (0..count)
            .map(|i| Rect {
                x: footer.x + pad + (w + gap) * (i + skip) as f64,
                y,
                w,
                h,
            })
            .collect()
    };
    if specs.iter().all(|(_, end)| *end == End::Trailing) {
        return columns(specs.len(), 2);
    }
    let widths: Vec<f64> = specs
        .iter()
        .map(|(text, end)| {
            let natural = text + s(size::BUTTON_PAD) * 2.0;
            match end {
                End::Trailing => natural.max(s(size::BUTTON_MIN)),
                End::Leading => natural.max(h),
            }
        })
        .collect();
    let total: f64 = widths.iter().sum::<f64>() + gap * (specs.len().saturating_sub(1)) as f64;
    // A gap of at least one more between the two ends, or the ends meet.
    if total + gap > inner {
        return columns(specs.len(), 1);
    }
    let mut out = vec![
        Rect {
            x: 0.0,
            y,
            w: 0.0,
            h
        };
        specs.len()
    ];
    let mut x = footer.x + pad;
    for (i, (_, end)) in specs.iter().enumerate() {
        if *end == End::Leading {
            out[i] = Rect {
                x,
                y,
                w: widths[i],
                h,
            };
            x += widths[i] + gap;
        }
    }
    let mut x = footer.x + footer.w - pad;
    for (i, (_, end)) in specs.iter().enumerate().rev() {
        if *end == End::Trailing {
            x -= widths[i];
            out[i] = Rect {
                x,
                y,
                w: widths[i],
                h,
            };
            x -= gap;
        }
    }
    out
}

/// The x where text between the two ends of a footer may start and end — the print preview's
/// *Page 2 of 5* goes here.
pub fn between(footer: Rect, placed: &[Rect], specs: &[(f64, End)], dpi: u32) -> (f64, f64) {
    let gap = scale(size::PAD, dpi) / 2.0;
    let left = placed
        .iter()
        .zip(specs)
        .filter(|(_, (_, end))| *end == End::Leading)
        .map(|(rect, _)| rect.x + rect.w + gap)
        .fold(footer.x + scale(size::PAD, dpi), f64::max);
    let right = placed
        .iter()
        .zip(specs)
        .filter(|(_, (_, end))| *end == End::Trailing)
        .map(|(rect, _)| rect.x - gap)
        .fold(footer.x + footer.w - scale(size::PAD, dpi), f64::min);
    (left, right.max(left))
}

/// How many rows a list modal shows before it scrolls: enough to be worth opening and few enough
/// that it is still a dialog. The screen can make it fewer — [`place`] decides that.
pub const LIST_ROWS: usize = 12;

/// The body a list of `rows` rows, the widest `widest` pixels across, asks for.
///
/// `chrome` is everything a row has beside its text — the selection pill, the padding, a checkbox,
/// a scroll bar — which the caller knows and this function should not.
pub fn list_body(rows: usize, widest: f64, chrome: f64, dpi: u32) -> (f64, f64) {
    let s = |v: f64| scale(v, dpi);
    let shown = rows.clamp(3, LIST_ROWS) as f64;
    // Wider than a message may be, since a function list is four columns and a source line is as
    // long as the cell it projects — but not so wide that a short list is a letterbox.
    let w = (widest + chrome).clamp(s(size::MIN_W - size::PAD * 2.0), s(880.0));
    // Two pixels for the list's own frame.
    (w, s(size::ROW_H) * shown + s(2.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORK: Edges = Edges {
        left: 0,
        top: 0,
        right: 1920,
        bottom: 1032,
    };

    fn within(inner: Edges, outer: Edges) -> bool {
        inner.left >= outer.left
            && inner.top >= outer.top
            && inner.right <= outer.right
            && inner.bottom <= outer.bottom
    }

    /// The bug this file exists for: the print preview asked for 640 × 860 *logical* pixels, which
    /// at 125% is taller than a 1080p screen's work area, and it opened with its buttons below the
    /// taskbar. Every size at every scaling now lands inside the work area.
    #[test]
    fn a_modal_never_leaves_the_work_area() {
        for dpi in [96, 120, 144, 168, 192, 288] {
            for want in [(300, 200), (800, 1075), (4000, 4000), (1, 1)] {
                for owner in [
                    WORK,
                    Edges {
                        left: -900,
                        top: -400,
                        right: 100,
                        bottom: 300,
                    },
                    Edges {
                        left: 1800,
                        top: 900,
                        right: 2600,
                        bottom: 1600,
                    },
                ] {
                    let placed = place(want, owner, WORK, dpi);
                    assert!(
                        within(placed, WORK),
                        "{want:?} at {dpi} over {owner:?}: {placed:?}"
                    );
                    assert!(placed.width() > 0 && placed.height() > 0);
                }
            }
        }
    }

    #[test]
    fn a_modal_that_fits_is_the_size_it_asked_for_and_centred_on_its_owner() {
        let owner = Edges {
            left: 100,
            top: 100,
            right: 1100,
            bottom: 800,
        };
        let placed = place((400, 300), owner, WORK, 96);
        assert_eq!((placed.width(), placed.height()), (400, 300));
        assert_eq!(placed.left + 200, 600);
        assert_eq!(placed.top + 150, 450);
    }

    #[test]
    fn a_work_area_that_does_not_start_at_zero_is_respected() {
        // A second monitor to the left, with the taskbar at the top.
        let work = Edges {
            left: -1280,
            top: 48,
            right: 0,
            bottom: 1024,
        };
        let placed = place((2000, 2000), work, work, 96);
        assert!(within(placed, work));
        assert_eq!(placed.left, -1280 + 16);
        assert_eq!(placed.top, 48 + 16);
    }

    #[test]
    fn the_frame_is_the_inverse_of_wanted() {
        for dpi in [96, 144] {
            for titled in [true, false] {
                let (w, h) = wanted((600.0, 120.0), dpi, titled);
                let frame = frame(f64::from(w), f64::from(h), dpi, titled);
                assert!((frame.body.w - 600.0).abs() < 1.0, "{frame:?}");
                assert!((frame.body.h - 120.0).abs() < 1.0, "{frame:?}");
                assert_eq!(frame.title.is_some(), titled);
                assert!((frame.footer.y + frame.footer.h - f64::from(h)).abs() < 1e-9);
            }
        }
    }

    #[test]
    fn a_frame_too_small_for_its_padding_has_an_empty_body_rather_than_a_negative_one() {
        let frame = frame(30.0, 50.0, 192, true);
        assert!(frame.body.w >= 0.0 && frame.body.h >= 0.0);
        assert!(frame.footer.h <= 50.0);
    }

    #[test]
    fn answers_alone_share_the_footer_and_a_lone_one_takes_the_trailing_half() {
        let footer = Rect {
            x: 0.0,
            y: 200.0,
            w: 448.0,
            h: 80.0,
        };
        let one = buttons(footer, 96, &[(30.0, End::Trailing)]);
        assert_eq!(one.len(), 1);
        assert!((one[0].x + one[0].w - (448.0 - 24.0)).abs() < 1e-9);
        assert!((one[0].w - (400.0 - 8.0) / 2.0).abs() < 1e-9);
        let three = buttons(footer, 96, &[(30.0, End::Trailing); 3]);
        assert!((three[0].x - 24.0).abs() < 1e-9);
        assert!((three[2].x + three[2].w - 424.0).abs() < 1e-9);
        assert!((three[0].w - three[2].w).abs() < 1e-9);
        for b in &three {
            assert!((b.y - 224.0).abs() < 1e-9 && (b.h - 32.0).abs() < 1e-9);
        }
    }

    #[test]
    fn leading_buttons_go_left_and_answers_right_and_none_overlap() {
        let footer = Rect {
            x: 0.0,
            y: 0.0,
            w: 800.0,
            h: 80.0,
        };
        let specs = [
            (50.0, End::Leading),
            (30.0, End::Leading),
            (40.0, End::Trailing),
            (45.0, End::Trailing),
        ];
        let placed = buttons(footer, 96, &specs);
        assert!((placed[0].x - 24.0).abs() < 1e-9);
        assert!(placed[1].x > placed[0].x + placed[0].w);
        assert!((placed[3].x + placed[3].w - 776.0).abs() < 1e-9);
        assert!(placed[2].x + placed[2].w < placed[3].x);
        assert!(
            placed[2].w >= 96.0,
            "an answer is never narrower than the minimum"
        );
        let (from, to) = between(footer, &placed, &specs, 96);
        assert!(from > placed[1].x + placed[1].w && to < placed[2].x && from < to);
    }

    #[test]
    fn a_footer_too_narrow_for_both_ends_falls_back_to_columns() {
        let footer = Rect {
            x: 0.0,
            y: 0.0,
            w: 300.0,
            h: 80.0,
        };
        let specs = [
            (90.0, End::Leading),
            (90.0, End::Trailing),
            (90.0, End::Trailing),
        ];
        let placed = buttons(footer, 96, &specs);
        for pair in placed.windows(2) {
            assert!(pair[0].x + pair[0].w <= pair[1].x + 1e-9);
        }
        assert!(
            placed
                .iter()
                .all(|b| b.x >= 24.0 - 1e-9 && b.x + b.w <= 276.0 + 1e-9)
        );
    }

    #[test]
    fn a_list_shows_between_three_and_a_dozen_rows() {
        let (_, short) = list_body(1, 100.0, 40.0, 96);
        let (_, long) = list_body(400, 100.0, 40.0, 96);
        assert!((short - (32.0 * 3.0 + 2.0)).abs() < 1e-9);
        assert!((long - (32.0 * LIST_ROWS as f64 + 2.0)).abs() < 1e-9);
        let (wide, _) = list_body(5, 5000.0, 40.0, 96);
        assert!((wide - 880.0).abs() < 1e-9);
    }
}
