// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Reading a projection back into a spreadsheet. **\[ODS\]**
//!
//! The twin of `odf::read`, and **not** tolerant in the same way. `odf::read` ignores what it
//! does not recognise because R5 says other people's files have to load; a projection is a
//! format *this project writes*, so an unknown node is a typo in something a person hand-wrote
//! and saying so with a line number is the kindness. That is `doc/dsl.md`'s "strictness on the
//! way out" pointed the other way round: the tolerance a projection offers is in accepting two
//! spellings of the same state (`at`/`row` beside `cell`), not in swallowing a third.
//!
//! Where it *is* lenient is where a human is typing: a missing property takes ODF's own
//! default, `"=SUM(…)"` in the value position is understood as a formula, and a range spelled
//! as one cell covers one cell.

use grind_core::projection::kdl::{KdlDocument, KdlNode, KdlValue};
use grind_core::projection::{Shape, Source, entry_span, node_span};

use crate::model::{CellValue, Document, NumberKind, Pos, Sheet};
use crate::numfmt::{self, Format, Kind, Map, Op, Part};
use crate::style::CellStyle;
use crate::{Error, Result, a1, filter, formula, locale, style};

/// Read a projection.
///
/// The `Source` built alongside is R6 for this form (D5, `grind_core::projection::source`): the
/// text as it came in, and where every cell sits in it, so that saving one edited cell rewrites
/// one value and leaves the comments, the blank lines and the hand alignment alone. It costs one
/// `record` per cell in the two places that read one, because the reader is already computing
/// the address — which is the argument for building the map here rather than in a second pass.
pub fn read(text: &str) -> Result<Document> {
    let (kind, body) = grind_core::projection::parse(text)?;
    if kind != grind_core::DocumentKind::Spreadsheet {
        return Err(Error::Odf(grind_core::Error::UnsupportedKind(Some(kind))));
    }
    let mut source = Source::new(text);
    let mut doc = Document {
        sheets: Vec::new(),
        ..Document::default()
    };
    for node in body.nodes() {
        match node.name().value() {
            "null-date" => {
                let (y, m, d) = ymd(node)?;
                doc.null_date = formula::date::days_from_civil(y, m, d);
            }
            "null-year" => doc.null_year = integer(node, 0)?,
            "name" => {
                let name = text_arg(node, 0)?;
                let expression = text_arg(node, 1)?;
                doc.names.insert(name.to_lowercase(), expression);
            }
            "sheet" => doc.sheets.push(sheet(node, &mut source)?),
            other => return Err(unknown(node, other, "a document")),
        }
    }
    // A document with no sheets is not one ODF can express, and every reader in this crate
    // hands back something openable. `Document::default` has the one empty sheet a *user*
    // gets, so an empty projection opens as a new document rather than as a broken one.
    if doc.sheets.is_empty() {
        doc.sheets.push(Sheet::new("Sheet1"));
    }
    doc.projection_source = Some(Box::new(source));
    Ok(doc)
}

// --- a sheet ---

fn sheet(node: &KdlNode, source: &mut Source) -> Result<Sheet> {
    let mut sheet = Sheet::new(text_arg(node, 0)?);
    for child in children(node) {
        match child.name().value() {
            "col" => {
                let col = track_index(child)?;
                if let Some(width) = string_prop(child, "width") {
                    sheet.set_col_width(col, Some(width));
                }
                if bool_prop(child, "hidden").unwrap_or(false) {
                    sheet.set_col_hidden(col, true);
                }
            }
            "row" => {
                let row = track_index(child)?;
                if let Some(height) = string_prop(child, "height") {
                    sheet.set_row_height(row, Some(height));
                }
                if bool_prop(child, "hidden").unwrap_or(false) {
                    sheet.set_row_hidden(row, true);
                }
            }
            "at" => at_block(&mut sheet, child, source)?,
            "cell" => cell(&mut sheet, child, source)?,
            "style" => {
                let value = cell_style(child)?;
                for pos in cells_of(child)? {
                    sheet.set_style(pos, value.clone());
                }
            }
            "format" => {
                let value = format(child)?;
                for pos in cells_of(child)? {
                    sheet.set_format(pos, value.clone());
                }
            }
            "filter" => sheet.set_filter(Some(autofilter(child)?)),
            other => return Err(unknown(child, other, "a sheet")),
        }
    }
    Ok(sheet)
}

