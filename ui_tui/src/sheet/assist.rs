// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Help while a formula is being typed — what may be completed, and what the call under the caret
//! wants next.
//!
//! **None of the thinking is here.** `grind_sheet::formula::assist` holds `prefix_at`,
//! `candidates`, `call_at` and `signature_parts` — hoisted into the core the day a second shell
//! wanted them (`CLAUDE.md`, W9), so a completion this shell offers is a function the evaluator
//! really has and a signature it shows is the one the GNOME window and the Windows one show. What
//! is left here is presentation state: which offer is highlighted, whether Escape has dismissed
//! the list for this word, and how the whole thing reads as one line of a terminal.
//!
//! **A band, not a popup**, for `ui_win32`'s reason expressed in a different medium: a terminal
//! has no second window to put a list in, and a shell that drew one over the grid would have to
//! own a z-order, a dismissal rule and a redraw region for a list that lives for three keystrokes.
//! One line under the formula line costs a row and reads left to right, which is what the rest of
//! this shell already does with a status line.
//!
//! **Enter is deliberately not claimed.** It commits the cell, as it always does: a formula
//! finished by pressing Enter is the common case, and an accidental completion in its place would
//! store the wrong thing. Tab accepts, Up/Down step, Escape dismisses this word's list and leaves
//! the edit open.

use std::ops::Range;

use grind_sheet::formula::assist as core_assist;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

pub use grind_sheet::formula::assist::Candidate;

/// How many offers fit on one line before the rest become a count.
///
/// Five, and the sixth is `+n` — a band is a line, and a line that ran off the window would hide
/// exactly the offer somebody is looking for. One more keystroke narrows the list.
pub const MAX_OFFERS: usize = 5;

/// What the band is showing, and what a key would do to it.
///
/// Recomputed from the edit buffer on every keystroke rather than updated in place: a remembered
/// completion goes stale the moment the caret moves, and this shell's whole rendering contract is
/// "asked for fresh, never stored".
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Assist {
    /// Everything worth offering for the word being typed. Empty when there is no such word, when
    /// nothing matches it, or when Escape dismissed this word's offers.
    pub offers: Vec<Candidate>,
    /// What accepting an offer replaces — **byte** offsets into the edit buffer.
    pub span: Range<usize>,
    /// Which offer is highlighted, and the one Tab accepts.
    pub chosen: usize,
    /// The call the caret is inside and which argument of it. Answered even while offers are
    /// showing, since the two are about different halves of the same line: `=SUM(AV|` is both a
    /// word to complete and `SUM`'s first argument.
    pub call: Option<(String, usize)>,
    /// The span whose offers Escape dismissed, so they do not come back on the next keystroke
    /// inside the same word. Typing past its end, or moving elsewhere, starts a new word and
    /// offers again — which is what makes Escape "not this one" rather than a mode.
    dismissed: Option<Range<usize>>,
}

impl Assist {
    /// Read the edit buffer at `caret` (a byte offset) and say what to show.
    ///
    /// `names` is the document's defined names, which the caller reads from `App::names` — nothing
    /// here touches a document.
    pub fn refresh(&mut self, text: &str, caret: usize, names: &[String]) {
        self.call = core_assist::call_at(text, caret);
        self.chosen = 0;
        self.offers.clear();
        self.span = 0..0;
        let Some(span) = core_assist::prefix_at(text, caret) else {
            // Nothing to complete here, so whatever Escape dismissed is finished business.
            self.dismissed = None;
            return;
        };
        self.span = span.clone();
        if self.dismissed.as_ref() == Some(&span) {
            return;
        }
        self.dismissed = None;
        let offers = core_assist::candidates(&text[span.clone()], names);
        // A single offer already typed out in full is not an offer: `=SUM|` has nothing left to
        // complete, and a list showing what is already there is a row of noise over the grid.
        if offers.len() == 1 && offers[0].name.eq_ignore_ascii_case(&text[span]) {
            return;
        }
        self.offers = offers;
    }

