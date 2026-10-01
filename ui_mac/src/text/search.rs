// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Where Find goes next on a page — `App::find_ignoring_case`'s hits, in document order, and the
//! one a step from the caret lands on, wrapping at either end. Portable, and tested here.
//!
//! `sheet/search.rs` is the grid's twin, over `grind_sheet::find::step`; that step is typed to
//! cells, so the page's is its own few lines with the same three rules: **Next** is the first
//! hit after the caret, **Previous** the last one before it, and **Here** the first at or after
//! it — so a search typed with the caret on a hit stays on that hit.

use grind_sheet::find::Towards;
use grind_text::{App, Caret};

/// A hit to select: where it starts and ends, and which of how many.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Found {
    pub from: Caret,
    pub to: Caret,
    pub index: usize,
    pub count: usize,
}

/// The hit a step from `at` lands on, or `None` when nothing holds `needle`. Case folded, as
/// every find bar in the suite folds it.
pub fn next(app: &App, needle: &str, at: Caret, towards: Towards) -> Option<Found> {
    let len = needle.chars().count();
    let hits: Vec<Caret> = app
        .find_ignoring_case(needle)
        .iter()
        .map(|hit| Caret {
            block: hit.index,
            offset: hit.offset,
        })
        .collect();
    if hits.is_empty() {
        return None;
    }
    let index = match towards {
        Towards::Next => hits.iter().position(|hit| *hit > at).unwrap_or(0),
        Towards::Previous => hits
            .iter()
            .rposition(|hit| *hit < at)
            .unwrap_or(hits.len() - 1),
        Towards::Here => hits.iter().position(|hit| *hit >= at).unwrap_or(0),
    };
    let from = hits[index];
    Some(Found {
        from,
        to: Caret {
            offset: from.offset + len,
            ..from
        },
        index,
        count: hits.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use grind_text::BlockKind;

    fn app() -> App {
        let app = App::new();
        app.set_text(0, "Rent is due").unwrap();
        app.insert(1, BlockKind::Paragraph, "no rent, then rent")
            .unwrap();
        app
    }

    fn at(block: usize, offset: usize) -> Caret {
        Caret { block, offset }
    }

    #[test]
    fn next_and_previous_go_round_the_document() {
        let app = app();
        let first = next(&app, "rent", at(0, 0), Towards::Here).unwrap();
        assert_eq!((first.from, first.to, first.count), (at(0, 0), at(0, 4), 3));
        let second = next(&app, "rent", first.from, Towards::Next).unwrap();
        assert_eq!((second.from, second.index), (at(1, 3), 1));
        let third = next(&app, "RENT", second.from, Towards::Next).unwrap();
        assert_eq!(third.from, at(1, 14), "case folded");
        assert_eq!(
            next(&app, "rent", third.from, Towards::Next).unwrap().from,
            at(0, 0),
            "wraps forwards"
        );
        assert_eq!(
            next(&app, "rent", at(0, 0), Towards::Previous)
                .unwrap()
                .from,
            at(1, 14),
            "and back"
        );
    }

    #[test]
    fn nothing_found_is_nothing() {
        assert_eq!(next(&app(), "lease", at(0, 0), Towards::Next), None);
    }
}
