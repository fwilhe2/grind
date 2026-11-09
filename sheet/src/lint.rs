// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The spreadsheet's lint rules (`doc/dsl.md` §4.3); [`grind_core::lint`] holds what a diagnostic
//! is (R8).
//!
//! Each rule asks the machinery that already answers its question: [`crate::graph::RefIndex`] for
//! what a formula reads (through `Engine::area`), and `recalculated` for whether a cached value is
//! still true, as [`crate::App::stale`] counts it. Addresses are `a1`'s, so each can be typed back
//! at `grind sheet get`.

/// The diagnostic vocabulary, `grind_core`'s, under this crate's path.
pub use grind_core::lint::{Diagnostic, Options, Report, Rule, Severity};

use grind_core::style::PALETTE;

use std::collections::{BTreeMap, BTreeSet};

use crate::a1;
use crate::formula::eval::{Address, Area, Engine};
use crate::formula::lex::{CellRef, Op};
use crate::formula::parse::{Expr, parse};
use crate::formula::shift::shift;
use crate::graph::RefIndex;
use crate::model::{CellValue, Document, Pos, Sheet};
use crate::spell::Lexicon;
use crate::style::CellStyle;
use crate::view::Names;

/// A cell whose cached value is not what its formula computes; ODF has no dirty bit, so every
/// reader shows the stale one until something recalculates. Only where this build can evaluate the
/// formula: one that would recalculate to an error is this engine's gap ([`crate::App::recalc`]'s
/// `spoiled`), not the document's.
pub const STALE_VALUE: Rule = Rule {
    id: "stale-value",
    severity: Severity::Warning,
    what: "a cell whose cached value disagrees with its formula",
};

/// A formula this build cannot parse, with the parser's reason. A warning: syntax outside
/// what grind reads (an inline array) is this build's gap, but a slip is the document's, and
/// either way the cell recalculates to `#NAME?`.
pub const UNREADABLE_FORMULA: Rule = Rule {
    id: "unreadable-formula",
    severity: Severity::Warning,
    what: "a formula that does not parse",
};

/// A formula naming a sheet the document does not have — what deleting a sheet leaves behind. An
/// error: it reads `#REF!` at the next recalculation.
pub const MISSING_SHEET: Rule = Rule {
    id: "missing-sheet",
    severity: Severity::Error,
    what: "a formula referencing a sheet that does not exist",
};

/// A formula reading a named single cell that holds nothing. Single-cell references only: an empty
/// cell inside a range is ordinary, and a reference finding one empty cell is the typo case — a
/// shifted column, a deleted row.
pub const EMPTY_REFERENCE: Rule = Rule {
    id: "empty-reference",
    severity: Severity::Warning,
    what: "a formula reading a cell that is empty",
};

/// A formula, or a typed number, where the cells either side of it in a line hold one formula
/// filled along it — the overwritten-formula mistake. A row or column whose neighbours break
/// their own lines there too is a subtotal or a section break, and left alone.
pub const INCONSISTENT_FORMULA: Rule = Rule {
    id: "inconsistent-formula",
    severity: Severity::Warning,
    what: "a cell breaking the formula filled either side of it",
};

/// `SUM`, `AVERAGE` and their kin over a line of numbers that stops one cell before its end or
/// starts one after its beginning, where that cell is a typed number with the same look as the
/// rest.
pub const SHORT_RANGE: Rule = Rule {
    id: "short-range",
    severity: Severity::Warning,
    what: "a range one cell short of the numbers it sums",
};

/// A number typed into a formula that a named single cell already holds — the name was meant,
/// and the two drift apart at the next change (`doc/view-modes.md` §4.2's magic constant, inside
/// formula text).
pub const HARD_CODED_INPUT: Rule = Rule {
    id: "hard-coded-input",
    severity: Severity::Warning,
    what: "a number in a formula that a named cell holds",
};

/// A computed cell nothing reads, beside a formula that is not its fill — the calculation went
/// on without it. A hint, off unless asked for: a row of independent results is ordinary too.
pub const UNREAD_RESULT: Rule = Rule {
    id: "unread-result",
    severity: Severity::Hint,
    what: "a computed cell nothing reads, mid-calculation",
};

/// A colour that is not one of `grind_core::style::PALETTE`'s — a hint, off unless asked for,
/// since the palette is a default and never a limit.
pub const OFF_PALETTE: Rule = Rule {
    id: "off-palette",
    severity: Severity::Hint,
    what: "a colour outside the default palette",
};

