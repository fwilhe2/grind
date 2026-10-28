// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `xl/worksheets/*.xml` — rows, cells, and their values. `<c r="B2" t="…" s="12"><v>…</v></c>`,
//! with `t` defaulting to `n`; the mapping is ECMA-376 §18.18.11's `ST_CellType`:
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
//! **A formula cell carries both** Excel's cached value and the expression ([`crate::formula`]);
//! one whose formula is refused keeps the value, and is counted.
//!
//! **Shared formulas** (`<f t="shared" ref="B2:B10" si="0">A2*2</f>`, followers carrying `<f
//! t="shared" si="0"/>`) are resolved after the whole sheet is read, since a master need not be
//! above its followers (`formulas/shared-groups.xlsx`: master F2, follower F1). The shift is
//! `grind_sheet::formula::shift`, the one a fill uses; a reference leaving the sheet becomes
//! `#REF!`.
//!
//! Position is **implicit** when `r` is absent — the next row, the next column — as Apache POI and
//! SheetJS write it; an explicit `r` moves the counter. `dimension` and `spans` are claims and
//! never read.
//!
//! **Geometry.** A row height is points, carried verbatim; a column width is characters, converted
//! by [`col_width`]. A `<col>` run to the sheet's last column is the sheet's background and is
//! carried over the used columns only (`grind_sheet::MAX_TRACK_RUN`'s rule), as is `<sheetFormatPr
//! defaultColWidth>`, since the model has no sheet default. An outline, frozen panes or a zero
//! size is an [`Appearance`].

use std::collections::HashMap;

use grind_sheet::formula::parse::Expr;
use grind_sheet::formula::{date, funcs, shift};
use grind_sheet::model::{CellValue, NumberKind, Pos, Sheet};
use grind_sheet::style::mm_length;
use grind_sheet::{MAX_COLS, MAX_ROWS, MAX_TRACK_RUN};

use crate::address;
use crate::dates;
use crate::formula::{self, Refusal};
use crate::report::{Dropped, Report};
use crate::strings::{self, Item};
use crate::styles::{Appearance, Styles};
use crate::xml::{Handled, Reader, Spreadsheet as _};

/// Cells one import will materialise, across every sheet — the bound `odf/read.rs` uses. An
/// `.xlsx` cannot amplify the way a repeated ODF row can, but a 512 MB `sheet1.xml` is still a
/// file somebody sent. Eight `scale/large-sheet.xlsx`es; a cell past it is counted in
/// [`Report::over_budget`].
pub const MAX_CELLS: usize = 4_000_000;

/// Text one import will materialise, in bytes. A shared string is stored once and may be named by
/// every cell, so a 100 KB package can ask for gigabytes; a text cell past it is over budget too.
pub const MAX_TEXT_BYTES: usize = if cfg!(test) { 1 << 20 } else { 512 << 20 };

/// What every sheet in one workbook reads its cells against.
pub struct Context<'a> {
    pub strings: &'a [Item],
    pub styles: &'a Styles,
    /// `workbookPr/@date1904`. In the 1904 system serials already agree with the document's
    /// null date and are carried unchanged; in the 1900 system a date needs [`dates::correct`].
    pub date_1904: bool,
    /// The document's null date, for `t="d"`'s ISO spelling.
    pub null_date: i64,
    /// Which names are sheet-local where: a formula naming one on that sheet is refused.
    pub names: &'a crate::workbook::Names,
    /// Sheets renamed on the way in, `(from, to)`; every formula follows them.
    pub renames: &'a [(String, String)],
    /// Every table in the workbook, so a structured reference becomes the range it names.
    pub tables: &'a crate::tables::Tables,
    /// Every sheet's name, by index, as the workbook spelled it.
    pub sheet_names: &'a [String],
}

/// Read one worksheet part into `sheet`. Flavour evidence and `mc:MustUnderstand` findings come
/// back through `report` and `seen`, since a Transitional workbook can hold a Strict worksheet
/// (`realworld/mixed-flavour.xlsx`). An XML error partway keeps every cell read before it,
/// silently — there is no `Dropped` for "the rest of a broken part".
pub fn read(
    bytes: &[u8],
    context: &Context<'_>,
    index: usize,
    sheet: &mut Sheet,
    report: &mut Report,
    seen: &mut crate::names::Seen,
) {
    let mut reader = Reader::new(bytes);
    let mut shared = Shared::default();
    let mut tracks = Tracks {
        digit: context.styles.digit_px(),
        ..Tracks::default()
    };
    let mut filter = None;
    let mut rules = crate::rules::Pending::default();
    if matches!(reader.root(), Ok(Some((ref root, _))) if root.is("worksheet")) {
        let _ = reader.children(|reader, name, attrs| {
            if name.is("sheetData") {
                rows(
                    reader,
                    context,
                    index,
                    sheet,
                    report,
                    &mut shared,
                    &mut tracks,
                )?;
            } else if name.is("cols") {
                cols(reader, sheet, report, &mut tracks)?;
            } else if name.is("sheetFormatPr") {
                // A stated default width is carried like a long `<col>` run, onto the columns
                // the sheet uses. A `baseColWidth` with no `defaultColWidth` beside it means a
                // default derived from a base, by arithmetic nobody here has measured; unless
                // it is Excel's own eight, it is counted instead.
                let number = |local: &str| {
                    attrs
                        .plain(local)
                        .and_then(|v| v.trim().parse::<f64>().ok())
                        .filter(|v| v.is_finite() && *v > 0.0)
                };
                tracks.default_width = number("defaultColWidth");
                // A default row height the author fixed is every row's that says nothing, as
                // the oracle draws it (`doc/xlsx-format.md` §4.6); one Excel derived from the
                // font is the height a shell gives a row anyway.
                tracks.default_height =
                    number("defaultRowHeight").filter(|_| attrs.flag("customHeight"));
                tracks.sheet_width |= tracks.default_width.is_none()
                    && number("baseColWidth").is_some_and(|base| (base - 8.0).abs() > 1e-9);
            } else if name.is("sheetViews") {
                tracks.pane |= panes(reader)?;
            } else if name.is("mergeCells") {
                merges(reader, sheet, report)?;
            } else if name.is("conditionalFormatting") {
                // The model's one rule type, where a `<cfRule>` is one (`rules.rs`).
                crate::rules::read(reader, attrs, context, index, &mut rules, report)?;
            } else if name.is("dataValidations") {
                let rules = count(reader, "dataValidation")?;
                report.drop_many(Dropped::DataValidation, rules);
            } else if name.is("sheetProtection") {
                // A UI lock with a hash beside it, not encryption: every value is readable.
                // `sheet` is what switches it on, and it defaults to off (§18.3.1.85).
                if attrs.flag("sheet") {
                    report.drop_one(Dropped::Protection);
                }
            } else if name.is("autoFilter") {
                filter = autofilter(reader, attrs, report)?;
            } else {
                return Ok(Handled::No);
            }
            Ok(Handled::Yes)
        });
    }
    shared.resolve(index, context, sheet, report);
    tracks.finish(sheet, report);
    rules.finish(sheet);
    if let Some(filter) = filter {
        apply_filter(sheet, filter, context.null_date);
    }
    report.must_understand.append(&mut reader.must_understand);
    seen.transitional |= reader.seen.transitional;
    seen.strict |= reader.seen.strict;
}

