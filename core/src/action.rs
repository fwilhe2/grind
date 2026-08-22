// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Actions: the only way the document changes, and the whole of undo/redo.
//!
//! Applying an action returns its inverse. That is the entire mechanism — the undo stack
//! is a stack of inverses, redo is the same trick run the other way, and no shell ever
//! implements history of its own (doc/plan.md, rule 2).

use serde::{Deserialize, Serialize};

use crate::model::{CellValue, Document, Pos, Sheet};
use crate::numfmt::Format;
use crate::style::CellStyle;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Action {
    SetCell {
        sheet: usize,
        pos: Pos,
        value: CellValue,
    },
    /// A formula and its cached value, which move together — doc/ods-format.md §4: a formula
    /// written without a cached value renders blank in LibreOffice until something
    /// recalculates it, so the two are never set apart.
    ///
    /// `formula: None` clears the formula and leaves an ordinary value cell behind, which is
    /// what the inverse of "set a formula on an empty cell" has to be.
    SetFormula {
        sheet: usize,
        pos: Pos,
        formula: Option<String>,
        value: CellValue,
    },
    /// A cell's number format, `None` being ODF's General — the absence of a data style
    /// (doc/ods-format.md §5.2). Display only: no value moves, which is why this is its own
    /// action rather than a field of [`Action::SetCell`].
    SetFormat {
        sheet: usize,
        pos: Pos,
        format: Option<Box<Format>>,
    },
    /// A cell's styling (doc/ods-format.md §5.1). Display only, like [`Action::SetFormat`],
    /// and separate from it because one may change without the other.
    SetStyle {
        sheet: usize,
        pos: Pos,
        style: Option<Box<CellStyle>>,
    },
    /// A column's `style:column-width`, `None` returning it to the default (§5.4).
    ///
    /// A track rather than a cell, so it has no [`Pos`] — and its own action rather than a
    /// flag on [`Action::SetStyle`] because a column's width and a cell's look are two
    /// different `style:style` families.
    SetColWidth {
        sheet: usize,
        col: u32,
        width: Option<String>,
    },
    /// A row's `style:row-height` — the twin of [`Action::SetColWidth`].
    SetRowHeight {
        sheet: usize,
        row: u32,
        height: Option<String>,
    },
    /// A sheet's autofilter (§9.4), `None` removing it. Boxed because it is much the
    /// largest variant here and every other one would grow to match.
    SetFilter {
        sheet: usize,
        filter: Option<Box<crate::filter::Filter>>,
    },
    /// A named expression (§5.11), `None` deleting it.
    ///
    /// Document-level rather than per-cell, which is the one thing that makes it unlike
    /// every other action here — see [`Document::names`] for why the key is lower-cased and
    /// why a named *range* is stored as the reference it stands for.
    SetName {
        name: String,
        expression: Option<String>,
    },
    /// Insert a sheet at `index`, carrying everything on it.
    ///
    /// A whole [`Sheet`] rather than a name because this is the inverse of
    /// [`Action::RemoveSheet`], and undoing a deletion has to bring the cells back.
    InsertSheet { index: usize, sheet: Box<Sheet> },
    /// Remove the sheet at `index`, cells and all.
    RemoveSheet { index: usize },
    /// Rename the sheet at `index` (`table:name`).
    ///
    /// Formulas naming the old sheet are **not** rewritten — see [`crate::App::rename_sheet`].
    RenameSheet { index: usize, name: String },
    /// Many changes, one undo step. Recalculation is why this exists: a user who types
    /// `recalc` and then `undo` means the whole recalculation, not its last cell.
    Batch(Vec<Action>),
}

