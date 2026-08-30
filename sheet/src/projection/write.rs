// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Projecting a spreadsheet: `Document` → text, tokens and anchors. **\[ODS\]**
//!
//! The twin of `odf::write`, and the same shape — walk the model once, emit as you go, pool
//! nothing that does not need pooling. What it adds is the two maps, and those cost nothing
//! because [`Emitter`] takes them as a side effect of being told what each thing *is*
//! (`doc/dsl.md` §6.1: syntax highlighting comes from the writer, not from a highlighter).
//!
//! One decision runs through the whole file: **a cell is projected in a grid when it is only a
//! value, and on a line of its own when it is more than one.** A grid is what makes a
//! spreadsheet readable as text; a formula, a date or a time needs a property beside it and a
//! grid has nowhere to put one. So the grid holds plain values, the holes it leaves are
//! `#null`, and everything else follows as a `cell`.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use grind_core::DocumentKind;
use grind_core::projection::Emitter;
use grind_core::projection::kdl::KdlValue;

use super::Projection;
use crate::a1;
use crate::formula::date;
use crate::model::{CellValue, Document, NumberKind, Pos, Sheet};
use crate::numfmt::{self, Format, Kind, Part};
use crate::style::{CellStyle, EDGES};

/// Project a whole document.
pub fn project(doc: &Document) -> Projection {
    let mut out = Emitter::new();
    out.header(DocumentKind::Spreadsheet);
    settings(&mut out, doc);
    for sheet in &doc.sheets {
        out.blank();
        out.begin("sheet");
        out.arg(sheet.name.as_str());
        out.anchor(sheet.name.clone());
        out.open();
        tracks(&mut out, sheet);
        grid(&mut out, sheet);
        cells(&mut out, sheet);
        styles(&mut out, sheet);
        formats(&mut out, sheet);
        filter(&mut out, sheet);
        out.close();
    }
    out.finish()
}

// --- the document level ---

/// The settings and the named expressions — everything that is the document's rather than a
/// sheet's. A setting at its default is not written, so an ordinary document opens straight
/// onto its first sheet.
fn settings(out: &mut Emitter, doc: &Document) {
    let mut wrote = false;
    if doc.null_date != date::DEFAULT_NULL_DATE {
        let (y, m, d) = date::civil_from_days(doc.null_date);
        out.blank();
        out.begin("null-date");
        out.arg(format!("{y:04}-{m:02}-{d:02}"));
        out.end();
        wrote = true;
    }
    if doc.null_year != date::DEFAULT_NULL_YEAR {
        if !wrote {
            out.blank();
        }
        out.begin("null-year");
        out.arg(i128::from(doc.null_year));
        out.end();
        wrote = true;
    }
    if !doc.names.is_empty() && !wrote {
        out.blank();
    }
    for (name, expression) in &doc.names {
        out.begin("name");
        out.arg(name.as_str());
        out.arg(expression.as_str());
        out.anchor(name.clone());
        out.end();
    }
}

// --- a sheet ---

/// `col` and `row` — a column's width, a row's height, and whether either is hidden by hand.
///
/// One node per track that has anything to say, addressed **1-based** like every other address
/// in the format: `a1.rs` is the only 0↔1 conversion in the workspace and this is a user's
/// spelling of a place, so it converts here and by that rule.
fn tracks(out: &mut Emitter, sheet: &Sheet) {
    let hidden_cols: BTreeSet<u32> = sheet.hidden_cols().collect();
    let widths: BTreeMap<u32, &str> = sheet.col_widths().collect();
    for col in widths
        .keys()
        .copied()
        .chain(hidden_cols.iter().copied())
        .collect::<BTreeSet<_>>()
    {
        out.begin("col");
        out.arg(i128::from(col) + 1);
        out.prop_some("width", widths.get(&col).copied());
        if hidden_cols.contains(&col) {
            out.prop("hidden", true);
        }
        out.end();
    }

    let hidden_rows: BTreeSet<u32> = sheet.manually_hidden_rows().collect();
    let heights: BTreeMap<u32, &str> = sheet.row_heights().collect();
    for row in heights
        .keys()
        .copied()
        .chain(hidden_rows.iter().copied())
        .collect::<BTreeSet<_>>()
    {
        out.begin("row");
        out.arg(i128::from(row) + 1);
        out.prop_some("height", heights.get(&row).copied());
        if hidden_rows.contains(&row) {
            out.prop("hidden", true);
        }
        out.end();
    }
}