/// Every name an expression uses, in any position.
pub(crate) fn names_in(expr: &Expr) -> Box<dyn Iterator<Item = &str> + '_> {
    match expr {
        Expr::Name(name) => Box::new(std::iter::once(name.as_str())),
        Expr::Call { args, .. } => Box::new(args.iter().flat_map(names_in)),
        Expr::Prefix(_, e) | Expr::Postfix(_, e) | Expr::Paren(e) => names_in(e),
        Expr::Binary(_, a, b) => Box::new(names_in(a).chain(names_in(b))),
        _ => Box::new(std::iter::empty()),
    }
}

/// How many `<local>` children the current element has — every one of them skipped whole.
fn count(reader: &mut Reader<'_>, local: &str) -> crate::Result<usize> {
    let mut n = 0;
    reader.children(|_, name, _| {
        n += usize::from(name.is(local));
        Ok(Handled::No)
    })?;
    Ok(n)
}

/// `<mergeCells>`: each `<mergeCell ref="B2:D2"/>` the model's merge over that range. Excel keeps
/// the value in the top-left cell, where the model's merge reads it; the others' contents (usually
/// a border's style) are carried out of sight, as ODF's covered cells are. A non-range or one
/// overlapping an earlier merge is counted; a one-cell merge is no merge, neither carried nor
/// counted.
fn merges(reader: &mut Reader<'_>, sheet: &mut Sheet, report: &mut Report) -> crate::Result<()> {
    reader.children(|_, name, attrs| {
        if !name.is("mergeCell") {
            return Ok(Handled::No);
        }
        let range = attrs.plain("ref").and_then(|r| {
            let (a, b) = r.split_once(':')?;
            Some((address::cell(a)?, address::cell(b)?))
        });
        let Some((start, end)) = range else {
            report.drop_one(Dropped::MergedCells);
            return Ok(Handled::Yes);
        };
        let (anchor, span) = grind_sheet::Span::between(start, end);
        // One cell is no merge, and so nothing lost.
        if span.is_single() {
            return Ok(Handled::Yes);
        }
        if !sheet.merges_within(anchor, span.end(anchor)).is_empty() {
            report.drop_one(Dropped::MergedCells);
            return Ok(Handled::Yes);
        }
        sheet.set_merge(anchor, Some(span));
        frame(sheet, anchor, span.end(anchor));
        Ok(Handled::Yes)
    })?;
    Ok(())
}

/// Give a merge's top-left cell the right edge of the top-right cell and the bottom edge of the
/// bottom-left one, as the oracle does: ODF draws a merged cell by its own style alone
/// (`doc/xlsx-format.md` §4.10).
fn frame(sheet: &mut Sheet, anchor: Pos, end: Pos) {
    let edge = |pos: Pos, side: usize| sheet.style(pos).and_then(|s| s.borders[side].clone());
    let mut style = sheet.style(anchor).cloned().unwrap_or_default();
    if end.col > anchor.col {
        style.borders[RIGHT] = edge(Pos::new(anchor.row, end.col), RIGHT);
    }
    if end.row > anchor.row {
        style.borders[BOTTOM] = edge(Pos::new(end.row, anchor.col), BOTTOM);
    }
    sheet.set_style(anchor, style);
}

/// [`grind_sheet::style::EDGES`]' order.
const RIGHT: usize = 1;
const BOTTOM: usize = 3;

/// `<autoFilter ref="A1:C9">` and its `<filterColumn>`s, as the model's filter. The model's
/// vocabulary is a set of values per column, exactly `<filters><filter val="…"/>` matched on
/// displayed text; `blank="1"` is the empty string. Any other criterion is left out and counted as
/// `Appearance::FilterCriterion` per column — a filter quietly keeping the wrong rows would be
/// worse than a visible gap.
fn autofilter(
    reader: &mut Reader<'_>,
    attrs: &crate::xml::Attrs,
    report: &mut Report,
) -> crate::Result<Option<grind_sheet::Filter>> {
    let range = attrs.plain("ref").and_then(|r| {
        let (a, b) = r.split_once(':').unwrap_or((r, r));
        Some((address::cell(a)?, address::cell(b)?))
    });
    let mut keep = std::collections::BTreeMap::new();
    let mut lost = 0;
    reader.children(|reader, name, attrs| {
        if !name.is("filterColumn") {
            return Ok(Handled::No);
        }
        let Some(field) = attrs
            .plain("colId")
            .and_then(|c| c.trim().parse::<u32>().ok())
        else {
            return Ok(Handled::Yes);
        };
        let mut values = std::collections::BTreeSet::new();
        let mut carried = false;
        let mut other = false;
        reader.children(|reader, name, attrs| {
            if !name.is("filters") {
                other |= name.ns == crate::names::Ns::Spreadsheet;
                return Ok(Handled::No);
            }
            carried = true;
            if attrs.flag("blank") {
                values.insert(String::new());
            }
            reader.children(|_, name, attrs| {
                if name.is("filter") {
                    values.insert(attrs.plain("val").unwrap_or_default().to_owned());
                } else if name.is("dateGroupItem") {
                    other = true;
                }
                Ok(Handled::No)
            })?;
            Ok(Handled::Yes)
        })?;
        if carried && !other {
            keep.insert(field, values);
        } else if carried || other {
            lost += 1;
        }
        Ok(Handled::Yes)
    })?;
    for _ in 0..lost {
        report.lose(Appearance::FilterCriterion);
    }
    let Some((start, end)) = range else {
        return Ok(None);
    };
    // The name LibreOffice gives an autofilter nobody named, as `grind sheet filter` does.
    let mut filter = grind_sheet::Filter::new("__Anonymous_Sheet_DB__0", start, end);
    filter.keep = keep;
    Ok(Some(filter))
}

