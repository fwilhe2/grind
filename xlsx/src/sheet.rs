// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `xl/worksheets/*.xml` — rows, cells, and their values.
//!
//! `<c r="B2" t="…" s="12"><v>…</v></c>`, with `t` defaulting to `n`. The mapping onto
//! `CellValue` is `doc/xlsx-import.md` Part II §2's table, which is ECMA-376 §18.18.11's
//! `ST_CellType`:
//!
//! | `t` | holds | becomes |
//! |---|---|---|
//! | `n` | a number | `Number` — a date or time kind, and the 1900 correction, iff its format says so |
//! | `s` | a shared-string index | `Text`, runs flattened |
//! | `str` | a formula's string result | `Text` |
//! | `inlineStr` | `<is>` | `Text`, runs flattened |
//! | `b` | `0` / `1` | `Bool` |
//! | `e` | `#DIV/0!` … | `Text`, the error's name — how the evaluator stores one already |
//! | `d` | an ISO 8601 date | `Number`, date kind |
//!
//! **Formulas are X2's.** A formula cell carries its cached value here and nothing else, which
//! is the value Excel computed and the one a reader of the document sees.
//!
//! Position is **implicit** when `r` is absent — the next row, the next column — because the
//! spec allows it and Apache POI and SheetJS both write it (`realworld/implicit-refs.xlsx`).
//! An explicit `r` moves the counter, so a row that mixes the two resumes from the last
//! explicit address rather than from the number of cells seen. `dimension` and `spans` are
//! claims and are never read.

use grind_sheet::formula::date;
use grind_sheet::model::{CellValue, NumberKind, Pos, Sheet};
use grind_sheet::{MAX_COLS, MAX_ROWS};

use crate::address;
use crate::dates;
use crate::report::{Dropped, Report};
use crate::strings::{self, Item};
use crate::styles::Styles;
use crate::xml::{Handled, Reader};

/// Cells one import will materialise, across every sheet — the same number `odf/read.rs`
/// bounds itself by, for the same reason. An `.xlsx` cannot *amplify* the way a repeated ODF
/// row can, since each cell costs bytes in the file, but a 512 MB `sheet1.xml` is still a
/// file somebody sent, and the model's memory is not the place to find out how big it was.
///
/// Four million is eight `scale/large-sheet.xlsx`es. A cell past it is counted in
/// [`Report::over_budget`] rather than carried, and the conversion is not lossless.
pub const MAX_CELLS: usize = 4_000_000;

/// What every sheet in one workbook reads its cells against.
pub struct Context<'a> {
    pub strings: &'a [Item],
    pub styles: &'a Styles,
    /// `workbookPr/@date1904`. In the 1904 system serials already agree with the document's
    /// null date and are carried unchanged; in the 1900 system a date needs [`dates::correct`].
    pub date_1904: bool,
    /// The document's null date, for `t="d"`'s ISO spelling.
    pub null_date: i64,
}

/// Read one worksheet part into `sheet`.
///
/// Returns the reader's flavour evidence and `mc:MustUnderstand` findings through `report` and
/// `seen`, because a worksheet is where they usually are: `realworld/mixed-flavour.xlsx` is a
/// Transitional workbook with a Strict worksheet, and X0, which opened no worksheet, could not
/// see it. An XML error partway through keeps every cell read before it — tolerance on the way
/// in — and is otherwise silent, since there is no `Dropped` for "the rest of a broken part".
pub fn read(
    bytes: &[u8],
    context: &Context<'_>,
    sheet: &mut Sheet,
    report: &mut Report,
    seen: &mut crate::names::Seen,
) {
    let mut reader = Reader::new(bytes);
    if matches!(reader.root(), Ok(Some((ref root, _))) if root.is("worksheet")) {
        let _ = reader.children(|reader, name, _| {
            if !name.is("sheetData") {
                return Ok(Handled::No);
            }
            rows(reader, context, sheet, report)?;
            Ok(Handled::Yes)
        });
    }
    report.must_understand.append(&mut reader.must_understand);
    seen.transitional |= reader.seen.transitional;
    seen.strict |= reader.seen.strict;
}

