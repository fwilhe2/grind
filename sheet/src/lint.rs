// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The spreadsheet's lint rules — `doc/dsl.md` §4.3, D6.
//!
//! The twin of `grind_text::lint`, and the same argument: the interesting rules are about
//! documents rather than about scripts, so they live with the document type whose vocabulary
//! they are written in. [`grind_core::lint`] holds only what a diagnostic *is* (R8).
//!
//! **Nothing here has a second opinion.** Every rule is asked through the machinery that
//! already answers its question — [`crate::graph::RefIndex`] for what a formula reads (which
//! resolves through `Engine::area`, the evaluator's own function), and `recalculated` for
//! whether a cached value is still true, which is what [`crate::App::stale`] counts. A linter
//! that resolved references its own way would disagree with the document's behaviour on
//! exactly the cases worth reporting.
//!
//! Addresses are `a1`'s, sheet-qualified, so every one of them is a string a user can type
//! back at `grind sheet get`.

use grind_core::lint::{Diagnostic, Options, Report, Rule, Severity};
use grind_core::style::PALETTE;

use crate::a1;
use crate::formula::eval::Address;
use crate::formula::lex::CellRef;
use crate::formula::parse::{Expr, parse};
use crate::graph::RefIndex;
use crate::model::{Document, Pos};
use crate::style::CellStyle;

/// A cell whose cached value is not what its formula computes.
///
/// Loop B's check, pointed at one document. The two are separate claims about the same cell —
/// it says `SUM(B2:B4)` and it says `1500` — and ODF has no dirty bit, so every reader
/// including LibreOffice shows the stale one until something recalculates.
///
/// **Only where this build can evaluate the formula.** A cell that would recalculate to an
/// error it does not currently hold is this engine's gap rather than the document's, and is the
/// `spoiled` count [`crate::App::recalc`] already reports; calling it a stale value would be
/// blaming a document for a function we have not implemented.
pub const STALE_VALUE: Rule = Rule {
    id: "stale-value",
    severity: Severity::Warning,
    what: "a cell whose cached value disagrees with its formula",
};

/// A formula naming a sheet the document does not have.
///
/// What deleting a sheet leaves behind (`doc/not-doing.md` §3, and the first row of §6.5's
/// refactoring table). An error: the cell reads `#REF!` the moment anything recalculates.
pub const MISSING_SHEET: Rule = Rule {
    id: "missing-sheet",
    severity: Severity::Error,
    what: "a formula referencing a sheet that does not exist",
};

/// A formula reading a named, single cell that holds nothing.
///
/// **Single-cell references only.** An empty cell inside `SUM([.B2:.B99])` is ordinary — a
/// range is written for the data it will hold — and reporting each one would bury every other
/// rule. A formula that names one cell and finds it empty is the typo case: a column shifted,
/// a row deleted, a reference pointing one cell past the data.
pub const EMPTY_REFERENCE: Rule = Rule {
    id: "empty-reference",
    severity: Severity::Warning,
    what: "a formula reading a cell that is empty",
};

/// A colour that is not one of `grind_core::style::PALETTE`'s. `grind_text::lint::OFF_PALETTE`
/// is the same rule for prose, and the same reasoning: a default a shell offers, never a limit,
/// so a hint and off unless asked for.
pub const OFF_PALETTE: Rule = Rule {
    id: "off-palette",
    severity: Severity::Hint,
    what: "a colour outside the default palette",
};

/// Something in the document the projection cannot spell — charts, for this application
/// (`doc/projection-sheet.md`'s one named gap).
///
/// The bijectivity guard as a diagnostic: converting this document to a `.grind` and back
/// would return everything except what this rule lists, by name and by address.
pub const UNSPELLABLE: Rule = Rule {
    id: "unspellable",
    severity: Severity::Warning,
    what: "a construct the projection cannot spell",
};

/// Every rule this application has, in the order `grind sheet lint --rules` prints them.
pub const RULES: [Rule; 5] = [
    STALE_VALUE,
    MISSING_SHEET,
    EMPTY_REFERENCE,
    OFF_PALETTE,
    UNSPELLABLE,
];