/// Put the filter on the sheet, and un-hide the rows **it** hides. Excel writes `hidden="1"` on
/// every filtered row, which the geometry pass carried as hidden by hand; the model derives
/// filtered rows from the filter, so those are left to it. A row the carried filter does not
/// account for stays hidden by hand, so the sheet shows what Excel showed.
fn apply_filter(sheet: &mut Sheet, filter: grind_sheet::Filter, null_date: i64) {
    sheet.set_filter(Some(filter));
    for row in sheet.hidden_rows(null_date, None) {
        sheet.set_row_hidden(row, false);
    }
}

/// ECMA-376 §18.3.1.13's unit for a column width: the maximum digit width of the default font, in
/// pixels at 96 dpi. Seven is Calibri 11's — the spec's worked example, Excel's default since
/// 2007 — and stands in for a font [`digit_px`] does not know.
pub const DIGIT_PX: f64 = 7.0;

/// The digit's advance, in em, of the fonts whose metrics are known, read from their
/// metric-compatible free twins (`doc/xlsx-format.md` §4.5).
const DIGIT_EM: &[(&str, f64)] = &[
    ("Calibri", 1038.0 / 2048.0),
    ("Carlito", 1038.0 / 2048.0),
    ("Arial", 1139.0 / 2048.0),
    ("Helvetica", 1139.0 / 2048.0),
    ("Liberation Sans", 1139.0 / 2048.0),
    ("Times New Roman", 1024.0 / 2048.0),
    ("Liberation Serif", 1024.0 / 2048.0),
    ("Cambria", 554.0 / 1000.0),
    ("Caladea", 554.0 / 1000.0),
    ("Arial Narrow", 934.0 / 2048.0),
    ("Liberation Sans Narrow", 934.0 / 2048.0),
    ("Courier New", 1229.0 / 2048.0),
    ("Liberation Mono", 1229.0 / 2048.0),
];

/// Excel's maximum digit width for a default font of `family` at `size` points: the digit's
/// advance at 96 dpi, rounded to a pixel — `MEASURED` against Excel's own widths.
pub fn digit_px(family: Option<&str>, size: f64) -> f64 {
    family
        .and_then(|family| {
            DIGIT_EM
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(family.trim()))
        })
        .map(|(_, em)| (em * size * 96.0 / 72.0).round())
        .filter(|px| *px >= 1.0)
        .unwrap_or(DIGIT_PX)
}

/// A column width in characters of `digit` pixels, as the ODF length the model stores, at 96
/// dpi. The width already includes Excel's padding — `9.140625` is its 64-pixel default column,
/// which the UI calls 8.43 characters — so nothing is added.
pub fn col_width(chars: f64, digit: f64) -> String {
    mm_length(chars * digit * 25.4 / 96.0)
}

/// What a sheet's tracks said beyond their sizes, collected across the whole part.
#[derive(Default)]
struct Tracks {
    /// The default font's digit width, the unit of every column width ([`digit_px`]).
    digit: f64,
    /// `<col>` runs longer than `MAX_TRACK_RUN`, applied once the sheet's extent is known.
    long: Vec<(u32, u32, Track)>,
    /// `<sheetFormatPr defaultColWidth>`: the width of every column no `<col>` mentions.
    default_width: Option<f64>,
    /// Every `<col>` range, as `start..end`, so that the default goes only where none did.
    mentioned: Vec<std::ops::Range<u32>>,
    /// `<sheetFormatPr defaultRowHeight>` under `customHeight`: every row's that states none.
    default_height: Option<f64>,
    /// The rows that state a height of their own, measured or set.
    sized_rows: std::collections::BTreeSet<u32>,
    /// `<col style>` per `<col>` range: the cell format of a cell in it that names none.
    col_styles: Vec<(std::ops::Range<u32>, usize)>,
    outline: bool,
    pane: bool,
    sheet_width: bool,
}

/// One `<col>` or `<row>`'s own attributes.
#[derive(Clone, Copy, Default)]
struct Track {
    /// In the track's own unit: characters for a column, points for a row.
    size: Option<f64>,
    hidden: bool,
}

impl Tracks {
    /// Put the long runs on the columns the sheet uses, and count what the sheet as a whole
    /// could not keep.
    fn finish(&mut self, sheet: &mut Sheet, report: &mut Report) {
        let used = sheet.used_cols();
        for (start, end, track) in std::mem::take(&mut self.long) {
            for col in start..end.min(used) {
                column(sheet, col, track, self.digit, report);
            }
        }
        if let Some(width) = self.default_width {
            for col in 0..used {
                if !self.mentioned.iter().any(|range| range.contains(&col)) {
                    sheet.set_col_width(col, Some(col_width(width, self.digit)));
                }
            }
        }
        if let Some(height) = self.default_height {
            for row in 0..sheet.used_rows() {
                if !self.sized_rows.contains(&row) && sheet.row_height(row).is_none() {
                    sheet.set_row_height(row, Some(format!("{height}pt")));
                }
            }
        }
        for (on, class) in [
            (self.outline, Appearance::Outline),
            (self.pane, Appearance::Pane),
            (self.sheet_width, Appearance::SheetDefaultWidth),
        ] {
            if on {
                report.lose(class);
            }
        }
    }
}

/// `<cols>`: each `<col min max>` is a 1-based, inclusive range.
fn cols(
    reader: &mut Reader<'_>,
    sheet: &mut Sheet,
    report: &mut Report,
    tracks: &mut Tracks,
) -> crate::Result<()> {
    reader.children(|_, name, attrs| {
        if !name.is("col") {
            return Ok(Handled::No);
        }
        let bound = |local: &str| {
            attrs
                .plain(local)
                .and_then(|v| v.trim().parse::<u32>().ok())
        };
        let (Some(min), Some(max)) = (bound("min"), bound("max")) else {
            return Ok(Handled::Yes);
        };
        let (start, end) = (min.max(1) - 1, max.min(MAX_COLS));
        if start >= end {
            return Ok(Handled::Yes);
        }
        let track = Track {
            size: attrs
                .plain("width")
                .and_then(|v| v.trim().parse::<f64>().ok())
                .filter(|w| w.is_finite() && *w >= 0.0),
            hidden: attrs.flag("hidden"),
        };
        tracks.outline |= outlined(attrs);
        tracks.mentioned.push(start..end);
        if let Some(style) = attrs.plain("style").and_then(|s| s.trim().parse().ok()) {
            tracks.col_styles.push((start..end, style));
        }
        if end - start <= MAX_TRACK_RUN {
            for col in start..end {
                column(sheet, col, track, tracks.digit, report);
            }
        } else {
            // The sheet's background. Carried over the columns in use once they are known; the
            // rest is only worth a sentence when the author chose it — a width Excel computed
            // for the whole sheet is its default, not a decision.
            tracks.sheet_width |= track.hidden || attrs.flag("customWidth");
            tracks.long.push((start, end, track));
        }
        Ok(Handled::Yes)
    })
}