impl Document {
    /// Apply `action`, returning the action that undoes it.
    ///
    /// Returns `None` if the action names a sheet that does not exist, so a bad index from
    /// a shell is an error rather than a panic.
    #[must_use]
    pub fn apply(&mut self, action: Action) -> Option<Action> {
        self.note(&action);
        match action {
            Action::SetCell { sheet, pos, value } => {
                let s = self.sheet_mut(sheet)?;
                let previous = s.get(pos);
                s.set(pos, value);
                Some(Action::SetCell {
                    sheet,
                    pos,
                    value: previous,
                })
            }
            Action::SetFormula {
                sheet,
                pos,
                formula,
                value,
            } => {
                let s = self.sheet_mut(sheet)?;
                let previous = Action::SetFormula {
                    sheet,
                    pos,
                    formula: s.formula(pos).map(str::to_owned),
                    value: s.get(pos),
                };
                match formula {
                    Some(f) => s.set_formula(pos, f),
                    None => s.clear_formula(pos),
                }
                s.set(pos, value);
                Some(previous)
            }
            Action::SetFormat { sheet, pos, format } => {
                let s = self.sheet_mut(sheet)?;
                let previous = s.format(pos).cloned().map(Box::new);
                match format {
                    Some(f) => s.set_format(pos, *f),
                    None => s.clear_format(pos),
                }
                Some(Action::SetFormat {
                    sheet,
                    pos,
                    format: previous,
                })
            }
            Action::SetStyle { sheet, pos, style } => {
                let s = self.sheet_mut(sheet)?;
                let previous = s.style(pos).cloned().map(Box::new);
                match style {
                    Some(style) => s.set_style(pos, *style),
                    None => s.clear_style(pos),
                }
                Some(Action::SetStyle {
                    sheet,
                    pos,
                    style: previous,
                })
            }
            Action::SetColWidth { sheet, col, width } => {
                let s = self.sheet_mut(sheet)?;
                let previous = s.col_width(col).map(str::to_owned);
                s.set_col_width(col, width);
                Some(Action::SetColWidth {
                    sheet,
                    col,
                    width: previous,
                })
            }
            Action::SetRowHeight { sheet, row, height } => {
                let s = self.sheet_mut(sheet)?;
                let previous = s.row_height(row).map(str::to_owned);
                s.set_row_height(row, height);
                Some(Action::SetRowHeight {
                    sheet,
                    row,
                    height: previous,
                })
            }
            Action::SetFilter { sheet, filter } => {
                let s = self.sheet_mut(sheet)?;
                let previous = s.filter().cloned().map(Box::new);
                s.set_filter(filter.map(|f| *f));
                Some(Action::SetFilter {
                    sheet,
                    filter: previous,
                })
            }
            Action::SetName { name, expression } => {
                let key = name.to_lowercase();
                let previous = match expression {
                    Some(e) => self.names.insert(key.clone(), e),
                    None => self.names.remove(&key),
                };
                Some(Action::SetName {
                    name: key,
                    expression: previous,
                })
            }
            // The three that move sheets rather than cells. They shift every later index,
            // which the undo stack survives for one reason: it is strictly ordered, so an
            // older entry is only ever applied *after* this one has been undone and the
            // index space it was recorded in is back. That is also why sheet handles are not
            // needed here, and the note is the whole argument against introducing them.
            Action::InsertSheet { index, sheet } => {
                if index > self.sheets.len() {
                    return None;
                }
                self.sheets.insert(index, *sheet);
                Some(Action::RemoveSheet { index })
            }
            Action::RemoveSheet { index } => {
                if index >= self.sheets.len() {
                    return None;
                }
                let sheet = self.sheets.remove(index);
                Some(Action::InsertSheet {
                    index,
                    sheet: Box::new(sheet),
                })
            }
            Action::RenameSheet { index, name } => {
                let s = self.sheet_mut(index)?;
                let previous = std::mem::replace(&mut s.name, name);
                Some(Action::RenameSheet {
                    index,
                    name: previous,
                })
            }
            // Applied in order, undone in the opposite order — two cells written in
            // sequence do not commute, so the inverse of `[a, b]` is `[b⁻¹, a⁻¹]`.
            Action::Batch(actions) => {
                // Checked before anything is written: bailing out halfway would leave the
                // document mutated *and* return no inverse, so there would be no way back.
                // A session file restored against a document with fewer sheets is the path
                // that gets here.
                if !actions.iter().all(|a| self.addressable(a)) {
                    return None;
                }
                let mut inverses = Vec::with_capacity(actions.len());
                for action in actions {
                    inverses.push(self.apply(action)?);
                }
                inverses.reverse();
                Some(Action::Batch(inverses))
            }
        }
    }