/// Something the projection cannot spell — charts (`doc/projection-sheet.md`'s one gap) — so
/// converting to a `.grind` and back would lose exactly what this lists.
pub const UNSPELLABLE: Rule = Rule {
    id: "unspellable",
    severity: Severity::Warning,
    what: "a construct the projection cannot spell",
};

/// A word in a text cell the dictionary does not know (`doc/spelling.md`), under
/// `grind_text::lint::MISSPELT`'s id. Runs only with a dictionary attached ([`lint_with`]), so
/// turning spelling off silences it. Only typed text is checked. A warning, never a reason for
/// `grind lint` to fail.
pub const MISSPELT: Rule = Rule {
    id: "misspelt",
    severity: Severity::Warning,
    what: "a word the spelling dictionary does not know",
};

/// Every rule this application has, in the order `grind sheet lint --rules` prints them.
pub const RULES: [Rule; 11] = [
    STALE_VALUE,
    UNREADABLE_FORMULA,
    MISSING_SHEET,
    EMPTY_REFERENCE,
    INCONSISTENT_FORMULA,
    SHORT_RANGE,
    HARD_CODED_INPUT,
    UNREAD_RESULT,
    OFF_PALETTE,
    UNSPELLABLE,
    MISSPELT,
];

/// Check a document against every rule `options` wants — all but [`MISSPELT`], which needs a
/// dictionary ([`lint_with`]).
pub fn lint(doc: &Document, options: &Options) -> Report {
    lint_with(doc, options, None)
}

