// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Display form — the formula a person types, and the canonical one a document stores.
//!
//! A document stores `of:=SUM([.B2:.B4])`; a formula bar shows `=SUM(B2:B4)`. The
//! difference is the brackets around references and nothing else — the `;` separator, the
//! function names and the operators are the same, because there is deliberately no
//! translation from another spreadsheet's syntax here (`CLAUDE.md`).
//!
//! Both directions are one code path each, and neither is a second parser:
//!
//! * [`to_display`] parses and re-serialises through `serialize::Bare`, so precedence and
//!   parenthesisation are the ones the canonical printer already computes.
//! * [`from_display`] *scans* — it finds the reference-shaped runs, re-brackets them, and
//!   hands the result to the existing lexer and parser to validate. So a formula that
//!   would not parse is rejected by the same grammar that reads the file, and the only
//!   thing this module decides is where a reference starts and stops.
//! * [`spans`] is that same scanner exposed, in **byte** ranges because Pango attribute
//!   indices are bytes. The editor's colourer and the committer therefore cannot disagree
//!   about what a reference is: there is one scanner.
//!
//! ## Disambiguation, which is the whole difficulty
//!
//! Unbracketed, a reference and a name are the same shape. The rules, in the order the
//! scanner applies them:
//!
//! * an identifier immediately followed by `(` is a **function** — so `LOG10(` is a call;
//! * an identifier that is cell-shaped is a **reference** — so a bare `LOG10` is cell
//!   LOG10. Excel's exact collision and Excel's resolution. It is unambiguous against
//!   defined names because [`crate::App::set_name`] already refuses a cell-shaped name;
//! * a letters-only or digits-only run is a reference only in a range (`B:B`, `2:2`) or
//!   with a sheet (`Data.B`) — otherwise `SALES` is a name, which is most of the words
//!   anyone gives a range;
//! * any other bare identifier is a **name**, passed through untouched;
//! * bare `TRUE` / `FALSE` become `TRUE()` / `FALSE()`, the functions §6.15 defines;
//! * an already-bracketed `[…]` is passed through verbatim, which is how the two forms
//!   [`to_display`] leaves bracketed survive: an external-source reference, and the range
//!   *operator* applied to two references (`[Sheet2.C22]:[.C33]`, whose second end does
//!   **not** inherit the first's sheet the way `[Sheet2.C22:.C33]` does).
//!
//! Function names keep the case they were typed in, as `sheet set` stores them: §5.6 makes
//! them case-insensitive, and normalising them would be this module rewriting a document's
//! formulas for cosmetics.
//!
//! One thing display form cannot spell: an **unquoted sheet name that does not start with
//! a letter**. `2024.A1` reads as a number followed by junk, so the scanner leaves it
//! alone; `'2024'.A1` works. The canonical printer already quotes any sheet name
//! containing a `.`, `$` or space, so nothing [`to_display`] writes can hit this.

use std::fmt;
use std::ops::Range;

use crate::a1;

use super::lex::{Reference, SyntaxError};
use super::parse::{Expr, parse};
use super::serialize::Bare;

/// What a scanned run of display-form text is.
///
/// Only the runs that *mean* something are reported; operators, separators, parentheses
/// and whitespace are the gaps between them, which [`from_display`] copies verbatim and a
/// colourer leaves in the default colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenKind {
    /// A reference, bracketed or not (§5.8).
    Ref,
    /// An identifier immediately followed by `(` (§5.6).
    Func,
    /// A named expression (§5.11).
    Name,
    /// A bare `TRUE` or `FALSE`, which canonical form spells as a call.
    Bool,
    /// A string literal, quotes included (§5.4).
    Text,
    /// A constant error, `#N/A` and friends (§5.12).
    Error,
    Number,
}

/// One scanned run, as a **byte** range into the text that was scanned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub range: Range<usize>,
    pub kind: TokenKind,
}

/// A display-form formula that will not convert, and **where** — a byte offset into the
/// text, so an editor can put the caret on it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayError {
    pub message: String,
    /// Byte offset into the display text.
    pub at: usize,
}