/// The cells that are only a value, as `at`/`row` grids.
///
/// Rows that carry at least one plain cell are grouped into maximal runs of *consecutive*
/// rows, and one `at` block covers each run starting at the leftmost column any of its rows
/// uses. A gap inside a row is `#null`; a gap at the end is simply where the row stops.
fn grid(out: &mut Emitter, sheet: &Sheet) {
    let rows = plain_rows(sheet);
    if rows.is_empty() {
        return;
    }
    let mut band: Vec<(u32, Vec<(u32, CellValue)>)> = Vec::new();
    let flush = |out: &mut Emitter, band: &mut Vec<(u32, Vec<(u32, CellValue)>)>| {
        if band.is_empty() {
            return;
        }
        let start_col = band
            .iter()
            .filter_map(|(_, cells)| cells.first().map(|(col, _)| *col))
            .min()
            .expect("a band's rows each hold a cell");
        let start = Pos::new(band[0].0, start_col);
        out.blank();
        out.begin("at");
        out.arg_word(&a1::format(None, start));
        // Deliberately unanchored: a band covers a *region*, and a region has no address. Its
        // rows and their cells anchor themselves, which is every place there is to point at.
        out.open();
        for (row, cells) in band.drain(..) {
            out.begin("row");
            out.anchor(address(sheet, Pos::new(row, start_col)));
            let last = cells.last().map_or(start_col, |(col, _)| *col);
            let mut by_col: BTreeMap<u32, CellValue> = cells.into_iter().collect();
            for col in start_col..=last {
                match by_col.remove(&col) {
                    Some(value) => {
                        out.arg(value_of(&value));
                        // Every cell of a grid row anchors itself, so *click a cell, highlight
                        // its value* is as exact here as it is for a `cell` node on a line of
                        // its own.
                        out.anchor_last(address(sheet, Pos::new(row, col)));
                    }
                    // A hole is where a cell the grid cannot hold *would* be, and that cell
                    // has a `cell` node below with its real address on it. Anchoring the hole
                    // as well would give one address two spans, and the narrower — this one —
                    // would win.
                    None => out.arg(KdlValue::Null),
                }
            }
            out.end();
        }
        out.close();
    };

    for (row, cells) in rows {
        let consecutive = band.last().is_some_and(|(last, _)| *last + 1 == row);
        if !band.is_empty() && !consecutive {
            flush(out, &mut band);
        }
        band.push((row, cells));
    }
    flush(out, &mut band);
}

/// Every row that has at least one *plain* cell, with those cells in column order.
///
/// Plain means: a value, and nothing else — no formula, no date or time subtype. Those need a
/// property, and a grid row has nowhere to put one.
fn plain_rows(sheet: &Sheet) -> Vec<(u32, Vec<(u32, CellValue)>)> {
    let extra: BTreeSet<Pos> = sheet
        .formulas()
        .map(|(pos, _)| pos)
        .chain(sheet.kinds().map(|(pos, _)| pos))
        .collect();
    let mut rows = Vec::new();
    for row in 0..sheet.used_rows() {
        let mut cells = Vec::new();
        for col in 0..sheet.used_cols() {
            let pos = Pos::new(row, col);
            if extra.contains(&pos) {
                continue;
            }
            let value = sheet.get(pos);
            if !value.is_empty() {
                cells.push((col, value));
            }
        }
        if !cells.is_empty() {
            rows.push((row, cells));
        }
    }
    rows
}

/// The cells a grid cannot hold: a formula, a date, a time.
fn cells(out: &mut Emitter, sheet: &Sheet) {
    let extra: BTreeSet<Pos> = sheet
        .formulas()
        .map(|(pos, _)| pos)
        .chain(sheet.kinds().map(|(pos, _)| pos))
        .collect();
    if extra.is_empty() {
        return;
    }
    out.blank();
    for pos in extra {
        out.begin("cell");
        out.arg_word(&a1::format(None, pos));
        let value = sheet.get(pos);
        if !value.is_empty() {
            out.arg(value_of(&value));
        }
        out.prop_some("formula", sheet.formula(pos));
        match sheet.kind(pos) {
            Some(NumberKind::Date) => out.prop("date", true),
            Some(NumberKind::Time) => out.prop("time", true),
            None => {}
        }
        out.anchor(address(sheet, pos));
        out.end();
    }
}