fn column(sheet: &mut Sheet, col: u32, track: Track, digit: f64, report: &mut Report) {
    match track.size {
        Some(width) if width > 0.0 => sheet.set_col_width(col, Some(col_width(width, digit))),
        // `width="0"` on a column that is not hidden is invisible all the same.
        Some(_) if !track.hidden => {
            sheet.set_col_hidden(col, true);
            report.lose(Appearance::ZeroSize);
        }
        _ => {}
    }
    if track.hidden {
        sheet.set_col_hidden(col, true);
    }
}

/// A `<row>`'s height and hidden-ness. A height the author set (`customHeight`) is the row's; one
/// without is Excel's measurement of what the row holds, carried as a measured height the content
/// may outgrow ([`Sheet::row_measured`]), as LibreOffice's own measurements are.
fn row_geometry(sheet: &mut Sheet, row: u32, attrs: &crate::xml::Attrs, report: &mut Report) {
    let height = attrs
        .plain("ht")
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|h| h.is_finite() && *h >= 0.0);
    let hidden = attrs.flag("hidden");
    match height {
        Some(ht) if ht > 0.0 && attrs.flag("customHeight") => {
            sheet.set_row_height(row, Some(format!("{ht}pt")));
        }
        Some(ht) if ht > 0.0 => sheet.set_row_measured(row, format!("{ht}pt")),
        Some(ht) if ht == 0.0 && attrs.flag("customHeight") && !hidden => {
            sheet.set_row_hidden(row, true);
            report.lose(Appearance::ZeroSize);
        }
        _ => {}
    }
    if hidden {
        sheet.set_row_hidden(row, true);
    }
}

fn outlined(attrs: &crate::xml::Attrs) -> bool {
    attrs
        .plain("outlineLevel")
        .and_then(|v| v.trim().parse::<u32>().ok())
        .is_some_and(|level| level > 0)
}

/// Whether any `<sheetView>` freezes or splits its panes. A `<pane>` with neither split is a
/// view with one pane, which is every sheet.
fn panes(reader: &mut Reader<'_>) -> crate::Result<bool> {
    let mut any = false;
    reader.children(|reader, name, _| {
        if !name.is("sheetView") {
            return Ok(Handled::No);
        }
        reader.children(|_, name, attrs| {
            if name.is("pane") {
                let split = |local: &str| {
                    attrs
                        .plain(local)
                        .and_then(|v| v.trim().parse::<f64>().ok())
                        .is_some_and(|v| v > 0.0)
                };
                any |= split("xSplit") || split("ySplit");
            }
            Ok(Handled::No)
        })?;
        Ok(Handled::Yes)
    })?;
    Ok(any)
}

/// The shared-formula groups of one worksheet, and the cells still waiting on one.
///
/// Resolution is deferred to the end of the sheet because document order says nothing about
/// which cell of a group is its master.
#[derive(Default)]
struct Shared {
    /// `si` → where the master sits, and what it says.
    masters: HashMap<u32, (Pos, Expr)>,
    followers: Vec<(Pos, u32)>,
}

impl Shared {
    fn resolve(&self, index: usize, context: &Context<'_>, sheet: &mut Sheet, report: &mut Report) {
        for &(at, si) in &self.followers {
            let Some((base, expr)) = self.masters.get(&si) else {
                // A group whose master was never seen, or whose master was itself refused.
                refuse(Refusal::Syntax, at, index, report);
                continue;
            };
            let moved = shift::shift(
                expr,
                i64::from(at.row) - i64::from(base.row),
                i64::from(at.col) - i64::from(base.col),
            );
            store_formula(sheet, at, &moved, index, context, report, false);
        }
    }
}

fn rows(
    reader: &mut Reader<'_>,
    context: &Context<'_>,
    index: usize,
    sheet: &mut Sheet,
    report: &mut Report,
    shared: &mut Shared,
    tracks: &mut Tracks,
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
        row_geometry(sheet, row, attrs, report);
        if attrs.plain("ht").is_some() {
            tracks.sized_rows.insert(row);
        }
        tracks.outline |= outlined(attrs);
        // A row's own style reaches the cells in it that name none — measured: the oracle draws
        // `geometry/rows.xlsx`'s A10, which has no `s`, in its row's bold. Only under
        // `customFormat`, which is what says the row's `s` is meant (§18.3.1.73). A column's
        // style reaches them where the row has none (`doc/xlsx-format.md` §4.4).
        let row_style = attrs
            .flag("customFormat")
            .then(|| attrs.plain("s").and_then(|s| s.parse::<usize>().ok()))
            .flatten();
        let mut at = Pos::new(row, 0);
        let col_styles = &tracks.col_styles;
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
                s: attrs
                    .plain("s")
                    .and_then(|s| s.parse().ok())
                    .or(row_style)
                    .or_else(|| {
                        col_styles
                            .iter()
                            .find(|(cols, _)| cols.contains(&at.col))
                            .map(|(_, style)| *style)
                    })
                    .unwrap_or(0),
            };
            let mut value = cell(reader)?;
            if let Some(f) = value.f.take() {
                take_formula(&f, at, index, context, sheet, report, shared);
            }
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
    /// `<f>`, which every kind of formula cell has — including the followers of a shared
    /// group, whose element is empty and carries only the group's number.
    f: Option<RawFormula>,
}

/// What a `<f>` said about itself, owned for the same reason [`RawCell`] is.
struct RawFormula {
    /// `normal` (the default), `shared`, `array` or `dataTable` (ECMA-376 §18.18.6).
    t: String,
    /// The shared group this cell belongs to.
    si: Option<u32>,
    text: String,
}

fn cell(reader: &mut Reader<'_>) -> crate::Result<Held> {
    let mut held = Held::default();
    reader.children(|reader, name, attrs| {
        if name.is("v") {
            held.v = Some(reader.text()?);
            return Ok(Handled::Yes);
        }
        if name.is("is") {
            held.inline = Some(strings::item(reader)?);
            return Ok(Handled::Yes);
        }
        if name.is("f") {
            let t = attrs.plain("t").unwrap_or("normal").to_owned();
            let si = attrs.plain("si").and_then(|s| s.parse().ok());
            // `ref` is the group's own range and is never read: a follower names its group by
            // number, so the range would be a second way to say the same thing.
            held.f = Some(RawFormula {
                t,
                si,
                text: reader.text()?,
            });
            return Ok(Handled::Yes);
        }
        // `<extLst>` is nobody's.
        Ok(Handled::No)
    })?;
    Ok(held)
}