    /// Whether there is a list to steer, which is what makes Tab and the arrows mean something
    /// other than what they usually mean while editing.
    pub fn is_offering(&self) -> bool {
        !self.offers.is_empty()
    }

    /// Up and down the list, wrapping — a list this short is quicker to wrap than to stop at.
    pub fn step(&mut self, delta: i32) {
        let count = self.offers.len() as i32;
        if count == 0 {
            return;
        }
        self.chosen = (self.chosen as i32 + delta).rem_euclid(count) as usize;
    }

    /// What accepting the highlighted offer replaces, and with what.
    pub fn accept(&mut self) -> Option<(Range<usize>, String)> {
        let offer = self.offers.get(self.chosen)?;
        let replacement = offer.insert.clone();
        let span = self.span.clone();
        self.offers.clear();
        Some((span, replacement))
    }

    /// Escape with a list up: close it, keep the edit, and do not offer for this word again.
    pub fn dismiss(&mut self) {
        self.dismissed = Some(self.span.clone());
        self.offers.clear();
    }

    /// Forget everything — an edit that ended, by any door.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Whether the band has anything to say at all.
    pub fn is_showing(&self) -> bool {
        self.is_offering() || self.call.is_some()
    }

    /// The band's whole content: the offers if there are any, otherwise the signature of the call
    /// the caret is in, with the argument being typed picked out.
    ///
    /// `friendly` picks which spelling of a signature is used — `Sum(Number: …)` or `SUM(…)`. This
    /// shell always asks for the second, because its formula line shows a formula in ODF's display
    /// syntax and a band that named the same function differently would be two names for one
    /// thing on adjacent rows. The parameter is here because `signature_parts` has it and the two
    /// windows that *do* offer the friendly reading pass the other value.
    pub fn line(&self, friendly: bool) -> Vec<Span<'static>> {
        let muted = Style::default().add_modifier(Modifier::DIM);
        let strong = Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD);
        if self.is_offering() {
            let mut spans = vec![Span::styled(" \u{25b8} ", muted)];
            for (index, offer) in self.offers.iter().take(MAX_OFFERS).enumerate() {
                if index > 0 {
                    spans.push(Span::styled("  ", muted));
                }
                spans.push(Span::styled(
                    offer.name.clone(),
                    match index == self.chosen {
                        true => strong,
                        false => muted,
                    },
                ));
            }
            if self.offers.len() > MAX_OFFERS {
                spans.push(Span::styled(
                    format!("  +{}", self.offers.len() - MAX_OFFERS),
                    muted,
                ));
            }
            // The key first and the summary after it, because the summary is what runs off the
            // right-hand edge of a narrow window and a hint nobody can see is not a hint. The
            // chosen offer's own summary is the line a reader actually wants; the price of a
            // band is that the other four are names alone.
            spans.push(Span::styled("   Tab accepts", muted));
            if let Some(offer) = self.offers.get(self.chosen)
                && !offer.detail.is_empty()
            {
                spans.push(Span::styled(format!("  \u{00b7}  {}", offer.detail), muted));
            }
            return spans;
        }
        let Some((name, argument)) = &self.call else {
            return Vec::new();
        };
        let Some((head, parts)) = core_assist::signature_parts(name, friendly) else {
            return Vec::new();
        };
        let mut spans = vec![
            Span::styled(" \u{25b8} ", muted),
            Span::styled(head, muted),
            Span::styled("(", muted),
        ];
        for (index, part) in parts.iter().enumerate() {
            if index > 0 {
                spans.push(Span::styled("; ", muted));
            }
            spans.push(Span::styled(
                part.clone(),
                match index == *argument {
                    true => strong,
                    false => muted,
                },
            ));
        }
        spans.push(Span::styled(")", muted));
        spans
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_of(spans: &[Span<'static>]) -> String {
        spans.iter().map(|s| s.content.as_ref()).collect()
    }

    /// The offers are the evaluator's own functions, so a shell cannot offer one that does not
    /// exist — and a defined name is offered beside them.
    #[test]
    fn a_half_typed_name_is_offered_from_the_catalogue_and_the_document() {
        let mut assist = Assist::default();
        assist.refresh("=SU", 3, &["subtotal".to_owned()]);
        assert!(assist.is_offering());
        let names: Vec<&str> = assist.offers.iter().map(|o| o.name.as_str()).collect();
        assert!(names.contains(&"SUM"), "{names:?}");
        assert!(names.contains(&"subtotal"), "the document's own: {names:?}");
        // Accepting inserts the call with its opening bracket, so the caret lands where the
        // first argument goes.
        assert!(
            assist.offers[assist.chosen].insert.ends_with('('),
            "{:?}",
            assist.offers[assist.chosen]
        );
    }

    #[test]
    fn there_is_nothing_to_offer_outside_a_formula() {
        let mut assist = Assist::default();
        assist.refresh("SU", 2, &[]);
        assert!(!assist.is_offering(), "plain text is not a formula");
        assist.refresh("", 0, &[]);
        assert!(!assist.is_offering());
    }

    /// Tab replaces the word that was being typed, and only that word.
    #[test]
    fn accepting_replaces_the_word_and_leaves_the_caret_where_the_argument_goes() {
        let mut assist = Assist::default();
        assist.refresh("=1+SU", 5, &[]);
        let sum = assist
            .offers
            .iter()
            .position(|offer| offer.name == "SUM")
            .expect("SUM is offered for SU");
        assist.step(sum as i32);
        let (span, insert) = assist.accept().expect("an offer");
        assert_eq!(span, 3..5, "the word, and only the word");
        assert_eq!(insert, "SUM(");
        assert!(!assist.is_offering(), "accepting closes the list");
    }

    /// Escape is "not this one", not a mode: the same word stays quiet, the next one offers again.
    #[test]
    fn escape_dismisses_this_word_only() {
        let mut assist = Assist::default();
        assist.refresh("=SU", 3, &[]);
        assist.dismiss();
        assist.refresh("=SU", 3, &[]);
        assert!(!assist.is_offering(), "still the same word");
        assist.refresh("=SUM(1;AV", 9, &[]);
        assert!(assist.is_offering(), "a new word offers again");
    }

    #[test]
    fn stepping_wraps_in_both_directions() {
        let mut assist = Assist::default();
        assist.refresh("=SU", 3, &[]);
        let count = assist.offers.len();
        assert!(count > 1, "the fixture needs more than one offer");
        assist.step(-1);
        assert_eq!(assist.chosen, count - 1);
        assist.step(1);
        assert_eq!(assist.chosen, 0);
    }

    /// With no word to complete, the band shows the signature of the call the caret is in, with
    /// the argument being typed picked out.
    #[test]
    fn the_band_shows_the_signature_of_the_call_the_caret_is_in() {
        let mut assist = Assist::default();
        assist.refresh("=SUM(A1;", 8, &[]);
        assert!(!assist.is_offering(), "nothing half-typed after the `;`");
        assert_eq!(
            assist.call.as_ref().map(|(name, at)| (name.as_str(), *at)),
            Some(("SUM", 1)),
            "inside SUM's second argument"
        );
        let line = text_of(&assist.line(false));
        assert!(line.contains("SUM"), "{line}");
        assert!(assist.is_showing());
    }

    /// A long list says how much of itself it is not showing rather than running off the window.
    #[test]
    fn a_long_list_is_cut_to_five_and_says_how_many_more() {
        let mut assist = Assist::default();
        assist.refresh("=A", 2, &[]);
        assert!(
            assist.offers.len() > MAX_OFFERS,
            "the catalogue has more than five functions starting with A"
        );
        let line = text_of(&assist.line(false));
        assert!(
            line.contains(&format!("+{}", assist.offers.len() - MAX_OFFERS)),
            "{line}"
        );
        assert!(line.contains("Tab accepts"));
    }
}
