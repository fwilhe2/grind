// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Loop C — round-trip differential against LibreOffice.
//!
//! The strictest of the three loops, and the only one that checks the *writer*. Reading
//! the spec tells you what is legal; only LibreOffice tells you what it does with it, and
//! the gap between those two is where a writer's bugs live.
//!
//! Both directions:
//!
//! * **out** — build a document, write it, have LO convert it, read LO's output back, and
//!   assert we get the same document. Catches everything we emit that LO drops, collapses
//!   or reinterprets.
//! * **back** — take an LO-authored file, read it, write it, convert *that*, read it back,
//!   and assert it still matches what we first read. Catches the writer losing something
//!   the reader understood.
//!
//! This is also the enforcement mechanism for the anti-bloat rule (doc/plan.md, rule 7): a
//! feature that does not survive this fails CI, so the feature line is defended by a
//! machine rather than by discipline.
//!
//! Needs `soffice` on `PATH`; skips with a notice without one.

use std::path::{Path, PathBuf};
use std::process::Command;

use sheet_core::filter::Filter;
use sheet_core::locale::Locale;
use sheet_core::model::NumberKind;
use sheet_core::numfmt::{Format, Kind, Map, Op, Part};
use sheet_core::style::{self, CellStyle};
use sheet_core::{CellValue, Document, Form, Pos, Sheet};

const DEFAULT_CORPUS: &str = "/home/florian/code/github.com/LibreOffice/core/sc/qa/unit/data";

/// How many corpus documents the "back" direction takes. Each soffice conversion is
/// seconds, so this is a sample rather than the whole corpus — loop A already reads all
/// 361, and what is under test here is the writer, which does not vary per file.
const SAMPLE: usize = 20;

/// Skip corpus documents bigger than this, measured as rows × columns. A cell-by-cell
/// comparison over a sheet the size of a whole grid is not a better test, only a slower one.
const MAX_COMPARED_CELLS: u64 = 200_000;

// --- driving LibreOffice ---------------------------------------------------------------

/// A scratch directory that cleans itself up.
struct Lab {
    dir: PathBuf,
}

impl Lab {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("sheet-loop-c-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("in")).unwrap();
        std::fs::create_dir_all(dir.join("out")).unwrap();
        Self { dir }
    }

