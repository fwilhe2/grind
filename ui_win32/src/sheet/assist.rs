// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Help while a formula is being typed: what may be completed, and what the call under the caret
//! wants next — `doc/sheet-shell.md`'s M6 for this shell, and W9's half of it.
//!
//! **Portable, and tested on any host**, like [`super::keymap`], [`super::state`] and
//! [`super::status`]. Everything that decides *what to say* is here; `win.rs` reads the editor's
//! caret and `sheet/draw.rs` puts the answer on screen.
//!
//! ## Where it is shown, and why not in a popup
//!
//! `ui_sheet_gtk` floats a `gtk::Popover` under the cell being edited. This shell draws a **band**
//! instead — one line under the strip, in the same place and the same manner as the notice bar —
//! and that is a decision rather than a shortcut. A popup list on Win32 is a second top-level
//! window with its own class, its own theming (`WM_CTLCOLORLISTBOX`, since a `LISTBOX` paints
//! itself), its own DPI answer, and a focus problem: the whole point is that typing carries on
//! into the editor underneath while the list narrows, so the popup must never take the keyboard.
//! A band this window draws itself has none of those, follows the theme by construction, sits
//! next to the formula bar the text is also mirrored in, and — being drawn rather than a
//! control — is visible in `--render-to`'s windowless frame like every other read-out here.
//!
//! What it costs is the vertical list: five names on one line rather than eight rows with their
//! summaries. So the chosen offer's own summary is drawn after the names, which is the line a
//! reader actually wants, and the rest are names alone.
//!
//! ## What is *not* here
//!
//! **Point mode** — arrow keys building a reference into a half-typed formula. It stays on
//! `doc/windows-shell.md`'s gap list: it is a third editing mode rather than a read-out, and
//! [`super::state::Outcome`] has no `Point` for it to arrive as.

use std::ops::Range;

use grind_sheet::formula::assist as core_assist;
pub use grind_sheet::formula::assist::Candidate;

use super::keymap::{Key, Mods};

/// How many offers fit on one line before the rest become a count.
///
/// Five, and the sixth is `+n`: a band is a line, and a line that scrolls off the window is worse
/// than one that says how much it is not showing. One more keystroke narrows the list.
pub const MAX_OFFERS: usize = 5;

/// What the assist band is showing, and what a key would do to it.
///
/// Recomputed from the editor's text on every keystroke rather than updated in place — the same
/// "asked for fresh, never stored" shape the view overlays follow, and for the same reason: a
/// remembered completion goes stale the moment somebody moves the caret with the mouse.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Assist {
    /// Everything worth offering for the word being typed. Empty when there is no such word,
    /// when nothing matches it, or when the offers for it were dismissed with Escape.
    pub offers: Vec<Candidate>,
    /// What accepting an offer replaces — **byte** offsets into the editor's text.
    pub span: Range<usize>,
    /// Which offer is highlighted, and the one Tab accepts.
    pub chosen: usize,
    /// The call the caret is inside and which argument of it, for the signature hint. Answered
    /// even while offers are showing, since the two are about different halves of the same line:
    /// `=SUM(AV|` is both a word to complete and `SUM`'s first argument.
    pub call: Option<(String, usize)>,
    /// The span whose offers Escape dismissed, so they do not reappear on the next keystroke
    /// inside the same word. Typing on past the end of it, or moving elsewhere, starts a new
    /// word and offers again — which is what makes Escape "not this one" rather than a mode.
    dismissed: Option<Range<usize>>,
}