/// `at A1 { row … }` — a grid of plain values, laid down from a top-left corner.
fn at_block(sheet: &mut Sheet, node: &KdlNode, source: &mut Source) -> Result<()> {
    let start = position(node, 0)?;
    for (row, child) in (start.row..).zip(children(node)) {
        if child.name().value() != "row" {
            return Err(unknown(child, child.name().value(), "an `at` block"));
        }
        for (offset, entry) in child
            .entries()
            .iter()
            .filter(|entry| entry.name().is_none())
            .enumerate()
        {
            let pos = Pos::new(row, start.col + offset as u32);
            // `#null` is a hole in the grid, not an empty string: it is how a row states that
            // the cell between its neighbours has nothing in it.
            if !matches!(entry.value(), KdlValue::Null) {
                sheet.set(pos, value(entry.value()));
                // A grid row is one line and a dozen splice sites: writing 4300 over 4200
                // changes those four bytes and nothing else on the line, which is what makes
                // `git diff` on a `.grind` read like a spreadsheet edit rather than a rewrite.
                // The hole is deliberately not a site — there is nothing there to replace, and
                // a cell that appears where one was is a structural change (D5's second
                // boundary, `grind_core::projection::source`).
                source.record(mark(sheet, pos), entry_span(entry), Shape::Value);
            }
        }
    }
    Ok(())
}

/// `cell B5 15400 formula="of:=SUM(…)" date=#true` — one cell that carries more than a value.
fn cell(sheet: &mut Sheet, node: &KdlNode, source: &mut Source) -> Result<()> {
    let pos = position(node, 0)?;
    // The whole node, because everything about this cell is on it: editing the formula rewrites
    // one line, and the `kdl` span stops before the indentation and the newline, so what is
    // spliced in is exactly what the writer would emit for a node at the left margin.
    source.record(mark(sheet, pos), node_span(node), Shape::Node);
    let mut formula = string_prop(node, "formula");
    if let Some(entry) = argument(node, 1) {
        match entry {
            // The authoring shorthand (`doc/dsl.md` §3.4): a string in the value position that
            // starts with `=` is the formula. Normalised to ODF's own spelling on the way in,
            // so a hand-written file and its re-projection differ by this string and nothing
            // else.
            KdlValue::String(text) if text.starts_with('=') && formula.is_none() => {
                formula = Some(format!("of:{text}"));
            }
            other => sheet.set(pos, value(other)),
        }
    }
    if let Some(formula) = formula {
        sheet.set_formula(pos, formula);
    }
    if bool_prop(node, "date").unwrap_or(false) {
        sheet.set_kind(pos, NumberKind::Date);
    }
    if bool_prop(node, "time").unwrap_or(false) {
        sheet.set_kind(pos, NumberKind::Time);
    }
    Ok(())
}

// --- styles ---

fn cell_style(node: &KdlNode) -> Result<CellStyle> {
    let mut out = CellStyle::default();
    if bool_prop(node, "bold").unwrap_or(false) {
        out.font_weight = Some("bold".to_owned());
    }
    if let Some(weight) = string_prop(node, "weight") {
        out.font_weight = Some(weight);
    }
    if bool_prop(node, "italic").unwrap_or(false) {
        out.font_style = Some("italic".to_owned());
    }
    if let Some(slant) = string_prop(node, "slant") {
        out.font_style = Some(slant);
    }
    out.font_size = string_prop(node, "size");
    // A palette name is resolved here rather than stored, so a hand-written `color=red` and a
    // document's own `#ff4136` are the same style and pool as one (`grind_core::style`).
    out.color = string_prop(node, "color").map(|c| colour(&c));
    out.background = string_prop(node, "background").map(|c| colour(&c));
    out.align = string_prop(node, "align");
    out.vertical_align = string_prop(node, "valign");
    out.wrap = match node.get("wrap") {
        Some(KdlValue::Bool(true)) => Some("wrap".to_owned()),
        Some(KdlValue::Bool(false)) => Some("no-wrap".to_owned()),
        Some(KdlValue::String(other)) => Some(other.clone()),
        _ => None,
    };
    if let Some(border) = string_prop(node, "border") {
        out.set_border(Some(border));
    }
    for (edge, slot) in style::EDGES.iter().zip(&mut out.borders) {
        if let Some(value) = string_prop(node, &format!("border-{edge}")) {
            *slot = Some(value);
        }
    }
    Ok(out)
}