/// Check a document against every rule `options` wants.
pub fn lint(doc: &Document, options: &Options) -> Report {
    let mut report = Report::default();
    if options.wants(&STALE_VALUE) {
        stale_values(doc, &mut report);
    }
    if options.wants(&MISSING_SHEET) {
        missing_sheets(doc, &mut report);
    }
    if options.wants(&EMPTY_REFERENCE) {
        empty_references(doc, &mut report);
    }
    if options.wants(&OFF_PALETTE) {
        off_palette(doc, &mut report);
    }
    if options.wants(&UNSPELLABLE) {
        unspellable(doc, &mut report);
    }
    report.sort();
    report
}

/// `Sheet1.B12` — a cell as a person types it, which is what every rule here reports against.
fn at(doc: &Document, address: Address) -> String {
    let name = doc.sheets.get(address.sheet).map(|s| s.name.as_str());
    a1::format(name, address.pos)
}

fn stale_values(doc: &Document, report: &mut Report) {
    for (sheet, pos, computed) in crate::stale_cells(doc) {
        let previous = doc.sheets[sheet].get(pos);
        // **No answer is not a wrong answer.** A formula cell with no cached value is how a
        // document written by hand looks — `doc/dsl.md` §3.4, and `cell B5 "=SUM([.B2:.B4])"`
        // is the *normal* spelling in a projection — so reporting one would make the feature's
        // own documents fail their own linter. `grind sheet recalc` fills them in; nothing
        // about the document contradicts itself until it does.
        if previous.is_empty() {
            continue;
        }
        let at = at(doc, Address::new(sheet, pos));
        if !report.push(Diagnostic::new(
            &STALE_VALUE,
            at,
            format!(
                "holds {} and its formula computes {}",
                shown(&previous),
                shown(&computed)
            ),
        )) {
            return;
        }
    }
}

/// A cell value as a diagnostic spells it — short, and quoted when it is text, so that an
/// empty cell and an empty string do not read the same.
fn shown(value: &crate::CellValue) -> String {
    use crate::CellValue::*;
    match value {
        Empty => "nothing".to_owned(),
        Number(n) => crate::formula::value::format_number(*n),
        Text(s) => format!("{s:?}"),
        Bool(b) => b.to_string(),
    }
}

fn missing_sheets(doc: &Document, report: &mut Report) {
    for (index, sheet) in doc.sheets.iter().enumerate() {
        for (pos, formula) in sheet.formulas() {
            let Ok(expr) = parse(formula) else {
                // A formula that will not parse reads nothing this can name, which is the
                // same answer `graph::RefIndex::build` gives it and for the same reason.
                continue;
            };
            let mut named = Vec::new();
            sheet_names(&expr, &mut named);
            for name in named {
                if doc
                    .sheets
                    .iter()
                    .any(|s| s.name.eq_ignore_ascii_case(&name))
                {
                    continue;
                }
                if !report.push(Diagnostic::new(
                    &MISSING_SHEET,
                    at(doc, Address::new(index, pos)),
                    format!("names the sheet {name:?}, which this document does not have"),
                )) {
                    return;
                }
            }
        }
    }
}

/// Every sheet name a formula's references mention, deduplicated in the order they appear.
///
/// Deliberately not `Engine::area`: that answers `None` for a missing sheet and for four other
/// reasons besides — an external document, a cuboid, an unresolvable name — and the diagnostic
/// has to say *which* sheet. So the names are read off the syntax, and whether one exists is
/// asked the way `Engine::sheet_index` asks it, case-insensitively (§5.11).
fn sheet_names(expr: &Expr, out: &mut Vec<String>) {
    let mut take = |cell: &CellRef| {
        if let Some(name) = &cell.sheet
            && !out.iter().any(|seen| seen.eq_ignore_ascii_case(name))
        {
            out.push(name.clone());
        }
    };
    match expr {
        Expr::Ref(reference) => {
            // An external document is not open, so nothing in it can be checked either (§5.8).
            if reference.source.is_some() {
                return;
            }
            take(&reference.start);
            if let Some(end) = &reference.end {
                take(end);
            }
        }
        Expr::Call { args, .. } => args.iter().for_each(|arg| sheet_names(arg, out)),
        Expr::Prefix(_, inner) | Expr::Postfix(_, inner) | Expr::Paren(inner) => {
            sheet_names(inner, out)
        }
        Expr::Binary(_, left, right) => {
            sheet_names(left, out);
            sheet_names(right, out);
        }
        Expr::Number(_) | Expr::Text(_) | Expr::Error(_) | Expr::Name(_) | Expr::Empty => {}
    }
}

