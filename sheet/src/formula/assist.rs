// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What a shell needs to help somebody while they are typing a formula: which word could be
//! completed, what to offer for it, and which call the caret is inside. Questions about display
//! syntax and a caret offset, answered once for every shell. What is offered comes from
//! [`funcs::catalog`] and [`friendly::signature`].

use std::ops::Range;

use super::display::{self, TokenKind};
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

/// The identifier being typed at `caret`, if an offer could replace it: a run of name characters
/// where a function could start (after `=`, `(`, `;` or an operator) that is not already a call.
/// `caret` is a byte offset, clamped.
pub fn prefix_at(text: &str, caret: usize) -> Option<Range<usize>> {
    if !text.starts_with('=') {
        return None;
    }
    let caret = caret.min(text.len());
    // Not a character boundary: an offset badly converted from UTF-16. Nothing to complete.
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
    // Already a call (`SUM(`), or a cell address, which is not a function name.
    if text[caret..].starts_with('(') {
        return None;
    }
    let before = text[..start].trim_end().chars().next_back()?;
    "=(;+-*/^&<>:,".contains(before).then_some(start..caret)
}

/// Everything worth offering for `prefix`: [`funcs::catalog`]'s functions first, then the
/// document's names (the caller's read of `App::names`).
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

/// Which call the caret is inside, and which argument of it — what a signature hint shows. Scanned
/// backwards over the display text, counting parentheses and skipping string literals. A finished
/// call (`=SUM(1;2)|`) does not count, nor does a grouping parenthesis.
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

/// Whether a reference may be inserted at `caret` — Point mode's predicate: a formula whose last
/// thing before the caret is an operator, a separator, `(` or the `=`. Not after `SUM(B2`, so an
/// arrow there moves on rather than extending a half-typed reference.
pub fn ref_eligible(text: &str, caret: usize) -> bool {
    if !text.starts_with('=') {
        return false;
    }
    let before = &text[..caret.min(text.len())];
    match before.trim_end().chars().next_back() {
        Some(c) => "=(;+-*/^&<>:,".contains(c),
        // Nothing but the `=` and whitespace.
        None => false,
    }
}

/// F4: the reference the caret is in or at the end of, and its next spelling in Excel's cycle `B2`
/// → `$B$2` → `B$2` → `$B2` → `B2`. Found with `display::spans`, the scanner that colours
/// references.
pub fn cycle_absolute(text: &str, caret: usize) -> Option<(Range<usize>, String)> {
    let span = display::spans(text).into_iter().find(|span| {
        span.kind == TokenKind::Ref && span.range.start <= caret && caret <= span.range.end
    })?;
    let mut reference = crate::a1::parse(&text[span.range.clone()]).ok()?;
    // Both axes of both ends move together, which is what the four steps mean.
    let (col, row) = reference
        .start
        .col
        .zip(reference.start.row)
        .map(|(c, r)| (c.absolute, r.absolute))
        .unwrap_or((false, false));
    let (col, row) = match (col, row) {
        (false, false) => (true, true),
        (true, true) => (false, true),
        (false, true) => (true, false),
        _ => (false, false),
    };
    for end in [Some(&mut reference.start), reference.end.as_mut()]
        .into_iter()
        .flatten()
    {
        if let Some(axis) = end.col.as_mut() {
            axis.absolute = col;
        }
        if let Some(axis) = end.row.as_mut() {
            axis.absolute = row;
        }
    }
    Some((span.range, display::reference_text(&reference)))
}

/// A function's signature split into a head and one part per parameter. `friendly` picks
/// [`friendly::signature`]'s plain-English labels, the ones [`friendly::explain`] uses, or the
/// spec's `Syntax:` line split on `;`. `None` for a name the catalog does not know.
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

/// Which part of [`signature_parts`] the `argument`th argument (0-based) falls in. Past the end,
/// the repeating tail: `SUM`'s fifth argument is still `Number…`, and `COUNTIFS`' fifth is the
/// range of its third pair. Without a repeating part it clamps to the last; an unknown function
/// hands the index back.
pub fn part_for(name: &str, argument: usize) -> usize {
    let Some((len, group)) = friendly::shape(name) else {
        return argument;
    };
    if argument < len || len == 0 {
        return argument;
    }
    match group {
        0 => len - 1,
        _ => len - group + (argument - (len - group)) % group,
    }
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
// The offer list and the band. Which keys steer it is each shell's own; what the offers are, how
// they narrow, what accepting replaces and what the band says are here.
// ---------------------------------------------------------------------------------------------

/// How many offers fit on one line; the rest become `+n`.
pub const MAX_OFFERS: usize = 5;

/// What the assist band is showing, and what a key would do to it. Recomputed from the editor's
/// text on every keystroke, since a stored completion goes stale when the caret moves by mouse.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Assist {
    /// Everything worth offering for the word being typed. Empty when there is no such word,
    /// when nothing matches it, or when the offers for it were dismissed with Escape.
    pub offers: Vec<Candidate>,
    /// What accepting an offer replaces — **byte** offsets into the editor's text.
    pub span: Range<usize>,
    /// Which offer is highlighted, and the one Tab accepts.
    pub chosen: usize,
    /// The call the caret is inside and which argument of it, for the signature hint — answered
    /// even while offers show: `=SUM(AV|` is both.
    pub call: Option<(String, usize)>,
    /// The span whose offers Escape dismissed, so they do not reappear inside the same word. A new
    /// word offers again.
    dismissed: Option<Range<usize>>,
}

