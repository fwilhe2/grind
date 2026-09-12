// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! "Format as table" (Excel's own feature, read the ODF way): one composite command over a
//! rectangle, built entirely from constructs the writer already emits —
//! `table:database-range` ([`crate::filter::Filter`]), per-cell `style:style`
//! ([`crate::style::CellStyle`]), ordinary aggregate formulas ([`TotalsFunction`]), and a
//! `table:named-expressions`
//! entry (`Document::names`).
//!
//! There is no new persisted "table" object: ODF has nothing resembling xlsx's `ListObject`,
//! and inventing one would mean either a synthetic element R2's schema rejects or a heuristic
//! for recognising it again on read — fragile, and in tension with R1's "no invented schema".
//! [`plan`] instead produces one [`Action::Batch`] of ordinary edits — the same thing
//! LibreOffice's own Table ▸ AutoFormat does — so applying it is one undo step and the result
//! is indistinguishable, once saved, from a document styled and filtered by hand. There is
//! correspondingly no "un-table" operation: clearing the effects means clearing the filter,
//! restyling the cells and deleting the name, same as it would after AutoFormat.

use std::collections::BTreeMap;

use crate::Result;
use crate::action::Action;
use crate::filter::Filter;
use crate::formula::lex::{Axis, CellRef, Reference};
use crate::model::{CellValue, Pos, Sheet};
use crate::style::CellStyle;

/// The shading a table is given. Not [`crate::style::PALETTE`] entries — banding wants tints
/// paler than any named colour there — but ordinary `#rrggbb` values a shell may restyle
/// afterwards through [`crate::App::set_style`] like any other cell.
const HEADER_BACKGROUND: &str = "#dddddd";
const BAND_A: &str = "#ffffff";
const BAND_B: &str = "#f2f2f2";
const TOTALS_BORDER: &str = "0.5pt solid #000000";

/// Which aggregate a totals row carries — Excel's own totals-row list, every entry of which
/// happens to be an ODF Small Group function (`doc/small-group.md`), so a totals row is
/// ordinary formulas a shell has no special reading of.
///
/// One function for the whole row rather than one per column: a per-column choice needs a
/// persisted table object to remember it in, and `table_format`'s own module comment is why
/// there is none. A column whose aggregate would be an error — `AVERAGE` over a column with no
/// numbers in it, `STDEV` over a single one — is left blank instead, the same way a text
/// column already is, since [`CellValue`] has no error variant to cache and a cached value
/// that lies is exactly what `grind lint`'s `stale-value` rule is for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TotalsFunction {
    /// `SUM` — the default, and the one whose row is labelled "Total" rather than by name.
    #[default]
    Sum,
    Average,
    /// `COUNTA` — every non-empty cell, text included, so a text column gets an answer too.
    Count,
    /// `COUNT` — the numbers only.
    CountNumbers,
    Min,
    Max,
    /// `STDEV` — the sample form, which needs two numbers.
    StdDev,
    /// `VAR` — the sample form, which needs two numbers.
    Var,
}