impl fmt::Display for DisplayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at byte {}", self.message, self.at)
    }
}

impl std::error::Error for DisplayError {}

/// Canonical → display form: `of:=SUM([.B2:.B4])` → `=SUM(B2:B4)`.
///
/// The leading `=` is part of the answer, because that is what a formula bar shows and what
/// [`from_display`] accepts back.
pub fn to_display(canonical: &str) -> Result<String, SyntaxError> {
    Ok(format!("={}", Bare(&parse(canonical)?)))
}

/// One reference, in display form — `B2`, `Data.B2:C9`, `$A$1`.
///
/// The same printer the whole formula goes through, so a reference a shell writes into an
/// editor is spelled exactly as the one [`to_display`] would have shown there.
pub fn reference_text(reference: &Reference) -> String {
    Bare(&Expr::Ref(reference.clone())).to_string()
}

/// Display form → canonical: `=SUM(B2:B4)` → `=SUM([.B2:.B4])`.
///
/// Validated and normalised by the *existing* lexer and parser, so what comes back is a
/// formula the file format can hold and the evaluator can read — never a rewrite of text
/// that happened to look right.
pub fn from_display(text: &str) -> Result<String, DisplayError> {
    // The intro is stripped rather than copied, so the offsets the parser reports are
    // offsets into what this function assembled and `origin` can map them straight back.
    let body_start = usize::from(text.starts_with('='));

    let mut out = String::new();
    // One source byte offset per char of `out` — a formula is short and a lookup table is
    // shorter than an interval map.
    let mut origin: Vec<usize> = Vec::new();
    let mut cursor = body_start;

    for span in spans(text) {
        if span.range.end <= body_start {
            continue;
        }
        emit(
            &mut out,
            &mut origin,
            &text[cursor..span.range.start],
            cursor,
            true,
        );
        let source = &text[span.range.clone()];
        let at = span.range.start;
        match span.kind {
            // Already bracketed: an external-source reference, left exactly as it stands.
            TokenKind::Ref if source.starts_with('[') => {
                emit(&mut out, &mut origin, source, at, true)
            }
            TokenKind::Ref => {
                let reference = a1::parse(source).map_err(|e| DisplayError {
                    message: e.to_string(),
                    at,
                })?;
                emit(&mut out, &mut origin, &reference.to_string(), at, false);
            }
            TokenKind::Bool => {
                emit(
                    &mut out,
                    &mut origin,
                    &format!("{}()", source.to_uppercase()),
                    at,
                    false,
                );
            }
            _ => emit(&mut out, &mut origin, source, at, true),
        }
        cursor = span.range.end;
    }
    emit(&mut out, &mut origin, &text[cursor..], cursor, true);

    let expr = parse(&out).map_err(|e| DisplayError {
        message: e.message,
        // `at` is a char offset into `out`; past the end means "at the end of the input".
        at: origin.get(e.at).copied().unwrap_or(text.len()),
    })?;
    Ok(format!("={expr}"))
}

/// Append `text` to `out`, recording where each char came from.
///
/// `verbatim` says whether the chars line up with the source: a rewritten run (`b2` →
/// `.B2`) has no per-char correspondence, so every char of it points at the run's start.
fn emit(out: &mut String, origin: &mut Vec<usize>, text: &str, src: usize, verbatim: bool) {
    for (i, c) in text.char_indices() {
        origin.push(if verbatim { src + i } else { src });
        out.push(c);
    }
}

/// Whether a bare word, written into display form, would be read back as a **reference**
/// rather than as the name it is — the `LOG10` collision, from the other side.
///
/// A document written elsewhere may declare a cell-shaped name (`date1`), which
/// [`crate::App::set_name`] refuses to create precisely because of this. Printing one
/// unbracketed produces text that means a different thing when it is read back, so anything
/// that *substitutes* a name into display form has to ask this first — `view::Names` does.
///
/// Asked of the scanner rather than of a second rule about what a cell looks like: there is
/// one scanner, and this is it answering a question about one word.
pub fn reads_as_reference(word: &str) -> bool {
    matches!(
        spans(word).as_slice(),
        [span] if span.kind == TokenKind::Ref && span.range == (0..word.len())
    )
}