/// `style` — one node per distinct cell style, over the rectangles it covers.
fn styles(out: &mut Emitter, sheet: &Sheet) {
    let mut pool: HashMap<&CellStyle, BTreeSet<Pos>> = HashMap::new();
    for (pos, style) in sheet.styles() {
        pool.entry(style).or_default().insert(pos);
    }
    if pool.is_empty() {
        return;
    }
    out.blank();
    // Sorted by where the style first appears, so the file reads top to bottom rather than in
    // whatever order a hash of the style happened to fall.
    let mut pooled: Vec<_> = pool.into_iter().collect();
    pooled.sort_by_key(|(_, cells)| *cells.iter().next().expect("a pooled style has a cell"));
    for (style, cells) in pooled {
        for (start, end) in rectangles(&cells) {
            out.begin("style");
            out.arg_word(&range(start, end));
            style_props(out, style);
            out.end();
        }
    }
}

/// A cell style's ODF values, with the two spellings a person actually writes.
///
/// `bold=#true` and `italic=#true` are sugar for the two values that account for very nearly
/// every styled cell there has ever been; anything else keeps ODF's own word
/// (`weight="600"`, `slant="oblique"`), because R1 says the format's semantics are the product
/// and a projection that could not spell one would not be bijective.
fn style_props(out: &mut Emitter, style: &CellStyle) {
    match style.font_weight.as_deref() {
        Some("bold") => out.prop("bold", true),
        Some(other) => out.prop("weight", other),
        None => {}
    }
    match style.font_style.as_deref() {
        Some("italic") => out.prop("italic", true),
        Some(other) => out.prop("slant", other),
        None => {}
    }
    out.prop_some("size", style.font_size.as_deref());
    out.prop_some("color", style.color.as_deref());
    out.prop_some("background", style.background.as_deref());
    out.prop_some("align", style.align.as_deref());
    out.prop_some("valign", style.vertical_align.as_deref());
    match style.wrap.as_deref() {
        Some("wrap") => out.prop("wrap", true),
        Some("no-wrap") => out.prop("wrap", false),
        Some(other) => out.prop("wrap", other),
        None => {}
    }
    match style.uniform_border() {
        Some(border) => out.prop("border", border),
        None => {
            for (edge, value) in EDGES.iter().zip(&style.borders) {
                out.prop_some(&format!("border-{edge}"), value.as_deref());
            }
        }
    }
}

/// `format` — one node per distinct number format, over the rectangles it covers.
fn formats(out: &mut Emitter, sheet: &Sheet) {
    let mut pool: HashMap<&Format, BTreeSet<Pos>> = HashMap::new();
    for (pos, format) in sheet.formats() {
        pool.entry(format).or_default().insert(pos);
    }
    if pool.is_empty() {
        return;
    }
    out.blank();
    let mut pooled: Vec<_> = pool.into_iter().collect();
    pooled.sort_by_key(|(_, cells)| *cells.iter().next().expect("a pooled format has a cell"));
    for (format, cells) in pooled {
        for (start, end) in rectangles(&cells) {
            out.begin("format");
            out.arg_word(&range(start, end));
            format_body(out, format, false);
        }
    }
}

/// A format, compactly when `numfmt::preset` can build it and part by part when it cannot.
///
/// Ends the node itself — the parts form opens a block and the preset form does not, and which
/// it is is precisely the question this function answers.
///
/// `parts_only` is what a `style:map` branch asks for. A branch is read back out of its block,
/// so it has to *have* one even when its format happens to be a preset: the compact spelling
/// would leave a `map` node with no children and the reader would build an empty format from
/// it. One flag rather than two functions, because everything else about the two is identical.
fn format_body(out: &mut Emitter, format: &Format, parts_only: bool) {
    let (kind, decimals, grouping, symbol) = format.preset_params();
    let same = |built: Format| built.in_locale(format.locale.clone()) == *format;
    if !parts_only && same(numfmt::preset(kind, decimals, grouping, &symbol)) {
        out.arg_word(kind_word(kind));
        if decimals != 0 {
            out.prop("decimals", i128::from(decimals));
        }
        if grouping {
            out.prop("grouping", true);
        }
        if !symbol.is_empty() {
            out.prop("symbol", symbol.as_str());
        }
        locale_prop(out, format);
        out.end();
        return;
    }
    if !parts_only && same(numfmt::datetime_preset()) {
        out.arg_word("datetime");
        locale_prop(out, format);
        out.end();
        return;
    }
    out.arg_word(kind_word(format.kind));
    locale_prop(out, format);
    out.open();
    for part in &format.parts {
        part_node(out, part);
    }
    for map in &format.maps {
        out.begin("map");
        out.arg_word(map.op.spelling());
        out.arg(map.value.as_str());
        format_body(out, &map.format, true);
    }
    out.close();
}