impl TotalsFunction {
    /// Every one of them, in the order a shell offers them. The single table a menu, a
    /// dropdown, a CLI flag and a `:command` argument all read, so no shell can offer a
    /// function the core cannot write or spell one of them differently.
    pub const ALL: &'static [TotalsFunction] = &[
        TotalsFunction::Sum,
        TotalsFunction::Average,
        TotalsFunction::Count,
        TotalsFunction::CountNumbers,
        TotalsFunction::Min,
        TotalsFunction::Max,
        TotalsFunction::StdDev,
        TotalsFunction::Var,
    ];

    /// The spelling a CLI flag or a `:command` argument takes, and what [`std::str::FromStr`] parses.
    pub fn id(self) -> &'static str {
        match self {
            TotalsFunction::Sum => "sum",
            TotalsFunction::Average => "average",
            TotalsFunction::Count => "count",
            TotalsFunction::CountNumbers => "count-numbers",
            TotalsFunction::Min => "min",
            TotalsFunction::Max => "max",
            TotalsFunction::StdDev => "stdev",
            TotalsFunction::Var => "var",
        }
    }

    /// The OpenFormula function the row's formulas call.
    pub fn function(self) -> &'static str {
        match self {
            TotalsFunction::Sum => "SUM",
            TotalsFunction::Average => "AVERAGE",
            TotalsFunction::Count => "COUNTA",
            TotalsFunction::CountNumbers => "COUNT",
            TotalsFunction::Min => "MIN",
            TotalsFunction::Max => "MAX",
            TotalsFunction::StdDev => "STDEV",
            TotalsFunction::Var => "VAR",
        }
    }

    /// What a menu, a dropdown or a palette row calls it.
    pub fn label(self) -> &'static str {
        match self {
            TotalsFunction::Sum => "Sum",
            TotalsFunction::Average => "Average",
            TotalsFunction::Count => "Count",
            TotalsFunction::CountNumbers => "Count numbers",
            TotalsFunction::Min => "Minimum",
            TotalsFunction::Max => "Maximum",
            TotalsFunction::StdDev => "Standard deviation",
            TotalsFunction::Var => "Variance",
        }
    }

    /// The word written into the totals row's leftmost cell. "Total" for a sum, because that
    /// is what a summed row is called in every spreadsheet there has ever been; the function's
    /// own name for the rest, since a row of averages labelled "Total" is a lie.
    pub fn row_label(self) -> &'static str {
        match self {
            TotalsFunction::Sum => "Total",
            TotalsFunction::StdDev => "Std dev",
            other => other.label(),
        }
    }

    /// Every id, comma-separated — what a shell puts in the sentence it says when somebody
    /// names one that does not exist.
    pub fn ids() -> String {
        Self::ALL
            .iter()
            .map(|f| f.id())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

impl std::str::FromStr for TotalsFunction {
    /// A sentence, not an [`crate::Error`]: this parses a word a person typed at a shell, which
    /// is that shell's own vocabulary rather than anything the document model has an error for.
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        // An underscore or a space for the hyphen, since a person typing at a `:command` line
        // will reach for any of the three.
        let wanted = s.trim().to_lowercase().replace(['_', ' '], "-");
        // `COUNTA` is spelled `count` here and `COUNT` is spelled `count-numbers`, which is
        // Excel's own wording for the pair and the reason the formulas' own names are *not*
        // accepted as aliases: `COUNT` would then mean the opposite of what it says.
        if wanted == "counta" {
            return Ok(TotalsFunction::Count);
        }
        Self::ALL
            .iter()
            .copied()
            .find(|f| f.id() == wanted)
            .ok_or_else(|| format!("{s}: expected one of {}", Self::ids()))
    }
}

impl std::fmt::Display for TotalsFunction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.id())
    }
}

/// What a caller chooses when formatting a range as a table.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TableOptions {
    /// The range's first row is a heading rather than data — `table:contains-header`, and
    /// whether the header itself is left out of banding.
    pub header: bool,
    /// Append a totals row below the range: the chosen aggregate over every column it can
    /// answer for, blank elsewhere, and the function's own word in the leftmost column.
    /// `None` appends no row at all.
    pub totals: Option<TotalsFunction>,
    /// The named range to create. `None` auto-generates the next unused `TableN`.
    pub name: Option<String>,
}

/// What [`plan`] computed: the batch to apply, and the name it settled on (so a caller that
/// left it to auto-generate can report what got picked).
pub struct TablePlan {
    pub actions: Vec<Action>,
    pub name: String,
}