fn rows(
    reader: &mut Reader<'_>,
    context: &Context<'_>,
    sheet: &mut Sheet,
    report: &mut Report,
) -> crate::Result<()> {
    // The row the *next* implicit `<row>` lands on.
    let mut next_row: u32 = 0;
    reader.children(|reader, name, attrs| {
        if !name.is("row") {
            return Ok(Handled::No);
        }
        let row = attrs
            .plain("r")
            .and_then(|r| r.parse::<u32>().ok())
            .filter(|r| (1..=MAX_ROWS).contains(r))
            .map_or(next_row, |r| r - 1);
        let mut at = Pos::new(row, 0);
        reader.children(|reader, name, attrs| {
            if !name.is("c") {
                return Ok(Handled::No);
            }
            // An explicit address wins outright, row and all, and moves the counter; an absent
            // or unreadable one is the next column along.
            if let Some(pos) = attrs.plain("r").and_then(address::cell) {
                at = pos;
            }
            if at.col >= MAX_COLS {
                // Implicit columns ran off the edge of the grid. Nothing to put them in.
                return Ok(Handled::Yes);
            }
            let raw = RawCell {
                t: attrs.plain("t").unwrap_or("n").to_owned(),
                s: attrs.plain("s").and_then(|s| s.parse().ok()).unwrap_or(0),
            };
            let value = cell(reader)?;
            store(sheet, at, &raw, value, context, report);
            next_row = next_row.max(at.row + 1);
            at.col += 1;
            Ok(Handled::Yes)
        })?;
        next_row = next_row.max(row + 1);
        Ok(Handled::Yes)
    })
}

/// A cell's own attributes, owned, so the walker can move on while they are still wanted.
struct RawCell {
    t: String,
    s: usize,
}

/// What a `<c>` held, before the type decides what it means.
#[derive(Default)]
struct Held {
    /// `<v>`'s text. `None` when there was no `<v>` at all, which is not the same as an empty
    /// one: `<c r="C1" s="1"/>` is a styled empty cell, and `<f>` with no `<v>` is a formula a
    /// non-Excel writer did not evaluate.
    v: Option<String>,
    /// `<is>`, for `t="inlineStr"`.
    inline: Option<Item>,
}

fn cell(reader: &mut Reader<'_>) -> crate::Result<Held> {
    let mut held = Held::default();
    reader.children(|reader, name, _| {
        if name.is("v") {
            held.v = Some(reader.text()?);
            return Ok(Handled::Yes);
        }
        if name.is("is") {
            held.inline = Some(strings::item(reader)?);
            return Ok(Handled::Yes);
        }
        // `<f>` is X2's; `<extLst>` is nobody's.
        Ok(Handled::No)
    })?;
    Ok(held)
}

fn store(
    sheet: &mut Sheet,
    at: Pos,
    raw: &RawCell,
    held: Held,
    context: &Context<'_>,
    report: &mut Report,
) {
    let mut kind = None;
    let value = match (raw.t.as_str(), held) {
        ("s", Held { v: Some(v), .. }) => match v
            .trim()
            .parse::<usize>()
            .ok()
            .and_then(|i| context.strings.get(i))
        {
            Some(item) => {
                if item.rich {
                    report.drop_one(Dropped::RichText);
                }
                CellValue::Text(item.text.clone())
            }
            // An index past the table is a claim about a string nobody wrote.
            None => CellValue::Empty,
        },
        (
            "inlineStr",
            Held {
                inline: Some(item), ..
            },
        ) => {
            if item.rich {
                report.drop_one(Dropped::RichText);
            }
            CellValue::Text(item.text)
        }
        // A producer that spells an inline string with `<v>` means the text all the same.
        ("inlineStr" | "str" | "e", Held { v: Some(v), .. }) => CellValue::Text(v),
        ("b", Held { v: Some(v), .. }) => CellValue::Bool(matches!(v.trim(), "1" | "true")),
        ("d", Held { v: Some(v), .. }) => match date::parse_date(&v, context.null_date) {
            Some(serial) => {
                kind = Some(context.styles.kind(raw.s).unwrap_or(NumberKind::Date));
                CellValue::Number(serial)
            }
            // Not a date after all. The text is still what the file said.
            None => CellValue::Text(v),
        },
        (_, Held { v: Some(v), .. }) => match number(&v) {
            Some(n) => {
                kind = context.styles.kind(raw.s);
                CellValue::Number(match kind {
                    Some(NumberKind::Date) if !context.date_1904 => dates::correct(n),
                    _ => n,
                })
            }
            None => CellValue::Text(v),
        },
        _ => CellValue::Empty,
    };
    if value.is_empty() {
        return;
    }
    if report.cells >= MAX_CELLS {
        report.over_budget += 1;
        return;
    }
    report.cells += 1;
    sheet.set(at, value);
    if let Some(kind) = kind {
        sheet.set_kind(at, kind);
    }
}