fn locale_prop(out: &mut Emitter, format: &Format) {
    if let Some(locale) = &format.locale {
        out.prop("locale", locale.tag());
    }
}

/// One `number:*` element as one node. The names are the ODF element's own, minus the prefix.
fn part_node(out: &mut Emitter, part: &Part) {
    match part {
        Part::Text(text) => {
            out.begin("text");
            out.arg(text.as_str());
        }
        Part::Number {
            decimals,
            min_decimals,
            min_int,
            grouping,
        } => {
            out.begin("number");
            if *decimals != 0 {
                out.prop("decimals", i128::from(*decimals));
            }
            if *min_decimals != 0 {
                out.prop("min-decimals", i128::from(*min_decimals));
            }
            if *min_int != 0 {
                out.prop("min-int", i128::from(*min_int));
            }
            if *grouping {
                out.prop("grouping", true);
            }
        }
        Part::Currency(symbol) => {
            out.begin("currency");
            out.arg(symbol.as_str());
        }
        Part::Year { long } => long_node(out, "year", *long),
        Part::Month { long, textual } => {
            out.begin("month");
            if *long {
                out.prop("long", true);
            }
            if *textual {
                out.prop("textual", true);
            }
        }
        Part::Day { long } => long_node(out, "day", *long),
        Part::DayOfWeek { long } => long_node(out, "day-of-week", *long),
        Part::Hours { long } => long_node(out, "hours", *long),
        Part::Minutes { long } => long_node(out, "minutes", *long),
        Part::Seconds { long, decimals } => {
            out.begin("seconds");
            if *long {
                out.prop("long", true);
            }
            if *decimals != 0 {
                out.prop("decimals", i128::from(*decimals));
            }
        }
        Part::AmPm => out.begin("am-pm"),
        Part::Boolean => out.begin("boolean"),
        Part::Content => out.begin("content"),
    }
    out.end();
}

fn long_node(out: &mut Emitter, name: &str, long: bool) {
    out.begin(name);
    if long {
        out.prop("long", true);
    }
}

fn kind_word(kind: Kind) -> &'static str {
    match kind {
        Kind::Number => "number",
        Kind::Percentage => "percentage",
        Kind::Currency => "currency",
        Kind::Date => "date",
        Kind::Time => "time",
        Kind::Boolean => "boolean",
        Kind::Text => "text",
    }
}

/// `filter` — the autofilter, and the values it keeps per field (§9.4).
fn filter(out: &mut Emitter, sheet: &Sheet) {
    let Some(filter) = sheet.filter() else {
        return;
    };
    out.blank();
    out.begin("filter");
    out.arg(filter.name.as_str());
    out.arg_word(&range(filter.start, filter.end));
    if filter.contains_header {
        out.prop("header", true);
    }
    if filter.buttons {
        out.prop("buttons", true);
    }
    if filter.keep.is_empty() {
        out.end();
        return;
    }
    out.open();
    for (field, values) in &filter.keep {
        out.begin("keep");
        out.arg(i128::from(*field));
        for value in values {
            out.arg(value.as_str());
        }
        out.end();
    }
    out.close();
}

// --- the small shared pieces ---

/// A cell's value as a KDL one.
///
/// A whole number is written as an integer, which is what the file would look like if a person
/// had typed it — and it reads back as the same `f64`, since the projection only ever writes
/// one it can spell exactly.
fn value_of(value: &CellValue) -> KdlValue {
    match value {
        CellValue::Empty => KdlValue::Null,
        CellValue::Number(n) => number(*n),
        CellValue::Text(text) => KdlValue::String(text.clone()),
        CellValue::Bool(b) => KdlValue::Bool(*b),
    }
}

