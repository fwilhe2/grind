// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Tables (`xl/tables/tableN.xml`) and the structured references that name them.
//!
//! A structured reference — `Sales[Amount]`, `Sales[[#This Row],[Tax]]` — is a *range with a
//! name for a spelling*: the table part says which range, and which column is which, so the
//! reference can be turned into the plain A1 range the model understands. Nothing about a table
//! is kept (the model has none) — only the answer to "which cells does this say" at the moment a
//! formula is read, which is why a table's later growth is not followed: the formula is as true
//! as the file's own geometry was.
//!
//! What cannot be answered stays a [`crate::formula::Refusal::StructuredReference`]: a table
//! this workbook does not have, a column it does not have, a row item that has no row
//! (`#Totals` on a table with no totals row), `[#This Row]` from a cell outside the table's
//! data rows, and any combination of row items that is not one contiguous run.

use crate::names::Ns;
use crate::xml::{Handled, Reader};
use grind_sheet::formula::lex::{Axis, CellRef, Reference};

/// One `<table>` part, in 0-based coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct Table {
    /// The `displayName` — what a formula calls it — compared case-insensitively.
    pub name: String,
    /// Index into the workbook's sheet list.
    pub sheet: usize,
    pub first_row: u32,
    pub last_row: u32,
    pub first_col: u32,
    pub columns: Vec<String>,
    pub header_rows: u32,
    pub totals_rows: u32,
}

/// Every table in a workbook.
#[derive(Clone, Debug, Default)]
pub struct Tables {
    pub list: Vec<Table>,
}

/// Where a formula stands: what `[#This Row]` and an unqualified reference are relative to.
pub struct Scope<'a> {
    pub tables: &'a Tables,
    /// The sheet the formula is on, and every sheet's name by index.
    pub sheet: usize,
    pub sheet_names: &'a [String],
    pub row: u32,
}

/// Read one table part. `None` for anything that is not a table with a range and columns.
pub fn read(bytes: &[u8], sheet: usize) -> Option<Table> {
    let mut reader = Reader::new(bytes);
    let (root, attrs) = reader.root().ok()??;
    if !(root.ns == Ns::Spreadsheet && root.local == "table") {
        return None;
    }
    let name = attrs
        .plain("displayName")
        .or_else(|| attrs.plain("name"))?
        .to_owned();
    let (from, to) = attrs.plain("ref")?.split_once(':').unwrap_or_else(|| {
        let r = attrs.plain("ref").unwrap_or("");
        (r, r)
    });
    let (from, to) = (crate::address::cell(from)?, crate::address::cell(to)?);
    let count = |attr: &str, default: u32| {
        attrs
            .plain(attr)
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(default)
    };
    let header_rows = count("headerRowCount", 1);
    let totals_rows = count("totalsRowCount", 0);
    let mut columns = Vec::new();
    let _ = reader.children(|reader, name, _| {
        if !(name.ns == Ns::Spreadsheet && name.local == "tableColumns") {
            return Ok(Handled::No);
        }
        reader.children(|_, name, attrs| {
            if name.ns == Ns::Spreadsheet && name.local == "tableColumn" {
                columns.push(attrs.plain("name").unwrap_or("").to_owned());
            }
            Ok(Handled::No)
        })?;
        Ok(Handled::Yes)
    });
    if columns.is_empty() || to.row < from.row || to.col < from.col {
        return None;
    }
    Some(Table {
        name,
        sheet,
        first_row: from.row,
        last_row: to.row,
        first_col: from.col,
        columns,
        header_rows,
        totals_rows,
    })
}