fn colour(value: &str) -> String {
    style::palette(value).map_or_else(|| value.to_owned(), str::to_owned)
}

// --- number formats ---

fn format(node: &KdlNode) -> Result<Format> {
    let word = word_arg(node, 1)?;
    let tag = string_prop(node, "locale");
    let locale = tag.as_deref().and_then(locale::Locale::parse);
    match node.children() {
        // No block: one of `numfmt::preset`'s, named by its parameters.
        None => {
            if word == "datetime" {
                return Ok(numfmt::datetime_preset().in_locale(locale));
            }
            let decimals = small(node, "decimals")?;
            let grouping = bool_prop(node, "grouping").unwrap_or(false);
            let symbol = string_prop(node, "symbol").unwrap_or_default();
            Ok(numfmt::preset(kind(node, &word)?, decimals, grouping, &symbol).in_locale(locale))
        }
        // A block: the parts, spelled out.
        Some(block) => {
            let mut out = Format::new(kind(node, &word)?).in_locale(locale);
            parts(&mut out, block)?;
            Ok(out)
        }
    }
}

fn parts(out: &mut Format, block: &KdlDocument) -> Result<()> {
    for node in block.nodes() {
        let part = match node.name().value() {
            "text" => Part::Text(text_arg(node, 0)?),
            "number" => Part::Number {
                decimals: small(node, "decimals")?,
                min_decimals: small(node, "min-decimals")?,
                min_int: small(node, "min-int")?,
                grouping: bool_prop(node, "grouping").unwrap_or(false),
            },
            "currency" => Part::Currency(text_arg(node, 0)?),
            "year" => Part::Year { long: long(node) },
            "month" => Part::Month {
                long: long(node),
                textual: bool_prop(node, "textual").unwrap_or(false),
            },
            "day" => Part::Day { long: long(node) },
            "day-of-week" => Part::DayOfWeek { long: long(node) },
            "hours" => Part::Hours { long: long(node) },
            "minutes" => Part::Minutes { long: long(node) },
            "seconds" => Part::Seconds {
                long: long(node),
                decimals: small(node, "decimals")?,
            },
            "am-pm" => Part::AmPm,
            "boolean" => Part::Boolean,
            "content" => Part::Content,
            "map" => {
                out.maps.push(branch(node)?);
                continue;
            }
            other => return Err(unknown(node, other, "a format")),
        };
        out.push(part);
    }
    Ok(())
}

/// `map ">=" "0" currency { … }` — one `style:map` branch (§5.1's red-negative currency).
fn branch(node: &KdlNode) -> Result<Map> {
    let spelling = word_arg(node, 0)?;
    let op = Op::SPELLINGS
        .iter()
        .find(|(text, _)| *text == spelling)
        .map(|(_, op)| *op)
        .ok_or_else(|| {
            Error::Odf(grind_core::Error::Projection(format!(
                "`{spelling}` is not a comparison — one of {}",
                Op::SPELLINGS
                    .iter()
                    .map(|(text, _)| *text)
                    .collect::<Vec<_>>()
                    .join(", ")
            )))
        })?;
    let value = text_arg(node, 1)?;
    // The branch's own format is spelled exactly as the outer one is, minus the range: the
    // third argument is its kind and its block holds its parts.
    let locale = string_prop(node, "locale")
        .as_deref()
        .and_then(locale::Locale::parse);
    let mut inner = Format::new(kind(node, &word_arg(node, 2)?)?).in_locale(locale);
    if let Some(block) = node.children() {
        parts(&mut inner, block)?;
    }
    Ok(Map {
        op,
        value,
        format: inner,
    })
}