/// Scan display-form text into the runs that mean something, in byte ranges.
///
/// Total: anything it does not recognise is a gap, so a half-typed formula scans without
/// complaint and an editor can colour what is there.
pub fn spans(text: &str) -> Vec<Span> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i].1;
        let start = i;
        match c {
            '"' => {
                i = quoted(&chars, i, '"');
                out.push(span(&chars, text, start, i, TokenKind::Text));
            }
            // §5.4's sibling: a `'…'` outside a reference is part of a `$$'name'`. Skipped
            // whole so that nothing inside it is scanned as a reference.
            '\'' => {
                i = match scan_ref(&chars, i) {
                    Some(end) => {
                        out.push(span(&chars, text, start, end, TokenKind::Ref));
                        end
                    }
                    None => quoted(&chars, i, '\''),
                };
            }
            '[' => {
                i = bracketed(&chars, i);
                out.push(span(&chars, text, start, i, TokenKind::Ref));
            }
            '#' => {
                i += 1;
                while matches!(chars.get(i), Some((_, c)) if c.is_ascii_alphanumeric() || *c == '/' || *c == '_')
                {
                    i += 1;
                }
                if matches!(chars.get(i), Some((_, '!' | '?'))) {
                    i += 1;
                }
                out.push(span(&chars, text, start, i, TokenKind::Error));
            }
            _ if is_name_start(c) => {
                // A function name is the one thing decided before anything else, because
                // `LOG10(` is a call and `LOG10` alone is a cell.
                let ident = dotted_ident(&chars, i);
                if matches!(chars.get(ident), Some((_, '('))) {
                    out.push(span(&chars, text, start, ident, TokenKind::Func));
                    i = ident;
                } else if let Some(end) = scan_ref(&chars, i) {
                    out.push(span(&chars, text, start, end, TokenKind::Ref));
                    i = end;
                } else {
                    let end = plain_ident(&chars, i);
                    let word: String = chars[start..end].iter().map(|(_, c)| *c).collect();
                    let kind = match word.eq_ignore_ascii_case("TRUE")
                        || word.eq_ignore_ascii_case("FALSE")
                    {
                        true => TokenKind::Bool,
                        false => TokenKind::Name,
                    };
                    out.push(span(&chars, text, start, end, kind));
                    i = end;
                }
            }
            '$' | '0'..='9' | '.' => {
                // `$A$1`, `$Data.$A$1` and the whole-row form `2:2` all start here; so does
                // an ordinary number, and only trying tells them apart.
                match scan_ref(&chars, i) {
                    Some(end) => {
                        out.push(span(&chars, text, start, end, TokenKind::Ref));
                        i = end;
                    }
                    None if c.is_ascii_digit() || c == '.' => {
                        i = number(&chars, i);
                        if i > start {
                            out.push(span(&chars, text, start, i, TokenKind::Number));
                        } else {
                            i = start + 1;
                        }
                    }
                    None => i += 1,
                }
            }
            _ => i += 1,
        }
    }
    out
}

fn span(chars: &[(usize, char)], text: &str, from: usize, to: usize, kind: TokenKind) -> Span {
    let byte = |i: usize| chars.get(i).map_or(text.len(), |(b, _)| *b);
    Span {
        range: byte(from)..byte(to),
        kind,
    }
}

fn is_name_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

/// A `FunctionName` — dots included, which is what makes `COM.MICROSOFT.CUBEMEMBER` one
/// token (§5.7).
fn dotted_ident(chars: &[(usize, char)], mut i: usize) -> usize {
    while matches!(chars.get(i), Some((_, c)) if c.is_alphanumeric() || *c == '_' || *c == '.') {
        i += 1;
    }
    i
}