/// Build the batch of edits "format as table" is. Pure — it reads `sheet` and `names` and
/// returns actions, applying none of them, so a caller can wrap the result in one mutation
/// alongside the undo bookkeeping every other multi-cell edit uses (see
/// [`crate::App::format_table`]).
pub fn plan(
    sheet_index: usize,
    start: Pos,
    end: Pos,
    options: &TableOptions,
    sheet: &Sheet,
    names: &BTreeMap<String, String>,
) -> Result<TablePlan> {
    let mut actions = Vec::new();

    actions.push(Action::SetFilter {
        sheet: sheet_index,
        filter: Some(Box::new(Filter {
            // The name LibreOffice gives an autofilter nobody named — the same default
            // `grind sheet filter` uses, so a table's autofilter looks like any other.
            name: "__Anonymous_Sheet_DB__0".to_owned(),
            start,
            end,
            contains_header: options.header,
            buttons: true,
            keep: BTreeMap::new(),
        })),
    });

    let first_data_row = if options.header {
        start.row.saturating_add(1)
    } else {
        start.row
    };

    if options.header {
        for col in start.col..=end.col {
            actions.push(restyle(sheet_index, Pos::new(start.row, col), sheet, |s| {
                s.font_weight = Some("bold".to_owned());
                s.background = Some(HEADER_BACKGROUND.to_owned());
            }));
        }
    }

    for (i, row) in (first_data_row..=end.row).enumerate() {
        let band = if i % 2 == 0 { BAND_A } else { BAND_B };
        for col in start.col..=end.col {
            actions.push(restyle(sheet_index, Pos::new(row, col), sheet, |s| {
                s.background = Some(band.to_owned());
            }));
        }
    }

    let mut table_end = end;
    if let Some(function) = options.totals {
        let totals_row = end.row.saturating_add(1);
        table_end.row = totals_row;

        for col in start.col..=end.col {
            actions.push(restyle(
                sheet_index,
                Pos::new(totals_row, col),
                sheet,
                |s| {
                    s.font_weight = Some("bold".to_owned());
                    s.borders[2] = Some(TOTALS_BORDER.to_owned()); // top edge — EDGES[2]
                },
            ));
        }
        actions.push(Action::SetCell {
            sheet: sheet_index,
            pos: Pos::new(totals_row, start.col),
            value: CellValue::Text(function.row_label().to_owned()),
        });
        for col in (start.col.saturating_add(1))..=end.col {
            // No answer means no formula: a column this aggregate cannot summarise (text under
            // `SUM`, one number under `STDEV`) is left blank rather than given a formula whose
            // cached value would have to be an error.
            let Some(answer) = summarise(function, sheet, col, first_data_row, end.row) else {
                continue;
            };
            let range = Reference {
                source: None,
                start: relative_cell(Pos::new(first_data_row, col)),
                end: (first_data_row != end.row).then(|| relative_cell(Pos::new(end.row, col))),
            };
            actions.push(Action::SetFormula {
                sheet: sheet_index,
                pos: Pos::new(totals_row, col),
                formula: Some(format!("{}({range})", function.function())),
                value: CellValue::Number(answer),
            });
        }
    }

    let name = match &options.name {
        Some(name) => {
            crate::validate_name(name)?;
            name.clone()
        }
        None => next_table_name(names),
    };
    actions.push(Action::SetName {
        name: name.clone(),
        expression: Some(absolute_reference(&sheet.name, start, table_end)),
    });

    Ok(TablePlan { actions, name })
}

/// A cell's current style, with one change applied — `App::set_style` *replaces* a cell's
/// style, so banding has to read-merge-write to avoid discarding a font or border the cell
/// already carried.
fn restyle(sheet: usize, pos: Pos, s: &Sheet, edit: impl FnOnce(&mut CellStyle)) -> Action {
    let mut style = s.style(pos).cloned().unwrap_or_default();
    edit(&mut style);
    Action::SetStyle {
        sheet,
        pos,
        style: Some(Box::new(style)),
    }
}

/// The answer the totals row's formula will come to for one column, or `None` when this
/// aggregate has nothing to say about it.
///
/// Computed here rather than by recalculating afterwards because [`plan`] runs with the write
/// lock held, and [`Action::SetFormula`] carries the cached value with it — `office:value` is
/// written whether or not anybody recalculates, so a wrong one here is a wrong one in the
/// file. Each arm mirrors `formula::funcs`' own definition of the function it caches: the
/// sequence functions skip text and empty cells (§6.3.7), the sample moments divide by n-1 in
/// two passes, and every case those two would answer with an error is a `None` instead.
fn summarise(
    function: TotalsFunction,
    s: &Sheet,
    col: u32,
    first_row: u32,
    last_row: u32,
) -> Option<f64> {
    let cells = || (first_row..=last_row).map(move |row| s.get(Pos::new(row, col)));
    let filled = cells().filter(|v| !v.is_empty()).count();
    if filled == 0 {
        return None;
    }
    let numbers: Vec<f64> = cells()
        .filter_map(|v| match v {
            CellValue::Number(n) => Some(n),
            _ => None,
        })
        .collect();

    match function {
        TotalsFunction::Count => return Some(filled as f64),
        TotalsFunction::CountNumbers => return Some(numbers.len() as f64),
        _ if numbers.is_empty() => return None,
        _ => {}
    }

    let n = numbers.len() as f64;
    let mean = numbers.iter().sum::<f64>() / n;
    Some(match function {
        TotalsFunction::Sum => numbers.iter().sum(),
        TotalsFunction::Average => mean,
        TotalsFunction::Min => numbers.iter().copied().fold(f64::INFINITY, f64::min),
        TotalsFunction::Max => numbers.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        TotalsFunction::StdDev | TotalsFunction::Var => {
            // The sample forms need two numbers; one of them is a division by zero, which
            // there is no cached value for.
            if numbers.len() < 2 {
                return None;
            }
            let variance = numbers.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0);
            if function == TotalsFunction::StdDev {
                variance.sqrt()
            } else {
                variance
            }
        }
        TotalsFunction::Count | TotalsFunction::CountNumbers => unreachable!("returned above"),
    })
}