/// [`lint`], and spelling as well when there is a `lexicon` to check it against.
pub fn lint_with(doc: &Document, options: &Options, lexicon: Option<&dyn Lexicon>) -> Report {
    let mut report = Report::default();
    if options.wants(&STALE_VALUE) {
        stale_values(doc, &mut report);
    }
    if options.wants(&UNREADABLE_FORMULA) {
        unreadable_formulas(doc, &mut report);
    }
    if options.wants(&MISSING_SHEET) {
        missing_sheets(doc, &mut report);
    }
    if options.wants(&EMPTY_REFERENCE) {
        empty_references(doc, &mut report);
    }
    if options.wants(&INCONSISTENT_FORMULA) {
        inconsistent_formulas(doc, &mut report);
    }
    if options.wants(&SHORT_RANGE) {
        short_ranges(doc, &mut report);
    }
    if options.wants(&HARD_CODED_INPUT) {
        hard_coded_inputs(doc, &mut report);
    }
    if options.wants(&UNREAD_RESULT) {
        unread_results(doc, &mut report);
    }
    if options.wants(&OFF_PALETTE) {
        off_palette(doc, &mut report);
    }
    if options.wants(&UNSPELLABLE) {
        unspellable(doc, &mut report);
    }
    if let Some(lexicon) = lexicon
        && options.wants(&MISSPELT)
    {
        misspelt(doc, lexicon, &mut report);
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
        // No cached value is not a wrong one: a hand-written document (`cell B5
        // "=SUM([.B2:.B4])"`) has none until `grind sheet recalc`.
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

/// A cell value as a diagnostic spells it — quoted when text, so an empty cell and an empty string
/// differ.
fn shown(value: &crate::CellValue) -> String {
    use crate::CellValue::*;
    match value {
        Empty => "nothing".to_owned(),
        Number(n) => crate::formula::value::format_number(*n),
        Text(s) => format!("{s:?}"),
        Bool(b) => b.to_string(),
    }
}

fn unreadable_formulas(doc: &Document, report: &mut Report) {
    for (index, sheet) in doc.sheets.iter().enumerate() {
        for (pos, formula) in sheet.formulas() {
            let Err(e) = parse(formula) else { continue };
            if !report.push(Diagnostic::new(
                &UNREADABLE_FORMULA,
                at(doc, Address::new(index, pos)),
                format!("{formula} does not parse: {e}"),
            )) {
                return;
            }
        }
    }
}

fn missing_sheets(doc: &Document, report: &mut Report) {
    for (index, sheet) in doc.sheets.iter().enumerate() {
        // A rule's condition is a formula too, reported at its base cell
        // (`doc/conditional-format.md`).
        let conditions = sheet
            .rules()
            .iter()
            .map(|rule| (rule.base, rule.condition.as_str()));
        for (pos, formula) in sheet.formulas().chain(conditions) {
            let Ok(expr) = parse(formula) else {
                // A formula that will not parse reads nothing nameable, as in
                // `graph::RefIndex::build`.
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

/// Every sheet name a formula's references mention, deduplicated in order — read off the syntax,
/// since `Engine::area`'s `None` has several causes and the diagnostic must name the sheet.
/// Existence is checked case-insensitively (§5.11).
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
            // A formula cell is not empty before it is computed.
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

/// What a cell holds, as far as a line of cells is concerned. A formula is one whatever its cached
/// value.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Empty,
    Formula,
    Number,
    Other,
}

fn kind(sheet: &Sheet, pos: Pos) -> Kind {
    if sheet.formula(pos).is_some() {
        return Kind::Formula;
    }
    match sheet.get(pos) {
        CellValue::Empty => Kind::Empty,
        CellValue::Number(_) => Kind::Number,
        _ => Kind::Other,
    }
}

/// `pos` moved `steps` cells along `(rows, cols)`, when that is still on the sheet.
fn step(pos: Pos, (rows, cols): (u32, u32), steps: u32) -> Option<Pos> {
    Some(Pos::new(
        pos.row.checked_add(rows * steps)?,
        pos.col.checked_add(cols * steps)?,
    ))
}

/// Every formula on a sheet that parses, by position.
fn parsed(sheet: &Sheet) -> BTreeMap<Pos, Expr> {
    sheet
        .formulas()
        .filter_map(|(pos, formula)| Some((pos, parse(formula).ok()?)))
        .collect()
}

/// The two directions a formula is filled in: down a column and along a row.
const LINES: [(u32, u32); 2] = [(1, 0), (0, 1)];

fn inconsistent_formulas(doc: &Document, report: &mut Report) {
    for (index, sheet) in doc.sheets.iter().enumerate() {
        let formulas = parsed(sheet);
        let mut reported = BTreeSet::new();
        for &first in formulas.keys() {
            for line in LINES {
                let Some((middle, last, expected)) = breach(sheet, &formulas, first, line) else {
                    continue;
                };
                // The same break one line over, on either side, is a column or row that is
                // meant to differ — a percentage between two totals.
                if reported.contains(&middle)
                    || !ordinary(sheet, [first, middle, last], line)
                    || [-1, 1].into_iter().any(|side| {
                        across(first, line, side)
                            .is_some_and(|other| breach(sheet, &formulas, other, line).is_some())
                    })
                {
                    continue;
                }
                let holds = match &formulas.get(&middle) {
                    Some(found) => format!("computes ={found}"),
                    None => match sheet.get(middle) {
                        CellValue::Number(n) => format!(
                            "holds the number {}",
                            crate::formula::value::format_number(n)
                        ),
                        _ => continue,
                    },
                };
                reported.insert(middle);
                let address = |pos| at(doc, Address::new(index, pos));
                if !report.push(Diagnostic::new(
                    &INCONSISTENT_FORMULA,
                    address(middle),
                    format!(
                        "{holds} where {} and {} either side fill ={}; here that would be ={expected}",
                        address(first),
                        address(last),
                        formulas[&first],
                    ),
                )) {
                    return;
                }
            }
        }
    }
}

/// Where the formula at `first`, filled two cells along `line`, is the formula there and the cell
/// in between holds something else — another formula or a typed number. Returns the middle, the
/// far end and the formula the middle would hold. A formula with nothing that moves when filled
/// (`=-3`) is no fill.
fn breach(
    sheet: &Sheet,
    formulas: &BTreeMap<Pos, Expr>,
    first: Pos,
    line: (u32, u32),
) -> Option<(Pos, Pos, Expr)> {
    let expr = formulas.get(&first)?;
    let (middle, last) = (step(first, line, 1)?, step(first, line, 2)?);
    let (rows, cols) = (i64::from(line.0), i64::from(line.1));
    let expected = shift(expr, rows, cols);
    if expected == *expr || formulas.get(&last)? != &shift(expr, 2 * rows, 2 * cols) {
        return None;
    }
    let differs = match (formulas.get(&middle), sheet.formula(middle)) {
        // Two formulas taking turns along the line is a pattern, not a break in one.
        (Some(found), _) => {
            let behind = Pos::new(
                first.row.wrapping_sub(line.0),
                first.col.wrapping_sub(line.1),
            );
            *found != expected
                && step(first, line, 3)
                    .and_then(|beyond| formulas.get(&beyond))
                    .is_none_or(|beyond| *beyond != shift(found, 2 * rows, 2 * cols))
                && formulas
                    .get(&behind)
                    .is_none_or(|behind| shift(behind, 2 * rows, 2 * cols) != *found)
        }
        // One that does not parse is `#NAME?` already, and says so itself.
        (None, Some(_)) => false,
        (None, None) => kind(sheet, middle) == Kind::Number,
    };
    differs.then_some((middle, last, expected))
}

/// `pos` moved one cell across `line`, to `side`.
fn across(pos: Pos, (rows, cols): (u32, u32), side: i64) -> Option<Pos> {
    let row = i64::from(pos.row) + side * i64::from(cols);
    let col = i64::from(pos.col) + side * i64::from(rows);
    Some(Pos::new(u32::try_from(row).ok()?, u32::try_from(col).ok()?))
}

/// Whether the cells beside `middle`, across the line, are the kind their own neighbours along
/// the line are — false for a subtotal row, whose label and blanks break every column at once.
fn ordinary(sheet: &Sheet, [first, middle, last]: [Pos; 3], line: (u32, u32)) -> bool {
    let beside = |pos: Pos, side: i64| across(pos, line, side);
    [-1, 1].into_iter().all(|side| {
        let (Some(a), Some(m), Some(b)) = (
            beside(first, side),
            beside(middle, side),
            beside(last, side),
        ) else {
            return true;
        };
        let (a, m, b) = (kind(sheet, a), kind(sheet, m), kind(sheet, b));
        a != b || m == a
    })
}

/// The functions a range of numbers is summarised by, where leaving a number out changes the
/// answer.
const AGGREGATES: [&str; 8] = [
    "SUM", "AVERAGE", "COUNT", "COUNTA", "MIN", "MAX", "PRODUCT", "MEDIAN",
];

fn short_ranges(doc: &Document, report: &mut Report) {
    let engine = Engine::new(doc);
    let mut found = Vec::new();
    for (index, sheet) in doc.sheets.iter().enumerate() {
        for (pos, expr) in parsed(sheet) {
            let from = Address::new(index, pos);
            let mut ranges = Vec::new();
            aggregated(&expr, &mut ranges);
            for reference in ranges {
                let Some(area) = engine.area(reference, from) else {
                    continue;
                };
                let Some(target) = doc.sheets.get(area.sheet) else {
                    continue;
                };
                for (outside, edge) in one_short(target, &area) {
                    found.push((from, area.clone(), Address::new(area.sheet, outside), edge));
                }
            }
        }
    }
    // A cell several formulas all leave out — running totals from one row on — is a boundary
    // somebody chose.
    let mut left_out: BTreeMap<Address, BTreeSet<Address>> = BTreeMap::new();
    for (from, _, outside, _) in &found {
        left_out.entry(*outside).or_default().insert(*from);
    }
    for (from, area, outside, edge) in found {
        if left_out[&outside].len() > 1 {
            continue;
        }
        let range = format!(
            "{}:{}",
            at(
                doc,
                Address::new(area.sheet, Pos::new(area.rows.start, area.cols.start))
            ),
            a1::format(None, Pos::new(area.rows.end - 1, area.cols.end - 1)),
        );
        if !report.push(Diagnostic::new(
            &SHORT_RANGE,
            at(doc, from),
            format!(
                "reads {range} and leaves out {}, the number just {edge} it",
                at(doc, outside),
            ),
        )) {
            return;
        }
    }
}

/// Every plain range an [`AGGREGATES`] call takes as an argument. A whole column or row is
/// bounded by what the sheet uses, so it cannot stop short.
fn aggregated<'e>(expr: &'e Expr, out: &mut Vec<&'e crate::formula::lex::Reference>) {
    match expr {
        Expr::Call { name, args } => {
            let summarises = AGGREGATES.iter().any(|f| f.eq_ignore_ascii_case(name));
            for arg in args {
                let mut arg = arg;
                while let Expr::Paren(inner) = arg {
                    arg = inner;
                }
                match arg {
                    Expr::Ref(reference)
                        if summarises
                            && reference.end.as_ref().is_some_and(|end| {
                                [&reference.start, end]
                                    .iter()
                                    .all(|c| c.row.is_some() && c.col.is_some())
                            }) =>
                    {
                        out.push(reference)
                    }
                    _ => aggregated(arg, out),
                }
            }
        }
        Expr::Prefix(_, inner) | Expr::Postfix(_, inner) | Expr::Paren(inner) => {
            aggregated(inner, out)
        }
        Expr::Binary(_, left, right) => {
            aggregated(left, out);
            aggregated(right, out);
        }
        Expr::Number(_)
        | Expr::Text(_)
        | Expr::Error(_)
        | Expr::Ref(_)
        | Expr::Name(_)
        | Expr::Empty => {}
    }
}

/// The cell just before or after a one-wide `area` that should have been in it: a typed number
/// with the look of the area's end cell, beside which the line of numbers stops.
fn one_short(sheet: &Sheet, area: &Area) -> Vec<(Pos, &'static str)> {
    let (along, cell): (std::ops::Range<u32>, Box<dyn Fn(u32) -> Pos>) =
        match (area.rows.len(), area.cols.len()) {
            (2.., 1) => {
                let col = area.cols.start;
                (area.rows.clone(), Box::new(move |row| Pos::new(row, col)))
            }
            (1, 2..) => {
                let row = area.rows.start;
                (area.cols.clone(), Box::new(move |col| Pos::new(row, col)))
            }
            _ => return Vec::new(),
        };
    let typed = |i: u32| kind(sheet, cell(i)) == Kind::Number;
    let alike = |a: u32, b: u32| {
        typed(a)
            && typed(b)
            && sheet.style(cell(a)) == sheet.style(cell(b))
            && sheet.format(cell(a)) == sheet.format(cell(b))
    };
    let mut found = Vec::new();
    if along.start > 0
        && alike(along.start - 1, along.start)
        && (along.start == 1 || !typed(along.start - 2))
    {
        found.push((cell(along.start - 1), "before"));
    }
    if alike(along.end, along.end - 1) && !typed(along.end + 1) {
        found.push((cell(along.end), "after"));
    }
    found
}

/// A whole number in a formula is as often a count, a day or an argument as an input — `12` is
/// months and a degree of freedom alike — so only a fraction is taken for a rate or a price.
fn trivial(n: f64) -> bool {
    n.fract() == 0.0
}

fn hard_coded_inputs(doc: &Document, report: &mut Report) {
    let names = Names::build(doc);
    let inputs: Vec<(f64, &str, Address)> = names
        .anchors()
        .iter()
        .filter(|anchor| !anchor.is_range())
        .filter_map(|anchor| {
            let pos = Pos::new(anchor.rows.start, anchor.cols.start);
            let sheet = doc.sheets.get(anchor.sheet)?;
            match (kind(sheet, pos), sheet.get(pos)) {
                (Kind::Number, CellValue::Number(n)) if !trivial(n) => {
                    Some((n, anchor.name.as_str(), Address::new(anchor.sheet, pos)))
                }
                _ => None,
            }
        })
        .collect();
    if inputs.is_empty() {
        return;
    }
    for (index, sheet) in doc.sheets.iter().enumerate() {
        for (pos, expr) in parsed(sheet) {
            let mut literals = Vec::new();
            numbers(&expr, &mut literals);
            literals.dedup();
            for n in literals {
                let Some((_, name, input)) = inputs.iter().find(|(value, ..)| *value == n) else {
                    continue;
                };
                if !report.push(Diagnostic::new(
                    &HARD_CODED_INPUT,
                    at(doc, Address::new(index, pos)),
                    format!(
                        "writes {} where the name {name} ({}) holds it",
                        crate::formula::value::format_number(n),
                        at(doc, *input)
                    ),
                )) {
                    return;
                }
            }
        }
    }
}

/// Every number typed into a formula, with a leading `-` and a trailing `%` applied, since `-19%`
/// is the literal somebody wrote.
fn numbers(expr: &Expr, out: &mut Vec<f64>) {
    match expr {
        Expr::Number(n) => out.push(*n),
        Expr::Prefix(Op::Sub, inner) if matches!(**inner, Expr::Number(_)) => {
            let mut one = Vec::new();
            numbers(inner, &mut one);
            out.extend(one.into_iter().map(|n| -n));
        }
        Expr::Postfix(Op::Percent, inner) if matches!(**inner, Expr::Number(_)) => {
            if let Expr::Number(n) = **inner {
                out.push(n / 100.0);
            }
        }
        Expr::Prefix(_, inner) | Expr::Postfix(_, inner) | Expr::Paren(inner) => {
            numbers(inner, out)
        }
        Expr::Binary(_, left, right) => {
            numbers(left, out);
            numbers(right, out);
        }
        Expr::Call { args, .. } => args.iter().for_each(|arg| numbers(arg, out)),
        Expr::Text(_) | Expr::Error(_) | Expr::Ref(_) | Expr::Name(_) | Expr::Empty => {}
    }
}

fn unread_results(doc: &Document, report: &mut Report) {
    let index = RefIndex::build(doc);
    // Past the cap a cell that is read can look unread.
    if index.truncated() {
        return;
    }
    let names = Names::build(doc);
    for (number, sheet) in doc.sheets.iter().enumerate() {
        // ponytail: a chart names its ranges as text this rule does not resolve, so a sheet with
        // one is skipped rather than told its plotted results are unread.
        if !sheet.charts().is_empty() {
            continue;
        }
        let formulas = parsed(sheet);
        for (&pos, expr) in &formulas {
            let here = Address::new(number, pos);
            if index.is_referenced(here) || names.anchors().iter().any(|a| a.contains(here)) {
                continue;
            }
            let reads = index.reads(here);
            if reads.is_empty() {
                continue;
            }
            // Beside it, not its fill, and reading everything it reads: the same inputs, taken
            // further without it.
            let next = LINES.into_iter().find_map(|line| {
                let next = step(pos, line, 1)?;
                let moved = shift(expr, i64::from(line.0), i64::from(line.1));
                let further = index.reads(Address::new(number, next));
                (formulas.get(&next)? != &moved
                    && reads.iter().all(|a| further.iter().any(|b| contains(b, a))))
                .then_some(next)
            });
            let Some(next) = next else { continue };
            if !report.push(Diagnostic::new(
                &UNREAD_RESULT,
                at(doc, here),
                format!(
                    "nothing reads it, though {} beside it reads the same cells and computes on",
                    at(doc, Address::new(number, next))
                ),
            )) {
                return;
            }
        }
    }
}

fn contains(outer: &Area, inner: &Area) -> bool {
    outer.sheet == inner.sheet
        && outer.rows.start <= inner.rows.start
        && inner.rows.end <= outer.rows.end
        && outer.cols.start <= inner.cols.start
        && inner.cols.end <= outer.cols.end
}

fn off_palette(doc: &Document, report: &mut Report) {
    for (index, sheet) in doc.sheets.iter().enumerate() {
        let ruled = sheet.rules().iter().map(|rule| (rule.base, &rule.style));
        for (pos, style) in sheet.styles().chain(ruled) {
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

/// The colours a cell style sets that the palette lacks — text and background, not borders, whose
/// colour is part of a line. `transparent` is no background.
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
                // A chart is anchored to a point, not a cell, so the address is the sheet and the
                // position is in the message.
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

fn misspelt(doc: &Document, lexicon: &dyn Lexicon, report: &mut Report) {
    for (index, sheet) in doc.sheets.iter().enumerate() {
        for wrong in crate::spell::check_sheet(index, sheet, None, lexicon) {
            if !report.push(Diagnostic::new(
                &MISSPELT,
                wrong.address(&sheet.name),
                format!(
                    "{:?} is not in the {} dictionary",
                    wrong.word,
                    lexicon.language()
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
        // `BESSELJ` is outside the Small Group: recalculating would give `#NAME?`, the engine's
        // gap, counted as `spoiled`.
        let mut doc = doc(&[]);
        doc.sheets[0].set_formula(Pos::new(0, 0), "=BESSELJ(1;1)".to_owned());
        doc.sheets[0].set(Pos::new(0, 0), CellValue::Number(0.44).clone());
        assert!(lint(&doc, &quiet()).is_empty());
    }

    #[test]
    fn a_formula_that_does_not_parse_says_why() {
        let mut doc = doc(&[]);
        doc.sheets[0].set_formula(Pos::new(1, 0), "=SUM([.A1:A2])".to_owned());
        let report = lint(&doc, &quiet());
        assert_eq!(ids(&report), ["unreadable-formula"]);
        assert_eq!(report.diagnostics[0].at, "Sheet1.A2");
        assert!(
            report.diagnostics[0]
                .message
                .contains("second end of a range")
        );
    }

    #[test]
    fn a_formula_naming_a_sheet_that_is_gone_says_which_one() {
        let mut doc = sheets(&[("Data", &[("A1", "1")]), ("Report", &[])]);
        doc.sheets[1].set_formula(Pos::new(0, 0), "=[Gone.A1]+[Data.A1]".to_owned());
        let report = lint(&doc, &quiet());
        assert_eq!(ids(&report), ["missing-sheet"]);
        assert_eq!(report.diagnostics[0].at, "Report.A1");
        assert!(report.diagnostics[0].message.contains("Gone"));

        // Case is no difference (§5.11).
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
        // A hand-written projection has formulas and no cached values; none of those references is
        // empty.
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

    /// Quantities in B, prices in C and `=[.Bn]*[.Cn]` in D, for rows 2 to `last`.
    fn order_lines(last: u32) -> Vec<(String, String)> {
        let mut cells = vec![
            ("B1".to_owned(), "Qty".to_owned()),
            ("C1".to_owned(), "Price".to_owned()),
        ];
        for row in 2..=last {
            cells.push((format!("B{row}"), format!("{row}")));
            cells.push((format!("C{row}"), "2.5".to_owned()));
            cells.push((format!("D{row}"), format!("=[.B{row}]*[.C{row}]")));
        }
        cells
    }

    fn doc_of(cells: &[(String, String)]) -> Document {
        let borrowed: Vec<(&str, &str)> = cells
            .iter()
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect();
        doc(&borrowed)
    }

    fn set(doc: &mut Document, address: &str, text: &str) {
        let reference = a1::parse(address).expect("an address");
        let pos = Pos::new(
            reference.start.row.expect("a row").index,
            reference.start.col.expect("a column").index,
        );
        match text.strip_prefix('=') {
            Some(_) => doc.sheets[0].set_formula(pos, text.to_owned()),
            None => {
                doc.sheets[0].clear_formula(pos);
                doc.sheets[0].set(pos, CellValue::Number(text.parse().expect("a number")));
            }
        }
    }

    fn found(doc: &Document, rule: &Rule) -> Vec<(String, String)> {
        lint(
            doc,
            &Options {
                hints: true,
                off: Vec::new(),
            },
        )
        .diagnostics
        .into_iter()
        .filter(|d| d.rule == rule.id)
        .map(|d| (d.at, d.message))
        .collect()
    }

    #[test]
    fn a_formula_filled_down_a_column_is_consistent() {
        assert!(found(&doc_of(&order_lines(8)), &INCONSISTENT_FORMULA).is_empty());
    }

    #[test]
    fn a_number_typed_over_a_filled_formula_is_reported_with_the_formula_it_replaced() {
        let mut doc = doc_of(&order_lines(8));
        set(&mut doc, "D5", "12.5");
        let found = found(&doc, &INCONSISTENT_FORMULA);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].0, "Sheet1.D5");
        assert!(found[0].1.contains("12.5"), "{}", found[0].1);
        assert!(found[0].1.contains("=[.B5]*[.C5]"), "{}", found[0].1);
    }

    #[test]
    fn a_formula_that_breaks_its_fill_is_reported() {
        let mut doc = doc_of(&order_lines(8));
        set(&mut doc, "D5", "=[.B5]*[.C4]");
        let found = found(&doc, &INCONSISTENT_FORMULA);
        assert_eq!(
            found.iter().map(|f| f.0.as_str()).collect::<Vec<_>>(),
            ["Sheet1.D5"]
        );
    }

    #[test]
    fn a_subtotal_row_breaks_every_column_and_is_left_alone() {
        let mut doc = doc_of(&order_lines(8));
        // Row 5 is a subtotal: its quantity and price are gone and D sums what is above.
        doc.sheets[0].set(Pos::new(4, 1), CellValue::Text("Subtotal".to_owned()));
        doc.sheets[0].set(Pos::new(4, 2), CellValue::Empty);
        set(&mut doc, "D5", "=SUM([.D2:.D4])");
        assert!(found(&doc, &INCONSISTENT_FORMULA).is_empty());
    }

    #[test]
    fn a_column_meant_to_differ_on_every_row_is_left_alone() {
        // D and F are totals and E a share between them, on every row: E breaks the row's fill
        // each time, which makes it a column of its own.
        let mut doc = doc(&[]);
        for row in 1..=5 {
            set(&mut doc, &format!("B{row}"), "2");
            set(&mut doc, &format!("C{row}"), "3");
            set(
                &mut doc,
                &format!("D{row}"),
                &format!("=[.B{row}]+[.C{row}]"),
            );
            set(&mut doc, &format!("E{row}"), &format!("=[.D{row}]/[.D$9]"));
            set(
                &mut doc,
                &format!("F{row}"),
                &format!("=[.C{row}]+[.D{row}]"),
            );
        }
        assert!(found(&doc, &INCONSISTENT_FORMULA).is_empty());
    }

    #[test]
    fn two_formulas_taking_turns_along_a_row_are_a_pattern() {
        let mut doc = doc(&[]);
        for col in ["B", "D", "F"] {
            set(&mut doc, &format!("{col}2"), &format!("=[.{col}1]*2"));
        }
        for col in ["C", "E", "G"] {
            set(&mut doc, &format!("{col}2"), &format!("=SUM([.{col}1])"));
        }
        assert!(found(&doc, &INCONSISTENT_FORMULA).is_empty());
    }

    #[test]
    fn a_sum_that_stops_one_row_short_is_reported() {
        let mut doc = doc_of(&order_lines(8));
        set(&mut doc, "B10", "=SUM([.B2:.B7])");
        let found = found(&doc, &SHORT_RANGE);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].0, "Sheet1.B10");
        assert!(found[0].1.contains("Sheet1.B8"), "{}", found[0].1);

        // The whole line is quiet; so is a sum that starts at the header.
        set(&mut doc, "B10", "=SUM([.B2:.B8])");
        set(&mut doc, "C10", "=SUM([.C1:.C8])");
        assert!(super::tests::found(&doc, &SHORT_RANGE).is_empty());
    }

    #[test]
    fn a_sum_that_starts_one_row_late_is_reported() {
        let mut doc = doc_of(&order_lines(8));
        set(&mut doc, "B10", "=SUM([.B3:.B8])");
        let found = found(&doc, &SHORT_RANGE);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].1.contains("Sheet1.B2"), "{}", found[0].1);
    }

    #[test]
    fn a_number_with_another_look_is_not_part_of_the_line() {
        // A year above the column is a heading, and is set apart by its style.
        let mut doc = doc_of(&order_lines(8));
        set(&mut doc, "B1", "2026");
        doc.sheets[0].set_style(
            Pos::new(0, 1),
            CellStyle {
                font_weight: Some("bold".to_owned()),
                ..CellStyle::default()
            },
        );
        set(&mut doc, "B10", "=SUM([.B2:.B8])");
        assert!(found(&doc, &SHORT_RANGE).is_empty());
    }

    #[test]
    fn a_literal_a_named_cell_holds_is_reported_and_arithmetic_is_not() {
        let mut doc = doc(&[("A1", "0.19"), ("B1", "100")]);
        doc.names
            .insert("tax_rate".to_owned(), "[$Sheet1.$A$1]".to_owned());
        set(&mut doc, "B2", "=[.B1]*19%");
        set(&mut doc, "B3", "=[.B1]*(1+[.A1])/2");
        let found = found(&doc, &HARD_CODED_INPUT);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].0, "Sheet1.B2");
        assert!(found[0].1.contains("tax_rate"), "{}", found[0].1);
    }

    #[test]
    fn a_result_the_next_formula_ignores_is_a_hint() {
        let mut doc = doc(&[("A1", "4"), ("B1", "2.5")]);
        set(&mut doc, "C1", "=[.A1]*[.B1]");
        set(&mut doc, "D1", "=[.A1]*[.B1]*1.19");
        let found = found(&doc, &UNREAD_RESULT);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].0, "Sheet1.C1");
        assert!(lint(&doc, &quiet()).is_empty(), "a hint, off by default");

        // Reading it is the fix; a fill beside it is not a calculation going on.
        set(&mut doc, "D1", "=[.C1]*1.19");
        assert!(super::tests::found(&doc, &UNREAD_RESULT).is_empty());
        assert!(super::tests::found(&doc_of(&order_lines(8)), &UNREAD_RESULT).is_empty());
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