/// One `<f>`: translated and stored, joined to its group, or refused by class.
fn take_formula(
    f: &RawFormula,
    at: Pos,
    index: usize,
    context: &Context<'_>,
    sheet: &mut Sheet,
    report: &mut Report,
    shared: &mut Shared,
) {
    // An array formula is §2.3.2's exclusion rather than a translation failure: the cell keeps
    // the value Excel cached for it and the expression goes, whatever it says.
    if f.t == "array" {
        refuse(Refusal::Array, at, index, report);
        return;
    }
    // A follower carries the group's number and nothing else. Its master may not have been
    // read yet, so it waits.
    if f.t == "shared" && f.text.trim().is_empty() {
        match f.si {
            Some(si) => shared.followers.push((at, si)),
            // A follower that names no group has nothing to be shifted from.
            None => refuse(Refusal::Syntax, at, index, report),
        }
        return;
    }
    // `dataTable` has no expression at all — it is a what-if table described by attributes —
    // so it lands here with empty text and is refused as syntax, which is the truth.
    let scope = crate::tables::Scope {
        tables: context.tables,
        sheet: index,
        sheet_names: context.sheet_names,
        row: at.row,
    };
    match formula::translate_in(&f.text, Some(&scope)) {
        Ok(expr) => {
            if f.t == "shared"
                && let Some(si) = f.si
            {
                shared.masters.insert(si, (at, expr.clone()));
            }
            store_formula(sheet, at, &expr, index, context, report, true);
        }
        Err(refusal) => refuse(refusal, at, index, report),
    }
}

/// The formula in ODF syntax and, where `scan` asks, the functions it names — or its refusal,
/// when it names a name sheet-local here. Renamed sheets are followed first. A shared group's
/// follower is not scanned: shifting moves references, never calls, and its master was.
fn store_formula(
    sheet: &mut Sheet,
    at: Pos,
    expr: &Expr,
    index: usize,
    context: &Context<'_>,
    report: &mut Report,
    scan: bool,
) {
    if names_in(expr).any(|name| context.names.is_local(index, name)) {
        refuse(Refusal::SheetLocalName, at, index, report);
        return;
    }
    let renamed;
    let expr = match context.renames.is_empty() {
        true => expr,
        false => {
            renamed = crate::workbook::renamed(expr, context.renames);
            &renamed
        }
    };
    // `=` rather than `of:=`: both are legal (§5.2) and this is the spelling everything else
    // in the workspace stores, so an imported formula and a typed one are the same string.
    let text = format!("={expr}");
    // Asked of the text rather than of the tree, so that what is reported is what the document
    // now says — one walker, the core's, and no second idea of what a call is.
    let used = match scan {
        true => funcs::used(&text).unwrap_or_default(),
        false => Vec::new(),
    };
    for name in used {
        if !funcs::implemented().contains(&name.as_str()) {
            report.unknown_functions.insert(name);
        }
    }
    sheet.set_formula(at, text);
    report.formulas += 1;
}

fn refuse(refusal: Refusal, at: Pos, index: usize, report: &mut Report) {
    if let Some(kind) = refusal.dropped() {
        report.drop_one(kind);
    }
    *report.refused.entry(refusal).or_default() += 1;
    report.untranslated.push((index, at));
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
        // A blank with a look that shows on one — a border, a fill — is a table's frame, and is
        // carried; a bold font on an empty cell shows nothing and is not written. Only what
        // would have shown is counted as lost: an underline on nothing loses nothing.
        if let Some(look) = context
            .styles
            .look(raw.s)
            .filter(|look| look.shows_when_empty())
        {
            if let Some(style) = context.styles.shared_style(raw.s) {
                report.styled += 1;
                sheet.set_style_shared(at, style.clone());
            }
            for lost in look.lost.iter().filter(|lost| lost.shows_when_empty()) {
                report.lose(*lost);
            }
        }
        return;
    }
    let text = match &value {
        CellValue::Text(text) => text.len(),
        _ => 0,
    };
    if report.cells >= MAX_CELLS || report.text_bytes + text > MAX_TEXT_BYTES {
        report.over_budget += 1;
        return;
    }
    report.cells += 1;
    report.text_bytes += text;
    sheet.set(at, value);
    if let Some(kind) = kind {
        sheet.set_kind(at, kind);
    }
    format(sheet, at, raw.s, context, report);
    look(sheet, at, raw.s, context, report);
}

/// Put the cell's style on it and count what it could not carry — translated once with the styles
/// part, cloned here.
fn look(sheet: &mut Sheet, at: Pos, s: usize, context: &Context<'_>, report: &mut Report) {
    let Some(look) = context.styles.look(s) else {
        return;
    };
    if let Some(style) = context.styles.shared_style(s) {
        report.styled += 1;
        sheet.set_style_shared(at, style.clone());
    }
    for lost in &look.lost {
        report.lose(*lost);
    }
    if look.family {
        report.drop_one(Dropped::FontFamily);
    }
    if look.unresolved_theme {
        report.drop_one(Dropped::ThemeColor);
    }
}

/// Put the cell's number format on it and count what its code could not say — translated once with
/// the styles part, cloned here. A format that would misstate the number is not there to clone,
/// and the cell shows its plain value; either way the loss is counted per cell.
fn format(sheet: &mut Sheet, at: Pos, s: usize, context: &Context<'_>, report: &mut Report) {
    if let Some(format) = context.styles.shared_format(s) {
        report.formatted += 1;
        sheet.set_format_shared(at, format.clone());
    }
    for lost in context.styles.lost(s) {
        *report.formats_lost.entry(*lost).or_default() += 1;
    }
}