fn kind(node: &KdlNode, word: &str) -> Result<Kind> {
    match word {
        "number" => Ok(Kind::Number),
        "percentage" => Ok(Kind::Percentage),
        "currency" => Ok(Kind::Currency),
        "date" => Ok(Kind::Date),
        "time" => Ok(Kind::Time),
        "boolean" => Ok(Kind::Boolean),
        "text" => Ok(Kind::Text),
        other => Err(at(
            node,
            format!(
                "`{other}` is not a number format — number, percentage, currency, date, time, \
                 boolean, text or datetime"
            ),
        )),
    }
}

fn long(node: &KdlNode) -> bool {
    bool_prop(node, "long").unwrap_or(false)
}

fn small(node: &KdlNode, name: &str) -> Result<u8> {
    match node.get(name) {
        None => Ok(0),
        Some(_) => u8::try_from(prop_integer(node, name)?).map_err(|_| {
            at(
                node,
                format!("{name} is larger than a number format can hold"),
            )
        }),
    }
}

// --- the autofilter ---

fn autofilter(node: &KdlNode) -> Result<filter::Filter> {
    let name = text_arg(node, 0)?;
    let (start, end) = rectangle(node, 1)?;
    let mut out = filter::Filter::new(name, start, end);
    out.contains_header = bool_prop(node, "header").unwrap_or(false);
    out.buttons = bool_prop(node, "buttons").unwrap_or(false);
    for child in children(node) {
        if child.name().value() != "keep" {
            return Err(unknown(child, child.name().value(), "a filter"));
        }
        let field = u32::try_from(integer(child, 0)?)
            .map_err(|_| at(child, "a filter field is a column number".to_owned()))?;
        let values = child
            .entries()
            .iter()
            .filter(|entry| entry.name().is_none())
            .skip(1)
            .map(|entry| text_of(entry.value()))
            .collect();
        out.keep.insert(field, values);
    }
    Ok(out)
}

// --- values, addresses and the small readers ---

/// A KDL value as a cell value.
///
/// Every KDL number becomes a `Number`, integer spelling or not: §4.3.1 has one numeric type
/// and the spelling is the file's business rather than the model's.
fn value(value: &KdlValue) -> CellValue {
    match value {
        KdlValue::String(text) => CellValue::Text(text.clone()),
        KdlValue::Integer(n) => CellValue::Number(*n as f64),
        KdlValue::Float(n) => CellValue::Number(*n),
        KdlValue::Bool(b) => CellValue::Bool(*b),
        KdlValue::Null => CellValue::Empty,
    }
}

