// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Help while a formula is being typed: what may be completed, and what the call under the
//! caret wants next — `doc/sheet-shell.md`'s M7 for the GTK window, `doc/windows-shell.md`'s
//! W9 for Win32, and the largest gap `doc/web-shell.md` names for this one.
//!
//! **Portable, and tested on the host** like [`super::keymap`] and [`super::layout`] —
//! nothing here touches the DOM. `mod.rs` reads the formula `<input>`'s text and caret, calls
//! [`Assist::refresh`], and draws [`band`] as a strip of `<span>`s in the `#assist` line under
//! the formula bar; [`on_key`] says which of Tab, the arrows and Escape the list claims before
//! the ordinary keymap ever sees them.
//!
//! ## Why a band and not a popover
//!
//! `ui_sheet_gtk` floats a `gtk::Popover` under the cell being edited. This shell draws a
//! line instead — the same call `ui_win32` makes, for a browser's own version of its reason: a
//! popover would be a second focusable element with its own layer and its own escape route,
//! where the whole point of an assist band is that the keyboard never leaves the `<input>`
//! while the list narrows. What that costs is the same thing it costs there: a row of names
//! rather than a column of them with their summaries, so the chosen offer's own summary is
//! drawn after the names — the one line a vertical list would have shown beside it.
//!
//! ## What is *not* here
//!
//! **Point mode** — arrow keys building a reference into a half-typed formula while the grid
//! still has the focus. It stays on `doc/web-shell.md`'s gap list: this shell edits in exactly
//! one place, the formula bar, and a caret that leaves it to build a reference is a third
//! editing mode this file has no `Reply` for.

use std::ops::Range;

use grind_sheet::formula::assist as core_assist;
pub use grind_sheet::formula::assist::Candidate;

/// How many offers fit on one line before the rest become a count.
pub const MAX_OFFERS: usize = 6;

/// What the assist band is showing, and what a key would do to it.
///
/// Recomputed from the formula bar's own text and caret on every keystroke and every caret
/// move rather than updated in place — the same "asked for fresh, never stored" shape the view
/// overlays follow, and for the same reason: a remembered completion goes stale the moment the
/// caret moves with the mouse.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Assist {
    /// Everything worth offering for the word being typed. Empty when there is no such word,
    /// when nothing matches it, or when the offers for it were dismissed with Escape.
    pub offers: Vec<Candidate>,
    /// What accepting an offer replaces — **byte** offsets into the formula bar's text.
    pub span: Range<usize>,
    /// Which offer is highlighted, and the one Tab accepts.
    pub chosen: usize,
    /// The call the caret is inside and which argument of it, for the signature hint. Answered
    /// even while offers are showing, since the two are about different halves of the same
    /// line: `=SUM(AV|` is both a word to complete and `SUM`'s first argument.
    pub call: Option<(String, usize)>,
    /// The span whose offers Escape dismissed, so they do not reappear on the next keystroke
    /// inside the same word. Typing on past the end of it, or moving elsewhere, starts a new
    /// word and offers again — which is what makes Escape "not this one" rather than a mode.
    dismissed: Option<Range<usize>>,
}

impl Assist {
    /// Read the formula bar's text at `caret` (a **byte** offset — the caller converts from
    /// the `<input>`'s own UTF-16 units) and say what to show.
    ///
    /// `names` is the document's defined names, which the caller reads from `App::names` —
    /// this module never touches a document.
    pub fn refresh(&mut self, text: &str, caret: usize, names: &[String]) {
        self.call = core_assist::call_at(text, caret);
        self.chosen = 0;
        self.offers.clear();
        self.span = 0..0;
        let Some(span) = core_assist::prefix_at(text, caret) else {
            // Nothing to complete here: whatever was dismissed is finished business, so
            // Escape does not have to be remembered past the word it was pressed in.
            self.dismissed = None;
            return;
        };
        self.span = span.clone();
        if self.dismissed.as_ref() == Some(&span) {
            return;
        }
        self.dismissed = None;
        let offers = core_assist::candidates(&text[span.clone()], names);
        // A single offer already typed out in full is not an offer — `=SUM|` has nothing left
        // to complete, and a list showing what is already there is noise over the grid.
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

    /// Up and down the list, wrapping — a list this short is quicker to wrap than to stop.
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

    /// Escape with a list up: close it and keep the edit, and do not offer for this word again.
    pub fn dismiss(&mut self) {
        self.dismissed = Some(self.span.clone());
        self.offers.clear();
    }

    /// Forget everything — an edit that ended, by any door.
    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

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

/// What a keystroke means **while a list of offers is up** — asked before
/// [`super::keymap::action_for`], and `None` for everything this list has no answer for, which
/// is almost every key.
///
/// The three it claims are the three that would otherwise do something the user did not mean:
/// Tab commits the cell and moves right, Up/Down would otherwise be left to the `<input>`'s own
/// (largely inert, on a single line) caret handling, and Escape throws the whole edit away.
/// **Enter is deliberately not claimed**: it commits, as it always does, because a formula
/// finished by pressing Enter is the common case and an accidental completion in its place
/// would store the wrong thing.
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

/// How strongly one run of the band is drawn — a CSS class in `mod.rs`, not a colour, so the
/// band follows the theme the same way every other reading in this shell does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ink {
    /// The theme's ordinary text: the parts of a signature the caret is not in.
    Plain,
    /// Quieter than the ground's text — separators, the summary, the offers not chosen.
    Muted,
    /// The accent, in bold: the argument being typed, and the offer Tab would take.
    Strong,
}

/// One run of the band, drawn left to right in the ink it carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Piece {
    pub text: String,
    pub ink: Ink,
}