/// A relative, same-sheet reference — what a formula written *in* the table means by its own
/// column.
fn relative_cell(pos: Pos) -> CellRef {
    CellRef {
        sheet: None,
        sheet_absolute: false,
        col: Some(Axis {
            index: pos.col,
            absolute: false,
        }),
        row: Some(Axis {
            index: pos.row,
            absolute: false,
        }),
    }
}

/// A name's definition: sheet-qualified and absolute on every axis, the way [`crate::a1`]'s
/// own `as_definition` writes one — reimplemented rather than called because that helper needs
/// an `&App` to default the sheet, and this runs with the write lock already held (`App`'s
/// other reads would deadlock against it).
fn absolute_reference(sheet_name: &str, start: Pos, end: Pos) -> String {
    let cell = |pos: Pos| CellRef {
        sheet: Some(sheet_name.to_owned()),
        sheet_absolute: true,
        col: Some(Axis {
            index: pos.col,
            absolute: true,
        }),
        row: Some(Axis {
            index: pos.row,
            absolute: true,
        }),
    };
    let reference = Reference {
        source: None,
        start: cell(start),
        end: (start != end).then(|| cell(end)),
    };
    reference.to_string()
}

/// The next unused `TableN`, so two tables on one document never collide.
fn next_table_name(names: &BTreeMap<String, String>) -> String {
    let mut n = 1u32;
    loop {
        let candidate = format!("Table{n}");
        if !names.contains_key(&candidate.to_lowercase()) {
            return candidate;
        }
        n = n.saturating_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Document;

    fn document() -> Document {
        let mut doc = Document::default();
        let s = doc.sheet_mut(0).unwrap();
        s.set(Pos::new(0, 0), CellValue::Text("Product".into()));
        s.set(Pos::new(0, 1), CellValue::Text("Qty".into()));
        for (row, (product, qty)) in [("Chair", 3.0), ("Desk", 1.0), ("Lamp", 4.0)]
            .into_iter()
            .enumerate()
        {
            let row = row as u32 + 1;
            s.set(Pos::new(row, 0), CellValue::Text(product.into()));
            s.set(Pos::new(row, 1), CellValue::Number(qty));
        }
        doc
    }

    #[test]
    fn a_header_is_excluded_from_banding_and_a_headerless_range_is_not() {
        let doc = document();
        let s = doc.sheet(0).unwrap();
        let options = TableOptions {
            header: true,
            totals: None,
            name: None,
        };
        let result = plan(0, Pos::new(0, 0), Pos::new(3, 1), &options, s, &doc.names).unwrap();
        let header_styled = result.actions.iter().any(|a| {
            matches!(a, Action::SetStyle { pos, style: Some(st), .. }
                if *pos == Pos::new(0, 0) && st.background.as_deref() == Some(HEADER_BACKGROUND))
        });
        assert!(header_styled, "the header row gets its own shade");
        let first_data_row_banded = result.actions.iter().any(|a| {
            matches!(a, Action::SetStyle { pos, style: Some(st), .. }
                if *pos == Pos::new(1, 0) && st.background.as_deref() == Some(BAND_A))
        });
        assert!(first_data_row_banded);

        let options = TableOptions {
            header: false,
            ..options
        };
        let result = plan(0, Pos::new(0, 0), Pos::new(3, 1), &options, s, &doc.names).unwrap();
        let row_zero_banded = result.actions.iter().any(|a| {
            matches!(a, Action::SetStyle { pos, style: Some(st), .. }
                if *pos == Pos::new(0, 0) && st.background.as_deref() == Some(BAND_A))
        });
        assert!(
            row_zero_banded,
            "with no header, row 0 is banded like any other"
        );
    }

    #[test]
    fn totals_sums_only_the_numeric_column_and_labels_the_first() {
        let doc = document();
        let s = doc.sheet(0).unwrap();
        let options = TableOptions {
            header: true,
            totals: Some(TotalsFunction::Sum),
            name: None,
        };
        let result = plan(0, Pos::new(0, 0), Pos::new(3, 1), &options, s, &doc.names).unwrap();

        let label = result.actions.iter().find_map(|a| match a {
            Action::SetCell { pos, value, .. } if *pos == Pos::new(4, 0) => Some(value.clone()),
            _ => None,
        });
        assert_eq!(label, Some(CellValue::Text("Total".into())));

        let sum = result.actions.iter().find_map(|a| match a {
            Action::SetFormula {
                pos,
                formula,
                value,
                ..
            } if *pos == Pos::new(4, 1) => Some((formula.clone(), value.clone())),
            _ => None,
        });
        let (formula, value) = sum.expect("qty column gets a SUM");
        assert_eq!(formula.as_deref(), Some("SUM([.B2:.B4])"));
        assert_eq!(value, CellValue::Number(8.0));

        // The text column gets no formula, but does get the totals row's border/weight.
        let text_col_formula = result
            .actions
            .iter()
            .any(|a| matches!(a, Action::SetFormula { pos, .. } if *pos == Pos::new(4, 0)));
        assert!(!text_col_formula);
    }

    /// The formula the totals row writes, and the value cached with it, for one column.
    fn totals_of(function: TotalsFunction, col: u32) -> Option<(String, CellValue)> {
        let doc = document();
        let s = doc.sheet(0).unwrap();
        let options = TableOptions {
            header: true,
            totals: Some(function),
            name: None,
        };
        let result = plan(0, Pos::new(0, 0), Pos::new(3, 1), &options, s, &doc.names).unwrap();
        result.actions.iter().find_map(|a| match a {
            Action::SetFormula {
                pos,
                formula: Some(formula),
                value,
                ..
            } if *pos == Pos::new(4, col) => Some((formula.clone(), value.clone())),
            _ => None,
        })
    }

    #[test]
    fn every_aggregate_writes_its_own_function_and_caches_its_own_answer() {
        // Qty is 3, 1, 4 — one worked answer per function, so a wrong cached value cannot pass
        // for a right one just because the formula text is right.
        let cases = [
            (TotalsFunction::Sum, "SUM", 8.0),
            (TotalsFunction::Average, "AVERAGE", 8.0 / 3.0),
            (TotalsFunction::Count, "COUNTA", 3.0),
            (TotalsFunction::CountNumbers, "COUNT", 3.0),
            (TotalsFunction::Min, "MIN", 1.0),
            (TotalsFunction::Max, "MAX", 4.0),
            // Sample moments over 3, 1, 4: mean 8/3, sum of squares 4.6667, over n-1.
            (TotalsFunction::Var, "VAR", 7.0 / 3.0),
            (TotalsFunction::StdDev, "STDEV", 1.527_525_231_651_946_4),
        ];
        for (function, name, expected) in cases {
            let (formula, value) = totals_of(function, 1).expect("the qty column is summarised");
            assert_eq!(formula, format!("{name}([.B2:.B4])"), "{function}");
            let CellValue::Number(n) = value else {
                panic!("{function}: expected a number");
            };
            assert!((n - expected).abs() < 1e-9, "{function}: {n} != {expected}");
        }
    }

    #[test]
    fn a_text_column_is_counted_but_never_summed() {
        // COUNTA is the one aggregate a column of product names has an answer for, and the
        // label cell is column A, so column B's *text* is not what is being read here — the
        // product column is A and carries the label instead. Use a column of text beside it.
        let mut doc = document();
        let s = doc.sheet_mut(0).unwrap();
        s.set(Pos::new(0, 2), CellValue::Text("Note".into()));
        for row in 1..=3 {
            s.set(Pos::new(row, 2), CellValue::Text("ok".into()));
        }
        let s = doc.sheet(0).unwrap();
        let formula_in_c = |function| {
            let options = TableOptions {
                header: true,
                totals: Some(function),
                name: None,
            };
            let result = plan(0, Pos::new(0, 0), Pos::new(3, 2), &options, s, &doc.names).unwrap();
            result.actions.iter().find_map(|a| match a {
                Action::SetFormula {
                    pos,
                    formula,
                    value,
                    ..
                } if *pos == Pos::new(4, 2) => Some((formula.clone(), value.clone())),
                _ => None,
            })
        };
        assert_eq!(
            formula_in_c(TotalsFunction::Count),
            Some((Some("COUNTA([.C2:.C4])".to_owned()), CellValue::Number(3.0)))
        );
        assert_eq!(
            formula_in_c(TotalsFunction::CountNumbers),
            Some((Some("COUNT([.C2:.C4])".to_owned()), CellValue::Number(0.0))),
            "COUNT over text is zero, which is an answer rather than an error"
        );
        assert_eq!(formula_in_c(TotalsFunction::Sum), None);
        assert_eq!(formula_in_c(TotalsFunction::Average), None);
    }

    #[test]
    fn a_sample_moment_over_one_number_is_left_blank_rather_than_cached_as_an_error() {
        let doc = document();
        let s = doc.sheet(0).unwrap();
        for function in [TotalsFunction::StdDev, TotalsFunction::Var] {
            let options = TableOptions {
                header: true,
                totals: Some(function),
                name: None,
            };
            // One data row: B2 alone, which STDEV and VAR both answer with #DIV/0!.
            let result = plan(0, Pos::new(0, 0), Pos::new(1, 1), &options, s, &doc.names).unwrap();
            let wrote = result
                .actions
                .iter()
                .any(|a| matches!(a, Action::SetFormula { pos, .. } if *pos == Pos::new(2, 1)));
            assert!(!wrote, "{function} over one number writes nothing");
        }
    }

    #[test]
    fn the_row_label_names_the_aggregate_and_only_a_sum_is_a_total() {
        let doc = document();
        let s = doc.sheet(0).unwrap();
        let label = |function| {
            let options = TableOptions {
                header: true,
                totals: Some(function),
                name: None,
            };
            let result = plan(0, Pos::new(0, 0), Pos::new(3, 1), &options, s, &doc.names).unwrap();
            result.actions.iter().find_map(|a| match a {
                Action::SetCell { pos, value, .. } if *pos == Pos::new(4, 0) => Some(value.clone()),
                _ => None,
            })
        };
        assert_eq!(
            label(TotalsFunction::Sum),
            Some(CellValue::Text("Total".into()))
        );
        assert_eq!(
            label(TotalsFunction::Average),
            Some(CellValue::Text("Average".into()))
        );
    }

    #[test]
    fn a_function_is_named_the_same_way_everywhere_it_is_named() {
        for function in TotalsFunction::ALL {
            // Every id parses back to itself, and every function it writes is one this build
            // can evaluate — a totals row calling something outside the Small Group would be
            // a `#NAME?` the moment anybody recalculated.
            assert_eq!(function.id().parse::<TotalsFunction>(), Ok(*function));
            assert!(crate::formula::funcs::implemented().contains(&function.function()));
            assert!(TotalsFunction::ids().contains(function.id()));
        }
        assert_eq!(
            "COUNT_NUMBERS".parse::<TotalsFunction>(),
            Ok(TotalsFunction::CountNumbers)
        );
        assert_eq!(
            "counta".parse::<TotalsFunction>(),
            Ok(TotalsFunction::Count),
            "the formula's own name for the one Excel renamed"
        );
        assert!("median".parse::<TotalsFunction>().is_err());
    }

    #[test]
    fn the_name_auto_increments_past_a_collision() {
        let doc = document();
        let s = doc.sheet(0).unwrap();
        let mut names = BTreeMap::new();
        names.insert("table1".to_owned(), "[.A1]".to_owned());
        let options = TableOptions {
            header: true,
            totals: None,
            name: None,
        };
        let result = plan(0, Pos::new(0, 0), Pos::new(3, 1), &options, s, &names).unwrap();
        assert_eq!(result.name, "Table2");
    }

    #[test]
    fn an_explicit_name_is_validated_like_any_other() {
        let doc = document();
        let s = doc.sheet(0).unwrap();
        let options = TableOptions {
            header: true,
            totals: None,
            name: Some("not a name".to_owned()),
        };
        assert!(plan(0, Pos::new(0, 0), Pos::new(3, 1), &options, s, &doc.names).is_err());
    }
}
