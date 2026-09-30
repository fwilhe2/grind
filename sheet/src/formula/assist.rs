// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What a shell needs to help somebody *while they are typing a formula*: which word could be
//! completed, what to offer for it, and which call the caret is inside.
//!
//! All three are questions about **display syntax and a caret offset**, which is why they are
//! here rather than in a shell. They were `ui_sheet_gtk`'s — `formula_ux::prefix_at`,
//! `formula_ux::candidates` and `state::call_at` — until the Windows shell wanted the same three
//! answers, and a second hand-written backwards scan over parentheses and string literals is
//! exactly the kind of near-copy this project moves into the core instead (the precedent is
//! `grind_core::search::score`, which came out of `ui_web` when a second shell wanted the same
//! ranking). Both shells now read one answer, so a signature hint cannot say `SUM`'s second
//! argument in one window and its third in another.
//!
//! Nothing here is a *capability* the CLI is missing (rule 4): what these functions offer comes
//! entirely from [`funcs::catalog`] and [`friendly::signature`], both of which `grind sheet
//! functions --long` already prints. What is added is the caret — where the user is — and a caret
//! is a UI's own state, not a document's.
//!
//! Pure, allocating only what it returns, and tested here rather than in any shell.

use std::ops::Range;

use super::{friendly, funcs};

/// One offer in a completion list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    /// What gets inserted, `(` included for a function — so accepting an offer leaves the caret
    /// where the first argument goes.
    pub insert: String,
    /// What the list shows: `SUM`, or the defined name as the document spells it.
    pub name: String,
    /// One line about it — the spec's own `Summary:` for a function.
    pub detail: String,
}

/// The identifier being typed at `caret`, if it is one an offer could replace.
///
/// A run of name characters that starts where a *function* could start — after `=`, `(`, `;` or
/// an operator — and is not already a call. Pure, and the reason a completion popup has no
/// opinion of its own about what a word is.
///
/// `caret` is a **byte** offset, clamped rather than trusted: a shell converts from whatever its
/// own control counts in (UTF-16 units, on Windows), and an offset one past the end is a
/// rounding error rather than a reason to panic.
pub fn prefix_at(text: &str, caret: usize) -> Option<Range<usize>> {
    if !text.starts_with('=') {
        return None;
    }
    let caret = caret.min(text.len());
    // Not a character boundary — a shell that converted from UTF-16 badly, or an offset into the
    // middle of a multi-byte character. Nothing to complete rather than a panic on the slice.
    if !text.is_char_boundary(caret) {
        return None;
    }
    let start = text[..caret]
        .char_indices()
        .rev()
        .take_while(|(_, c)| is_name_char(*c))
        .last()
        .map(|(i, _)| i)?;
    if start == 0 {
        return None;
    }
    // Already a call: `SUM(` is finished business, and so is `A1` — a cell address is not a
    // function name, and offering to turn one into a call would be wrong twice over.
    if text[caret..].starts_with('(') {
        return None;
    }
    let before = text[..start].trim_end().chars().next_back()?;
    "=(;+-*/^&<>:,".contains(before).then_some(start..caret)
}

/// Everything worth offering for `prefix`: functions first, then the document's own names.
///
/// Neither list is written here — the functions are [`funcs::catalog`]'s, so a shell cannot offer
/// one the evaluator does not have, and the names are the caller's read of `App::names`.
pub fn candidates(prefix: &str, names: &[String]) -> Vec<Candidate> {
    let upper = prefix.to_uppercase();
    let functions = funcs::catalog()
        .iter()
        .filter(|info| info.name.starts_with(&upper))
        .map(|info| Candidate {
            insert: format!("{}(", info.name),
            name: info.name.to_owned(),
            detail: info.brief.to_owned(),
        });
    let named = names
        .iter()
        .filter(|name| name.to_uppercase().starts_with(&upper))
        .map(|name| Candidate {
            insert: name.clone(),
            name: name.clone(),
            detail: "defined name".to_owned(),
        });
    functions.chain(named).collect()
}