    fn input(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.dir.join("in").join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    /// Convert every staged input to flat XML in **one** soffice invocation.
    ///
    /// One invocation because startup dominates: a couple of seconds each time, against
    /// milliseconds per document once it is up. The private `UserInstallation` profile is
    /// not optional — without it this fights the developer's own running LibreOffice for
    /// the profile lock and either blocks or silently does nothing.
    fn convert(&self, inputs: &[PathBuf]) -> PathBuf {
        let out = self.dir.join("out");
        let status = Command::new("soffice")
            .arg("--headless")
            .arg(format!(
                "-env:UserInstallation=file://{}",
                self.dir.join("profile").display()
            ))
            .args(["--convert-to", "fods", "--outdir"])
            .arg(&out)
            .args(inputs)
            .status()
            .expect("soffice failed to start");
        assert!(status.success(), "soffice exited with {status}");
        out
    }
}

impl Drop for Lab {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn have_soffice() -> bool {
    Command::new("soffice")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Read back what LO produced for one staged input, by name.
fn converted(out: &Path, input: &Path) -> Document {
    let name = input.file_stem().unwrap().to_str().unwrap();
    let path = out.join(format!("{name}.fods"));
    assert!(
        path.exists(),
        "LibreOffice produced no output for {}: it could not open what we wrote",
        input.display()
    );
    sheet_core::read_file(&path).unwrap_or_else(|e| panic!("re-reading {}: {e}", path.display()))
}

// --- comparison ------------------------------------------------------------------------

/// Are these the same value, to the precision LibreOffice is capable of preserving?
///
/// The one loosening in this loop, named here rather than hidden in a tolerance constant.
/// LO writes every ODF double at 15 significant digits, where a double needs 17 to
/// round-trip (doc/ods-format.md §3.4, citing `sal/rtl/math.cxx:364-366` — LO's own reader
/// special-cases the `DBL_MAX` its writer mangles). Comparing exactly here would not be a
/// stricter test of our writer; it would be a test of LibreOffice's, and one it fails on
/// `1/3`. Everything else compares exactly.
///
/// The tolerance follows from the 15 digits: rounding there costs up to half an ulp at
/// that precision, which is a relative 5e-15 when the leading digit is 1. `1e-14` is that
/// with one factor of two in hand — still four orders of magnitude tighter than any real
/// writer bug, which loses a digit or a decimal point rather than a fifteenth place.
fn same(a: &CellValue, b: &CellValue) -> bool {
    match (a, b) {
        (CellValue::Number(x), CellValue::Number(y)) => {
            x == y || (x - y).abs() <= 1e-14 * x.abs().max(y.abs())
        }
        _ => a == b,
    }
}

/// Every way two documents differ, as sentences. Not `assert_eq!` on the whole document:
/// the interesting output is *which cell* moved, and a `Debug` dump of two sheets is not
/// something anyone reads.
fn differences(label: &str, want: &Document, got: &Document) -> Vec<String> {
    let mut out = Vec::new();
    if want.sheets.len() != got.sheets.len() {
        out.push(format!(
            "{label}: {} sheets in, {} out",
            want.sheets.len(),
            got.sheets.len()
        ));
        return out;
    }
    // §5.11 names, which are document-level rather than per-sheet.
    if want.names != got.names {
        out.push(format!(
            "{label}: names {:?}, back as {:?}",
            want.names, got.names
        ));
    }
    // The epoch (§3.4 `HOST-NULL-DATE`), which changes what every serial number *means* —
    // a document that came back with a different one has silently moved all its dates.
    if (want.null_date, want.null_year) != (got.null_date, got.null_year) {
        out.push(format!(
            "{label}: epoch {:?}, back as {:?}",
            (want.null_date, want.null_year),
            (got.null_date, got.null_year)
        ));
    }
    for (i, (w, g)) in want.sheets.iter().zip(&got.sheets).enumerate() {
        if w.name != g.name {
            out.push(format!(
                "{label}: sheet {i} named {:?}, back as {:?}",
                w.name, g.name
            ));
        }
        // §5.4, compared as the measurement rather than as the string: LibreOffice respells
        // every length in centimetres and quantises it to 1/100 mm, so `2.5cm` comes back
        // `2.499cm`.
        //
        // Within the used extent only, and for the same reason number formats are compared
        // as display text rather than as a struct: outside it, a width is not something the
        // *document* says. ODF has no way to declare a column without declaring its width,
        // so an LO file spells its own default across every unused column to the sheet's
        // edge, and LO drops those again on the way back. Asserting there would test which
        // widths LibreOffice considers worth writing. A sized column past the content is
        // still written — `a_sized_track_past_the_used_extent_is_still_declared` in
        // `odf::write` is what holds that.
        for (col, width) in w.col_widths().filter(|(c, _)| *c < w.used_cols()) {
            if !same_length(width, g.col_width(col)) {
                out.push(format!(
                    "{label}: sheet {i} column {col} was {width:?} wide, back as {:?}",
                    g.col_width(col)
                ));
            }
        }
        for (row, height) in w.row_heights() {
            if !same_length(height, g.row_height(row)) {
                out.push(format!(
                    "{label}: sheet {i} row {row} was {height:?} high, back as {:?}",
                    g.row_height(row)
                ));
            }
        }
        // §9.4. Compared as the model rather than as XML, since the range address has
        // several legal spellings and LO picks its own.
        if w.filter() != g.filter() {
            out.push(format!(
                "{label}: sheet {i} filter was {:?}, back as {:?}",
                w.filter(),
                g.filter()
            ));
        }
        let rows = w.used_rows().max(g.used_rows());
        let cols = w.used_cols().max(g.used_cols());
        for row in 0..rows {
            for col in 0..cols {
                let pos = Pos::new(row, col);
                if !same(&w.get(pos), &g.get(pos)) {
                    out.push(format!(
                        "{label}: sheet {i} r{row}c{col} was {:?}, back as {:?}",
                        w.get(pos),
                        g.get(pos)
                    ));
                }
                if w.formula(pos) != g.formula(pos) {
                    out.push(format!(
                        "{label}: sheet {i} r{row}c{col} formula was {:?}, back as {:?}",
                        w.formula(pos),
                        g.formula(pos)
                    ));
                }
                // §4.3.2/§4.3.3: a date and a time are Numbers, so a lost `NumberKind` is
                // invisible to `same` above — and is exactly how a date document comes back
                // as a column of five-digit integers.
                if w.kind(pos) != g.kind(pos) {
                    out.push(format!(
                        "{label}: sheet {i} r{row}c{col} kind was {:?}, back as {:?}",
                        w.kind(pos),
                        g.kind(pos)
                    ));
                }
                // §5, compared as the text the user sees rather than as a `Format` struct.
                // Style *names* are LibreOffice's to renumber, and the struct is one step
                // too literal in the other direction: a date cell we write with no format of
                // its own comes back carrying the ISO date style the writer supplies for it,
                // which is not a difference in what the document says. What must not change
                // is the rendering.
                if !same_style(w.style(pos), g.style(pos)) {
                    out.push(format!(
                        "{label}: sheet {i} r{row}c{col} style was {:?}, back as {:?}",
                        w.style(pos),
                        g.style(pos)
                    ));
                }
                let (before, after) = (shown(w, pos, want), shown(g, pos, got));
                if before != after {
                    out.push(format!(
                        "{label}: sheet {i} r{row}c{col} displayed {before:?}, back as \
                         {after:?}"
                    ));
                }
                if out.len() > 20 {
                    out.push(format!("{label}: ... and more"));
                    return out;
                }
            }
        }
    }
    out
}

/// The same length, in millimetres, to within LibreOffice's own quantisation.
///
/// A tenth of a millimetre: LO stores lengths in 1/100 mm and rounds to three decimal places
/// of a centimetre, so `0.9in` (22.86 mm) comes back as `2.286cm` and `21pt` as `0.741cm`.
fn same_length(want: &str, got: Option<&str>) -> bool {
    match (style::length_mm(want), got.and_then(style::length_mm)) {
        (Some(a), Some(b)) => (a - b).abs() < 0.1,
        _ => false,
    }
}

/// Are these the same cell style, allowing for LibreOffice's own normalisation?
///
/// The one place styles are not compared as written, and for a measured reason
/// (doc/ods-format.md §5.4): LO converts a border width to its internal unit and back, so
/// `0.5pt` returns as `0.51pt`. Everything else about a border — the line style, the colour
/// — and every other property must come back exactly.
fn same_style(a: Option<&CellStyle>, b: Option<&CellStyle>) -> bool {
    let (Some(a), Some(b)) = (a, b) else {
        return a.is_none() && b.is_none();
    };
    let borders = a.borders.iter().zip(&b.borders).all(|(a, b)| {
        match (
            a.as_deref().and_then(style::border_parts),
            b.as_deref().and_then(style::border_parts),
        ) {
            (Some((wa, sa, ca)), Some((wb, sb, cb))) => {
                (wa - wb).abs() < 0.05 && sa == sb && ca == cb
            }
            _ => a == b,
        }
    });
    borders
        && CellStyle {
            borders: Default::default(),
            ..a.clone()
        } == CellStyle {
            borders: Default::default(),
            ..b.clone()
        }
}

/// What a cell displays: through its number format, or through the plain spelling of its
/// value when it has none.
fn shown(sheet: &Sheet, pos: Pos, doc: &Document) -> String {
    match sheet.format(pos) {
        Some(format) => format.render(&sheet.get(pos), doc.null_date),
        None => sheet_core::numfmt::general(&sheet.get(pos), sheet.kind(pos), doc.null_date),
    }
}

// --- direction "out": ours -> LibreOffice -> ours ---------------------------------------

/// One named document to push through LO, built cell by cell.
fn case(name: &str, cells: &[(u32, u32, CellValue)]) -> (String, Document) {
    let mut doc = Document {
        sheets: vec![Sheet::new("Data")],
        ..Default::default()
    };
    let sheet = doc.sheet_mut(0).unwrap();
    for (row, col, value) in cells {
        sheet.set(Pos::new(*row, *col), value.clone());
    }
    (name.to_owned(), doc)
}

/// Dates and times — §4.3.2, §4.3.3, and the reason [`Sheet::kind`] exists.
///
/// A date is stored as a Number, so every one of these cells would survive `same()` as a
/// bare float. The check that matters is the `NumberKind` beside it: if LibreOffice gets a
/// serial number where the document meant a date, the user sees 30347 instead of a date and
/// the feature is not real.
fn dates() -> (String, Document) {
    let e = sheet_core::formula::date::DEFAULT_NULL_DATE;
    let day = |y, m, d| sheet_core::formula::date::serial(y, m, d, e);

    let mut doc = Document {
        sheets: vec![Sheet::new("Data")],
        ..Default::default()
    };
    let sheet = doc.sheet_mut(0).unwrap();
    for (row, col, value, kind) in [
        (0, 0, day(1983, 1, 31), NumberKind::Date),
        // The epoch itself, and the day either side of it — where an off-by-one lands.
        (0, 1, day(1899, 12, 30), NumberKind::Date),
        (0, 2, day(1899, 12, 31), NumberKind::Date),
        // Before the epoch: a negative serial, which §4.3.3 says evaluators *may* support
        // and which the corpus contains.
        (0, 3, day(1899, 1, 1), NumberKind::Date),
        // 1900 is not a leap year here; a reader that thinks it is lands a day out.
        (0, 4, day(1900, 3, 1), NumberKind::Date),
        // A date carrying a time is a DateTime (§4.3.4) and takes the combined spelling.
        (1, 0, day(2026, 8, 16) + 0.5, NumberKind::Date),
        (2, 0, 0.5, NumberKind::Time),
        (2, 1, 0.0, NumberKind::Time),
        // Sub-second precision, and a duration longer than a day — both real corpus cells
        // and both things a naive formatter quietly rounds away.
        (
            2,
            2,
            (5.0 * 3600.0 + 35.0 * 60.0 + 31.2) / 86_400.0,
            NumberKind::Time,
        ),
        (2, 3, 1.40625, NumberKind::Time),
    ] {
        sheet.set(Pos::new(row, col), CellValue::Number(value));
        sheet.set_kind(Pos::new(row, col), kind);
    }
    ("dates".to_owned(), doc)
}

/// Number formats — §5.2, and the phase that makes a date print as a date.
///
/// Every family in the Small Group's reach, each on a cell whose *value* also has to
/// survive: a format that comes back changed shows the user something else, and a format
/// that comes back attached to a different cell is worse.
fn formats() -> (String, Document) {
    let mut doc = Document {
        sheets: vec![Sheet::new("Data")],
        ..Default::default()
    };
    let epoch = doc.null_date;

    let number = |decimals, min_decimals, grouping| {
        let mut f = Format::new(Kind::Number);
        f.push(Part::Number {
            decimals,
            min_decimals,
            min_int: 1,
            grouping,
        });
        f
    };
    let mut percent = Format::new(Kind::Percentage);
    percent.push(Part::Number {
        decimals: 1,
        min_decimals: 1,
        min_int: 1,
        grouping: false,
    });
    percent.push(Part::Text("%".into()));

    // §5.1's two-branch currency: the style itself is the negative case, spelling its own
    // minus, and a `style:map` switches to the plain one at zero and above. Both cells below
    // wear it, so the file has to carry the branch *and* apply the right one to each.
    let mut plain = Format::new(Kind::Currency);
    plain.push(Part::Number {
        decimals: 2,
        min_decimals: 2,
        min_int: 1,
        grouping: true,
    });
    plain.push(Part::Text("\u{a0}".into()));
    plain.push(Part::Currency("\u{20ac}".into()));
    let mut currency = Format::new(Kind::Currency);
    currency.push(Part::Text("-".into()));
    currency.parts.extend(plain.parts.iter().cloned());
    currency.maps.push(Map {
        op: Op::Ge,
        value: "0".into(),
        format: plain,
    });

    // A date spelled the way a European locale spells it — the case that proves the format
    // is carried rather than the value being re-derived from `office:date-value`.
    let mut date = Format::new(Kind::Date);
    date.push(Part::Day { long: true });
    date.push(Part::Text(".".into()));
    date.push(Part::Month {
        long: true,
        textual: false,
    });
    date.push(Part::Text(".".into()));
    date.push(Part::Year { long: true });

    let mut clock = Format::new(Kind::Time);
    clock.push(Part::Hours { long: true });
    clock.push(Part::Text(":".into()));
    clock.push(Part::Minutes { long: true });
    clock.push(Part::Text(" ".into()));
    clock.push(Part::AmPm);

    // The same shape of format in another locale: `1.234,50` rather than `1,234.50`, from
    // the same parts. Loop C compares what the cell *displays*, so a lost locale fails here.
    let german = number(2, 2, true).in_locale(Locale::parse("de-DE"));

    let sheet = doc.sheet_mut(0).unwrap();
    let cells: Vec<(CellValue, Option<NumberKind>, Format)> = vec![
        (CellValue::Number(1234.5), None, german),
        (CellValue::Number(1234.5), None, number(2, 2, true)),
        // The same family with different attributes: two styles, not one, and a pool that
        // conflates them shows one of the cells the wrong number of digits.
        (CellValue::Number(1234.5), None, number(0, 0, false)),
        (CellValue::Number(0.075), None, percent),
        (CellValue::Number(-19.99), None, currency.clone()),
        (CellValue::Number(19.99), None, currency),
        (
            CellValue::Number(sheet_core::formula::date::serial(2026, 8, 16, epoch)),
            Some(NumberKind::Date),
            date,
        ),
        (CellValue::Number(0.5), Some(NumberKind::Time), clock),
    ];
    for (row, (value, kind, format)) in cells.into_iter().enumerate() {
        let pos = Pos::new(row as u32, 0);
        sheet.set(pos, value);
        if let Some(kind) = kind {
            sheet.set_kind(pos, kind);
        }
        sheet.set_format(pos, format);
    }
    // A formatted cell beside an unformatted one holding the same value: the style must not
    // spread sideways, which is the failure a repeated-cell optimisation introduces.
    sheet.set(Pos::new(0, 1), CellValue::Number(1234.5));

    ("formats".to_owned(), doc)
}

/// Cell styling — §5.1, and the half of phase 5 that is not about the value.
///
/// Deliberately no `fo:font-family`: LibreOffice replaces it with a `style:font-name`
/// pointing into `office:font-face-decls` (§5.4), so it is not something a document can
/// carry through unchanged and the model does not pretend otherwise.
fn styles() -> (String, Document) {
    let mut doc = Document {
        sheets: vec![Sheet::new("Data")],
        ..Default::default()
    };
    let sheet = doc.sheet_mut(0).unwrap();

    let header = CellStyle {
        font_weight: Some("bold".into()),
        background: Some("#ffff00".into()),
        align: Some("center".into()),
        ..Default::default()
    };
    let emphasis = CellStyle {
        font_style: Some("italic".into()),
        color: Some("#ff0000".into()),
        font_size: Some("14pt".into()),
        ..Default::default()
    };
    let mut boxed = CellStyle {
        vertical_align: Some("middle".into()),
        wrap: Some("wrap".into()),
        ..Default::default()
    };
    boxed.set_border(Some("0.5pt solid #000000".into()));
    // Edges that disagree, which is the case the `fo:border` shorthand cannot express.
    let mut ruled = CellStyle::default();
    ruled.borders[0] = Some("1pt solid #0000ff".into());
    ruled.borders[3] = Some("0.06pt dashed #00ff00".into());

    for (row, style) in [header, emphasis, boxed, ruled].into_iter().enumerate() {
        let pos = Pos::new(row as u32, 0);
        sheet.set(pos, CellValue::Text(format!("styled {row}")));
        sheet.set_style(pos, style);
    }
    // A styled cell that also carries a number format: one `style:style` holds both, and
    // the pool has to key on the pair or one of them is lost.
    let both = Pos::new(4, 0);
    sheet.set(both, CellValue::Number(0.5));
    sheet.set_format(
        both,
        Format {
            kind: Kind::Percentage,
            parts: vec![
                Part::Number {
                    decimals: 0,
                    min_decimals: 0,
                    min_int: 1,
                    grouping: false,
                },
                Part::Text("%".into()),
            ],
            locale: None,
            maps: Vec::new(),
        },
    );
    sheet.set_style(
        both,
        CellStyle {
            font_weight: Some("bold".into()),
            ..Default::default()
        },
    );
    // The same styling on a second cell, so the pool is exercised rather than trusted.
    let twin = Pos::new(5, 0);
    sheet.set(twin, CellValue::Number(1.0));
    sheet.set_style(
        twin,
        CellStyle {
            font_weight: Some("bold".into()),
            ..Default::default()
        },
    );

    ("styles".to_owned(), doc)
}

/// Column widths and row heights (§5.4) — a `style:style` of family `table-column` or
/// `table-row`, named from the track declaration rather than from a cell.
///
/// Three units, because the model keeps the string the document wrote and LibreOffice
/// respells all of them in centimetres: what has to survive is the *measurement*, which is
/// why the comparison is in millimetres. A sized column past the last value is the case that
/// catches a writer bounding its declarations by the used extent.
fn tracks() -> (String, Document) {
    let mut doc = Document {
        sheets: vec![Sheet::new("Data")],
        ..Default::default()
    };
    let sheet = doc.sheet_mut(0).unwrap();
    for row in 0..3u32 {
        sheet.set(Pos::new(row, 0), CellValue::Text(format!("row {row}")));
    }
    sheet.set_col_width(0, Some("3.5cm".into()));
    sheet.set_col_width(1, Some("0.9in".into()));
    // Past the used extent: nothing is written in column E, and it is still 5cm wide.
    sheet.set_col_width(4, Some("5cm".into()));
    sheet.set_row_height(0, Some("14mm".into()));
    sheet.set_row_height(2, Some("21pt".into()));
    ("tracks".to_owned(), doc)
}

/// An autofilter (§9.4): the range, the values it keeps, and the rows it therefore hides.
fn filtered() -> (String, Document) {
    let mut doc = Document {
        sheets: vec![Sheet::new("Data")],
        ..Default::default()
    };
    let sheet = doc.sheet_mut(0).unwrap();
    for (row, product) in ["Product", "Chair", "Desk", "Lamp"].iter().enumerate() {
        sheet.set(
            Pos::new(row as u32, 0),
            CellValue::Text((*product).to_owned()),
        );
    }
    let mut filter = Filter::new("__Anonymous_Sheet_DB__0", Pos::new(0, 0), Pos::new(3, 0));
    filter
        .keep
        .insert(0, ["Chair".to_owned(), "Desk".to_owned()].into());
    sheet.set_filter(Some(filter));
    ("filtered".to_owned(), doc)
}

fn cases() -> Vec<(String, Document)> {
    let n = |x: f64| CellValue::Number(x);
    let t = |s: &str| CellValue::Text(s.to_owned());

    let mut named = case(
        "named-expressions",
        &[(0, 0, n(1.0)), (1, 0, n(2.0)), (2, 0, n(3.0))],
    );
    // A named range and a named expression, which ODF spells differently and we store the
    // same way (§5.11, `Document::names`). LO has to give both back recognisably.
    named
        .1
        .names
        .insert("data_range".into(), "[$Data.$A$1:.$A$3]".into());
    named
        .1
        .names
        .insert("total".into(), "of:=SUM([$Data.$A$1:.$A$3])".into());

    let mut all = vec![
        // The degenerate document: one sheet, nothing in it. Legal, and the shape most
        // likely to be written as something LO rejects outright (§3.2).
        ("empty".to_owned(), Document::default()),
        named,
        formats(),
        styles(),
        tracks(),
        filtered(),
        case(
            "numbers",
            &[
                (0, 0, n(0.0)),
                (0, 1, n(-1.5)),
                (0, 2, n(1e15)),
                (0, 3, n(0.1)),
                // Precision is the point: a double that needs all 17 significant digits
                // to survive is where a writer that formats with `{:.6}` gets caught.
                (0, 4, n(0.123_456_789_012_345_67)),
                (1, 0, n(1.0 / 3.0)),
            ],
        ),
        case(
            "text",
            &[
                (0, 0, t("plain")),
                // XML metacharacters, in an attribute and in text.
                (0, 1, t("<tag> & \"quoted\" 'single'")),
                (0, 2, t("Grüße — ünïcodé ✓")),
                // Whitespace: readers collapse runs inside text:p, so all three of these
                // come back wrong unless text:s is written.
                (0, 3, t("  leading")),
                (0, 4, t("trailing  ")),
                (1, 0, t("inner    spaces")),
                (1, 1, t("tab\there")),
                // Multi-line, including a blank line, which is the case that dies if
                // paragraphs are joined by testing whether the buffer is empty.
                (1, 2, t("line1\nline2")),
                (1, 3, t("\nleading blank")),
                (1, 4, t("a\n\nb")),
                // Text that looks like a number must stay text.
                (2, 0, t("42")),
                (2, 1, t("")),
            ],
        ),
        case(
            "booleans",
            &[
                (0, 0, CellValue::Bool(true)),
                (0, 1, CellValue::Bool(false)),
            ],
        ),
        dates(),
        // Sparse: gaps are written as repeats, and a repeat that is off by one moves every
        // later cell silently.
        case(
            "sparse",
            &[
                (0, 0, n(1.0)),
                (0, 7, n(2.0)),
                (40, 0, n(3.0)),
                (40, 7, n(4.0)),
            ],
        ),
    ];

    let mut many = Document {
        sheets: vec![
            Sheet::new("First"),
            Sheet::new("Second"),
            // A space is what `sheet add` lets a person type, and it is the character a
            // reference has to quote (§5.8) — so it is the one worth putting through
            // LibreOffice rather than assuming.
            Sheet::new("Q3 Actuals"),
        ],
        ..Default::default()
    };
    many.sheet_mut(0).unwrap().set(Pos::new(0, 0), n(1.0));
    many.sheet_mut(2).unwrap().set(Pos::new(2, 2), t("third"));
    all.push(("sheets".to_owned(), many));

    all
}

#[test]
fn documents_we_write_survive_libreoffice() {
    if !have_soffice() {
        eprintln!("skipping loop C: no soffice on PATH");
        return;
    }

    let lab = Lab::new("out");
    let cases = cases();
    // Both physical forms, every case. They share a content writer but not a container,
    // and the container is where §1.1's byte-level rules live.
    let staged: Vec<_> = cases
        .iter()
        .flat_map(|(name, doc)| {
            [(Form::Flat, "fods"), (Form::Package, "ods")].map(|(form, ext)| {
                let bytes = sheet_core::write_bytes(doc, form).unwrap();
                // Distinct stems: LO names its output after the input's stem, so
                // `x.ods` and `x.fods` would both convert onto `x.fods`.
                let path = lab.input(&format!("{name}-{ext}.{ext}"), &bytes);
                (name.clone(), doc, path)
            })
        })
        .collect();

    let out = lab.convert(&staged.iter().map(|(_, _, p)| p.clone()).collect::<Vec<_>>());

    let mut failures = Vec::new();
    for (_, doc, path) in &staged {
        let label = path.file_name().unwrap().to_str().unwrap();
        failures.extend(differences(label, doc, &converted(&out, path)));
    }

    for f in &failures {
        eprintln!("  {f}");
    }
    assert!(
        failures.is_empty(),
        "loop C (out): {} differences",
        failures.len()
    );
}

// --- direction "back": LibreOffice -> ours -> LibreOffice -> ours ------------------------

/// How many cells in this document actually hold something.
fn values(doc: &Document) -> usize {
    doc.sheets
        .iter()
        .map(|s| {
            (0..s.used_rows())
                .flat_map(|r| (0..s.used_cols()).map(move |c| Pos::new(r, c)))
                .filter(|p| !s.get(*p).is_empty())
                .count()
        })
        .sum()
}

/// Corpus documents to push back out through the writer.
///
/// Formula-bearing documents are excluded, and this is the phase-3 exit criterion talking
/// (doc/plan.md: "loop C green for value-only documents"), not a convenience: LO
/// recalculates on load, so a formula cell's value after a round trip is a statement about
/// an evaluator that does not exist until phase 4. Phase 4 lifts this filter.
fn sample(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for dir in ["ods", "fods"] {
        let Ok(entries) = std::fs::read_dir(root.join(dir)) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("ods" | "fods")
            ) {
                files.push(path);
            }
        }
    }
    let mut eligible: Vec<_> = files
        .iter()
        .filter_map(|path| {
            // Unreadable or encrypted: loop A owns that verdict, not this one.
            let doc = sheet_core::read_file(path).ok()?;
            let biggest = doc
                .sheets
                .iter()
                .map(|s| u64::from(s.used_rows()) * u64::from(s.used_cols()))
                .max()?;
            let formula_free = doc.sheets.iter().all(|s| s.formula_count() == 0);
            (formula_free && biggest < MAX_COMPARED_CELLS).then(|| (values(&doc), path.clone()))
        })
        .collect();