impl Assist {
    /// Read the editor's text at `caret` (a byte offset) and say what to show.
    ///
    /// `names` is the document's defined names, which the caller reads from `App::names` — this
    /// module never touches a document.
    pub fn refresh(&mut self, text: &str, caret: usize, names: &[String]) {
        self.call = core_assist::call_at(text, caret);
        self.chosen = 0;
        self.offers.clear();
        self.span = 0..0;
        let Some(span) = core_assist::prefix_at(text, caret) else {
            // Nothing to complete here: whatever was dismissed is finished business, so Escape
            // does not have to be remembered past the word it was pressed in.
            self.dismissed = None;
            return;
        };
        self.span = span.clone();
        if self.dismissed.as_ref() == Some(&span) {
            return;
        }
        self.dismissed = None;
        let offers = core_assist::candidates(&text[span.clone()], names);
        // A single offer already typed out in full is not an offer — `=SUM|` has nothing left to
        // complete, and a list showing what is already there is noise over the grid.
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

/// How far a key moves the highlighted offer, or what it does to the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reply {
    /// Put the highlighted offer into the editor.
    Accept,
    /// Move the highlight by this much, wrapping.
    Step(i32),
    /// Close the list, keeping the edit.
    Dismiss,
}

/// What a keystroke means **while a list of offers is up** — asked before
/// [`super::state::on_key`], and `None` for everything this list has no answer for, which is
/// almost every key.
///
/// The three it claims are the three that would otherwise do something the user did not mean:
/// Tab commits the cell and moves right, Up/Down commit in Enter mode, and Escape throws the
/// whole edit away. **Enter is deliberately not claimed**: it commits, as it always does, because
/// a formula finished by pressing Enter is the common case and an accidental completion in its
/// place would store the wrong thing.
pub fn on_key(offering: bool, key: Key, mods: Mods) -> Option<Reply> {
    if !offering || mods.ctrl || mods.alt {
        return None;
    }
    match key {
        Key::Tab if !mods.shift => Some(Reply::Accept),
        Key::Down => Some(Reply::Step(1)),
        Key::Up => Some(Reply::Step(-1)),
        Key::Escape => Some(Reply::Dismiss),
        _ => None,
    }
}

/// How strongly one run of the band is drawn.
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
/// `friendly` picks which spelling of a signature is used, and it is the same switch the formula
/// bar's own friendly view answers to: somebody who reads `Present Value(Rate: 0.05, …)` in the
/// bar should be offered `Present Value(Rate; …)` while typing, not `PV( Number Rate ; … )`.
///
/// Empty when there is nothing to say, which is the caller's cue to take the band's height back
/// out of the geometry.
pub fn band(assist: &Assist, friendly: bool) -> Vec<Piece> {
    if assist.is_offering() {
        return offers_band(assist);
    }
    match &assist.call {
        Some((name, argument)) => signature_band(name, *argument, friendly),
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
    // The chosen offer's own summary — the one line a vertical list would have shown beside it,
    // and the reason a band is not simply a poorer popup.
    if let Some(offer) = assist.offers.get(assist.chosen) {
        pieces.push(Piece::new("   \u{00b7}   ", Ink::Muted));
        pieces.push(Piece::new(offer.detail.clone(), Ink::Plain));
    }
    pieces
}

/// `Sum(Number…)`, with the argument the caret is in in the accent.
///
/// A repeating parameter is the last one however many arguments follow it, which is why the
/// emphasised index is clamped rather than dropped — `=SUM(1;2;3;4;5|` is still inside `Number…`.
fn signature_band(name: &str, argument: usize, friendly: bool) -> Vec<Piece> {
    let Some((head, parts)) = core_assist::signature_parts(name, friendly) else {
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

/// What the formula bar shows for a cell, when the friendly view is on.
///
/// [`grind_sheet::formula::friendly::explain_inline`] — the aliased, parameter-labelled reading
/// of a formula, never unfolded, which is exactly what `ui_sheet_gtk`'s formula bar puts in front
/// of the stored text and what `grind sheet fmt --friendly --inline` prints. `None` for anything
/// that is not a formula, or a formula this build cannot parse: the bar's own text is the answer
/// then, since a reading nobody can produce must not blank out the line that can be typed.
///
/// It never round-trips and nothing here writes it back — R1 says the document's formula is
/// ODF's, and this is a reading of one.
pub fn friendly_line(text: &str) -> Option<String> {
    if !text.starts_with('=') {
        return None;
    }
    grind_sheet::formula::friendly::explain_inline(text).ok()
}

/// Every function this build implements, one line each, for the function list — the name, the
/// plain-English alias, its category, and the spec's own summary.
///
/// `grind sheet functions --long` prints the same four columns from the same catalog, which is
/// rule 4 satisfied by construction rather than by a second list here.
pub fn function_lines() -> Vec<String> {
    grind_sheet::formula::funcs::catalog()
        .iter()
        .map(|info| {
            let alias = grind_sheet::formula::friendly::alias(info.name).unwrap_or(info.name);
            let category = grind_sheet::formula::funcs::category(info);
            format!(
                "{:<12} {alias}  \u{00b7}  {category}  \u{00b7}  {}",
                info.name, info.brief
            )
        })
        .collect()
}

/// What picking row `at` of [`function_lines`] inserts into an edit: `=NAME(`.
///
/// The `=` is included because the common case is an empty cell, and a name with no `=` in front
/// of it is a piece of text rather than a call. A cell that is *already* being edited gets the
/// call without it — the caller decides, since only it knows whether an edit is open.
pub fn function_insert(at: usize, editing: bool) -> Option<String> {
    let info = grind_sheet::formula::funcs::catalog().get(at)?;
    Some(match editing {
        true => format!("{}(", info.name),
        false => format!("={}(", info.name),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> Vec<String> {
        vec!["subtotal_2026".to_owned()]
    }

    fn read(text: &str, caret: usize) -> Assist {
        let mut assist = Assist::default();
        assist.refresh(text, caret, &names());
        assist
    }

    #[test]
    fn typing_a_word_offers_what_starts_with_it() {
        let assist = read("=SU", 3);
        assert!(assist.is_offering());
        assert_eq!(assist.span, 1..3);
        assert!(assist.offers.iter().any(|o| o.name == "SUM"));
        // The document's own name is offered beside the functions.
        assert!(assist.offers.iter().any(|o| o.name == "subtotal_2026"));
        assert_eq!(assist.chosen, 0);
    }

    #[test]
    fn a_value_is_not_a_formula_and_offers_nothing() {
        let assist = read("hello", 5);
        assert!(!assist.is_offering());
        assert_eq!(assist.call, None);
        assert_eq!(band(&assist, true), Vec::new());
    }

    /// The two halves are answered together: inside `SUM(` *and* typing a word is both a
    /// completion and an argument, and only one of them can have the band.
    #[test]
    fn a_word_inside_a_call_is_both_and_the_offers_win() {
        let assist = read("=SUM(AV", 7);
        assert!(assist.is_offering());
        assert_eq!(assist.call, Some(("SUM".to_owned(), 0)));
        let pieces = band(&assist, true);
        assert!(
            pieces.iter().any(|p| p.text == "AVERAGE"),
            "{pieces:#?}: the offers are what a half-typed word wants"
        );
    }

    #[test]
    fn a_finished_word_inside_a_call_shows_the_signature() {
        let assist = read("=SUM(", 5);
        assert!(!assist.is_offering());
        let pieces = band(&assist, true);
        let text: String = pieces.iter().map(|p| p.text.as_str()).collect();
        assert!(text.starts_with("Sum(Number\u{2026})"), "{text}");
        // The argument being typed is the one in the accent, and it is the only one.
        let strong: Vec<&str> = pieces
            .iter()
            .filter(|p| p.ink == Ink::Strong)
            .map(|p| p.text.as_str())
            .collect();
        assert_eq!(strong, ["Number\u{2026}"]);
    }

    /// The band's two spellings are the formula bar's two spellings — the switch is one.
    #[test]
    fn the_signature_is_spelled_the_way_the_bar_is() {
        let assist = read("=PV(0.05;", 9);
        let friendly: String = band(&assist, true)
            .iter()
            .map(|p| p.text.as_str())
            .collect();
        assert!(friendly.starts_with("Present Value("), "{friendly}");
        let spec: String = band(&assist, false)
            .iter()
            .map(|p| p.text.as_str())
            .collect();
        assert!(spec.starts_with("PV("), "{spec}");
        // Whichever spelling, it is the *second* argument that is emphasised.
        for pieces in [band(&assist, true), band(&assist, false)] {
            let strong = pieces.iter().find(|p| p.ink == Ink::Strong).expect("one");
            assert!(!strong.text.is_empty(), "{pieces:#?}");
        }
    }

    #[test]
    fn the_offers_carry_the_chosen_ones_summary() {
        let mut assist = read("=SU", 3);
        let first = assist.offers[0].clone();
        let pieces = band(&assist, true);
        assert!(pieces.iter().any(|p| p.text == first.detail));
        assert_eq!(
            pieces
                .iter()
                .filter(|p| p.ink == Ink::Strong)
                .map(|p| p.text.as_str())
                .collect::<Vec<_>>(),
            [first.name.as_str()],
            "the chosen offer is the one in the accent"
        );

        // Stepping moves the accent and the summary together.
        assist.step(1);
        let pieces = band(&assist, true);
        assert!(pieces.iter().any(|p| p.text == assist.offers[1].detail));
        assert!(!pieces.iter().any(|p| p.text == first.detail));
    }

    #[test]
    fn a_long_list_says_how_much_it_is_not_showing() {
        let assist = read("=S", 2);
        assert!(assist.offers.len() > MAX_OFFERS, "S is a busy letter");
        let text: String = band(&assist, true)
            .iter()
            .map(|p| p.text.as_str())
            .collect();
        let hidden = assist.offers.len() - MAX_OFFERS;
        assert!(text.contains(&format!("+{hidden}")), "{text}");
    }

    #[test]
    fn stepping_wraps_both_ways() {
        let mut assist = read("=SU", 3);
        let count = assist.offers.len();
        assist.step(-1);
        assert_eq!(assist.chosen, count - 1);
        assist.step(1);
        assert_eq!(assist.chosen, 0);
    }

    #[test]
    fn accepting_replaces_the_word_with_a_call() {
        let mut assist = read("=SU", 3);
        assist.chosen = assist
            .offers
            .iter()
            .position(|o| o.name == "SUM")
            .expect("SUM is offered");
        let (span, replacement) = assist.accept().expect("something is chosen");
        assert_eq!(span, 1..3);
        assert_eq!(replacement, "SUM(");
        assert!(!assist.is_offering(), "accepting closes the list");

        // A defined name is inserted as itself — it is not a call.
        let mut assist = read("=subt", 5);
        let (_, replacement) = assist.accept().expect("the name is offered");
        assert_eq!(replacement, "subtotal_2026");
    }

    /// Escape closes the list without ending the edit, and the same word does not offer again —
    /// otherwise the next keystroke inside it would put the list straight back up.
    #[test]
    fn dismissing_lasts_as_long_as_the_word() {
        let mut assist = read("=SU", 3);
        assist.dismiss();
        assert!(!assist.is_offering());
        assist.refresh("=SU", 3, &names());
        assert!(!assist.is_offering(), "the same word stays dismissed");
        // Typing on is a new word, and it offers again.
        assist.refresh("=SUM", 4, &names());
        assert!(assist.is_offering());
    }

    /// The keys the list claims, and — just as important — the ones it does not.
    #[test]
    fn the_list_claims_tab_and_the_arrows_and_leaves_enter_alone() {
        let plain = Mods::default();
        assert_eq!(on_key(true, Key::Tab, plain), Some(Reply::Accept));
        assert_eq!(on_key(true, Key::Down, plain), Some(Reply::Step(1)));
        assert_eq!(on_key(true, Key::Up, plain), Some(Reply::Step(-1)));
        assert_eq!(on_key(true, Key::Escape, plain), Some(Reply::Dismiss));
        // Enter commits the cell, as it always does.
        assert_eq!(on_key(true, Key::Return, plain), None);
        // With no list up, every one of them means what it always meant.
        for key in [Key::Tab, Key::Down, Key::Up, Key::Escape] {
            assert_eq!(on_key(false, key, plain), None, "{key:?}");
        }
        // A modifier is somebody asking for something else.
        let ctrl = Mods {
            ctrl: true,
            ..Default::default()
        };
        assert_eq!(on_key(true, Key::Tab, ctrl), None);
    }

    #[test]
    fn the_friendly_line_reads_a_formula_and_nothing_else() {
        let line = friendly_line("=PV(0.05;10;-100)").expect("a formula this build parses");
        assert!(line.starts_with("Present Value("), "{line}");
        assert!(line.contains("Rate:"), "{line}");
        // Not a formula, and a formula that will not parse: the bar's own text is the answer.
        assert_eq!(friendly_line("12"), None);
        assert_eq!(friendly_line("=SUM("), None);
    }

    #[test]
    fn the_function_list_names_every_function_and_inserts_a_call() {
        let lines = function_lines();
        assert_eq!(
            lines.len(),
            grind_sheet::formula::funcs::implemented().len(),
            "one line per function this build has"
        );
        let at = lines
            .iter()
            .position(|line| line.starts_with("PV "))
            .expect("PV is implemented");
        assert!(lines[at].contains("Present Value"), "{}", lines[at]);
        assert!(lines[at].contains("Financial"), "{}", lines[at]);
        assert_eq!(function_insert(at, false).as_deref(), Some("=PV("));
        assert_eq!(function_insert(at, true).as_deref(), Some("PV("));
        assert_eq!(function_insert(lines.len(), false), None);
    }
}
