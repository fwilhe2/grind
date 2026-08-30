// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Renaming a sheet, in the formulas that name it — `doc/dsl.md` §6.5's first row, D10.
//!
//! [`shift`](super::shift) is the sibling: that one moves a reference to a different *cell*,
//! this one points it at a differently *named* sheet. Both are AST rewrites re-serialised
//! through the printer that already exists, because a textual substitution over a formula is
//! wrong in both directions — `Sales` appears inside `SalesTax` and inside `"Sales"`, and a
//! sheet whose name needs quoting is not spelled the same before and after
//! (`[$'Old Sheet'.A1]` → `[$New.A1]`).
//!
//! **Why this is a refactoring rather than a feature.** `doc/not-doing.md` recorded renaming a
//! sheet as a known loss: formulas naming it went stale and recalculated to `#REF!`. That is the
//! row `doc/dsl.md` §6.5 says to do first *because it fixes something that is currently wrong*,
//! and the shape it takes is the one that section argues for — a core operation returning an
//! `Action`, so rule 4 puts it on the CLI and rule 2 makes three hundred rewritten formulas one
//! Ctrl+Z.

use super::lex::{CellRef, Reference};
use super::parse::{Expr, intro, parse};

/// Whether this reference's own sheet locator names `from` — §5.11's case-insensitive match,
/// which is how the evaluator resolves one ([`super::eval::Engine::area`]).
fn names(cell: &CellRef, from: &str) -> bool {
    cell.sheet
        .as_deref()
        .is_some_and(|sheet| sheet.eq_ignore_ascii_case(from))
}

fn rename_cellref(cell: &CellRef, from: &str, to: &str) -> CellRef {
    match names(cell, from) {
        true => CellRef {
            sheet: Some(to.to_owned()),
            ..cell.clone()
        },
        false => cell.clone(),
    }
}

/// One reference with `from` renamed to `to`.
///
/// Public because a formula is not the only thing in a document that names a sheet: a chart
/// holds its ranges as address *strings* (`chart::Series::values`), and rewriting one means
/// parsing it, coming through here, and printing it back the way charts spell theirs.
pub fn rename_in_reference(r: &Reference, from: &str, to: &str) -> Reference {
    Reference {
        // An external document's sheets are not this document's, so a reference into one is
        // left exactly as it is (§5.8 Source) — the same line `Engine::area` draws.
        source: r.source.clone(),
        start: match r.source.is_some() {
            true => r.start.clone(),
            false => rename_cellref(&r.start, from, to),
        },
        end: r.end.as_ref().map(|end| match r.source.is_some() {
            true => end.clone(),
            false => rename_cellref(end, from, to),
        }),
    }
}

/// Every reference in `expr` that names `from`, pointed at `to` instead.
pub fn rename(expr: &Expr, from: &str, to: &str) -> Expr {
    match expr {
        Expr::Ref(r) => Expr::Ref(rename_in_reference(r, from, to)),
        Expr::Call { name, args } => Expr::Call {
            name: name.clone(),
            args: args.iter().map(|a| rename(a, from, to)).collect(),
        },
        Expr::Prefix(op, e) => Expr::Prefix(*op, Box::new(rename(e, from, to))),
        Expr::Postfix(op, e) => Expr::Postfix(*op, Box::new(rename(e, from, to))),
        Expr::Binary(op, l, r) => Expr::Binary(
            *op,
            Box::new(rename(l, from, to)),
            Box::new(rename(r, from, to)),
        ),
        Expr::Paren(e) => Expr::Paren(Box::new(rename(e, from, to))),
        // A named expression is a name, not a reference: what it *stands for* names the sheet,
        // and that text is rewritten where it is stored (`Document::names`) rather than here.
        Expr::Number(_) | Expr::Text(_) | Expr::Error(_) | Expr::Name(_) | Expr::Empty => {
            expr.clone()
        }
    }
}

