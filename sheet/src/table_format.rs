// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! "Format as table" (Excel's own feature, read the ODF way): one composite command over a
//! rectangle, built entirely from constructs the writer already emits —
//! `table:database-range` ([`crate::filter::Filter`]), per-cell `style:style`
//! ([`crate::style::CellStyle`]), ordinary `SUM` formulas, and a `table:named-expressions`
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

/// What a caller chooses when formatting a range as a table.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TableOptions {
    /// The range's first row is a heading rather than data — `table:contains-header`, and
    /// whether the header itself is left out of banding.
    pub header: bool,
    /// Append a totals row below the range: `SUM` over every column that holds a number in
    /// the body, blank elsewhere, with "Total" in the leftmost column.
    pub totals: bool,
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
    if options.totals {
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
            value: CellValue::Text("Total".to_owned()),
        });
        for col in (start.col.saturating_add(1))..=end.col {
            if !column_has_a_number(sheet, col, first_data_row, end.row) {
                continue;
            }
            let range = Reference {
                source: None,
                start: relative_cell(Pos::new(first_data_row, col)),
                end: (first_data_row != end.row).then(|| relative_cell(Pos::new(end.row, col))),
            };
            actions.push(Action::SetFormula {
                sheet: sheet_index,
                pos: Pos::new(totals_row, col),
                formula: Some(format!("SUM({range})")),
                value: CellValue::Number(sum_column(sheet, col, first_data_row, end.row)),
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

fn column_has_a_number(s: &Sheet, col: u32, first_row: u32, last_row: u32) -> bool {
    (first_row..=last_row).any(|row| matches!(s.get(Pos::new(row, col)), CellValue::Number(_)))
}

fn sum_column(s: &Sheet, col: u32, first_row: u32, last_row: u32) -> f64 {
    (first_row..=last_row)
        .filter_map(|row| match s.get(Pos::new(row, col)) {
            CellValue::Number(n) => Some(n),
            _ => None,
        })
        .sum()
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
            totals: false,
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
            totals: true,
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

    #[test]
    fn the_name_auto_increments_past_a_collision() {
        let doc = document();
        let s = doc.sheet(0).unwrap();
        let mut names = BTreeMap::new();
        names.insert("table1".to_owned(), "[.A1]".to_owned());
        let options = TableOptions {
            header: true,
            totals: false,
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
            totals: false,
            name: Some("not a name".to_owned()),
        };
        assert!(plan(0, Pos::new(0, 0), Pos::new(3, 1), &options, s, &doc.names).is_err());
    }
}
