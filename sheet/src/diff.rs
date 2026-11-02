// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What changed between two spreadsheets, by what the cells mean rather than by the XML that
//! carries them (`grind diff`, `doc/ideas.md` §1).
//!
//! A cell whose formula or typed value changed is an edit. A formula that is the same and computes
//! something else is a consequence of the edits, and kept apart so a one-cell change does not read
//! as fifty. Sheets are matched by name; a sheet missing from one side and added at the same
//! position on the other is a rename.

use std::collections::BTreeSet;

use serde::Serialize;

use crate::a1;
use crate::formula::parse::parse;
use crate::model::{CellValue, Document, Pos, Sheet};

#[derive(Debug, Default, PartialEq, Serialize)]
pub struct Diff {
    pub sheets: Vec<SheetDiff>,
    pub names: Vec<NameChange>,
}

impl Diff {
    pub fn is_empty(&self) -> bool {
        self.sheets.is_empty() && self.names.is_empty()
    }
}

#[derive(Debug, PartialEq, Serialize)]
pub struct SheetDiff {
    /// The sheet's name after; before, for one that was removed.
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub renamed_from: Option<String>,
    pub change: SheetChange,
    /// For an added or removed sheet, how many cells it holds; the cells are not listed.
    pub cells: usize,
    /// Cells whose formula or typed value changed, appeared or went.
    pub edited: Vec<CellChange>,
    /// Cells whose formula is the same and whose value is not.
    pub recomputed: Vec<CellChange>,
    /// Ranges whose look or number format changed, as `B2:B7`.
    pub restyled: Vec<String>,
    /// What else about the sheet changed, by name: `merges`, `column widths`, …
    pub also: Vec<&'static str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SheetChange {
    Added,
    Removed,
    Changed,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct CellChange {
    /// `B7`, on the sheet the change is listed under.
    pub at: String,
    #[serde(skip)]
    pub pos: Pos,
    pub before: Option<Content>,
    pub after: Option<Content>,
}

/// What a cell holds: its formula, if any, and the value it shows — text quoted, so `"12"` and
/// `12` differ.
#[derive(Debug, PartialEq, Serialize)]
pub struct Content {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub formula: Option<String>,
    pub value: String,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct NameChange {
    pub name: String,
    pub before: Option<String>,
    pub after: Option<String>,
}

pub fn diff(before: &Document, after: &Document) -> Diff {
    let mut out = Diff::default();
    let mut unmatched_after: Vec<usize> = Vec::new();
    let mut matched_before = BTreeSet::new();
    let mut pairs = Vec::new();
    for (index, sheet) in after.sheets.iter().enumerate() {
        match before.sheets.iter().position(|s| s.name == sheet.name) {
            Some(old) => {
                matched_before.insert(old);
                pairs.push((old, index));
            }
            None => unmatched_after.push(index),
        }
    }
    let mut unmatched_before: Vec<usize> = (0..before.sheets.len())
        .filter(|i| !matched_before.contains(i))
        .collect();
    unmatched_after.retain(
        |&index| match unmatched_before.iter().position(|&old| old == index) {
            Some(at) => {
                unmatched_before.remove(at);
                pairs.push((index, index));
                false
            }
            None => true,
        },
    );
    pairs.sort_by_key(|&(_, index)| index);

    let mut sheets: Vec<(usize, SheetDiff)> = Vec::new();
    for (old, new) in pairs {
        let (was, now) = (&before.sheets[old], &after.sheets[new]);
        let changed = compare(was, now);
        if was.name != now.name || !changed.is_empty() {
            sheets.push((
                new,
                SheetDiff {
                    name: now.name.clone(),
                    renamed_from: (was.name != now.name).then(|| was.name.clone()),
                    change: SheetChange::Changed,
                    cells: 0,
                    edited: changed.edited,
                    recomputed: changed.recomputed,
                    restyled: changed.restyled,
                    also: changed.also,
                },
            ));
        }
    }
    for index in unmatched_after {
        sheets.push((index, whole(&after.sheets[index], SheetChange::Added)));
    }
    sheets.sort_by_key(|(index, _)| *index);
    out.sheets = sheets.into_iter().map(|(_, sheet)| sheet).collect();
    out.sheets.extend(
        unmatched_before
            .into_iter()
            .map(|old| whole(&before.sheets[old], SheetChange::Removed)),
    );

    let names: BTreeSet<&String> = before.names.keys().chain(after.names.keys()).collect();
    for name in names {
        let (was, now) = (before.names.get(name), after.names.get(name));
        if was != now {
            out.names.push(NameChange {
                name: name.clone(),
                before: was.cloned(),
                after: now.cloned(),
            });
        }
    }
    out
}

fn whole(sheet: &Sheet, change: SheetChange) -> SheetDiff {
    SheetDiff {
        name: sheet.name.clone(),
        renamed_from: None,
        change,
        cells: sheet.holding().len(),
        edited: Vec::new(),
        recomputed: Vec::new(),
        restyled: Vec::new(),
        also: Vec::new(),
    }
}

#[derive(Default)]
struct Changed {
    edited: Vec<CellChange>,
    recomputed: Vec<CellChange>,
    restyled: Vec<String>,
    also: Vec<&'static str>,
}

impl Changed {
    fn is_empty(&self) -> bool {
        self.edited.is_empty()
            && self.recomputed.is_empty()
            && self.restyled.is_empty()
            && self.also.is_empty()
    }
}

fn compare(was: &Sheet, now: &Sheet) -> Changed {
    let mut out = Changed::default();
    let cells: BTreeSet<Pos> = was.holding().union(&now.holding()).copied().collect();
    for pos in cells {
        let (before, after) = (content(was, pos), content(now, pos));
        let same_formula = match (was.formula(pos), now.formula(pos)) {
            (Some(a), Some(b)) => same(a, b),
            (None, None) => true,
            _ => false,
        };
        if same_formula && before == after {
            continue;
        }
        let change = CellChange {
            at: a1::format(None, pos),
            pos,
            before,
            after,
        };
        match same_formula && was.formula(pos).is_some() {
            true => out.recomputed.push(change),
            false => out.edited.push(change),
        }
    }

    let looks: BTreeSet<Pos> = [was, now]
        .iter()
        .flat_map(|s| {
            s.styles()
                .map(|(pos, _)| pos)
                .chain(s.formats().map(|(pos, _)| pos))
        })
        .collect();
    let restyled: Vec<Pos> = looks
        .into_iter()
        .filter(|&pos| was.style(pos) != now.style(pos) || was.format(pos) != now.format(pos))
        .collect();
    out.restyled = ranges(restyled);

    let parts: [(&'static str, bool); 9] = [
        ("merges", was.merges().ne(now.merges())),
        ("conditional formats", was.rules() != now.rules()),
        ("charts", was.charts() != now.charts()),
        ("checkboxes", was.checkboxes().ne(now.checkboxes())),
        ("filter", was.filter() != now.filter()),
        ("column widths", was.col_widths().ne(now.col_widths())),
        ("row heights", was.row_heights().ne(now.row_heights())),
        ("hidden columns", was.hidden_cols().ne(now.hidden_cols())),
        (
            "hidden rows",
            was.manually_hidden_rows().ne(now.manually_hidden_rows()),
        ),
    ];
    out.also = parts
        .into_iter()
        .filter(|(_, changed)| *changed)
        .map(|(part, _)| part)
        .collect();
    out
}

/// Whether two formulas say the same thing, spelled however each writer spells it.
fn same(a: &str, b: &str) -> bool {
    a == b
        || match (parse(a), parse(b)) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        }
}

fn content(sheet: &Sheet, pos: Pos) -> Option<Content> {
    let formula = sheet.formula(pos).map(|source| match parse(source) {
        Ok(expr) => format!("={expr}"),
        Err(_) => source.to_owned(),
    });
    let value = sheet.get(pos);
    if formula.is_none() && value.is_empty() {
        return None;
    }
    Some(Content {
        formula,
        value: shown(&value),
    })
}

fn shown(value: &CellValue) -> String {
    match value {
        CellValue::Empty => String::new(),
        CellValue::Number(n) => crate::formula::value::format_number(*n),
        CellValue::Text(s) => format!("{s:?}"),
        CellValue::Bool(b) => b.to_string().to_uppercase(),
    }
}

/// Cells as ranges down each column — `B2:B7`, `D4` — column by column.
pub fn ranges(mut cells: Vec<Pos>) -> Vec<String> {
    cells.sort_by_key(|pos| (pos.col, pos.row));
    let mut out = Vec::new();
    let mut run: Option<(Pos, Pos)> = None;
    for pos in cells {
        run = match run {
            Some((start, end)) if end.col == pos.col && end.row + 1 == pos.row => {
                Some((start, pos))
            }
            Some(done) => {
                out.push(range(done));
                Some((pos, pos))
            }
            None => Some((pos, pos)),
        };
    }
    out.extend(run.map(range));
    out
}

fn range((start, end): (Pos, Pos)) -> String {
    match start == end {
        true => a1::format(None, start),
        false => format!("{}:{}", a1::format(None, start), a1::format(None, end)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `(row, column, text)`, a leading `=` making it a formula.
    type Cells<'a> = &'a [(u32, u32, &'a str)];

    fn book(sheets: &[(&str, Cells)]) -> Document {
        let mut doc = Document::default();
        doc.sheets.clear();
        for (name, cells) in sheets {
            let mut sheet = Sheet::new(*name);
            for &(row, col, text) in *cells {
                let pos = Pos::new(row, col);
                match text.starts_with('=') || text.starts_with("of:=") {
                    true => sheet.set_formula(pos, text.to_owned()),
                    false => sheet.set(
                        pos,
                        text.parse()
                            .map_or_else(|_| CellValue::Text(text.to_owned()), CellValue::Number),
                    ),
                }
            }
            doc.sheets.push(sheet);
        }
        doc
    }

    #[test]
    fn a_document_against_itself_is_no_change() {
        let doc = book(&[("Sheet1", &[(0, 0, "1"), (1, 0, "=[.A1]*2")])]);
        assert!(diff(&doc, &doc).is_empty());
    }

    #[test]
    fn an_edit_is_kept_apart_from_what_it_recomputed() {
        let mut before = book(&[("Sheet1", &[(0, 0, "1"), (1, 0, "=[.A1]*2")])]);
        before.sheets[0].set(Pos::new(1, 0), CellValue::Number(2.0));
        let mut after = before.clone();
        after.sheets[0].set(Pos::new(0, 0), CellValue::Number(5.0));
        after.sheets[0].set(Pos::new(1, 0), CellValue::Number(10.0));

        let found = diff(&before, &after);
        let sheet = &found.sheets[0];
        assert_eq!(sheet.edited.len(), 1);
        assert_eq!(sheet.edited[0].at, "A1");
        assert_eq!(sheet.recomputed.len(), 1);
        assert_eq!(sheet.recomputed[0].at, "A2");
        assert_eq!(sheet.recomputed[0].after.as_ref().unwrap().value, "10");
    }

    #[test]
    fn a_formula_spelled_differently_is_the_same_formula() {
        let before = book(&[("Sheet1", &[(1, 0, "of:=[.A1]*2")])]);
        let after = book(&[("Sheet1", &[(1, 0, "=[.A1] * 2")])]);
        assert!(diff(&before, &after).is_empty());
    }

    #[test]
    fn sheets_are_matched_by_name_and_a_rename_by_position() {
        let before = book(&[("Data", &[(0, 0, "1")]), ("Old", &[(0, 0, "x")])]);
        let after = book(&[
            ("Data", &[(0, 0, "1")]),
            ("New", &[(0, 0, "x")]),
            ("Extra", &[(0, 0, "1"), (0, 1, "2")]),
        ]);
        let found = diff(&before, &after);
        assert_eq!(found.sheets.len(), 2, "{found:#?}");
        assert_eq!(found.sheets[0].name, "New");
        assert_eq!(found.sheets[0].renamed_from.as_deref(), Some("Old"));
        assert!(found.sheets[0].edited.is_empty());
        assert_eq!(found.sheets[1].change, SheetChange::Added);
        assert_eq!(found.sheets[1].cells, 2);

        let back = diff(&after, &before);
        assert!(
            back.sheets
                .iter()
                .any(|s| s.change == SheetChange::Removed && s.name == "Extra")
        );
    }

    #[test]
    fn a_restyled_column_is_one_range_and_names_are_compared() {
        let before = book(&[("Sheet1", &[(0, 0, "1"), (1, 0, "2"), (2, 0, "3")])]);
        let mut after = before.clone();
        for row in 0..3 {
            after.sheets[0].set_style(
                Pos::new(row, 0),
                crate::style::CellStyle {
                    font_weight: Some("bold".to_owned()),
                    ..Default::default()
                },
            );
        }
        after
            .names
            .insert("total".to_owned(), "[$Sheet1.$A$3]".to_owned());
        let found = diff(&before, &after);
        assert_eq!(found.sheets[0].restyled, ["A1:A3"]);
        assert!(found.sheets[0].edited.is_empty());
        assert_eq!(found.names.len(), 1);
        assert_eq!(found.names[0].before, None);
    }
}
