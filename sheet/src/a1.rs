// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Addressing — **the only 0↔1 conversion in the workspace**, and the module every shell
//! addresses cells through.
//!
//! An address as a person writes one is ODF reference syntax without the brackets: `A1`,
//! `$B$7`, `A1:D20`, `Data.B2`, `'Q3 Actuals'.A1:.C9`. It is not parsed here. [`parse`]
//! wraps the string in `[…]` and hands it to the formula lexer, so a shell and a formula
//! cannot disagree about what an address means, and whole-column forms (`A:A`) work because
//! §5.8 already describes them.
//!
//! The inbound 1-based/0-based conversion is therefore not here either: `lex::Axis` is
//! already 0-based, so that half lives in the lexer where every caller shares it. The
//! outbound `+ 1` in [`format()`] is the only index arithmetic outside the lexer, and lives
//! here rather than in a shell — a second shell doing its own would be a second chance to
//! be off by one.

use crate::formula::lex::{self, Reference, Token};
use crate::{App, Error, Pos, Result};

/// A user's address as a reference.
///
/// The one piece of syntax a shell adds is the leading `.`: ODF writes a same-sheet
/// reference as `[.A1]`, and asking a user to type the dot would be noise. Each end of a
/// range gets its own, since `[.A1:.B2]` is the ODF form — but only when that end does not
/// already name a sheet, so `Data.B2:C3` becomes `[Data.B2:.C3]` and §5.8's rule that the
/// second end inherits the first's sheet does the rest.
pub fn parse(addr: &str) -> Result<Reference> {
    if addr.trim().is_empty() {
        return Err(Error::Formula("empty address".to_owned()));
    }
    let bracketed: Vec<String> = split_range(addr)
        .into_iter()
        .map(|part| match sheet_dot(&part) {
            // Already names a sheet: keep the name exactly as typed, since only the cell
            // half has a case rule.
            Some(dot) => format!("{}{}", &part[..=dot], part[dot + 1..].to_uppercase()),
            None => format!(".{}", part.to_uppercase()),
        })
        .collect();
    let source = format!("[{}]", bracketed.join(":"));
    parse_bracketed(&source).map_err(|e| Error::Formula(format!("{addr}: {e}")))
}

/// A reference already spelled the way the lexer wants it — `[Sheet1.B3:Sheet1.B9]` — with no
/// case-folding or quote-splitting, because the caller's own address already is one.
///
/// The tail of [`parse`], pulled out for `chart`'s `table:cell-range-address`
/// (`doc/chart-format.md`): ODF's own grammar for that attribute (rng:382) is the same
/// `sheet-name.COLROW[:sheet-name.COLROW]` shape a formula reference already is, just without
/// the `[…]` a *user's* typed address needs and a document's own attribute never has, so one
/// wrap-and-lex is every caller this format has.
pub fn parse_bracketed(source: &str) -> Result<Reference> {
    let tokens = lex::lex(source).map_err(|e| Error::Formula(e.to_string()))?;
    match tokens.as_slice() {
        [Token::Ref(reference)] => Ok(reference.clone()),
        _ => Err(Error::Formula(format!(
            "{source}: not a cell address or range"
        ))),
    }
}

/// Split on the `:` that separates the two ends of a range, ignoring one inside a quoted
/// sheet name — `'Q3: Actuals'.A1` is one end, not two.
fn split_range(addr: &str) -> Vec<String> {
    let mut parts = vec![String::new()];
    let mut quoted = false;
    for c in addr.chars() {
        match c {
            '\'' => {
                quoted = !quoted;
                parts.last_mut().expect("never empty").push(c);
            }
            ':' if !quoted => parts.push(String::new()),
            _ => parts.last_mut().expect("never empty").push(c),
        }
    }
    parts
}