impl Piece {
    fn new(text: impl Into<String>, ink: Ink) -> Self {
        Self {
            text: text.into(),
            ink,
        }
    }
}

/// The band's whole content, as runs — offers if there are any, otherwise the signature of the
/// call the caret is in.
///
/// Always the friendly spelling (`Sum(Number…)` rather than `SUM( { NumberSequenceList N } +
/// )`): this shell has no friendly-formulas toggle the way the two desktop windows do, and a
/// hint that reads is worth more here than one that matches the spec's own `Syntax:` line —
/// the formula bar beside it is already in A1, not ODF's, so there was never one spelling this
/// band had to answer to.
///
/// Empty when there is nothing to say, which is the caller's cue to hide `#assist`.
pub fn band(assist: &Assist) -> Vec<Piece> {
    if assist.is_offering() {
        return offers_band(assist);
    }
    match &assist.call {
        Some((name, argument)) => signature_band(name, *argument),
        None => Vec::new(),
    }
}

/// The offers, the chosen one in the accent, and its own summary after them.
fn offers_band(assist: &Assist) -> Vec<Piece> {
    let mut pieces = Vec::new();
    for (at, offer) in assist.offers.iter().take(MAX_OFFERS).enumerate() {
        if at > 0 {
            pieces.push(Piece::new("   ", Ink::Muted));
        }
        let ink = match at == assist.chosen {
            true => Ink::Strong,
            false => Ink::Muted,
        };
        pieces.push(Piece::new(offer.name.clone(), ink));
    }
    let hidden = assist.offers.len().saturating_sub(MAX_OFFERS);
    if hidden > 0 {
        pieces.push(Piece::new(format!("   +{hidden}"), Ink::Muted));
    }
    if let Some(offer) = assist.offers.get(assist.chosen) {
        pieces.push(Piece::new("   \u{00b7}   ", Ink::Muted));
        pieces.push(Piece::new(offer.detail.clone(), Ink::Plain));
    }
    pieces
}

/// `SUM(Number…)`, with the argument the caret is in in the accent.
///
/// A repeating parameter is the last one however many arguments follow it, which is why the
/// emphasised index is clamped rather than dropped — `=SUM(1;2;3;4;5|` is still inside
/// `Number…`.
fn signature_band(name: &str, argument: usize) -> Vec<Piece> {
    let Some((head, parts)) = core_assist::signature_parts(name, true) else {
        return Vec::new();
    };
    let last = parts.len().saturating_sub(1);
    let mut pieces = vec![Piece::new(head, Ink::Plain), Piece::new("(", Ink::Muted)];
    for (at, part) in parts.iter().enumerate() {
        if at > 0 {
            pieces.push(Piece::new("; ", Ink::Muted));
        }
        let ink = match at == argument.min(last) {
            true => Ink::Strong,
            false => Ink::Plain,
        };
        pieces.push(Piece::new(part.clone(), ink));
    }
    pieces.push(Piece::new(")", Ink::Muted));
    if let Some(brief) = core_assist::brief(name) {
        pieces.push(Piece::new("   \u{00b7}   ", Ink::Muted));
        pieces.push(Piece::new(brief, Ink::Muted));
    }
    pieces
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
    fn a_function_prefix_offers_functions_and_names() {
        let assist = read("=SU", 3);
        assert!(assist.is_offering());
        assert_eq!(assist.span, 1..3);
        assert!(assist.offers.iter().any(|o| o.name == "SUM"));
        // "SU" is a prefix of "subtotal_2026" too, case-insensitively.
        assert!(assist.offers.iter().any(|o| o.name == "subtotal_2026"));
        assert_eq!(assist.chosen, 0);
    }

    #[test]
    fn plain_text_offers_nothing() {
        let assist = read("hello", 5);
        assert!(!assist.is_offering());
        assert_eq!(assist.call, None);
        assert_eq!(band(&assist), Vec::new());
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
    fn accepting_replaces_the_span_and_clears_the_offers() {
        let mut assist = read("=SU", 3);
        let first = assist.offers[0].clone();
        let (span, replacement) = assist.accept().expect("SU has offers");
        assert_eq!(span, 1..3);
        assert_eq!(replacement, first.insert);
        assert!(!assist.is_offering());
    }

    #[test]
    fn stepping_wraps() {
        let mut assist = read("=S", 2);
        let count = assist.offers.len();
        assert!(count > 1, "S is a busy letter");
        assist.step(-1);
        assert_eq!(assist.chosen, count - 1);
        assist.step(1);
        assert_eq!(assist.chosen, 0);
    }

    #[test]
    fn escape_dismisses_only_the_word_it_was_pressed_in() {
        let mut assist = read("=SU", 3);
        assert!(assist.is_offering());
        assist.dismiss();
        assert!(!assist.is_offering());
        // Asking again for exactly the same word does not bring the list back...
        assist.refresh("=SU", 3, &[]);
        assert!(!assist.is_offering());
        // ...but a different word does, even a later prefix of the same call.
        assist.refresh("=SUM(1;S", 8, &[]);
        assert!(assist.is_offering());
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