impl Scope<'_> {
    /// The range a structured reference to `table` with this bracketed `body` (the text
    /// between its outermost `[` and `]`) names.
    pub fn resolve(&self, table: &str, body: &str) -> Option<Reference> {
        let table = self
            .tables
            .list
            .iter()
            .find(|t| t.name.eq_ignore_ascii_case(table))?;
        let items = items(body)?;

        let (mut head, mut data, mut tail, mut all, mut this_row) =
            (false, false, false, false, false);
        let mut span: Option<(String, String)> = None;
        for item in items {
            match item {
                Item::Special(s) => match s.to_ascii_lowercase().as_str() {
                    "#headers" => head = true,
                    "#data" => data = true,
                    "#totals" => tail = true,
                    "#all" => all = true,
                    "#this row" => this_row = true,
                    _ => return None,
                },
                Item::This(col) => {
                    this_row = true;
                    span = Some((col.clone(), col));
                }
                Item::Columns(a, b) => match span {
                    None => span = Some((a, b)),
                    Some(_) => return None,
                },
            }
        }

        let data_first = table.first_row + table.header_rows;
        let data_last = table.last_row.checked_sub(table.totals_rows)?;
        let (rows, relative) = if this_row {
            if head
                || data
                || tail
                || all
                || self.sheet != table.sheet
                || self.row < data_first
                || self.row > data_last
            {
                return None;
            }
            ((self.row, self.row), true)
        } else {
            let (head, data, tail) = match all {
                true => (true, true, true),
                false if !(head || data || tail) => (false, true, false),
                false => (head, data, tail),
            };
            if (head && table.header_rows == 0) || (tail && table.totals_rows == 0) {
                // `#All` is whatever the table has; a *named* item it lacks is no row at all.
                if !all {
                    return None;
                }
            }
            let head = head && table.header_rows > 0;
            let tail = tail && table.totals_rows > 0;
            if data_first > data_last && data {
                return None;
            }
            let first = match (head, data) {
                (true, _) => table.first_row,
                (false, true) => data_first,
                (false, false) => table.last_row,
            };
            let last = match (tail, data) {
                (true, _) => table.last_row,
                (false, true) => data_last,
                (false, false) => table.first_row,
            };
            // Headers and totals with the data between them left out is not a range.
            if head && tail && !data {
                return None;
            }
            if !head && !tail && !data {
                return None;
            }
            ((first, last), false)
        };

        let (c1, c2) = match span {
            None => (0, table.columns.len() - 1),
            Some((a, b)) => {
                let find = |name: &str| {
                    table
                        .columns
                        .iter()
                        .position(|c| c.eq_ignore_ascii_case(name))
                };
                let (a, b) = (find(&a)?, find(&b)?);
                (a.min(b), a.max(b))
            }
        };
        let col0 = table.first_col + u32::try_from(c1).ok()?;
        let col1 = table.first_col + u32::try_from(c2).ok()?;

        let qualifier = (self.sheet != table.sheet)
            .then(|| self.sheet_names.get(table.sheet).cloned())
            .flatten();
        if self.sheet != table.sheet && qualifier.is_none() {
            return None;
        }
        let cell = |col: u32, row: u32| CellRef {
            sheet: None,
            sheet_absolute: false,
            col: Some(Axis {
                index: col,
                absolute: true,
            }),
            row: Some(Axis {
                index: row,
                absolute: !relative,
            }),
        };
        let mut start = cell(col0, rows.0);
        start.sheet = qualifier;
        let end = (rows.0 != rows.1 || col0 != col1).then(|| cell(col1, rows.1));
        Some(Reference {
            source: None,
            start,
            end,
        })
    }
}

enum Item {
    /// `[#Headers]` and friends, verbatim with the `#`.
    Special(String),
    /// `@Col` — what the UI spells `[#This Row],[Col]`.
    This(String),
    /// `[Col]` or `[Col1]:[Col2]`.
    Columns(String, String),
}

/// The comma-separated items of a structured reference's body.
fn items(body: &str) -> Option<Vec<Item>> {
    let chars: Vec<char> = body.chars().collect();
    let mut at = 0;
    let skip = |at: &mut usize| {
        while chars.get(*at).is_some_and(|c| c.is_whitespace()) {
            *at += 1;
        }
    };
    skip(&mut at);
    // `Table[Col]` and `Table[#All]` have no inner brackets: the body *is* the one item.
    if chars.first() != Some(&'[') {
        let text = unescape(&chars)?;
        return Some(vec![match text.as_str() {
            "" => Item::Special("#Data".into()),
            s if s.starts_with('#') => Item::Special(s.to_owned()),
            s if s.starts_with('@') => Item::This(s[1..].trim().to_owned()),
            s => Item::Columns(s.to_owned(), s.to_owned()),
        }]);
    }
    let mut out = Vec::new();
    loop {
        skip(&mut at);
        let first = group(&chars, &mut at)?;
        skip(&mut at);
        if chars.get(at) == Some(&':') {
            at += 1;
            skip(&mut at);
            let second = group(&chars, &mut at)?;
            if first.starts_with('#') || second.starts_with('#') {
                return None;
            }
            out.push(Item::Columns(first, second));
        } else if first.starts_with('#') {
            out.push(Item::Special(first));
        } else {
            out.push(Item::Columns(first.clone(), first));
        }
        skip(&mut at);
        match chars.get(at) {
            None => return Some(out),
            Some(',') => at += 1,
            Some(_) => return None,
        }
    }
}