/// One stored formula with `from` renamed to `to`, or `None` when nothing changed.
///
/// `None` covers three cases a caller treats alike and this function distinguishes on purpose:
/// the formula does not name that sheet, the formula does not parse — the same honesty
/// `App::fill` shows a formula it cannot read, since rewriting text it did not understand is how
/// a refactoring corrupts a document — and the rewrite is the text that was already there.
///
/// The **intro is kept**: a document spelling its formulas `of:=` gets them back that way, so
/// renaming one sheet does not respell every formula in the file (R6).
pub fn rename_in_formula(formula: &str, from: &str, to: &str) -> Option<String> {
    let expr = parse(formula).ok()?;
    let renamed = rename(&expr, from, to);
    if renamed == expr {
        return None;
    }
    let rewritten = format!("{}{renamed}", intro(formula));
    (rewritten != formula).then_some(rewritten)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn renamed(formula: &str, from: &str, to: &str) -> Option<String> {
        rename_in_formula(formula, from, to)
    }

    #[test]
    fn a_reference_follows_the_sheet_it_names() {
        assert_eq!(
            renamed("of:=SUM([Data.A1:.A9])", "Data", "Figures").as_deref(),
            Some("of:=SUM([Figures.A1:.A9])")
        );
        // Both ends, when they really are two sheets.
        assert_eq!(
            renamed("=[Data.A1]+[Other.A1]", "Other", "Third").as_deref(),
            Some("=[Data.A1]+[Third.A1]")
        );
        // Every corner of the AST is walked, not just the top.
        assert_eq!(
            renamed("=IF([Data.A1]>0;-([Data.B1]%);\"Data\")", "Data", "D").as_deref(),
            Some("=IF([D.A1]>0;-([D.B1]%);\"Data\")"),
            "a string that happens to spell the name is text, not a reference"
        );
    }

    #[test]
    fn the_match_is_case_insensitive_and_the_new_spelling_is_the_one_given() {
        assert_eq!(
            renamed("=[dATA.A1]", "Data", "Figures").as_deref(),
            Some("=[Figures.A1]"),
            "§5.11 matches names case-insensitively, so the evaluator's answer is the rule"
        );
    }

    #[test]
    fn a_name_that_needs_quoting_is_quoted_and_one_that_does_not_is_not() {
        assert_eq!(
            renamed("=[$'Old Sheet'.$A$1]", "Old Sheet", "New").as_deref(),
            Some("=[$New.$A$1]"),
            "the absolute marker and the absolute axes survive; only the name changes"
        );
        let quoted = renamed("=[Data.A1]", "Data", "New Sheet").expect("it changed");
        assert_eq!(
            quoted, "=['New Sheet'.A1]",
            "a name with a space in it comes back quoted — which is why this is not a textual \
             substitution — and the `$` is the reference's own absoluteness, not the quoting"
        );
        // And what comes out reads back in, which is the property that makes a rewrite safe.
        assert_eq!(
            parse(&quoted).expect("the rewrite parses").to_string(),
            "['New Sheet'.A1]"
        );
    }

    #[test]
    fn nothing_is_rewritten_that_did_not_change() {
        assert_eq!(
            renamed("=[.A1]+1", "Data", "Figures"),
            None,
            "no sheet named"
        );
        assert_eq!(renamed("=[Other.A1]", "Data", "Figures"), None);
        // A formula this build cannot parse is left exactly as it is: rewriting text nobody
        // understood is how a refactoring corrupts a document.
        assert_eq!(renamed("=SUM(", "Data", "Figures"), None);
        assert_eq!(
            renamed("=BESSELJ([Data.A1];1)", "Data", "D").as_deref(),
            Some("=BESSELJ([D.A1];1)"),
            "an unimplemented *function* still parses, so its references are still rewritten"
        );
    }

    #[test]
    fn an_external_documents_sheet_is_not_this_documents_sheet() {
        assert_eq!(
            renamed("=['file:///other.ods'#Data.A1]", "Data", "Figures"),
            None,
            "§5.8: a reference into a document that is not open names somebody else's sheet"
        );
    }
}
