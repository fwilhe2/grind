// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Help while a formula is typed: what may be completed, and what the call under the caret wants
//! next. What to offer, how the list narrows and what the band says are
//! `grind_sheet::formula::assist`'s; here is which **keys** steer the list. `mod.rs` reads the
//! formula `<input>`'s text and caret, calls [`Assist::refresh`], and draws [`band`] in the
//! `#assist` line under the formula bar.
//!
//! **A band, not a popover**: a popover would be a second focusable element with its own layer and
//! escape route, where the keyboard must never leave the `<input>` while the list narrows. The
//! cost is a row of names rather than a column with summaries, so the chosen offer's summary
//! follows the names.

pub use grind_sheet::formula::assist::{Assist, Ink, Piece};

/// What accepting the highlighted offer, stepping the list or dismissing it means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reply {
    /// Put the highlighted offer into the formula bar.
    Accept,
    /// Move the highlight by this much, wrapping.
    Step(i32),
    /// Close the list, keeping the edit.
    Dismiss,
}

/// What a keystroke means **while offers are up** — asked before [`super::keymap::action_for`],
/// and `None` for nearly every key. It claims Tab (else commit and move right), Up/Down and Escape
/// (else the whole edit is thrown away). **Enter is not claimed**: it commits, the common way to
/// finish a formula, where an accidental completion would store the wrong thing.
pub fn on_key(offering: bool, key: &str, ctrl: bool, alt: bool, shift: bool) -> Option<Reply> {
    if !offering || ctrl || alt {
        return None;
    }
    match key {
        "Tab" if !shift => Some(Reply::Accept),
        "ArrowDown" => Some(Reply::Step(1)),
        "ArrowUp" => Some(Reply::Step(-1)),
        "Escape" => Some(Reply::Dismiss),
        _ => None,
    }
}

/// The band's content as runs — offers if any, else the signature of the call the caret is in.
/// Always the friendly spelling (`Sum(Number…)`): this shell has no friendly-formulas toggle, and
/// the formula bar beside it is in A1, not ODF's spec syntax. Empty when there is nothing to say,
/// the cue to hide `#assist`.
pub fn band(assist: &Assist) -> Vec<Piece> {
    grind_sheet::formula::assist::band(assist, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(text: &str, caret: usize) -> Assist {
        let names = [
            "subtotal_2026".to_owned(),
            "tax_rate".to_owned(),
            "Data".to_owned(),
        ];
        let mut assist = Assist::default();
        assist.refresh(text, caret, &names);
        assist
    }

    #[test]
    fn a_finished_call_shows_a_signature_hint_instead_of_offers() {
        let assist = read("=SUM(AV", 7);
        assert!(assist.is_offering());
        assert_eq!(assist.call, Some(("SUM".to_owned(), 0)));
        let pieces = band(&assist);
        assert!(pieces.iter().any(|p| p.text == "AVERAGE"));
    }

    #[test]
    fn a_signature_hint_bolds_the_argument_the_caret_is_in() {
        let assist = read("=SUM(", 5);
        assert!(!assist.is_offering());
        let pieces = band(&assist);
        let text: String = pieces.iter().map(|p| p.text.as_str()).collect();
        // The friendly spelling, like the summary that already labels a finished formula.
        assert!(text.starts_with("Sum("), "{text}");
        assert!(
            pieces
                .iter()
                .any(|p| p.text == "Number…" && p.ink == Ink::Strong)
        );
    }

    #[test]
    fn on_key_claims_only_tab_the_arrows_and_escape_while_offering() {
        assert_eq!(
            on_key(true, "Tab", false, false, false),
            Some(Reply::Accept)
        );
        assert_eq!(on_key(true, "Tab", false, false, true), None);
        assert_eq!(
            on_key(true, "ArrowDown", false, false, false),
            Some(Reply::Step(1))
        );
        assert_eq!(
            on_key(true, "ArrowUp", false, false, false),
            Some(Reply::Step(-1))
        );
        assert_eq!(
            on_key(true, "Escape", false, false, false),
            Some(Reply::Dismiss)
        );
        // Enter is left for the ordinary keymap to commit.
        assert_eq!(on_key(true, "Enter", false, false, false), None);
        // Nothing is claimed with no list up, or with a modifier held.
        assert_eq!(on_key(false, "Tab", false, false, false), None);
        assert_eq!(on_key(true, "ArrowDown", true, false, false), None);
    }
}