/// Which call the caret is inside, and which argument of it — what a signature hint shows.
///
/// Scanned backwards over the display text, counting parentheses and skipping string literals,
/// because the caret is where the user is and the call that matters is the innermost one
/// containing it. A finished call (`=SUM(1;2)|`) is not one the caret is inside, and a bare
/// grouping parenthesis is not a call at all.
pub fn call_at(text: &str, caret: usize) -> Option<(String, usize)> {
    let head: Vec<char> = text[..caret.min(text.len())].chars().collect();
    let mut depth = 0i32;
    let mut argument = 0usize;
    let mut i = head.len();
    let mut in_string = false;
    while i > 0 {
        i -= 1;
        match head[i] {
            // Quotes are counted from the left, so a backwards scan flips at every one.
            '"' => in_string = !in_string,
            _ if in_string => {}
            ')' => depth += 1,
            ';' if depth == 0 => argument += 1,
            '(' if depth > 0 => depth -= 1,
            '(' => {
                // The name in front of it, if there is one — otherwise this was a grouping
                // parenthesis and the call, if any, is further out.
                let end = i;
                while i > 0 && is_name_char(head[i - 1]) {
                    i -= 1;
                }
                if i == end {
                    argument = 0;
                    continue;
                }
                return Some((head[i..end].iter().collect(), argument));
            }
            _ => {}
        }
    }
    None
}

/// A function's signature split into a head and one part per parameter — the two spellings a
/// hint can be drawn in, and the only difference between them.
///
/// `friendly` picks: [`friendly::signature`]'s plain-English labels (`Sum`, `Number…`), which are
/// the same names [`friendly::explain`] labels a finished formula with, or the spec's own
/// `Syntax:` line split on `;`, types and brackets and all. Two spellings of one signature rather
/// than two signatures, so what a user reads while typing and what they read afterwards agree.
///
/// `None` for a name the catalog does not know, which is a shell's cue to show nothing rather
/// than to invent a signature.
pub fn signature_parts(name: &str, friendly: bool) -> Option<(String, Vec<String>)> {
    if friendly {
        return friendly::signature(name);
    }
    let info = funcs::catalog()
        .iter()
        .find(|info| info.name.eq_ignore_ascii_case(name))?;
    let (head, rest) = info.signature.split_once('(')?;
    let rest = rest.strip_suffix(')').unwrap_or(rest);
    Some((
        head.trim().to_owned(),
        rest.split(';').map(|part| part.trim().to_owned()).collect(),
    ))
}

/// The one line saying what a function does — the spec's own `Summary:`, for a hint that has
/// room for it.
pub fn brief(name: &str) -> Option<&'static str> {
    funcs::catalog()
        .iter()
        .find(|info| info.name.eq_ignore_ascii_case(name))
        .map(|info| info.brief)
}

/// What a name is made of, in both a function name and a defined one.
fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '.'
}

// ---------------------------------------------------------------------------------------------
// The offer machine and the band — hoisted out of `ui_win32/src/sheet/assist.rs` the day the
// Mac's cell editor (`doc/macos-shell.md`, M8) would have been its second copy. Which *keys*
// steer it is each shell's own (a Windows key code, a Mac selector); what a list of offers is,
// how it narrows, what accepting replaces, and what a band of runs says, are here.
// ---------------------------------------------------------------------------------------------

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
        self.call = call_at(text, caret);
        self.chosen = 0;
        self.offers.clear();
        self.span = 0..0;
        let Some(span) = prefix_at(text, caret) else {
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
        let offers = candidates(&text[span.clone()], names);
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
    let Some((head, parts)) = signature_parts(name, friendly) else {
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
    if let Some(brief) = brief(name) {
        pieces.push(Piece::new("   \u{00b7}   ", Ink::Muted));
        pieces.push(Piece::new(brief, Ink::Muted));
    }
    pieces
}

/// What the formula bar shows for a cell, when the friendly view is on.
///
/// [`friendly::explain_inline`] — the aliased, parameter-labelled reading
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
    friendly::explain_inline(text).ok()
}