impl Assist {
    /// Read the editor's text at `caret` (a byte offset) and say what to show. `names` is the
    /// caller's read of `App::names`.
    pub fn refresh(&mut self, text: &str, caret: usize, names: &[String]) {
        self.call = call_at(text, caret);
        self.chosen = 0;
        self.offers.clear();
        self.span = 0..0;
        let Some(span) = prefix_at(text, caret) else {
            self.dismissed = None;
            return;
        };
        self.span = span.clone();
        if self.dismissed.as_ref() == Some(&span) {
            return;
        }
        self.dismissed = None;
        let offers = candidates(&text[span.clone()], names);
        // A single offer already typed out in full is not an offer.
        if offers.len() == 1 && offers[0].name.eq_ignore_ascii_case(&text[span]) {
            return;
        }
        self.offers = offers;
    }

    /// Whether there is a list to steer, which gives Tab and the arrows their meaning while
    /// editing.
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

/// The band's content as runs: the offers if there are any, else the signature of the call the
/// caret is in. `friendly` is the formula bar's own switch, so the hint and the bar agree. Empty
/// when there is nothing to say.
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
    // The chosen offer's own summary.
    if let Some(offer) = assist.offers.get(assist.chosen) {
        pieces.push(Piece::new("   \u{00b7}   ", Ink::Muted));
        pieces.push(Piece::new(offer.detail.clone(), Ink::Plain));
    }
    pieces
}

/// `Sum(Number…)`, with the argument the caret is in ([`part_for`]) in the accent.
fn signature_band(name: &str, argument: usize, friendly: bool) -> Vec<Piece> {
    let Some((head, parts)) = signature_parts(name, friendly) else {
        return Vec::new();
    };
    let emphasised = part_for(name, argument);
    let mut pieces = vec![Piece::new(head, Ink::Plain), Piece::new("(", Ink::Muted)];
    for (at, part) in parts.iter().enumerate() {
        if at > 0 {
            pieces.push(Piece::new("; ", Ink::Muted));
        }
        let ink = match at == emphasised {
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

/// What the formula bar shows for a cell when the friendly view is on:
/// [`friendly::explain_inline`], as `grind sheet fmt --friendly --inline` prints. `None` for
/// anything that is not a formula or will not parse, and the bar shows its own text. A reading
/// only; nothing writes it back.
pub fn friendly_line(text: &str) -> Option<String> {
    if !text.starts_with('=') {
        return None;
    }
    friendly::explain_inline(text).ok()
}

/// Every function this build implements, one line each: name, plain-English alias, category and
/// the spec's summary — the four columns of `grind sheet functions --long`.
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

/// What picking row `at` of [`function_lines`] inserts: `=NAME(`, or `NAME(` into an edit already
/// open.
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

    /// A caret badly converted from UTF-16 is nothing to complete, not a panic.
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

    /// Which part of a signature an argument falls in: itself up to the end, then the
    /// repeating tail — one parameter, or a group taken round again.
    #[test]
    fn an_argument_past_the_end_lands_in_the_repeating_part() {
        assert_eq!(part_for("VLOOKUP", 2), 2);
        assert_eq!(part_for("VLOOKUP", 9), 3); // nothing repeats: the last, clamped
        assert_eq!(part_for("SUM", 4), 0); // `{ Number } +`
        // `COUNTIFS( R1 ; C1 [ ; R2 ; C2 ]... )`: the fifth argument is the third range.
        assert_eq!(part_for("COUNTIFS", 3), 3);
        assert_eq!(part_for("COUNTIFS", 4), 2);
        assert_eq!(part_for("COUNTIFS", 5), 3);
        assert_eq!(part_for("SUMIFS", 5), 3); // one fixed parameter ahead of the pairs
        assert_eq!(part_for("NOSUCHFUNCTION", 3), 3);
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

    /// Inside `SUM(` and typing a word is both a completion and an argument; the offers get the
    /// band.
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
        // (`subt` alone would offer SUBTOTAL first, which is a function.)
        let mut assist = read("=subtotal_", 10);
        let (_, replacement) = assist.accept().expect("the name is offered");
        assert_eq!(replacement, "subtotal_2026");
    }

    /// Escape closes the list without ending the edit, and the same word does not offer again.
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

    /// Excel's cycle, because everybody's fingers know it.
    #[test]
    fn f4_walks_the_four_spellings_and_comes_back() {
        let mut text = "=SUM(B2:B4)".to_owned();
        let mut seen = Vec::new();
        for _ in 0..5 {
            let (span, replacement) = cycle_absolute(&text, 6).expect(&text);
            text.replace_range(span, &replacement);
            seen.push(replacement);
        }
        assert_eq!(
            seen,
            ["$B$2:$B$4", "B$2:B$4", "$B2:$B4", "B2:B4", "$B$2:$B$4"]
        );
    }

    #[test]
    fn f4_does_nothing_where_there_is_no_reference() {
        assert!(cycle_absolute("=SUM(1;2)", 6).is_none());
        assert!(cycle_absolute("hello", 2).is_none());
    }
}