/// Where the sheet name of this end stops, if it has one: the last `.` outside quotes.
///
/// Everything after it is the cell, and only the cell may be case-folded — §5.8 spells a
/// column `[A-Z]+`, so `data.b2` has to reach the lexer as `data.B2`, but a sheet called
/// `data` is not the same string as `DATA` and must survive untouched.
fn sheet_dot(part: &str) -> Option<usize> {
    let mut quoted = false;
    let mut last = None;
    for (i, c) in part.char_indices() {
        match c {
            '\'' => quoted = !quoted,
            '.' if !quoted => last = Some(i),
            _ => {}
        }
    }
    last
}

/// The sheet a reference names, and the inclusive cell range it covers.
///
/// A missing axis is a whole column or row (§5.8) and is clamped to the sheet's used extent
/// — the same bound the evaluator uses, and the reason `view A:A` reads what is there
/// instead of a million empty rows.
///
/// A reference past the end of the sheet is refused here rather than in the lexer, which
/// has to keep parsing whatever a foreign file says (R5). This is the only place an address
/// becomes a place, so it is the one place that can ask whether the place exists — and it
/// has to ask, because the grammar reads a bare word as a whole column: `SALES` is
/// `[.SALES]`, column 8708380, and a shell handed that scrolls off the end of the world.
pub fn resolve(app: &App, reference: &Reference) -> Result<(usize, Pos, Pos)> {
    let sheet = sheet_index(app, reference.start.sheet.as_deref())?;
    if reference.source.is_some() {
        return Err(Error::Formula(
            "external document references are out of scope".to_owned(),
        ));
    }
    let (rows, cols) = app.used_extent(sheet)?;

    let end = reference.end.as_ref().unwrap_or(&reference.start);
    // A whole column starts at row 0 and ends at the last used row; likewise a whole row.
    let start = Pos::new(axis(&reference.start.row, 0), axis(&reference.start.col, 0));
    let stop = Pos::new(
        axis(&end.row, rows.saturating_sub(1)),
        axis(&end.col, cols.saturating_sub(1)),
    );
    for pos in [start, stop] {
        if pos.row >= crate::MAX_ROWS || pos.col >= crate::MAX_COLS {
            return Err(Error::Formula(format!(
                "{} is past the end of the sheet",
                format_pos(pos)
            )));
        }
    }
    Ok((sheet, start, stop))
}

/// A position for an error message, without pretending a column past `XFD` has a name.
fn format_pos(pos: Pos) -> String {
    match pos.col < crate::MAX_COLS {
        true => format(None, pos),
        false => format!("column {}", pos.col + 1),
    }
}

fn axis(a: &Option<lex::Axis>, whole: u32) -> u32 {
    a.map_or(whole, |a| a.index)
}

/// A sheet by name, for the commands that address one directly rather than through a cell
/// reference. Same lookup, so `sheet rename data …` and `sheet set data.A1 …` cannot
/// disagree about which sheet `data` is.
pub fn sheet(app: &App, name: &str) -> Result<usize> {
    sheet_index(app, Some(name))
}

/// Sheet names match case-insensitively, as they do in a formula.
fn sheet_index(app: &App, name: Option<&str>) -> Result<usize> {
    let Some(name) = name else {
        return Ok(0);
    };
    (0..app.sheet_count())
        .find(|&i| {
            app.sheet_name(i)
                .is_ok_and(|n| n.eq_ignore_ascii_case(name))
        })
        .ok_or_else(|| Error::BadSheet(format!("no such sheet: {name}")))
}

/// A position as a user reads it. The only `+ 1` anywhere.
pub fn format(sheet: Option<&str>, pos: Pos) -> String {
    let cell = format!("{}{}", lex::column_name(pos.col), pos.row + 1);
    match sheet {
        Some(name) => format!("{name}.{cell}"),
        None => cell,
    }
}