/// The integer spelling when it is exact, the float spelling otherwise.
///
/// `2^53` is where an `f64` stops holding every integer, and past it an integer spelling would
/// be a different number on the way back.
fn number(n: f64) -> KdlValue {
    const EXACT: f64 = 9_007_199_254_740_992.0;
    match n.is_finite() && n.fract() == 0.0 && n.abs() < EXACT {
        true => KdlValue::Integer(n as i128),
        false => KdlValue::Float(n),
    }
}

/// A cell's address as the span map spells it: sheet-qualified, so two sheets' `A1` are two
/// anchors.
fn address(sheet: &Sheet, pos: Pos) -> String {
    a1::format(Some(&sheet.name), pos)
}

/// A rectangle as a person writes one — `B5` when it is a single cell.
fn range(start: Pos, end: Pos) -> String {
    match start == end {
        true => a1::format(None, start),
        false => format!("{}:{}", a1::format(None, start), a1::format(None, end)),
    }
}

/// A set of cells as the rectangles that cover it, each cell in exactly one.
///
/// Horizontal runs first, then a vertical merge of runs that span the same columns on
/// consecutive rows — which is the shape a styled block actually has, and cheap. It is not a
/// minimal cover and does not need to be: every cell is covered exactly once whatever the
/// packing, so the only thing at stake is how many lines the file has.
fn rectangles(cells: &BTreeSet<Pos>) -> Vec<(Pos, Pos)> {
    // `Pos` orders by row then column, so consecutive entries of one row arrive in order.
    let mut runs: Vec<(u32, u32, u32)> = Vec::new(); // (row, first col, last col)
    for pos in cells {
        match runs.last_mut() {
            Some((row, _, last)) if *row == pos.row && *last + 1 == pos.col => *last = pos.col,
            _ => runs.push((pos.row, pos.col, pos.col)),
        }
    }
    // Merge downwards: a run is extended by the run below it that spans the same columns.
    let mut out: Vec<(Pos, Pos)> = Vec::new();
    let mut open: BTreeMap<(u32, u32), (u32, u32)> = BTreeMap::new(); // cols -> (first row, last row)
    for (row, first, last) in runs {
        match open.get_mut(&(first, last)) {
            Some((_, bottom)) if *bottom + 1 == row => *bottom = row,
            Some((top, bottom)) => {
                out.push((Pos::new(*top, first), Pos::new(*bottom, last)));
                open.insert((first, last), (row, row));
            }
            None => {
                open.insert((first, last), (row, row));
            }
        }
    }
    for ((first, last), (top, bottom)) in open {
        out.push((Pos::new(top, first), Pos::new(bottom, last)));
    }
    out.sort_by_key(|(start, _)| *start);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_block_of_cells_becomes_one_rectangle() {
        let cells: BTreeSet<Pos> = (0..3)
            .flat_map(|row| (1..4).map(move |col| Pos::new(row, col)))
            .collect();
        assert_eq!(
            rectangles(&cells),
            vec![(Pos::new(0, 1), Pos::new(2, 3))],
            "three equal runs on consecutive rows are one block"
        );
    }

    #[test]
    fn every_cell_is_covered_exactly_once() {
        // A deliberately awkward shape: an L, a hole, and a stray.
        let cells: BTreeSet<Pos> = [(0, 0), (0, 1), (1, 0), (2, 0), (2, 1), (2, 2), (5, 7)]
            .into_iter()
            .map(|(row, col)| Pos::new(row, col))
            .collect();
        let mut covered: Vec<Pos> = Vec::new();
        for (start, end) in rectangles(&cells) {
            for row in start.row..=end.row {
                for col in start.col..=end.col {
                    covered.push(Pos::new(row, col));
                }
            }
        }
        covered.sort();
        assert_eq!(covered.len(), cells.len(), "no cell twice, none invented");
        assert_eq!(covered, cells.into_iter().collect::<Vec<_>>());
    }

    #[test]
    fn a_whole_number_is_written_as_one() {
        assert_eq!(number(4200.0).to_string(), "4200");
        assert_eq!(number(-0.5).to_string(), "-0.5");
        assert_eq!(number(1e300).to_string(), "1e300");
        // Past 2^53 an integer spelling would not read back as the same double.
        assert!(matches!(number(1e17), KdlValue::Float(_)));
    }

    #[test]
    fn a_range_of_one_cell_is_spelled_as_the_cell() {
        assert_eq!(range(Pos::new(0, 1), Pos::new(0, 1)), "B1");
        assert_eq!(range(Pos::new(0, 1), Pos::new(4, 2)), "B1:C5");
    }
}