/// `<v>` as a number. `xsd:double`'s spellings — exponents, a leading `+` — plus the one real
/// producers add, surrounding whitespace. Not a finite number is not a number: `INF` and `NaN`
/// are `xsd:double` lexical forms that no cell can hold, and they are carried as the text the
/// file wrote rather than as a value this model would then refuse to write.
fn number(v: &str) -> Option<f64> {
    v.trim().parse::<f64>().ok().filter(|n| n.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAIN: &str = crate::names::MAIN_T;

    fn sheet_of(body: &str, strings: &[Item], styles: &Styles, date_1904: bool) -> (Sheet, Report) {
        let xml = format!(r#"<worksheet xmlns="{MAIN}"><sheetData>{body}</sheetData></worksheet>"#);
        let context = Context {
            strings,
            styles,
            date_1904,
            null_date: date::DEFAULT_NULL_DATE,
        };
        let mut sheet = Sheet::new("S");
        let mut report = Report::default();
        read(
            xml.as_bytes(),
            &context,
            &mut sheet,
            &mut report,
            &mut Default::default(),
        );
        (sheet, report)
    }

    fn plain(body: &str) -> Sheet {
        sheet_of(body, &[], &Styles::default(), false).0
    }

    fn at(addr: &str) -> Pos {
        address::cell(addr).unwrap()
    }

    #[test]
    fn every_cell_type() {
        let strings = [Item {
            text: "shared".into(),
            rich: false,
        }];
        let (sheet, report) = sheet_of(
            r#"<row r="1">
                 <c r="A1"><v>42.5</v></c>
                 <c r="B1" t="s"><v>0</v></c>
                 <c r="C1" t="str"><f>"a"&amp;"b"</f><v>ab</v></c>
                 <c r="D1" t="inlineStr"><is><t>inline</t></is></c>
                 <c r="E1" t="b"><v>1</v></c>
                 <c r="F1" t="b"><v>0</v></c>
                 <c r="G1" t="e"><v>#DIV/0!</v></c>
                 <c r="H1" t="d"><v>2024-03-17</v></c>
               </row>"#,
            &strings,
            &Styles::default(),
            false,
        );
        assert_eq!(sheet.get(at("A1")), CellValue::Number(42.5));
        assert_eq!(sheet.get(at("B1")), CellValue::Text("shared".into()));
        assert_eq!(sheet.get(at("C1")), CellValue::Text("ab".into()));
        assert_eq!(sheet.get(at("D1")), CellValue::Text("inline".into()));
        assert_eq!(sheet.get(at("E1")), CellValue::Bool(true));
        assert_eq!(sheet.get(at("F1")), CellValue::Bool(false));
        assert_eq!(sheet.get(at("G1")), CellValue::Text("#DIV/0!".into()));
        assert_eq!(
            sheet.get(at("H1")),
            CellValue::Number(date::serial(2024, 3, 17, date::DEFAULT_NULL_DATE))
        );
        assert_eq!(sheet.kind(at("H1")), Some(NumberKind::Date));
        assert_eq!(report.cells, 8);
    }

    /// Row 3 mixes the two: the implicit counter resumes from the last explicit `r`, not from
    /// the number of cells seen. Row 5 jumps.
    #[test]
    fn position_is_implicit_when_r_is_absent() {
        let sheet = plain(
            r#"<row><c><v>11</v></c><c><v>12</v></c></row>
               <row><c><v>21</v></c></row>
               <row><c><v>31</v></c><c r="D3"><v>34</v></c><c><v>35</v></c></row>
               <row r="5"><c><v>51</v></c></row>
               <row><c><v>61</v></c></row>"#,
        );
        for (addr, want) in [
            ("A1", 11.0),
            ("B1", 12.0),
            ("A2", 21.0),
            ("A3", 31.0),
            ("D3", 34.0),
            ("E3", 35.0),
            ("A5", 51.0),
            ("A6", 61.0),
        ] {
            assert_eq!(sheet.get(at(addr)), CellValue::Number(want), "{addr}");
        }
        assert_eq!(sheet.get(at("B3")), CellValue::Empty);
    }

    #[test]
    fn empty_and_self_closed_cells_carry_nothing_and_do_not_derail_the_row() {
        let (sheet, report) = sheet_of(
            r#"<row r="1"><c r="A1" s="1"/><c r="B1"/><c r="C1"><v>3</v></c></row>"#,
            &[],
            &Styles::default(),
            false,
        );
        assert_eq!(sheet.get(at("C1")), CellValue::Number(3.0));
        assert_eq!(report.cells, 1);
        let sheet = plain(""); // `<sheetData></sheetData>`: no rows at all is a sheet
        assert_eq!(sheet.used_rows(), 0);
    }

    fn date_styles() -> Styles {
        let xml = format!(
            r#"<styleSheet xmlns="{MAIN}"><cellXfs><xf numFmtId="0"/><xf numFmtId="14"/><xf numFmtId="46"/><xf numFmtId="2"/></cellXfs></styleSheet>"#
        );
        crate::styles::read(xml.as_bytes())
    }

    /// The same `<v>` under four formats: only the date is corrected, and only the date and
    /// the time get a kind.
    #[test]
    fn the_format_decides_whether_a_number_is_a_date() {
        let styles = date_styles();
        let (sheet, _) = sheet_of(
            r#"<row r="1"><c r="A1" s="1"><v>59</v></c><c r="B1" s="2"><v>1</v></c><c r="C1" s="3"><v>59</v></c><c r="D1"><v>59</v></c></row>"#,
            &[],
            &styles,
            false,
        );
        assert_eq!(
            sheet.get(at("A1")),
            CellValue::Number(60.0),
            "a 1900 date below the phantom day"
        );
        assert_eq!(sheet.kind(at("A1")), Some(NumberKind::Date));
        assert_eq!(
            sheet.get(at("B1")),
            CellValue::Number(1.0),
            "a duration has no epoch"
        );
        assert_eq!(sheet.kind(at("B1")), Some(NumberKind::Time));
        assert_eq!(
            sheet.get(at("C1")),
            CellValue::Number(59.0),
            "0.00 is a number"
        );
        assert_eq!(sheet.kind(at("C1")), None);
        assert_eq!(
            sheet.get(at("D1")),
            CellValue::Number(59.0),
            "no format is not a date"
        );
    }

    #[test]
    fn the_1904_system_is_not_corrected() {
        let (sheet, _) = sheet_of(
            r#"<row r="1"><c r="A1" s="1"><v>59</v></c></row>"#,
            &[],
            &date_styles(),
            true,
        );
        assert_eq!(sheet.get(at("A1")), CellValue::Number(59.0));
    }

    #[test]
    fn a_rich_shared_string_is_counted_once_per_cell() {
        let strings = [Item {
            text: "a b".into(),
            rich: true,
        }];
        let (_, report) = sheet_of(
            r#"<row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="s"><v>0</v></c><c r="C1" t="s"><v>7</v></c></row>"#,
            &strings,
            &Styles::default(),
            false,
        );
        assert_eq!(report.dropped[&Dropped::RichText], 2);
        assert_eq!(report.cells, 2, "an index past the table carries nothing");
    }

    #[test]
    fn numbers_in_every_spelling_the_schema_allows() {
        let sheet = plain(
            r#"<row r="1"><c><v>1.2345E-7</v></c><c><v>+5</v></c><c><v>-0</v></c><c><v>1.500</v></c><c><v>INF</v></c></row>"#,
        );
        assert_eq!(sheet.get(at("A1")), CellValue::Number(1.2345e-7));
        assert_eq!(sheet.get(at("B1")), CellValue::Number(5.0));
        assert_eq!(sheet.get(at("C1")), CellValue::Number(-0.0));
        assert_eq!(sheet.get(at("D1")), CellValue::Number(1.5));
        assert_eq!(
            sheet.get(at("E1")),
            CellValue::Text("INF".into()),
            "not a number a cell can hold"
        );
    }

    #[test]
    fn the_far_corner_is_one_cell_not_seventeen_billion() {
        let sheet = plain(r#"<row r="1048576"><c r="XFD1048576"><v>6</v></c></row>"#);
        assert_eq!(
            sheet.get(Pos::new(1_048_575, 16_383)),
            CellValue::Number(6.0)
        );
    }
}