/// An `Identifier` (§5.11): no dots, so a sheet name stops at the separator.
fn plain_ident(chars: &[(usize, char)], mut i: usize) -> usize {
    while matches!(chars.get(i), Some((_, c)) if c.is_alphanumeric() || *c == '_') {
        i += 1;
    }
    i
}

/// Past the closing `quote`, a doubled one being a literal (§5.2, §5.4).
fn quoted(chars: &[(usize, char)], mut i: usize, quote: char) -> usize {
    i += 1;
    while i < chars.len() {
        if chars[i].1 == quote {
            if matches!(chars.get(i + 1), Some((_, c)) if *c == quote) {
                i += 2;
                continue;
            }
            return i + 1;
        }
        i += 1;
    }
    i
}

/// Past the `]` closing a bracketed reference, quoted names and IRIs skipped whole.
fn bracketed(chars: &[(usize, char)], mut i: usize) -> usize {
    i += 1;
    while i < chars.len() {
        match chars[i].1 {
            ']' => return i + 1,
            '\'' => i = quoted(chars, i, '\''),
            _ => i += 1,
        }
    }
    i
}

/// §5.3 Constant Number, so that `1.5e-3` is one run and `1:3` is not a number at all.
fn number(chars: &[(usize, char)], mut i: usize) -> usize {
    while matches!(chars.get(i), Some((_, c)) if c.is_ascii_digit()) {
        i += 1;
    }
    if matches!(chars.get(i), Some((_, '.'))) {
        i += 1;
        while matches!(chars.get(i), Some((_, c)) if c.is_ascii_digit()) {
            i += 1;
        }
    }
    if matches!(chars.get(i), Some((_, 'e' | 'E'))) {
        let mut j = i + 1;
        if matches!(chars.get(j), Some((_, '+' | '-'))) {
            j += 1;
        }
        if matches!(chars.get(j), Some((_, c)) if c.is_ascii_digit()) {
            i = j;
            while matches!(chars.get(i), Some((_, c)) if c.is_ascii_digit()) {
                i += 1;
            }
        }
    }
    i
}

/// Where a reference starting at `i` ends, if one does.
///
/// A single end has to be *complete* — both axes, or a sheet — because a bare `SALES` is a
/// name. Inside a range either end may be half open, which is what `B:B` and `2:2` are.
fn scan_ref(chars: &[(usize, char)], i: usize) -> Option<usize> {
    let (first, complete) = scan_end(chars, i)?;
    let mut end = complete.then_some(first).filter(|e| ends_here(chars, *e));
    if matches!(chars.get(first), Some((_, ':')))
        && let Some((second, _)) = scan_end(chars, first + 1)
        && ends_here(chars, second)
    {
        end = Some(second);
    }
    end
}

/// Whether a reference really stops at `i`.
///
/// `A1B` and `A1.x` are not references with something after them; they are not references,
/// and stopping mid-identifier is how a scanner silently corrupts a name. The `(` is the
/// same rule one step out: `$O6:CHOOSE(…)` is a range whose second end is a *call*, so the
/// range candidate is rejected and the `$O6` on its own is what remains.
fn ends_here(chars: &[(usize, char)], i: usize) -> bool {
    !matches!(chars.get(i), Some((_, c))
        if c.is_alphanumeric() || *c == '_' || *c == '.' || *c == '(')
}