/// A reference as a *name* has to be written: sheet-qualified and absolute on every axis.
///
/// Both halves matter, and neither is decoration. A named range with no sheet resolves
/// against whichever sheet the formula that mentions it sits on, so `total` would mean a
/// different range read from a second sheet; and a relative one shifts with the formula's
/// own position, so `SUM(total)` two rows down would sum two rows down. §5.11 names are
/// document-level, and this is what LibreOffice writes.
///
/// The sheet defaults to the first, which is what a user typing a bare `B2:B9` means.
pub fn as_definition(app: &App, reference: &Reference) -> Result<String> {
    if reference.source.is_some() {
        return Err(Error::Formula(
            "external document references are out of scope".to_owned(),
        ));
    }
    let default = app.sheet_name(0)?;
    let mut out = reference.clone();
    for end in [Some(&mut out.start), out.end.as_mut()]
        .into_iter()
        .flatten()
    {
        if end.sheet.is_none() {
            end.sheet = Some(default.clone());
        }
        end.sheet_absolute = true;
        for axis in [end.col.as_mut(), end.row.as_mut()].into_iter().flatten() {
            axis.absolute = true;
        }
    }
    Ok(out.to_string())
}

/// What a user typed where a name's *definition* goes, as an expression [`App::set_name`]
/// takes.
///
/// A leading `=` means a formula and everything else is an address, which is `set`'s rule
/// and therefore the one already in the user's hands. A named range and a named expression
/// are one thing in the model, so this is the only place the two spellings differ — and it
/// lives here rather than in a command so that a dialog and a command line cannot disagree
/// about what `B2:B4` defines.
pub fn definition(app: &App, target: &str) -> Result<String> {
    let target = target.trim();
    match target.strip_prefix('=') {
        Some(formula) => Ok(formula.to_owned()),
        // Brackets tolerated on the way in: a definition read back out of a document wears
        // them, and retyping what was shown has to work.
        None => {
            let bare = target
                .strip_prefix('[')
                .and_then(|rest| rest.strip_suffix(']'))
                .unwrap_or(target);
            as_definition(app, &parse(bare)?)
        }
    }
}

/// The reference covering a rectangle — what a shell builds when a user *points* at cells
/// rather than typing an address.
///
/// Relative on every axis, because that is what pointing means: the formula is being
/// written where it will live, and a reference typed by hand is relative unless a `$` says
/// otherwise. [`crate::formula::display::reference_text`] prints it the way a formula bar
/// shows it.
pub fn reference(sheet: Option<&str>, start: Pos, end: Pos) -> Reference {
    let cell = |pos: Pos| lex::CellRef {
        sheet: sheet.map(str::to_owned),
        sheet_absolute: sheet.is_some(),
        col: Some(lex::Axis {
            index: pos.col,
            absolute: false,
        }),
        row: Some(lex::Axis {
            index: pos.row,
            absolute: false,
        }),
    };
    Reference {
        source: None,
        start: cell(start),
        // One cell is one end: `[.B2]`, never `[.B2:.B2]`, which is what a user reads back.
        end: (start != end).then(|| cell(end)),
    }
}