fn empty_references(doc: &Document, report: &mut Report) {
    let index = RefIndex::build(doc);
    for from in index.formula_cells().collect::<Vec<_>>() {
        for area in index.reads(from) {
            if area.rows.len() != 1 || area.cols.len() != 1 {
                continue;
            }
            let pos = Pos::new(area.rows.start, area.cols.start);
            let Some(sheet) = doc.sheets.get(area.sheet) else {
                continue;
            };
            // A cell holding a formula is not empty even before anything is computed: it has a
            // value in waiting, and `doc/dsl.md` §3.4 makes writing one without its answer the
            // normal way to author a projection.
            if !sheet.get(pos).is_empty() || sheet.formula(pos).is_some() {
                continue;
            }
            if !report.push(Diagnostic::new(
                &EMPTY_REFERENCE,
                at(doc, from),
                format!(
                    "reads {}, which is empty",
                    at(doc, Address::new(area.sheet, pos))
                ),
            )) {
                return;
            }
        }
    }
}

fn off_palette(doc: &Document, report: &mut Report) {
    for (index, sheet) in doc.sheets.iter().enumerate() {
        for (pos, style) in sheet.styles() {
            for colour in colours(style) {
                if !report.push(Diagnostic::new(
                    &OFF_PALETTE,
                    at(doc, Address::new(index, pos)),
                    format!("{colour} is not one of the palette's colours"),
                )) {
                    return;
                }
            }
        }
    }
}

/// The colours a cell style sets that the palette does not have — the text colour and the
/// background, not the borders: a border's colour arrives inside `"0.5pt solid #000000"`, and
/// what a person picks there is a *line* rather than a colour from a swatch.
///
/// `transparent` is ODF's word for no background at all rather than a colour.
fn colours(style: &CellStyle) -> Vec<&str> {
    [style.color.as_deref(), style.background.as_deref()]
        .into_iter()
        .flatten()
        .filter(|colour| *colour != "transparent")
        .filter(|colour| {
            !PALETTE
                .iter()
                .any(|(_, hex)| hex.eq_ignore_ascii_case(colour))
        })
        .collect()
}