    // Densest first. Taking the alphabetically first twenty instead gives a sample of
    // regression fixtures — single-cell documents that reproduce one import bug each — and
    // three hundred cells to check the whole writer against.
    eligible.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    eligible.truncate(SAMPLE);
    eligible.sort_by(|a, b| a.1.cmp(&b.1));
    eligible.into_iter().map(|(_, path)| path).collect()
}

#[test]
fn libreoffice_documents_survive_our_writer() {
    if !have_soffice() {
        eprintln!("skipping loop C: no soffice on PATH");
        return;
    }
    let root = PathBuf::from(
        std::env::var("SHEET_LO_CORPUS").unwrap_or_else(|_| DEFAULT_CORPUS.to_owned()),
    );
    if !root.is_dir() {
        eprintln!(
            "skipping loop C (back): no LibreOffice corpus at {}",
            root.display()
        );
        return;
    }

    let files = sample(&root);
    assert!(
        !files.is_empty(),
        "no value-only documents found in {}",
        root.display()
    );

    let lab = Lab::new("back");
    let staged: Vec<_> = files
        .iter()
        .enumerate()
        .map(|(i, path)| {
            let doc = sheet_core::read_file(path).unwrap();
            let bytes = sheet_core::write_bytes(&doc, Form::Package).unwrap();
            // Numbered: corpus stems are not unique across `ods/` and `fods/`.
            let staged = lab.input(&format!("{i:03}.ods"), &bytes);
            (path.clone(), doc, staged)
        })
        .collect();

    // A comparison finds nothing in a document that holds nothing, so the sample has to be
    // shown to have substance — otherwise a drifting filter turns this into twenty empty
    // documents agreeing with twenty empty documents, which passes forever.
    let cells: usize = staged.iter().map(|(_, doc, _)| values(doc)).sum();
    eprintln!(
        "loop C (back): {} value-only documents, {cells} cells",
        files.len()
    );
    assert!(
        cells > 5_000,
        "sample holds only {cells} cells; it is not testing the writer"
    );

    let out = lab.convert(&staged.iter().map(|(_, _, p)| p.clone()).collect::<Vec<_>>());

    let mut failures = Vec::new();
    for (original, doc, path) in &staged {
        let label = original.file_name().unwrap().to_str().unwrap();
        failures.extend(differences(label, doc, &converted(&out, path)));
    }

    for f in failures.iter().take(30) {
        eprintln!("  {f}");
    }
    assert!(
        failures.is_empty(),
        "loop C (back): {} differences",
        failures.len()
    );
}

// --- R7's kb/ corpus: rendered, not just read -------------------------------------------

/// R7's hand-written half (`kb.rs`'s `KB`), named again rather than shared: each test binary
/// is its own crate, and eight file names is cheaper than a shared support module.
const KB: [&str; 8] = [
    "filter.fods",
    "fizzbuzz.fods",
    "formula.fods",
    "minimal.fods",
    "minimal-libreoffice.fods",
    "minimal-libreoffice-cleanup.fods",
    "minimal-with-styles.fods",
    "named-range.fods",
];

/// `minimal.fods`'s date, time and two booleans (`r0c5`, `r0c6`, `r0c10`, `r0c11`) carry no
/// `table:style-name` at all — the whole point of that fixture (`kb.rs`: "twelve cells, no
/// styles"). §5.2 leaves a styleless cell's display implementation-defined, and the two
/// implementations pick differently: we still honour `office:value-type` and show a date, a
/// time and a boolean; LibreOffice's own round trip degrades all four to a bare
/// `office:value-type="float"`, so it shows the serial number and `1`/`0`. Both are legal
/// readings of the same file, so this is the one named exception rather than a bug either
/// side needs to fix.
fn expected_render_gap(name: &str, sheet: usize, pos: Pos) -> bool {
    name == "minimal.fods" && sheet == 0 && pos.row == 0 && matches!(pos.col, 5 | 6 | 10 | 11)
}

/// `kb.rs` checks these eight read and round-trip; this checks they *display* the same as
/// LibreOffice shows them, cell by cell, formulas recalculated on both sides. Narrower than
/// [`differences`] on purpose — `fizzbuzz.fods` and `formula.fods` carry no cached values at
/// all, so what they have in common with LibreOffice is only ever the rendered text, never
/// the raw `CellValue` LO's own recalculation happens to produce.
///
/// **Not** a claim that a shell shows this the instant it opens one of these files. Neither
/// side auto-recalculates on open: `ui_gtk::main`'s `open_file` never calls `App::recalc`,
/// only F9/"Recalculate Now" does, and `a_document_of_formulas_with_no_cached_values_
/// recalculates` (`kb.rs`) documents LibreOffice doing the same — `fizzbuzz.fods` is blank
/// in both until something recalculates it. `soffice --convert-to`, which is what builds the
/// LibreOffice side here, recalculates as part of the conversion, and the explicit
/// `app.recalc()` a few lines down matches that on our side. So this test is "our formula
/// engine agrees with LibreOffice's once both have recalculated", not "opening the file
/// looks right" — the latter is a real, separate, as-designed gap and not what a green run
/// here says anything about.
#[test]
fn kb_documents_render_the_same_in_libreoffice() {
    if !have_soffice() {
        eprintln!("skipping loop C (kb render): no soffice on PATH");
        return;
    }

    let lab = Lab::new("kb-render");
    let staged: Vec<_> = KB
        .iter()
        .map(|name| {
            let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/data/kb")
                .join(name);
            let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
            (*name, lab.input(name, &bytes))
        })
        .collect();
    let out = lab.convert(&staged.iter().map(|(_, p)| p.clone()).collect::<Vec<_>>());

    let mut failures = Vec::new();
    for (name, path) in &staged {
        let app = sheet_core::App::new();
        app.open_file(path)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        app.recalc().unwrap_or_else(|e| panic!("{name}: {e}"));

        let got = converted(&out, path);

        for sheet in 0..app.sheet_count().max(got.sheets.len()) {
            let (want_rows, want_cols) = app.used_extent(sheet).unwrap_or((0, 0));
            let got_sheet = got.sheet(sheet);
            let got_extent = got_sheet.map_or((0, 0), |s| (s.used_rows(), s.used_cols()));
            let rows = want_rows.max(got_extent.0);
            let cols = want_cols.max(got_extent.1);

            let ours = app
                .get_viewport(sheet, 0..rows, 0..cols)
                .unwrap_or_else(|e| panic!("{name} sheet {sheet}: {e}"));
            for row in 0..rows {
                for col in 0..cols {
                    let pos = Pos::new(row, col);
                    let want = ours.text(row, col).unwrap_or("");
                    let theirs = match got_sheet {
                        Some(s) => shown(s, pos, &got),
                        None => String::new(),
                    };
                    if want != theirs && !expected_render_gap(name, sheet, pos) {
                        failures.push(format!(
                            "{name}: sheet {sheet} r{row}c{col} rendered {want:?}, \
                             LibreOffice shows {theirs:?}"
                        ));
                    }
                }
            }
        }
    }

    for f in failures.iter().take(30) {
        eprintln!("  {f}");
    }
    assert!(
        failures.is_empty(),
        "loop C (kb render): {} differences",
        failures.len()
    );
}