/// `[…]` at `at`, with `'` escapes undone; leaves `at` after the `]`.
fn group(chars: &[char], at: &mut usize) -> Option<String> {
    if chars.get(*at) != Some(&'[') {
        return None;
    }
    *at += 1;
    let start = *at;
    while *at < chars.len() {
        match chars[*at] {
            '\'' => *at += 2,
            ']' => {
                let text = unescape(&chars[start..*at])?;
                *at += 1;
                return Some(text);
            }
            _ => *at += 1,
        }
    }
    None
}

fn unescape(chars: &[char]) -> Option<String> {
    let mut out = String::new();
    let mut it = chars.iter();
    while let Some(&c) = it.next() {
        match c {
            '\'' => out.push(*it.next()?),
            c => out.push(c),
        }
    }
    Some(out.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sales() -> Tables {
        Tables {
            list: vec![Table {
                name: "Sales".into(),
                sheet: 0,
                first_row: 0,
                last_row: 5,
                first_col: 0,
                columns: vec!["Region".into(), "Amount".into(), "Tax rate".into()],
                header_rows: 1,
                totals_rows: 1,
            }],
        }
    }

    fn show(body: &str, sheet: usize, row: u32) -> Option<String> {
        let tables = sales();
        let names = ["Data".to_owned(), "Other".to_owned()];
        let scope = Scope {
            tables: &tables,
            sheet,
            sheet_names: &names,
            row,
        };
        scope
            .resolve("sales", body)
            .map(|r| format!("{}", grind_sheet::formula::parse::Expr::Ref(r)))
    }

    #[test]
    fn a_column_is_its_data_rows() {
        assert_eq!(show("Amount", 0, 9).as_deref(), Some("[.$B$2:.$B$5]"));
        assert_eq!(show("[Amount]", 0, 9).as_deref(), Some("[.$B$2:.$B$5]"));
        assert_eq!(show("[tax rate]", 0, 9).as_deref(), Some("[.$C$2:.$C$5]"));
    }

    #[test]
    fn row_items_pick_rows_and_must_be_contiguous() {
        assert_eq!(
            show("[#Headers],[Amount]", 0, 9).as_deref(),
            Some("[.$B$1]")
        );
        assert_eq!(show("[#Totals],[Amount]", 0, 9).as_deref(), Some("[.$B$6]"));
        assert_eq!(
            show("[#Headers],[#Data]", 0, 9).as_deref(),
            Some("[.$A$1:.$C$5]")
        );
        assert_eq!(show("#All", 0, 9).as_deref(), Some("[.$A$1:.$C$6]"));
        assert_eq!(show("[#Headers],[#Totals]", 0, 9), None);
    }

    #[test]
    fn this_row_stays_relative_and_only_inside_the_table() {
        assert_eq!(
            show("[#This Row],[Amount]", 0, 3).as_deref(),
            Some("[.$B4]")
        );
        assert_eq!(show("@Amount", 0, 3).as_deref(), Some("[.$B4]"));
        assert_eq!(show("[#This Row],[Amount]", 0, 0), None);
        assert_eq!(show("[#This Row],[Amount]", 1, 3), None);
    }

    #[test]
    fn a_span_of_columns_and_another_sheet() {
        assert_eq!(
            show("[Region]:[Amount]", 0, 9).as_deref(),
            Some("[.$A$2:.$B$5]")
        );
        assert_eq!(show("Amount", 1, 9).as_deref(), Some("[Data.$B$2:.$B$5]"));
    }

    #[test]
    fn what_the_table_lacks_is_refused() {
        assert_eq!(show("Nope", 0, 9), None);
        assert_eq!(show("[#Bogus]", 0, 9), None);
    }

    #[test]
    fn an_escaped_bracket_is_part_of_a_name() {
        let mut tables = sales();
        tables.list[0].columns[1] = "A[1]".into();
        let names = ["Data".to_owned()];
        let scope = Scope {
            tables: &tables,
            sheet: 0,
            sheet_names: &names,
            row: 9,
        };
        assert!(scope.resolve("Sales", "A'[1']").is_some());
    }

    #[test]
    fn a_table_part_is_read() {
        let xml = format!(
            r#"<table xmlns="{}" id="1" name="T" displayName="T" ref="B2:C4"><tableColumns count="2"><tableColumn id="1" name="a"/><tableColumn id="2" name="b"/></tableColumns></table>"#,
            crate::names::MAIN_T
        );
        let t = read(xml.as_bytes(), 3).expect("a table");
        assert_eq!(
            (t.first_row, t.last_row, t.first_col, t.sheet),
            (1, 3, 1, 3)
        );
        assert_eq!(t.columns, ["a", "b"]);
        assert!(read(b"<nope/>", 0).is_none());
    }
}
