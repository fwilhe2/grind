// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Help while a formula is typed — the Mac's half (M8), portable.
//!
//! What to offer, how the list narrows, what accepting replaces and what the band says are
//! `grind_sheet::formula::assist`'s, shared with the Windows pane. What is this shell's is which
//! **selectors** steer the list, because a Mac's keys arrive as selectors from the cell editor's
//! field editor (decision 7): `insertTab:` takes the offer, `moveUp:` and `moveDown:` step
//! through the list, `cancelOperation:` closes it for this word — and **Return is deliberately
//! left to commit the cell**, as in every other shell, since a formula finished by pressing
//! Return is the common case and a completion in its place would store the wrong thing.

use std::ops::Range;

pub use grind_sheet::formula::assist::{Assist, Ink, Piece, band, friendly_line, function_insert};

/// What a selector does to a list of offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reply {
    Accept,
    /// Move the highlight by this much, wrapping.
    Step(i32),
    /// Close the list, keeping the edit.
    Dismiss,
}

/// What `selector` means **while a list of offers is up** — asked before the edit machine, and
/// `None` for everything the list has no answer for, which is almost every selector.
pub fn on_selector(offering: bool, selector: &str) -> Option<Reply> {
    if !offering {
        return None;
    }
    match selector {
        "insertTab:" => Some(Reply::Accept),
        "moveDown:" => Some(Reply::Step(1)),
        "moveUp:" => Some(Reply::Step(-1)),
        "cancelOperation:" => Some(Reply::Dismiss),
        _ => None,
    }
}

/// `text` with the byte range `span` replaced by `with`, and the byte offset the caret goes to —
/// just after what went in, so typing carries on inside the call.
pub fn replaced(text: &str, span: Range<usize>, with: &str) -> (String, usize) {
    let start = span.start.min(text.len());
    let end = span.end.clamp(start, text.len());
    let mut out = String::with_capacity(text.len() + with.len());
    out.push_str(&text[..start]);
    out.push_str(with);
    out.push_str(&text[end..]);
    (out, start + with.len())
}

/// What the formula read-out shows: while a cell is being edited, what the editor holds;
/// otherwise the active cell's own text, read through `friendly_line` when the friendly view is
/// on and the text is a formula this build can read. Nothing here parses back — R1 says the
/// document's formula is ODF's, and this is a reading of it.
pub fn read_out(text: &str, editing: bool, friendly: bool) -> String {
    match (editing, friendly) {
        (false, true) => friendly_line(text).unwrap_or_else(|| text.to_owned()),
        _ => text.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_claims_tab_and_the_arrows_and_leaves_return_alone() {
        assert_eq!(on_selector(true, "insertTab:"), Some(Reply::Accept));
        assert_eq!(on_selector(true, "moveDown:"), Some(Reply::Step(1)));
        assert_eq!(on_selector(true, "moveUp:"), Some(Reply::Step(-1)));
        assert_eq!(on_selector(true, "cancelOperation:"), Some(Reply::Dismiss));
        assert_eq!(on_selector(true, "insertNewline:"), None, "Return commits");
        assert_eq!(on_selector(true, "insertBacktab:"), None);
        for selector in ["insertTab:", "moveDown:", "moveUp:", "cancelOperation:"] {
            assert_eq!(on_selector(false, selector), None, "{selector}");
        }
    }

    /// `=SU` and Tab: M8's exit criterion, portably.
    #[test]
    fn typing_su_and_tab_takes_the_offer() {
        let mut assist = Assist::default();
        assist.refresh("=SU", 3, &[]);
        assist.chosen = assist
            .offers
            .iter()
            .position(|offer| offer.name == "SUM")
            .expect("SUM is offered");
        let reply = on_selector(assist.is_offering(), "insertTab:");
        assert_eq!(reply, Some(Reply::Accept));
        let (span, with) = assist.accept().expect("an offer");
        assert_eq!(replaced("=SU", span, &with), ("=SUM(".to_owned(), 5));
    }

    #[test]
    fn a_replacement_is_cut_to_the_text() {
        assert_eq!(replaced("=a+SU", 3..5, "SUM("), ("=a+SUM(".into(), 7));
        assert_eq!(replaced("ab", 5..9, "x"), ("abx".into(), 3));
    }

    #[test]
    fn the_read_out_is_friendly_only_at_rest() {
        let pv = "=PV(0.05;10;-100)";
        assert!(read_out(pv, false, true).starts_with("Present Value("));
        assert_eq!(
            read_out(pv, true, true),
            pv,
            "what is being typed is what shows"
        );
        assert_eq!(read_out(pv, false, false), pv);
        assert_eq!(
            read_out("=SUM(", false, true),
            "=SUM(",
            "unreadable: its own text"
        );
        assert_eq!(read_out("12", false, true), "12");
    }
}