/// One end of a reference: `('$'? SheetName '.')? '$'? Column? '$'? Row?`.
///
/// Returns where it ends and whether it stands on its own — both axes, or a sheet to
/// qualify a whole column.
fn scan_end(chars: &[(usize, char)], start: usize) -> Option<(usize, bool)> {
    let mut i = start;
    let mut sheet = false;

    // The sheet locator is only taken when the `.` is actually there; otherwise `$A$1`'s
    // first `$` would be eaten as the sheet-absolute marker and the column would lose it.
    let mut j = i + usize::from(matches!(chars.get(i), Some((_, '$'))));
    let name_end = match chars.get(j) {
        Some((_, '\'')) => Some(quoted(chars, j, '\'')),
        // A sheet name that does not start with a letter has to be quoted here — see the
        // module docs. Unquoted, `2024.A1` is a number and this scanner leaves it alone.
        Some((_, c)) if is_name_start(*c) => Some(plain_ident(chars, j)),
        _ => None,
    };
    if let Some(name_end) = name_end
        && matches!(chars.get(name_end), Some((_, '.')))
    {
        sheet = true;
        i = name_end + 1;
        j = i;
    }
    let _ = j;

    if matches!(chars.get(i), Some((_, '$'))) {
        i += 1;
    }
    let letters = i;
    while matches!(chars.get(i), Some((_, c)) if c.is_ascii_alphabetic()) {
        i += 1;
    }
    let has_col = i > letters;
    let digits_marker = i;
    if matches!(chars.get(i), Some((_, '$'))) {
        i += 1;
    }
    let digits = i;
    while matches!(chars.get(i), Some((_, c)) if c.is_ascii_digit()) {
        i += 1;
    }
    let has_row = i > digits;
    if !has_row {
        // A `$` with no row behind it belongs to whatever comes next, not to this end.
        i = digits_marker;
    }
    if !has_col && !has_row {
        return None;
    }
    Some((i, (has_col && has_row) || sheet))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn there_and_back(canonical: &str) -> String {
        let display = to_display(canonical).expect(canonical);
        from_display(&display).expect(&display)
    }

    #[test]
    fn a_reference_loses_its_brackets_and_gets_them_back() {
        for (canonical, display) in [
            ("=[.B2]", "=B2"),
            ("=SUM([.B2:.B4])", "=SUM(B2:B4)"),
            ("=[$Data.$A$1]", "=$Data.$A$1"),
            ("=[.B:.B]", "=B:B"),
            ("=[.1:.3]", "=1:3"),
            ("=['Q3 Actuals'.A1:.C9]", "='Q3 Actuals'.A1:C9"),
            ("=[Sheet1.A1:Sheet3.B2]", "=Sheet1.A1:Sheet3.B2"),
        ] {
            assert_eq!(to_display(canonical).expect(canonical), display);
            assert_eq!(from_display(display).expect(display), canonical);
        }
    }

    #[test]
    fn an_external_reference_keeps_its_brackets_in_both_directions() {
        // Nothing evaluates one and the syntax has no bare spelling for `'…'#`.
        let canonical = "=['file:///tmp/other.ods'#Sheet1.A1]";
        assert_eq!(to_display(canonical).unwrap(), canonical);
        assert_eq!(there_and_back(canonical), canonical);
    }

    #[test]
    fn a_function_is_told_from_a_cell_by_the_parenthesis() {
        // Excel's exact collision: LOG10 is a function and a cell address.
        assert_eq!(from_display("=LOG10(2)").unwrap(), "=LOG10(2)");
        assert_eq!(from_display("=LOG10").unwrap(), "=[.LOG10]");
        assert_eq!(
            from_display("=COM.MICROSOFT.X(1)").unwrap(),
            "=COM.MICROSOFT.X(1)"
        );
    }

    #[test]
    fn a_bare_word_that_is_not_cell_shaped_stays_a_name() {
        assert_eq!(from_display("=SUM(SALES)").unwrap(), "=SUM(SALES)");
        assert_eq!(from_display("=$$'year end'").unwrap(), "=$$'year end'");
        // The quoted part of a `$$'…'` is skipped whole, so nothing inside it is scanned.
        assert_eq!(from_display("=$$'A1 and B2'").unwrap(), "=$$'A1 and B2'");
    }

    #[test]
    fn a_pointed_reference_prints_the_way_it_would_be_typed() {
        use crate::Pos;
        let one = a1::reference(None, Pos::new(1, 1), Pos::new(1, 1));
        assert_eq!(reference_text(&one), "B2");
        let range = a1::reference(None, Pos::new(1, 1), Pos::new(3, 1));
        assert_eq!(reference_text(&range), "B2:B4");
        let elsewhere = a1::reference(Some("Data"), Pos::new(0, 0), Pos::new(8, 2));
        assert_eq!(reference_text(&elsewhere), "$Data.A1:C9");
        // And what it prints is what the scanner reads back.
        for reference in [one, range, elsewhere] {
            let text = reference_text(&reference);
            assert_eq!(
                from_display(&format!("=SUM({text})")).expect(&text),
                format!("=SUM({})", reference)
            );
        }
    }

    #[test]
    fn a_range_operator_between_two_references_keeps_its_brackets() {
        // Not the same expression as `[Sheet2.C22:.C33]`, and display form cannot tell them
        // apart — so this one is shown as it is stored.
        let canonical = "=AND([Sheet2.C22]:[.C33])";
        assert_eq!(to_display(canonical).unwrap(), canonical);
        assert_eq!(there_and_back(canonical), canonical);
        // A range whose second end is a call is the same story.
        let canonical = "=SUM([.$O6]:CHOOSE([.$H$2];[.$P6]))";
        assert_eq!(there_and_back(canonical), canonical);
    }

    #[test]
    fn lower_case_is_folded_the_way_the_command_line_folds_it() {
        assert_eq!(from_display("=sum(b2:b4)").unwrap(), "=sum([.B2:.B4])");
        assert_eq!(from_display("=data.b2").unwrap(), "=[data.B2]");
    }

    #[test]
    fn bare_logicals_become_the_functions_they_are() {
        // §6.15.9, §6.15.2: TRUE and FALSE are functions, and every other spreadsheet lets
        // you type them without the parentheses.
        assert_eq!(from_display("=TRUE").unwrap(), "=TRUE()");
        assert_eq!(
            from_display("=IF(A1;true;FALSE)").unwrap(),
            "=IF([.A1];TRUE();FALSE())"
        );
    }

    #[test]
    fn text_and_numbers_are_not_scanned_for_references() {
        assert_eq!(from_display("=\"A1 is B2\"").unwrap(), "=\"A1 is B2\"");
        assert_eq!(from_display("=1.5e-3+2").unwrap(), "=0.0015+2");
        assert_eq!(from_display("=A1&\"x\"").unwrap(), "=[.A1]&\"x\"");
    }

    #[test]
    fn an_error_constant_survives() {
        assert_eq!(from_display("=IF(A1;#N/A;1)").unwrap(), "=IF([.A1];#N/A;1)");
    }

    #[test]
    fn what_will_not_parse_says_where() {
        // §5.6 allows an empty parameter, so the broken formula has to be really broken.
        let e = from_display("=SUM(B2))").unwrap_err();
        assert_eq!(
            &"=SUM(B2))"[e.at..],
            ")",
            "the caret lands on the trailing `)`"
        );
        // The offset is a *byte* offset into the display text, past a multi-byte name.
        let text = "=SUMÅÅ(1))";
        let e = from_display(text).unwrap_err();
        assert!(
            text.is_char_boundary(e.at),
            "a byte offset lands on a char boundary"
        );
        assert_eq!(&text[e.at..], ")");
    }

    #[test]
    fn spans_are_byte_ranges_the_editor_can_colour() {
        let text = "=SUM(B2:B4;Data.A1)";
        let found = spans(text);
        let kinds: Vec<_> = found
            .iter()
            .map(|s| (&text[s.range.clone()], s.kind))
            .collect();
        assert_eq!(
            kinds,
            [
                ("SUM", TokenKind::Func),
                ("B2:B4", TokenKind::Ref),
                ("Data.A1", TokenKind::Ref),
            ]
        );
    }

    #[test]
    fn a_half_typed_formula_scans_without_complaint() {
        // The editor calls this on every keystroke, including `=SUM(B2:`.
        for text in ["=", "=SUM(", "=SUM(B2:", "=SUM(B2:B", "=A", "='"] {
            let _ = spans(text);
        }
        assert_eq!(
            spans("=SUM(B2:").iter().map(|s| s.kind).collect::<Vec<_>>(),
            [TokenKind::Func, TokenKind::Ref]
        );
    }
}