/// Whatever a value's text is — a bare word and a quoted string are the same thing to KDL, and
/// a filter set item written as a number is the display text it matches.
fn text_of(value: &KdlValue) -> String {
    match value {
        KdlValue::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// The address a cell is recorded under — the writer's own [`super::write::address`], so that a
/// site recorded on the way in is looked up by the same string on the way out.
fn mark(sheet: &Sheet, pos: Pos) -> String {
    super::write::address(sheet, pos)
}

fn argument(node: &KdlNode, index: usize) -> Option<&KdlValue> {
    node.entries()
        .iter()
        .filter(|entry| entry.name().is_none())
        .nth(index)
        .map(|entry| entry.value())
}

fn text_arg(node: &KdlNode, index: usize) -> Result<String> {
    argument(node, index)
        .map(text_of)
        .ok_or_else(|| at(node, format!("needs {} arguments", index + 1)))
}

/// An argument that has to be a word rather than a number — a kind, a comparison.
fn word_arg(node: &KdlNode, index: usize) -> Result<String> {
    match argument(node, index) {
        Some(KdlValue::String(word)) => Ok(word.clone()),
        _ => Err(at(node, format!("needs a word as argument {}", index + 1))),
    }
}

fn integer(node: &KdlNode, index: usize) -> Result<i64> {
    match argument(node, index) {
        Some(KdlValue::Integer(n)) => {
            i64::try_from(*n).map_err(|_| at(node, "out of range".into()))
        }
        _ => Err(at(
            node,
            format!("needs a number as argument {}", index + 1),
        )),
    }
}

fn prop_integer(node: &KdlNode, name: &str) -> Result<i64> {
    match node.get(name) {
        Some(KdlValue::Integer(n)) => {
            i64::try_from(*n).map_err(|_| at(node, format!("{name} is out of range")))
        }
        _ => Err(at(node, format!("{name} is a number"))),
    }
}

fn string_prop(node: &KdlNode, name: &str) -> Option<String> {
    node.get(name).map(text_of)
}

fn bool_prop(node: &KdlNode, name: &str) -> Option<bool> {
    node.get(name)?.as_bool()
}

fn children(node: &KdlNode) -> &[KdlNode] {
    node.children().map_or(&[], KdlDocument::nodes)
}

/// A 1-based track number as the 0-based index the core uses — `a1.rs`'s rule, for the one
/// address in this format that is a bare number rather than a name.
fn track_index(node: &KdlNode) -> Result<u32> {
    let n = integer(node, 0)?;
    u32::try_from(n - 1).map_err(|_| at(node, "a row or column is numbered from 1".to_owned()))
}

/// One cell address (`B5`) as a position.
fn position(node: &KdlNode, index: usize) -> Result<Pos> {
    let (start, end) = span(node, index)?;
    match start == end {
        true => Ok(start),
        false => Err(at(node, "needs one cell, not a range".to_owned())),
    }
}

/// A range argument (`B1:C5`, or `B5` for one cell) as its two corners.
fn rectangle(node: &KdlNode, index: usize) -> Result<(Pos, Pos)> {
    span(node, index)
}

/// Every cell of a node's range argument — what a `style` or a `format` covers.
fn cells_of(node: &KdlNode) -> Result<Vec<Pos>> {
    let (start, end) = span(node, 0)?;
    Ok((start.row..=end.row)
        .flat_map(|row| (start.col..=end.col).map(move |col| Pos::new(row, col)))
        .collect())
}

/// The shared half: an address argument through `a1::parse`, so a projection and a formula
/// cannot disagree about what `B1:C5` means.
fn span(node: &KdlNode, index: usize) -> Result<(Pos, Pos)> {
    let text = word_arg(node, index)?;
    let reference = a1::parse(&text)?;
    let end = reference
        .end
        .clone()
        .unwrap_or_else(|| reference.start.clone());
    let corner = |axis: &Option<formula::lex::Axis>| {
        axis.map(|a| a.index)
            .ok_or_else(|| at(node, format!("`{text}` has to name a row and a column")))
    };
    let start = Pos::new(corner(&reference.start.row)?, corner(&reference.start.col)?);
    let stop = Pos::new(corner(&end.row)?, corner(&end.col)?);
    match start.row <= stop.row && start.col <= stop.col {
        true => Ok((start, stop)),
        false => Err(at(node, format!("`{text}` runs backwards"))),
    }
}

/// `null-date "1899-12-30"`.
fn ymd(node: &KdlNode) -> Result<(i64, i64, i64)> {
    let text = text_arg(node, 0)?;
    let mut fields = text.split('-');
    let mut next = || fields.next().and_then(|f| f.parse::<i64>().ok());
    match (next(), next(), next(), fields.next()) {
        (Some(y), Some(m), Some(d), None) => Ok((y, m, d)),
        _ => Err(at(node, format!("`{text}` is not a YYYY-MM-DD date"))),
    }
}

// --- diagnostics ---

/// A complaint about a node, with where it is.
///
/// `kdl` gives every node a span, so a projection's errors can point at a line the way a
/// compiler's do — which is most of what makes a hand-written format usable at all.
fn at(node: &KdlNode, message: String) -> Error {
    Error::Odf(grind_core::Error::Projection(format!(
        "`{}` at offset {}: {message}",
        node.name().value(),
        node.span().offset()
    )))
}

fn unknown(node: &KdlNode, name: &str, inside: &str) -> Error {
    at(node, format!("`{name}` is not something {inside} holds"))
}