    /// Record what this action touches, for R6's splicing writer (`odf::source`).
    ///
    /// Here rather than in each arm because every action carries the address it writes to,
    /// and because this must run whether or not the action turns out to be addressable — a
    /// batch that is rejected has changed nothing, and a batch that is applied has recorded
    /// everything before the first write.
    fn note(&mut self, action: &Action) {
        match action {
            Action::SetCell { sheet, pos, .. } | Action::SetFormula { sheet, pos, .. } => {
                self.edits.cells.insert((*sheet, *pos));
            }
            // A format or a style is a `style:style` the source file does not have, so it
            // cannot be spliced into one. Sticky, and it still records the cell so a
            // document that later regenerates is not depending on which flag was set first.
            Action::SetFormat { sheet, pos, .. } | Action::SetStyle { sheet, pos, .. } => {
                self.edits.cells.insert((*sheet, *pos));
                self.edits.only_values = false;
            }
            // Not a cell at all: `table:named-expressions` is its own element, somewhere
            // R6's splicing writer does not touch. Left unsaid, a new name would be dropped
            // on the next save — the document's bytes would come back with the cells edited
            // and the name gone.
            // Not a cell either: a track style lives in `office:automatic-styles` and is
            // named from a `<table:table-column>`, neither of which the splice touches.
            Action::SetColWidth { .. } | Action::SetRowHeight { .. } => {
                self.edits.only_values = false
            }
            // Not a cell either: `table:database-ranges` is its own element, and the
            // `table:visibility` it implies sits on rows rather than cells.
            Action::SetFilter { .. } => self.edits.only_values = false,
            Action::SetName { .. } => self.edits.only_values = false,
            // Not a cell either, and worse: adding or removing a sheet shifts every later
            // index, so the `(sheet, pos)` keys already in `cells` would name the wrong
            // sheet. Regenerating is what makes that harmless — the splice map is never
            // consulted again.
            Action::InsertSheet { .. }
            | Action::RemoveSheet { .. }
            | Action::RenameSheet { .. } => {
                self.edits.only_values = false;
            }
            Action::Batch(actions) => actions.iter().for_each(|a| self.note(a)),
        }
    }

    /// Whether every sheet `action` names exists, so applying it cannot fail partway.
    fn addressable(&self, action: &Action) -> bool {
        match action {
            Action::SetCell { sheet, .. }
            | Action::SetFormula { sheet, .. }
            | Action::SetFormat { sheet, .. }
            | Action::SetStyle { sheet, .. }
            | Action::SetColWidth { sheet, .. }
            | Action::SetRowHeight { sheet, .. }
            | Action::SetFilter { sheet, .. } => self.sheet(*sheet).is_some(),
            // Names are document-level, so there is no sheet index to be wrong about.
            Action::SetName { .. } => true,
            // One past the end is where a sheet is appended, so an insert is checked with
            // `<=` and a removal with `<`.
            Action::InsertSheet { index, .. } => *index <= self.sheets.len(),
            Action::RemoveSheet { index } => *index < self.sheets.len(),
            Action::RenameSheet { index, .. } => self.sheet(*index).is_some(),
            Action::Batch(actions) => actions.iter().all(|a| self.addressable(a)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(row: u32, value: f64) -> Action {
        Action::SetCell {
            sheet: 0,
            pos: Pos::new(row, 0),
            value: CellValue::Number(value),
        }
    }

    /// Two writes to the *same* cell in one batch: the inverse only restores the original if
    /// it undoes them in the opposite order. Writing them to different cells would pass
    /// whether or not the reversal is there, which is why they overlap.
    #[test]
    fn a_batch_undoes_in_the_opposite_order() {
        let mut doc = Document::default();
        let inverse = doc
            .apply(Action::Batch(vec![set(0, 1.0), set(0, 2.0), set(1, 9.0)]))
            .unwrap();
        assert_eq!(
            doc.sheet(0).unwrap().get(Pos::new(0, 0)),
            CellValue::Number(2.0)
        );

        assert!(doc.apply(inverse).is_some());
        assert_eq!(doc.sheet(0).unwrap().get(Pos::new(0, 0)), CellValue::Empty);
        assert_eq!(doc.sheet(0).unwrap().get(Pos::new(1, 0)), CellValue::Empty);
    }

    /// A batch that cannot be applied in full must not be applied at all — a half-applied
    /// batch returns no inverse, so there would be no way back.
    #[test]
    fn a_batch_naming_a_missing_sheet_writes_nothing() {
        let mut doc = Document::default();
        let bad = Action::SetCell {
            sheet: 7,
            pos: Pos::new(0, 0),
            value: CellValue::Number(1.0),
        };
        assert!(doc.apply(Action::Batch(vec![set(0, 1.0), bad])).is_none());
        assert_eq!(
            doc.sheet(0).unwrap().get(Pos::new(0, 0)),
            CellValue::Empty,
            "the first action must not have landed"
        );
    }
}