fn unspellable(doc: &Document, report: &mut Report) {
    for sheet in &doc.sheets {
        for (index, chart) in sheet.charts().iter().enumerate() {
            if !report.push(Diagnostic::new(
                &UNSPELLABLE,
                // A chart is anchored to a point on the sheet rather than to a cell, so the
                // address is the sheet and the position is in the message — the one rule here
                // whose subject `a1` has no spelling for.
                sheet.name.clone(),
                format!(
                    "chart {} ({}) at {},{} has no projection node, so a .grind of this document would not carry it",
                    index + 1,
                    chart.kind.class(),
                    chart.x,
                    chart.y
                ),
            )) {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CellValue, Sheet};

    /// A document of one sheet, from `(address, value-or-formula)` pairs: a leading `=` is a
    /// formula with no cached value, anything else is a literal.
    fn doc(cells: &[(&str, &str)]) -> Document {
        sheets(&[("Sheet1", cells)])
    }

    fn sheets(spec: &[(&str, &[(&str, &str)])]) -> Document {
        let mut doc = Document::default();
        doc.sheets.clear();
        for (name, cells) in spec {
            let mut sheet = Sheet::new(*name);
            for (address, text) in *cells {
                let reference = a1::parse(address).expect("an address");
                let start = &reference.start;
                let pos = Pos::new(
                    start.row.expect("a row").index,
                    start.col.expect("a column").index,
                );
                match text.strip_prefix('=') {
                    Some(_) => sheet.set_formula(pos, (*text).to_owned()),
                    None => sheet.set(
                        pos,
                        match text.parse::<f64>() {
                            Ok(n) => CellValue::Number(n),
                            Err(_) => CellValue::Text((*text).to_owned()),
                        },
                    ),
                }
            }
            doc.sheets.push(sheet);
        }
        doc
    }

    fn ids(report: &Report) -> Vec<&str> {
        report.diagnostics.iter().map(|d| d.rule).collect()
    }

    fn quiet() -> Options {
        Options::default()
    }

    #[test]
    fn a_cached_value_that_disagrees_with_its_formula_is_reported() {
        let mut doc = doc(&[("A1", "2"), ("A2", "3")]);
        doc.sheets[0].set_formula(Pos::new(2, 0), "=[.A1]+[.A2]".to_owned());
        doc.sheets[0].set(Pos::new(2, 0), CellValue::Number(99.0));

        let report = lint(&doc, &quiet());
        assert_eq!(ids(&report), ["stale-value"]);
        assert_eq!(report.diagnostics[0].at, "Sheet1.A3");
        assert!(
            report.diagnostics[0].message.contains("99")
                && report.diagnostics[0].message.contains('5'),
            "{}",
            report.diagnostics[0].message
        );

        // Correct the cache and the document is quiet.
        doc.sheets[0].set(Pos::new(2, 0), CellValue::Number(5.0));
        assert!(lint(&doc, &quiet()).is_empty());
    }

    #[test]
    fn a_function_this_build_does_not_have_is_not_the_documents_fault() {
        // `BESSELJ` is Part 4 and outside the Small Group, so recalculating would replace a
        // perfectly good cached value with `#NAME?`. That is this engine's gap, not a stale
        // value, and `App::recalc` already counts it as `spoiled`.
        let mut doc = doc(&[]);
        doc.sheets[0].set_formula(Pos::new(0, 0), "=BESSELJ(1;1)".to_owned());
        doc.sheets[0].set(Pos::new(0, 0), CellValue::Number(0.44).clone());
        assert!(lint(&doc, &quiet()).is_empty());
    }

    #[test]
    fn a_formula_naming_a_sheet_that_is_gone_says_which_one() {
        let mut doc = sheets(&[("Data", &[("A1", "1")]), ("Report", &[])]);
        doc.sheets[1].set_formula(Pos::new(0, 0), "=[Gone.A1]+[Data.A1]".to_owned());
        let report = lint(&doc, &quiet());
        assert_eq!(ids(&report), ["missing-sheet"]);
        assert_eq!(report.diagnostics[0].at, "Report.A1");
        assert!(report.diagnostics[0].message.contains("Gone"));

        // Case is not a difference: §5.11 matches names case-insensitively, and the lint asks
        // the question the same way the evaluator does.
        doc.sheets[1].set_formula(Pos::new(0, 0), "=[dATA.A1]".to_owned());
        assert!(lint(&doc, &quiet()).is_empty());
    }

    #[test]
    fn reading_one_empty_cell_is_reported_and_an_empty_cell_in_a_range_is_not() {
        let mut doc = doc(&[("B2", "1"), ("B3", "2")]);
        // B4 is empty and inside the range; C1 is empty and named on its own.
        doc.sheets[0].set_formula(Pos::new(5, 0), "=SUM([.B2:.B4])+[.C1]".to_owned());
        let report = lint(&doc, &quiet());
        assert_eq!(ids(&report), ["empty-reference"]);
        assert!(report.diagnostics[0].message.contains("Sheet1.C1"));
    }

    #[test]
    fn a_cell_holding_a_formula_and_no_answer_yet_is_not_empty() {
        // What a hand-written projection looks like (`doc/dsl.md` §3.4): every formula, no
        // cached values. Calling each of those references empty would make the feature's own
        // documents fail their own linter.
        let mut doc = doc(&[("A1", "2")]);
        doc.sheets[0].set_formula(Pos::new(1, 0), "=[.A1]*2".to_owned());
        doc.sheets[0].set_formula(Pos::new(2, 0), "=[.A2]*2".to_owned());
        let report = lint(&doc, &quiet());
        assert!(
            !ids(&report).contains(&"empty-reference"),
            "{:?}",
            report.diagnostics
        );
    }

    #[test]
    fn an_off_palette_colour_is_a_hint_and_silent_by_default() {
        let mut doc = doc(&[("A1", "1")]);
        doc.sheets[0].set_style(
            Pos::new(0, 0),
            CellStyle {
                color: Some("#123456".to_owned()),
                background: Some("#ff4136".to_owned()),
                ..CellStyle::default()
            },
        );
        assert!(lint(&doc, &quiet()).is_empty());
        let report = lint(
            &doc,
            &Options {
                hints: true,
                off: Vec::new(),
            },
        );
        assert_eq!(ids(&report), ["off-palette"], "only the one not in it");
        assert!(report.diagnostics[0].message.contains("#123456"));
    }

    #[test]
    fn every_rule_has_a_unique_id_and_a_description() {
        for (i, rule) in RULES.iter().enumerate() {
            assert!(!rule.what.is_empty(), "{} says what it checks", rule.id);
            assert!(
                RULES[..i].iter().all(|other| other.id != rule.id),
                "{} appears twice",
                rule.id
            );
        }
    }
}