/// `<v>` as a number: `xsd:double`'s spellings plus surrounding whitespace. `INF` and `NaN` are
/// `xsd:double` forms no cell can hold, carried as the text the file wrote.
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
            names: &Default::default(),
            renames: &[],
            tables: &Default::default(),
            sheet_names: &[],
        };
        let mut sheet = Sheet::new("S");
        let mut report = Report::default();
        read(
            xml.as_bytes(),
            &context,
            0,
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
        crate::styles::read(xml.as_bytes(), &[])
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
    fn one_shared_string_named_by_every_cell_is_carried_only_up_to_the_text_budget() {
        let strings = [Item {
            text: "x".repeat(300 << 10),
            rich: false,
        }];
        let cells: String = (1..=10)
            .map(|row| format!(r#"<row r="{row}"><c r="A{row}" t="s"><v>0</v></c></row>"#))
            .collect();
        let (sheet, report) = sheet_of(&cells, &strings, &Styles::default(), false);
        assert_eq!(report.cells, 3);
        assert_eq!(report.over_budget, 7);
        assert!(sheet.get(at("A4")).is_empty());
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

    /// What is stored is ODF's syntax, and the cached value is untouched beside it.
    #[test]
    fn a_formula_cell_carries_both_halves() {
        let (sheet, report) = sheet_of(
            r#"<row r="1"><c r="A1"><v>3</v></c><c r="B1"><f>A1*2</f><v>6</v></c></row>"#,
            &[],
            &Styles::default(),
            false,
        );
        assert_eq!(sheet.formula(at("B1")), Some("=[.A1]*2"));
        assert_eq!(sheet.get(at("B1")), CellValue::Number(6.0));
        assert_eq!(report.formulas, 1);
        assert!(report.untranslated.is_empty());
    }

    /// Apache POI and SheetJS do not evaluate, so they write this by default.
    #[test]
    fn a_formula_with_no_cached_value_is_still_a_formula() {
        let (sheet, report) = sheet_of(
            r#"<row r="1"><c r="B1"><f>SUM(A1:A2)</f></c></row>"#,
            &[],
            &Styles::default(),
            false,
        );
        assert_eq!(sheet.formula(at("B1")), Some("=SUM([.A1:.A2])"));
        assert_eq!(sheet.get(at("B1")), CellValue::Empty);
        assert_eq!(report.formulas, 1);
    }

    /// The group's master is at B1; every follower is it, shifted. The second group's master
    /// is *below* its follower, which is why resolution waits for the whole sheet.
    #[test]
    fn a_shared_group_is_its_master_shifted() {
        let (sheet, report) = sheet_of(
            r#"<row r="1">
                 <c r="B1"><f t="shared" ref="B1:B3" si="0">A1*2</f><v>2</v></c>
                 <c r="F1"><f t="shared" si="2"/></c>
               </row>
               <row r="2">
                 <c r="B2"><f t="shared" si="0"/><v>4</v></c>
                 <c r="F2"><f t="shared" ref="F1:F2" si="2">A1*10</f><v>10</v></c>
               </row>
               <row r="3"><c r="B3"><f t="shared" si="0"/><v>6</v></c></row>"#,
            &[],
            &Styles::default(),
            false,
        );
        assert_eq!(sheet.formula(at("B1")), Some("=[.A1]*2"));
        assert_eq!(sheet.formula(at("B2")), Some("=[.A2]*2"));
        assert_eq!(sheet.formula(at("B3")), Some("=[.A3]*2"));
        assert_eq!(sheet.formula(at("F2")), Some("=[.A1]*10"));
        // Shifted up off the sheet: the same `#REF!` a delete produces.
        assert_eq!(sheet.formula(at("F1")), Some("=#REF!*10"));
        assert_eq!(report.formulas, 5);
    }

    /// An absolute axis does not move with the group, which is the entire reason `$` exists.
    #[test]
    fn a_shared_group_moves_only_its_relative_axes() {
        let (sheet, _) = sheet_of(
            r#"<row r="1"><c r="G1"><f t="shared" ref="G1:G2" si="3">$A$1+A1</f><v>2</v></c></row>
               <row r="2"><c r="G2"><f t="shared" si="3"/><v>3</v></c></row>"#,
            &[],
            &Styles::default(),
            false,
        );
        assert_eq!(sheet.formula(at("G2")), Some("=[.$A$1]+[.A2]"));
    }

    /// The cell keeps Excel's value and loses the expression, and the loss is counted — by
    /// kind where the model has one, and in `untranslated` always.
    #[test]
    fn a_refused_formula_keeps_its_value_and_is_counted() {
        let (sheet, report) = sheet_of(
            r#"<row r="1">
                 <c r="A1"><f t="array" ref="A1:A1">B1:B3*2</f><v>2</v></c>
                 <c r="B1"><f>SUM(Sales[Amount])</f><v>7</v></c>
                 <c r="C1"><f>[1]Sheet1!$A$1</f><v>9</v></c>
                 <c r="D1"><f>SUM(A1:B2 B1:B3)</f><v>4</v></c>
               </row>"#,
            &[],
            &Styles::default(),
            false,
        );
        for addr in ["A1", "B1", "C1", "D1"] {
            assert_eq!(sheet.formula(at(addr)), None, "{addr}");
        }
        assert_eq!(sheet.get(at("B1")), CellValue::Number(7.0));
        assert_eq!(report.dropped[&Dropped::ArrayFormula], 1);
        assert_eq!(report.dropped[&Dropped::StructuredReference], 1);
        assert_eq!(report.dropped[&Dropped::ExternalLink], 1);
        // The intersection operator is expressible in ODF and outside the Small Group, so it
        // is a lost formula and not a dropped construct.
        assert_eq!(report.untranslated.len(), 4);
        assert_eq!(report.formulas, 0);
        assert!(!report.lossless());
        // Where and why are the same four cells counted twice, which is the invariant that
        // keeps the scoreboard honest.
        assert_eq!(
            report.refused.values().sum::<usize>(),
            report.untranslated.len()
        );
        assert_eq!(report.refused[&Refusal::Array], 1);
        assert_eq!(report.refused[&Refusal::Intersection], 1);
    }

    /// A name grind cannot evaluate is reported and the formula carried anyway.
    #[test]
    fn an_unimplemented_function_is_named_rather_than_refused() {
        let (sheet, report) = sheet_of(
            r#"<row r="1">
                 <c r="A1"><f>NOSUCHFUNCTION(B1)</f><v>1</v></c>
                 <c r="B1"><f>_xlfn.XLOOKUP(3,C1:C2,D1:D2)</f><v>2</v></c>
                 <c r="C1"><f>SUM(D1:D2)</f><v>3</v></c>
               </row>"#,
            &[],
            &Styles::default(),
            false,
        );
        assert_eq!(sheet.formula(at("A1")), Some("=NOSUCHFUNCTION([.B1])"));
        assert_eq!(
            sheet.formula(at("B1")),
            Some("=COM.MICROSOFT.XLOOKUP(3;[.C1:.C2];[.D1:.D2])")
        );
        assert_eq!(
            report.unknown_functions,
            ["COM.MICROSOFT.XLOOKUP", "NOSUCHFUNCTION"]
                .map(str::to_owned)
                .into_iter()
                .collect()
        );
        assert!(report.lossless(), "an unknown name loses nothing");
    }

    /// A whole worksheet part, for the tests that need what sits beside `<sheetData>`.
    fn worksheet(inner: &str, styles: &Styles) -> (Sheet, Report) {
        let xml = format!(r#"<worksheet xmlns="{MAIN}">{inner}</worksheet>"#);
        let context = Context {
            strings: &[],
            styles,
            date_1904: false,
            null_date: date::DEFAULT_NULL_DATE,
            names: &Default::default(),
            renames: &[],
            tables: &Default::default(),
            sheet_names: &[],
        };
        let mut sheet = Sheet::new("S");
        let mut report = Report::default();
        read(
            xml.as_bytes(),
            &context,
            0,
            &mut sheet,
            &mut report,
            &mut Default::default(),
        );
        (sheet, report)
    }

    /// A `<col>` is a range; a width is characters of a seven-pixel digit; a run to the edge
    /// of the sheet stops where the sheet does.
    #[test]
    fn columns_are_ranges_and_widths_are_characters() {
        let (sheet, report) = worksheet(
            r#"<cols>
                 <col min="1" max="1" width="9.140625" customWidth="1"/>
                 <col min="2" max="3" width="20" customWidth="1" hidden="1"/>
                 <col min="4" max="4" width="0" customWidth="1"/>
                 <col min="5" max="16384" width="12" customWidth="1"/>
               </cols>
               <sheetData><row r="1"><c r="F1"><v>1</v></c></row></sheetData>"#,
            &Styles::default(),
        );
        // 9.140625 characters is Excel's 64-pixel default: two-thirds of an inch.
        assert_eq!(
            sheet.col_width(0),
            Some(col_width(9.140625, DIGIT_PX).as_str())
        );
        assert_eq!(col_width(9.140625, DIGIT_PX), "16.929mm");
        assert!(sheet.col_hidden(1) && sheet.col_hidden(2));
        assert_eq!(
            sheet.col_width(2),
            Some(col_width(20.0, DIGIT_PX).as_str()),
            "hidden, and sized"
        );
        assert!(sheet.col_hidden(3), "width 0 is invisible, so it is hidden");
        assert_eq!(sheet.col_width(3), None);
        // E:XFD reaches the columns the sheet uses — E and F — and no further.
        assert_eq!(sheet.col_width(5), Some(col_width(12.0, DIGIT_PX).as_str()));
        assert_eq!(sheet.col_width(6), None);
        assert_eq!(report.appearance_lost[&Appearance::ZeroSize], 1);
        assert_eq!(
            report.appearance_lost[&Appearance::SheetDefaultWidth],
            1,
            "a width chosen for the whole sheet has no home"
        );
    }

    /// A merge's frame is the top-right cell's right edge and the bottom-left cell's bottom,
    /// whatever the others say (`doc/xlsx-format.md` §4.10).
    #[test]
    fn a_merge_is_framed_by_the_cells_at_its_corners() {
        let styles = crate::styles::read(
            format!(
                r#"<styleSheet xmlns="{}"><fonts><font/></fonts>
                   <borders><border/>
                     <border><right style="thick"/></border>
                     <border><right style="thin"/><bottom style="thin"/></border>
                     <border><bottom style="medium"/></border>
                     <border><left style="thin"/><right style="thin"/></border></borders>
                   <cellXfs><xf/><xf borderId="1"/><xf borderId="2"/><xf borderId="3"/><xf borderId="4"/></cellXfs>
                   </styleSheet>"#,
                crate::names::MAIN_T
            )
            .as_bytes(),
            &[],
        );
        let (sheet, _) = worksheet(
            r#"<sheetData>
                 <row r="1"><c r="A1" s="4" t="inlineStr"><is><t>M</t></is></c><c r="C1" s="1"/></row>
                 <row r="3"><c r="A3" s="3"/><c r="C3" s="2"/></row>
               </sheetData>
               <mergeCells><mergeCell ref="A1:C3"/></mergeCells>"#,
            &styles,
        );
        let edges = sheet.style(at("A1")).unwrap().borders.clone();
        let width = |edge: &Option<String>| {
            edge.as_deref()
                .and_then(grind_sheet::style::border_parts)
                .map(|(w, _, _)| w)
        };
        assert!(width(&edges[0]).is_some(), "its own left");
        assert_eq!(
            width(&edges[1]),
            width(&sheet.style(at("C1")).unwrap().borders[1])
        );
        assert_eq!(
            width(&edges[3]),
            width(&sheet.style(at("A3")).unwrap().borders[3])
        );
    }

    /// Excel's own widths, measured per default font (`doc/xlsx-format.md` §4.5).
    #[test]
    fn the_digit_is_the_default_fonts_rounded_to_a_pixel() {
        for (family, size, px) in [
            ("Calibri", 11.0, 7.0),
            ("Calibri", 12.0, 8.0),
            ("Arial", 8.0, 6.0),
            ("Arial", 10.0, 7.0),
            ("arial", 11.0, 8.0),
            ("Arial", 12.0, 9.0),
            ("Arial", 16.0, 12.0),
            ("Arial", 18.0, 13.0),
            ("Some Font Nobody Measured", 20.0, DIGIT_PX),
        ] {
            assert_eq!(digit_px(Some(family), size), px, "{family} {size}");
        }
        assert_eq!(digit_px(None, 11.0), DIGIT_PX);
    }

    /// A cell naming no format takes its row's, then its column's (`doc/xlsx-format.md` §4.4).
    #[test]
    fn a_cell_naming_no_format_takes_its_rows_then_its_columns() {
        let styles = crate::styles::read(
            format!(
                r#"<styleSheet xmlns="{MAIN}"><fonts><font/><font><b/></font><font><i/></font></fonts>
                   <cellXfs><xf/><xf fontId="1"/><xf fontId="2"/></cellXfs></styleSheet>"#
            )
            .as_bytes(),
            &[],
        );
        let (sheet, _) = worksheet(
            r#"<cols><col min="1" max="1" width="9" style="2"/></cols>
               <sheetData>
                 <row r="1"><c r="A1"><v>1</v></c><c r="B1"><v>1</v></c><c r="C1" s="0"><v>1</v></c></row>
                 <row r="2" s="1" customFormat="1"><c r="A2"><v>1</v></c></row>
               </sheetData>"#,
            &styles,
        );
        let italic = |a: &str| sheet.style(at(a)).and_then(|s| s.font_style.clone());
        let bold = |a: &str| sheet.style(at(a)).and_then(|s| s.font_weight.clone());
        assert_eq!(italic("A1"), Some("italic".into()));
        assert_eq!(sheet.style(at("B1")), None);
        assert_eq!(sheet.style(at("C1")), None, "its own format, the default");
        assert_eq!((bold("A2"), italic("A2")), (Some("bold".into()), None));
    }

    /// A fixed `defaultRowHeight` is every used row's that states no `ht`; a derived one is
    /// nobody's.
    #[test]
    fn a_fixed_default_row_height_fills_the_rows_that_state_none() {
        let rows = r#"<sheetData>
                 <row r="1"><c r="A1"><v>1</v></c></row>
                 <row r="2" ht="30"><c r="A2"><v>1</v></c></row>
                 <row r="3" ht="40" customHeight="1"><c r="A3"><v>1</v></c></row>
                 <row r="4"><c r="A4"><v>1</v></c></row>
               </sheetData>"#;
        let (sheet, _) = worksheet(
            &format!(r#"<sheetFormatPr customHeight="1" defaultRowHeight="15.75"/>{rows}"#),
            &Styles::default(),
        );
        assert_eq!(sheet.row_height(0), Some("15.75pt"));
        assert!(
            sheet.row_measured(1),
            "Excel measured it: content may grow it"
        );
        assert_eq!(sheet.row_height(2), Some("40pt"));
        assert_eq!(sheet.row_height(3), Some("15.75pt"));
        assert_eq!(sheet.row_height(4), None, "past the sheet");

        let (sheet, _) = worksheet(
            &format!(r#"<sheetFormatPr defaultRowHeight="15"/>{rows}"#),
            &Styles::default(),
        );
        assert_eq!(sheet.row_height(0), None);
    }

    /// `defaultColWidth` is every used column's that no `<col>` mentions — the model has no
    /// sheet default to hold it.
    #[test]
    fn the_sheets_default_width_fills_the_columns_nothing_mentions() {
        let (sheet, report) = worksheet(
            r#"<sheetFormatPr defaultColWidth="11.53515625" defaultRowHeight="12.8"/>
               <cols><col min="2" max="2" width="0" hidden="1" customWidth="1"/></cols>
               <sheetData><row r="1"><c r="C1"><v>1</v></c></row></sheetData>"#,
            &Styles::default(),
        );
        assert_eq!(
            sheet.col_width(0),
            Some(col_width(11.53515625, DIGIT_PX).as_str())
        );
        assert_eq!(sheet.col_width(1), None, "mentioned, and hidden at zero");
        assert_eq!(
            sheet.col_width(2),
            Some(col_width(11.53515625, DIGIT_PX).as_str())
        );
        assert_eq!(sheet.col_width(3), None, "past the sheet's content");
        assert!(report.appearance_lost.is_empty());

        let (_, report) = worksheet(
            r#"<sheetFormatPr baseColWidth="12"/><sheetData/>"#,
            &Styles::default(),
        );
        assert_eq!(report.appearance_lost[&Appearance::SheetDefaultWidth], 1);
    }

    /// A height is points, carried where the author set it; a measured one is left to the
    /// shell, and zero is hidden.
    #[test]
    fn rows_carry_the_heights_their_authors_set() {
        let (sheet, report) = worksheet(
            r#"<sheetData>
                 <row r="1" ht="30" customHeight="1"><c r="A1"><v>1</v></c></row>
                 <row r="2" ht="18.75"><c r="A2"><v>2</v></c></row>
                 <row r="3" ht="25" hidden="1" customHeight="1"/>
                 <row r="4" ht="0" customHeight="1"/>
                 <row r="5" outlineLevel="1" hidden="1"/>
               </sheetData>"#,
            &Styles::default(),
        );
        assert_eq!(sheet.row_height(0), Some("30pt"));
        assert_eq!(sheet.row_height(1), Some("18.75pt"));
        assert!(
            sheet.row_measured(1) && !sheet.row_measured(0),
            "Excel's measurement, not the author's"
        );
        assert!(sheet.row_manually_hidden(2));
        assert_eq!(sheet.row_height(2), Some("25pt"), "hidden, and remembers");
        assert!(sheet.row_manually_hidden(3), "zero height");
        assert!(sheet.row_manually_hidden(4));
        assert_eq!(report.appearance_lost[&Appearance::ZeroSize], 1);
        assert_eq!(report.appearance_lost[&Appearance::Outline], 1);
    }

    #[test]
    fn a_frozen_pane_is_counted_and_a_zoom_is_not() {
        let (_, report) = worksheet(
            r#"<sheetViews><sheetView zoomScale="150" showGridLines="0">
                 <pane ySplit="1" topLeftCell="A2" state="frozen"/>
               </sheetView></sheetViews><sheetData/>"#,
            &Styles::default(),
        );
        assert_eq!(report.appearance_lost[&Appearance::Pane], 1);
        let (_, report) = worksheet(
            r#"<sheetViews><sheetView zoomScale="150"/></sheetViews><sheetData/>"#,
            &Styles::default(),
        );
        assert!(report.appearance_lost.is_empty());
    }

    /// A row's `s` reaches the cells in it that name none, under `customFormat` only; a cell's
    /// own `s` wins; and a bordered cell with no value is carried, border and all.
    #[test]
    fn a_rows_style_reaches_its_unstyled_cells() {
        let xml = format!(
            r#"<styleSheet xmlns="{MAIN}"><fonts><font/><font><b/></font><font><i/></font></fonts>
                 <borders><border/><border><left style="thin"/></border></borders>
                 <cellXfs><xf/><xf fontId="1"/><xf fontId="2"/><xf borderId="1"/></cellXfs></styleSheet>"#
        );
        let styles = crate::styles::read(xml.as_bytes(), &[]);
        let (sheet, report) = worksheet(
            r#"<sheetData>
                 <row r="1" s="1" customFormat="1">
                   <c r="A1"><v>1</v></c><c r="B1" s="2"><v>2</v></c><c r="C1" s="3"/>
                 </row>
                 <row r="2" s="1"><c r="A2"><v>3</v></c></row>
               </sheetData>"#,
            &styles,
        );
        let weight = |addr| sheet.style(at(addr)).and_then(|s| s.font_weight.clone());
        assert_eq!(weight("A1"), Some("bold".into()), "the row's");
        assert_eq!(weight("B1"), None, "its own, which is italic");
        assert_eq!(weight("A2"), None, "no customFormat, no row style");
        // A bordered blank is carried: the ODF reader keeps a blank's own style now.
        assert_eq!(
            sheet.style(at("C1")).and_then(|s| s.borders[0].clone()),
            Some("0.74pt solid #000000".into())
        );
        assert_eq!(report.styled, 3);
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