/// Whether a reference names exactly one cell — what `get` needs and `view` does not.
pub fn is_single(reference: &Reference) -> bool {
    reference.end.is_none() && reference.start.row.is_some() && reference.start.col.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(addr: &str) -> Reference {
        parse(addr).expect(addr)
    }

    #[test]
    fn a_bare_address_gains_the_dot_odf_wants() {
        let r = at("A1");
        assert_eq!(r.start.sheet, None);
        assert_eq!(r.start.col.unwrap().index, 0);
        assert_eq!(r.start.row.unwrap().index, 0, "A1 is row 0 in the core");
        assert!(r.end.is_none());
        assert!(is_single(&r));
    }

    #[test]
    fn absolutes_and_ranges_come_from_the_lexer() {
        let r = at("$B$7");
        assert!(r.start.col.unwrap().absolute && r.start.row.unwrap().absolute);
        assert_eq!(r.start.col.unwrap().index, 1);
        assert_eq!(r.start.row.unwrap().index, 6);

        let r = at("A1:D20");
        assert_eq!(r.end.as_ref().unwrap().col.unwrap().index, 3);
        assert_eq!(r.end.as_ref().unwrap().row.unwrap().index, 19);
        assert!(!is_single(&r));
    }

    #[test]
    fn the_second_end_of_a_range_inherits_the_first_sheet() {
        // §5.8. The CLI writes `[Data.B2:.C3]`; the lexer fills the sheet in.
        let r = at("Data.B2:C3");
        assert_eq!(r.start.sheet.as_deref(), Some("Data"));
        assert_eq!(r.end.as_ref().unwrap().sheet.as_deref(), Some("Data"));
    }

    #[test]
    fn a_quoted_sheet_name_may_hold_spaces_dots_and_colons() {
        assert_eq!(
            at("'Q3 Actuals'.A1").start.sheet.as_deref(),
            Some("Q3 Actuals")
        );
        assert_eq!(
            at("'Q3.Actuals'.A1").start.sheet.as_deref(),
            Some("Q3.Actuals")
        );
        // The colon is inside the name, so this is one end and not a range.
        assert!(at("'Q3: Actuals'.A1").end.is_none());
    }

    /// §5.8 spells a column `[A-Z]+`, but nobody types that way at a shell. Only the cell
    /// half is folded — a sheet name is a name.
    #[test]
    fn the_cell_may_be_typed_in_lower_case_but_the_sheet_keeps_its_own() {
        assert_eq!(at("b2"), at("B2"));
        assert_eq!(at("$b$2"), at("$B$2"));
        assert_eq!(at("data.b2").start.sheet.as_deref(), Some("data"));
        assert_eq!(at("data.b2").start.col.unwrap().index, 1);
        assert_eq!(at("'my sheet'.a1").start.sheet.as_deref(), Some("my sheet"));
    }

    #[test]
    fn a_whole_column_has_no_row() {
        let r = at("A:A");
        assert!(r.start.row.is_none());
        assert!(!is_single(&r));
    }

    #[test]
    fn anything_that_is_not_an_address_is_rejected() {
        for bad in ["SUM(A1)", "", "  ", "1+1", "A1:B2:C3"] {
            assert!(parse(bad).is_err(), "{bad} should not parse as an address");
        }
    }

    /// A bare word is a whole column to the grammar, so `SALES` parses — but column 8708380
    /// is not a place on a sheet, and a shell told to go there scrolls into nothing. The
    /// parse still succeeds, because tolerance on the way in is the reader's rule too.
    #[test]
    fn a_reference_past_the_end_of_the_sheet_resolves_to_nothing() {
        let app = App::new();
        for word in ["SALES", "TOTAL", "ZZZZ"] {
            let reference = parse(word).expect("a bare word lexes as a whole column");
            assert!(
                resolve(&app, &reference).is_err(),
                "{word} is past the end of the sheet"
            );
        }
        // The last real cell is still reachable, and so is a whole column inside the sheet.
        assert!(resolve(&app, &at("XFD1048576")).is_ok());
        assert!(resolve(&app, &at("A:A")).is_ok());
    }

    #[test]
    fn formatting_is_the_inverse_of_parsing() {
        for addr in ["A1", "B7", "Z100", "AA1", "XFD1048576"] {
            let r = at(addr);
            let pos = Pos::new(r.start.row.unwrap().index, r.start.col.unwrap().index);
            assert_eq!(format(None, pos), addr);
        }
        assert_eq!(format(Some("Data"), Pos::new(1, 1)), "Data.B2");
    }

    /// The name box and `sheet name` share this rule: `=` means a formula, anything else an
    /// address that gets sheet-qualified and made absolute — a bracketed definition read
    /// back out of a document is accepted as-is, since retyping what was shown must work.
    #[test]
    fn a_leading_equals_is_a_formula_and_everything_else_an_address() {
        let app = App::new();
        assert_eq!(definition(&app, "B2:B4").unwrap(), "[$Sheet1.$B$2:.$B$4]");
        assert_eq!(definition(&app, "=MAX(total)").unwrap(), "MAX(total)");
        assert_eq!(
            definition(&app, "[$Sheet1.$B$2:.$B$4]").unwrap(),
            "[$Sheet1.$B$2:.$B$4]"
        );
        assert!(definition(&app, "not an address").is_err());
    }
}