/// Every function this build implements, one line each, for the function list — the name, the
/// plain-English alias, its category, and the spec's own summary.
///
/// `grind sheet functions --long` prints the same four columns from the same catalog, which is
/// rule 4 satisfied by construction rather than by a second list here.
pub fn function_lines() -> Vec<String> {
    funcs::catalog()
        .iter()
        .map(|info| {
            let alias = friendly::alias(info.name).unwrap_or(info.name);
            let category = funcs::category(info);
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
    let info = funcs::catalog().get(at)?;
    Some(match editing {
        true => format!("{}(", info.name),
        false => format!("={}(", info.name),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_prefix_is_a_word_where_a_function_could_start() {
        assert_eq!(prefix_at("=SU", 3), Some(1..3));
        assert_eq!(prefix_at("=SUM(AV", 7), Some(5..7));
        assert_eq!(prefix_at("=1+VLO", 6), Some(3..6));
        // A word, and `B2` is offerable — a defined name can begin that way.
        assert_eq!(prefix_at("=SUM(B2", 7), Some(5..7));
        assert_eq!(prefix_at("SUM", 3), None); // not a formula
        assert_eq!(prefix_at("=SUM(", 5), None); // nothing typed yet
        assert_eq!(prefix_at("=SUM(1;2)", 9), None);
    }

    /// A caret a shell converted badly is nothing to complete, not a panic. `EM_GETSEL` counts
    /// UTF-16 units and this counts bytes, which is exactly where such an offset comes from.
    #[test]
    fn a_caret_off_a_character_boundary_offers_nothing() {
        let text = "=SÜM"; // `Ü` is two bytes, so 3 is inside it
        assert_eq!(prefix_at(text, 3), None);
        assert_eq!(prefix_at(text, 5), Some(1..5));
        assert_eq!(prefix_at(text, 99), Some(1..5), "past the end is the end");
    }

    #[test]
    fn candidates_come_from_the_catalog_and_the_document() {
        let names = vec!["expenses".to_owned(), "excess".to_owned()];
        let offers = candidates("su", &names);
        assert!(offers.iter().any(|c| c.name == "SUM" && c.insert == "SUM("));
        assert!(offers.iter().all(|c| c.name.starts_with("SU")));

        // Names come after functions, and are inserted without a parenthesis.
        let offers = candidates("ex", &names);
        assert_eq!(
            offers.last().map(|c| c.insert.clone()),
            Some("excess".to_owned())
        );
        assert!(offers.iter().any(|c| c.name == "EXP"));
    }

    /// What a signature hint needs: which call the caret is in, and which argument.
    #[test]
    fn the_caret_knows_which_argument_it_is_in() {
        assert_eq!(call_at("=SUM(", 5), Some(("SUM".to_owned(), 0)));
        assert_eq!(call_at("=SUM(1;2", 8), Some(("SUM".to_owned(), 1)));
        assert_eq!(call_at("=SUM(1;2;3", 10), Some(("SUM".to_owned(), 2)));
        // The innermost call wins, and a closed one is not it.
        assert_eq!(call_at("=IF(A1;SUM(1;2", 14), Some(("SUM".to_owned(), 1)));
        assert_eq!(call_at("=IF(A1;SUM(1;2);", 16), Some(("IF".to_owned(), 2)));
        // A `;` inside a string is not an argument separator, and a bare group is no call.
        assert_eq!(call_at("=SUM(\"a;b\";2", 12), Some(("SUM".to_owned(), 1)));
        assert_eq!(call_at("=(1+2", 5), None);
        assert_eq!(call_at("=A1", 3), None);
        // Dollars are part of a reference, not of anything the scan cares about.
        assert_eq!(call_at("=SUM($A$13:$A$15", 16), Some(("SUM".to_owned(), 0)));
    }

    /// The two spellings of one signature, and that they describe the same parameters.
    #[test]
    fn a_signature_comes_in_the_specs_words_and_in_plain_ones() {
        let (head, parts) = signature_parts("VLOOKUP", false).expect("in the catalog");
        assert_eq!(head, "VLOOKUP");
        assert!(parts[0].contains("Lookup"), "{parts:?}");
        assert!(parts.iter().any(|p| p.contains("Column")), "{parts:?}");

        let (head, labels) = signature_parts("vlookup", true).expect("case does not matter");
        assert_eq!(head, "Vertical Lookup");
        assert_eq!(labels.len(), parts.len(), "one label per parameter");

        // A repeating parameter carries the ellipsis in the friendly spelling.
        let (head, labels) = signature_parts("SUM", true).expect("in the catalog");
        assert_eq!((head.as_str(), labels.len()), ("Sum", 1));
        assert!(labels[0].ends_with('\u{2026}'), "{labels:?}");

        assert_eq!(signature_parts("NOSUCHFUNCTION", false), None);
        assert_eq!(signature_parts("NOSUCHFUNCTION", true), None);
    }

    #[test]
    fn a_brief_is_the_specs_own_summary() {
        let summary = brief("sum").expect("in the catalog");
        assert!(summary.to_lowercase().contains("sum"), "{summary}");
        assert_eq!(brief("NOSUCHFUNCTION"), None);
    }

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
            funcs::implemented().len(),
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
