// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Serialising a document back to ODF. **\[ODS\]**, with `package` below it **\[GENERIC\]**.
//!
//! Templates from doc/ods-format.md §7; the package layout from §1.1/§1.3. The two forms
//! share one content writer and differ in exactly two places — the root element name and
//! whether `office:mimetype` sits on it (§7.3) — so there is no second serialiser to keep
//! in step.
//!
//! The output is deliberately minimal (§1.4): no `styles.xml`, no `meta.xml` and no
//! `settings.xml`, because there is nothing yet to put in them. `office:automatic-styles`
//! is written when — and only when — some cell carries a number format, and it holds
//! exactly the formats in use: §5.3's pooling rule is not an optimisation here, it is the
//! only construct ODF has for saying a cell looks a certain way.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt::Write as _;
use std::sync::LazyLock;

use super::names::{CALCEXT, CHART, DRAW, FO, NUMBER, OFFICE, STYLE, SVG, TABLE, TEXT, XLINK};
// Packaging, the manifest, the ODF version and XML escaping are the same for every document
// type (§1.1, §1.3), so they live in `grind-core` and are reached here by the names this
// file always used.
use crate::chart::Axis;
use crate::formula::date;
use crate::model::{CellValue, Document, NumberKind, Pos, Sheet};
use crate::numfmt::{self, Format, Kind, Part};
use crate::style::{CellStyle, EDGES};
use crate::{MAX_ROWS, Result};
use grind_core::odf::envelope;
use grind_core::odf::package::{self, SubDocument, VERSION, write_package_with};
use grind_core::odf::xml::esc;

/// The media type, byte for byte. Sniffed by readers at a fixed offset in the package
/// form (§1.1), so it is not somewhere to be creative.
pub const MIMETYPE: &str = "application/vnd.oasis.opendocument.spreadsheet";

/// Which physical form to write. Generic to every document type, so it lives in `grind-core`
/// (§1) and is re-exported here for the callers that always spelled it `write::Form`.
pub use grind_core::Form;

pub fn write(doc: &Document, form: Form) -> Result<Vec<u8>> {
    // The third form is not XML at all, so it leaves before any of this file runs
    // (`doc/dsl.md` §9). It is here rather than one layer up because `write_bytes` is the
    // one door out of the crate, and a form that only *some* callers knew to handle would
    // be a form that escapes through the others.
    if form == Form::Projection {
        return Ok(crate::projection::save(doc));
    }
    // A save into the *other* form of the file this document came from is a save into its own
    // form — with every guard that has — moved across by `forms::convert`, which carries every
    // part and refuses what the other form has no place for. Starting from nothing instead used
    // to drop the page layout, the named styles and every element the model does not read, with
    // no error at all.
    if let Some(source) = doc.source.as_deref()
        && source.form != form
        && source.form != Form::Projection
    {
        let own = write(doc, source.form)?;
        let converted = grind_core::odf::forms::convert(&own, form)?;
        verify(doc, &converted)?;
        return Ok(converted);
    }
    // The file this document came from, when it is being saved in the same form.
    let source = doc.source.as_deref().filter(|source| source.form == form);

    // R6 first: a document that came from a file and has only had cells edited goes back as
    // that file with those cells replaced. Otherwise the content regenerates — and is **merged
    // into the original** (`envelope::merge`), so the styles, master pages, metadata and
    // settings the model does not own are carried rather than dropped. Saving never makes an
    // existing file worse.
    if let Some(spliced) = splice(doc, form) {
        // Whatever touched somebody's file is checked before it can replace it: a save that
        // fails leaves the original on disk, and one that writes something unreadable does not.
        if let Some(source) = source {
            envelope::check_against(&source.bytes, &spliced)?;
            let allowed = overwritten(doc, source);
            let lost = envelope::losses_allowing(&source.bytes, &spliced, &owned, &allowed)
                .unwrap_or_default();
            if !lost.is_empty() {
                return Err(grind_core::Error::WouldLose(lost).into());
            }
            verify(doc, &spliced)?;
        }
        return match source.and_then(|source| source.package.as_deref()) {
            Some(original) => Ok(envelope::repackage(original, &spliced, &[], &[], &[])?),
            None => Ok(spliced),
        };
    }

    // Directories a regenerated chart must not take: everything already in the package except
    // the charts this build read, which the regenerated ones replace.
    let original = source.and_then(|source| source.package.as_deref());
    let taken: HashSet<String> = match (original, source) {
        (Some(original), Some(source)) => package::entry_names(original)
            .into_iter()
            .filter_map(|name| name.split_once('/').map(|(dir, _)| dir.to_owned()))
            .filter(|dir| !source.chart_parts.contains(dir))
            .collect(),
        _ => HashSet::new(),
    };
    let origin = source.filter(|source| source.body.is_some());
    let (generated, objects) = content(doc, form, &taken, origin);
    let content =
        match source.and_then(|source| envelope::merge(&source.bytes, generated.as_bytes())) {
            Some(merged) if original.is_none() => patch_locale(merged, doc, source),
            Some(merged) => merged,
            None => generated.into_bytes(),
        };

    // Whatever touched somebody's file is checked before it can replace it: a save that fails
    // leaves the original on disk, and one that writes something unreadable — or that drops
    // what the model never read — does not.
    if let Some(source) = source {
        envelope::check_against(&source.bytes, &content)?;
        // A sheet somebody deleted takes its contents with it — that is the edit, not a loss.
        let removed: Vec<std::ops::Range<usize>> = source
            .tables
            .iter()
            .enumerate()
            .filter(|(ti, _)| !doc.sheets.iter().any(|s| s.origin.table == Some(*ti)))
            .map(|(_, t)| t.range.clone())
            .collect();
        let mut allowed = removed;
        allowed.extend(overwritten(doc, source));
        let lost = envelope::losses_allowing(&source.bytes, &content, &owned, &allowed)
            .unwrap_or_default();
        if !lost.is_empty() {
            return Err(grind_core::Error::WouldLose(lost).into());
        }
        verify(doc, &content)?;
    }
    match (form, original, source) {
        (Form::Package, Some(original), Some(source)) => {
            // `styles.xml`: the file's own, with the locale patched in where it changed, or the
            // generated one merged into it where the file has no default cell style of its own.
            let mut replace: Vec<(String, Vec<u8>)> = Vec::new();
            let original_styles = package::styles_xml(original);
            let generated_styles = common_styles_part(doc);
            match (&original_styles, &generated_styles) {
                (Some(own), Some(ours)) => {
                    let merged =
                        envelope::merge(own, ours.as_bytes()).unwrap_or_else(|| own.clone());
                    let patched = patch_locale(merged, doc, Some(source));
                    if patched != *own {
                        replace.push(("styles.xml".to_owned(), patched));
                    }
                }
                (Some(own), None) => {
                    let patched = patch_locale(own.clone(), doc, Some(source));
                    if patched != *own {
                        replace.push(("styles.xml".to_owned(), patched));
                    }
                }
                (None, Some(ours)) => {
                    replace.push(("styles.xml".to_owned(), ours.clone().into_bytes()))
                }
                (None, None) => {}
            }
            let mut added = Vec::new();
            if original_styles.is_none() && generated_styles.is_some() {
                added.push(
                    "<manifest:file-entry manifest:full-path=\"styles.xml\" \
                     manifest:media-type=\"text/xml\"/>"
                        .to_owned(),
                );
            }
            for sub in &objects {
                replace.push((
                    format!("{}/content.xml", sub.directory),
                    sub.content.clone().into_bytes(),
                ));
                added.extend(package::subdocument_entries(sub));
            }
            Ok(envelope::repackage(
                original,
                &content,
                &replace,
                &source.chart_parts,
                &added,
            )?)
        }
        (Form::Package, _, _) => {
            let styles = common_styles_part(doc);
            Ok(write_package_with(
                MIMETYPE,
                &String::from_utf8_lossy(&content),
                styles.as_deref(),
                &objects,
            )?)
        }
        _ => Ok(content),
    }
}

/// The paragraphs of every cell somebody typed a new value into: the text that value replaced —
/// a hyperlink or a run of bold in it included — is the edit, not a loss, exactly as typing over
/// such a cell in LibreOffice replaces it. An annotation beside the paragraphs is not text the
/// value replaced, and is not in these ranges.
fn overwritten(doc: &Document, source: &super::source::Source) -> Vec<std::ops::Range<usize>> {
    let bytes = &source.bytes;
    let mut out = Vec::new();
    for sheet in &doc.sheets {
        let Some((ti, t)) = sheet
            .origin
            .table
            .and_then(|ti| source.tables.get(ti).map(|t| (ti, t)))
        else {
            continue;
        };
        for pos in &sheet.origin.values {
            let Some(e) = t
                .rows
                .iter()
                .find(|e| (e.first..e.first.saturating_add(e.repeat)).contains(&pos.row))
            else {
                continue;
            };
            let cells = source
                .rows
                .get(&(ti, e.first))
                .cloned()
                .unwrap_or_else(|| scan_cells(bytes, e).0);
            let Some(cell) = cells.iter().find(|c| c.cols.contains(&pos.col)) else {
                continue;
            };
            out.extend(paragraphs(bytes, cell.range.clone()));
        }
    }
    out
}

/// The `text:p` children of an element, by their extents.
fn paragraphs(bytes: &[u8], element: std::ops::Range<usize>) -> Vec<std::ops::Range<usize>> {
    let mut out = Vec::new();
    let Some(open_end) = bytes[element.clone()].iter().position(|c| *c == b'>') else {
        return out;
    };
    let mut at = element.start + open_end + 1;
    let end = element.end;
    while at < end {
        let Some(lt) = bytes[at..end].iter().position(|c| *c == b'<') else {
            break;
        };
        let start = at + lt;
        if bytes.get(start + 1) == Some(&b'/') {
            break;
        }
        let Some(tag_end) = bytes[start..end].iter().position(|c| *c == b'>') else {
            break;
        };
        let Some(range) = grind_core::odf::xml::element_extent(bytes, start..start + tag_end + 1)
        else {
            break;
        };
        let tag = &bytes[start..start + tag_end + 1];
        let name_end = tag
            .iter()
            .position(|c| c.is_ascii_whitespace() || *c == b'>' || *c == b'/')
            .unwrap_or(tag.len());
        if tag[..name_end].ends_with(b":p") {
            out.push(range.clone());
        }
        at = range.end;
    }
    out
}

/// **The edits themselves, checked**: what is about to replace somebody's file is read back and
/// every sheet's name, and every cell's value and formula, compared with the document being
/// saved. A splice that kept the file's bytes where it should have written the model's would
/// otherwise be a save that quietly undid an edit — the one failure worse than a refused save.
/// Styles are not compared: a cell written into a column with a default style of its own reads
/// back with it, which is the file's meaning and not a lost edit.
fn verify(doc: &Document, content: &[u8]) -> Result<()> {
    let refuse = |why: String| -> Result<()> {
        Err(grind_core::Error::Xml(format!(
            "not saved: the file would not read back as the document ({why}) — this is a bug"
        ))
        .into())
    };
    let back = match super::read(content) {
        Ok(back) => back,
        Err(error) => return refuse(error.to_string()),
    };
    if back.sheets.len() != doc.sheets.len() {
        return refuse(format!(
            "{} sheets, not {}",
            back.sheets.len(),
            doc.sheets.len()
        ));
    }
    for (mine, theirs) in doc.sheets.iter().zip(&back.sheets) {
        if mine.name != theirs.name {
            return refuse(format!(
                "sheet {:?} came back as {:?}",
                mine.name, theirs.name
            ));
        }
        if !mine.formulas().eq(theirs.formulas()) {
            let at = mine
                .formulas()
                .zip(theirs.formulas())
                .find(|(a, b)| a != b)
                .map_or(String::new(), |((pos, _), _)| {
                    crate::a1::format(Some(&mine.name), pos)
                });
            return refuse(format!("a formula at {at}"));
        }
        // Values on every row either side carries anything on — the sparse way round a sheet
        // whose one far cell would make its rectangle enormous.
        let cols = mine.used_cols().max(theirs.used_cols());
        let rows: BTreeSet<u32> = mine
            .rows_carrying()
            .into_iter()
            .chain(theirs.rows_carrying())
            .flatten()
            .collect();
        for row in rows {
            for col in 0..cols {
                let pos = Pos::new(row, col);
                if mine.get(pos) != theirs.get(pos) {
                    return refuse(crate::a1::format(Some(&mine.name), pos));
                }
            }
        }
    }
    Ok(())
}

/// Whether a body key ([`envelope::body_vocabulary`]) is this writer's to change: what it
/// writes from the model when it rewrites a row, the column declarations, a sheet's charts, the
/// names or the filters. A count of anything else going down — a merge, an annotation, rich
/// text in a cell, a validation, a header-row group — is content the model never saw, and the
/// save is refused rather than allowed to drop it.
fn owned(key: &str) -> bool {
    let (element, attribute) = match key.split_once('@') {
        Some((element, attribute)) => (element, Some(attribute)),
        None => (key, None),
    };
    let in_ns = |uri: &str, k: &str| k.starts_with(&format!("{{{uri}}}"));
    let is = |uri: &str, locals: &[&str]| {
        locals
            .iter()
            .any(|local| element == format!("{{{uri}}}{local}"))
    };
    // A chart is the model's, all of it: its frame, its own document (inline in the flat form)
    // and that document's styles — nothing else in a spreadsheet's body is in these namespaces.
    let chart = in_ns(CHART, element)
        || in_ns(STYLE, element)
        || is(DRAW, &["frame", "object"])
        || is(
            OFFICE,
            &[
                "document",
                "document-content",
                "automatic-styles",
                "body",
                "chart",
            ],
        )
        || is(TABLE, &["shapes"]);
    if chart {
        return true;
    }
    let modelled = is(OFFICE, &["spreadsheet"])
        || is(TABLE, &["table", "table-column", "table-row", "table-cell"])
        || is(TEXT, &["p", "s", "tab", "line-break"])
        || is(
            TABLE,
            &["named-expressions", "named-expression", "named-range"],
        )
        || is(
            TABLE,
            &["database-ranges", "database-range", "filter", "filter-and"],
        )
        || is(TABLE, &["filter-condition", "filter-set-item"])
        || is(TABLE, &["calculation-settings", "null-date"]);
    let Some(attribute) = attribute else {
        return modelled;
    };
    let attr = |uri: &str, local: &str| attribute == format!("{{{uri}}}{local}");
    // LibreOffice's mirror of the value type (R4), restated on every cell it writes.
    if attr(CALCEXT, "value-type") {
        return true;
    }
    match () {
        _ if is(TABLE, &["table-cell"]) => {
            attr(OFFICE, "value-type")
                || attr(OFFICE, "value")
                || attr(OFFICE, "date-value")
                || attr(OFFICE, "time-value")
                || attr(OFFICE, "boolean-value")
                || attr(OFFICE, "string-value")
                || attr(OFFICE, "currency")
                || attr(TABLE, "formula")
                || attr(TABLE, "number-columns-repeated")
                || attr(TABLE, "style-name")
        }
        _ if is(TABLE, &["table-row"]) => {
            attr(TABLE, "style-name")
                || attr(TABLE, "number-rows-repeated")
                || attr(TABLE, "visibility")
        }
        _ if is(TABLE, &["table-column"]) => {
            attr(TABLE, "style-name")
                || attr(TABLE, "number-columns-repeated")
                || attr(TABLE, "visibility")
                || attr(TABLE, "default-cell-style-name")
        }
        // The rest of a table's start tag is kept whatever happens to its name.
        _ if is(TABLE, &["table"]) => true,
        _ if is(TABLE, &["named-expression", "named-range"]) => true,
        _ if is(
            TABLE,
            &[
                "database-range",
                "filter",
                "filter-condition",
                "filter-set-item",
            ],
        ) =>
        {
            true
        }
        _ if is(TABLE, &["calculation-settings"]) => attr(TABLE, "null-year"),
        _ if is(TABLE, &["null-date"]) => true,
        _ if is(TEXT, &["s"]) => attr(TEXT, "c"),
        _ => false,
    }
}

/// The document's locale written into the default cell style a file *already has*, where it
/// differs from what the file said (`Source::locale`). Only `fo:language` and `fo:country`
/// change; every other property of that style is the file's own. A file with no default cell
/// style gets the generated one through [`envelope::merge`] instead, so there is nothing here
/// for it to do.
fn patch_locale(bytes: Vec<u8>, doc: &Document, source: Option<&super::source::Source>) -> Vec<u8> {
    let Some(source) = source else { return bytes };
    if source.locale == doc.locale {
        return bytes;
    }
    let Some(range) = envelope::default_style_range(&bytes, "table-cell") else {
        return bytes;
    };
    let Some(fo) = envelope::prefix_for(&bytes, FO) else {
        return bytes;
    };
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return bytes;
    };
    let style = &text[range.clone()];
    let language = format!("{fo}:language");
    let country = format!("{fo}:country");
    let (lang, ctry) = match &doc.locale {
        Some(locale) => (
            Some(locale.language.as_str()),
            (!locale.country.is_empty()).then_some(locale.country.as_str()),
        ),
        None => (None, None),
    };
    let changes = [(language.as_str(), lang), (country.as_str(), ctry)];
    let patched = match style.find(":text-properties") {
        Some(at) => {
            let tag_start = style[..at].rfind('<').unwrap_or(at);
            format!(
                "{}{}",
                &style[..tag_start],
                envelope::set_attributes(&style[tag_start..], &changes)
            )
        }
        // A default style with no text properties of its own: they go in before its end tag.
        None => match style.rfind("</") {
            Some(close) => {
                let prefix = envelope::prefix_for(&bytes, STYLE).unwrap_or_else(|| "style".into());
                format!(
                    "{}{}{}",
                    &style[..close],
                    envelope::set_attributes(&format!("<{prefix}:text-properties/>"), &changes),
                    &style[close..]
                )
            }
            None => return bytes,
        },
    };
    [&text[..range.start], patched.as_str(), &text[range.end..]]
        .concat()
        .into_bytes()
}

/// The file this document was read from, with the edited cells put back in place.
///
/// `None` means "not applicable, regenerate" — never "failed". Every condition below is a
/// documented boundary of the trick rather than an error, and `odf::source` says why each
/// one is where it is.
fn splice(doc: &Document, form: Form) -> Option<Vec<u8>> {
    let source = doc.source.as_deref()?;
    // Saving as the other form is a conversion, not an edit.
    if source.form != form || !doc.edits.only_values {
        return None;
    }
    // An edit a filter judges regenerates: it can change which rows are hidden, and
    // `table:visibility` lives on the row rather than in the cell being spliced.
    if doc.edits.cells.iter().any(|(i, pos)| {
        doc.sheet(*i)
            .and_then(Sheet::filter)
            .is_some_and(|f| f.affects(*pos))
    }) {
        return None;
    }

    // Which elements have to be rewritten. Every edited cell must sit in one the file
    // actually spelled — one that does not means regenerating, because a document half in
    // its original bytes and half not would lose the other half silently.
    //
    // Keyed by element rather than by cell: several edited cells can share one repeated
    // element, and rewriting it once from the sheet covers all of them.
    let mut targets: BTreeMap<usize, (usize, u32, &super::source::Cell)> = BTreeMap::new();
    for (i, pos) in &doc.edits.cells {
        let at = source.covering(*i, pos.row, pos.col)?;
        doc.sheet(*i)?;
        targets.insert(at.range.start, (*i, pos.row, at));
    }

    // In file order, so the untouched stretches between are copied without seeking back.
    // Elements do not overlap by construction — they are siblings — but a corrupted span
    // would produce tangled bytes rather than an error, so refuse instead of trusting it.
    let mut patches = Vec::with_capacity(targets.len());
    for (i, row, at) in targets.into_values() {
        let sheet = doc.sheet(i)?;
        patches.push((at.range.clone(), rewrite(sheet, row, at, doc.null_date)));
    }
    if patches.windows(2).any(|w| w[0].0.end > w[1].0.start) {
        return None;
    }

    let mut out = Vec::with_capacity(source.bytes.len());
    let mut at = 0usize;
    for (range, text) in patches {
        out.extend_from_slice(source.bytes.get(at..range.start)?);
        out.extend_from_slice(text.as_bytes());
        at = range.end;
    }
    out.extend_from_slice(source.bytes.get(at..)?);
    Some(out)
}

/// One source element, re-emitted from the sheet's current contents.
///
/// Usually one cell in and one cell out. The interesting case is the repeated element: a
/// `table:number-columns-repeated="5"` covering five empty cells, with a value now written
/// into the middle one, comes back as *three* elements — the run before, the changed cell,
/// the run after — which is still a one-line diff, and is what keeps R6 true for the
/// overwhelmingly common case of writing into a cell that had no value.
///
/// Runs are re-formed by looking at the sheet, so a value written into a repeated run and
/// then cleared again collapses the run back. A formula never joins a run: it is
/// position-dependent, and repeating one would move it.
fn rewrite(sheet: &Sheet, row: u32, at: &super::source::Cell, null_date: i64) -> String {
    let mut out = String::new();
    let mut col = at.cols.start;
    while col < at.cols.end {
        let pos = Pos::new(row, col);
        let (value, formula, kind) = (sheet.get(pos), sheet.formula(pos), sheet.kind(pos));
        let repeat = match formula.is_some() {
            true => 1,
            false => (col..at.cols.end)
                .take_while(|c| {
                    let p = Pos::new(row, *c);
                    sheet.get(p) == value && sheet.formula(p).is_none() && sheet.kind(p) == kind
                })
                .count() as u32,
        };
        cell(
            &mut out,
            &value,
            formula,
            kind,
            (effective(sheet, pos), sheet.style(pos), None),
            null_date,
            repeat,
            // The element's own unmanaged attributes, verbatim — its style name, its merge
            // spans. They applied to every column it covered, so every piece it splits into
            // keeps them.
            &at.keep,
        );
        col += repeat.max(1);
    }
    out
}

/// The `content.xml` payload, which in the flat form is the whole document (§7.1–7.3) — and,
/// in the package form, the charts' own documents, which live beside it there
/// ([`Objects`]).
fn content(
    doc: &Document,
    form: Form,
    taken: &HashSet<String>,
    origin: Option<&super::source::Source>,
) -> (String, Vec<SubDocument>) {
    let root = match form {
        Form::Package => "office:document-content",
        // The flat form is one XML document; the projection never reaches here, because
        // `write` dispatches it before there is any XML to name a root of.
        Form::Flat | Form::Projection => "office:document",
    };

    // Written for the file the document came from, the pool holds only what this save writes
    // itself (everything else is the file's own bytes), under names the file does not use.
    let plans = origin.map(|source| plan(doc, source));
    let scopes: Option<Vec<Scope>> = plans
        .as_ref()
        .map(|plans| plans.iter().map(|plan| plan.scope.clone_scope()).collect());
    let mut pool = Pool::scoped(doc, scopes.as_deref());
    if let Some(source) = origin {
        pool.avoid(&envelope::automatic_style_names(&source.bytes));
        pool.adopt_bases(&source.bytes);
    }
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    // Declare only the namespaces this part actually uses (§1.4). `table:formula`'s `of:`
    // prefix is part of the *string*, not a namespace-resolved name (§4), so no `xmlns:of`.
    let _ = write!(
        out,
        "<{root} xmlns:office=\"{OFFICE}\" xmlns:table=\"{TABLE}\" xmlns:text=\"{TEXT}\""
    );
    // The style namespaces appear only in a document that has styles (§1.4), and the two
    // that only a *cell* style uses only when one does. A chart's own `style:style
    // style:family="chart"` needs the same prefix even when no cell in the sheet is styled at
    // all, or `style:` would go undeclared.
    // The flat form's `office:styles` holds the document's locale, on the default cell style,
    // which needs both `style:` and `fo:` even in a document with no cell styled at all.
    let common = (form == Form::Flat).then(|| common_styles(doc)).flatten();
    if !pool.is_empty() || doc.sheets.iter().any(|s| !s.charts().is_empty()) || common.is_some() {
        let _ = write!(out, " xmlns:style=\"{STYLE}\"");
    }
    if pool.styles_cells() {
        let _ = write!(out, " xmlns:number=\"{NUMBER}\" xmlns:fo=\"{FO}\"");
    } else if common.is_some() {
        let _ = write!(out, " xmlns:fo=\"{FO}\"");
    }
    // A chart's own document needs all four — `doc/chart-format.md`'s embedded shape — and
    // only a document that actually has a chart pays for the declarations.
    if doc.sheets.iter().any(|s| !s.charts().is_empty()) {
        let _ = write!(
            out,
            " xmlns:draw=\"{DRAW}\" xmlns:chart=\"{CHART}\" xmlns:svg=\"{SVG}\" xmlns:xlink=\"{XLINK}\""
        );
    }
    let _ = write!(out, " office:version=\"{VERSION}\"");
    if form == Form::Flat {
        let _ = write!(out, " office:mimetype=\"{MIMETYPE}\"");
    }
    out.push_str(">\n");
    // `office:styles` before `office:automatic-styles`, which is the schema's order.
    if let Some(common) = &common {
        out.push_str(common);
    }
    // Before the body, which is where the schema puts it.
    if !pool.is_empty() {
        pool.write(&mut out);
    }
    out.push_str(" <office:body>\n  <office:spreadsheet>\n");
    let mut objects = Objects::new(form, taken.clone());
    match (origin, &plans) {
        (Some(source), Some(plans)) => {
            // The file's own content follows its own start tag, whitespace and all.
            out.pop();
            out.push_str(&origin_body(doc, source, plans, &pool, &mut objects));
            out.push('\n');
        }
        _ => {
            out.push_str(&calculation_settings(doc));
            for sheet in &doc.sheets {
                table(
                    &mut out,
                    sheet,
                    doc.null_date,
                    doc.locale.as_ref(),
                    &pool,
                    &mut objects,
                );
            }
            out.push_str(&named_expressions(doc));
            // §9.4, and `table-database-ranges` is `table-functions`' second member — so after
            // the names, for the same schema reason. One range per sheet: [`Sheet::filter`].
            database_ranges(&mut out, doc);
        }
    }
    let _ = write!(out, "  </office:spreadsheet>\n </office:body>\n</{root}>\n");
    (out, objects.documents)
}

/// `table:calculation-settings` for the epoch, and only when it is not the default — writing
/// the default would be correct but LibreOffice omits it, and matching that keeps our output
/// diffable against a file it wrote. First in the body, which is where the schema puts it.
fn calculation_settings(doc: &Document) -> String {
    if doc.null_date == date::DEFAULT_NULL_DATE && doc.null_year == date::DEFAULT_NULL_YEAR {
        return String::new();
    }
    let year = match doc.null_year != date::DEFAULT_NULL_YEAR {
        true => format!(" table:null-year=\"{}\"", doc.null_year),
        false => String::new(),
    };
    format!(
        "   <table:calculation-settings{year}><table:null-date table:value-type=\"date\" \
         table:date-value=\"{}\"/></table:calculation-settings>\n",
        date::format_date(0.0, doc.null_date)
    )
}

/// §5.11, and *after* the tables: the schema's `office-spreadsheet-content-epilogue` (line 8263)
/// is where `table-functions` sits, and `table:named-expressions` is its first member.
/// LibreOffice reads them in either position, so loop C cannot see this — only the RELAX NG
/// schema can, which is what `kb.rs` validates against. Every name is written as
/// `table:named-expression`, since the reader stores a named range as the reference it stands
/// for and the two forms are interchangeable on the way out.
fn named_expressions(doc: &Document) -> String {
    if doc.names.is_empty() {
        return String::new();
    }
    let mut out = String::from("   <table:named-expressions>\n");
    for (name, expression) in &doc.names {
        let _ = writeln!(
            out,
            "    <table:named-expression table:name=\"{}\" table:expression=\"{}\"/>",
            esc(name),
            esc(expression)
        );
    }
    out.push_str("   </table:named-expressions>\n");
    out
}

// --- writing a sheet back into the table it was read from -----------------------------------
//
// A regenerating save of a document read from a file is a splice over the file's own
// `office:spreadsheet`: every table nobody touched is its own bytes, and a table somebody did is
// its own bytes with patches — the rows that changed, the columns if a width did, the charts if
// one did, the start tag if the sheet was renamed. Everything the model does not read (a merge,
// an annotation, a validation, a header-row group, a vendor's attribute) lives in the bytes that
// are not patched, which is how saving never makes the file worse. What a patch cannot carry is
// caught by `losses` and refused.

/// What a save does with one row element of the file.
enum RowFix {
    /// Its own bytes.
    Keep,
    /// Its own bytes, split around the rows it stands for that changed — a repeated element is
    /// split the way a repeated cell is, into copies of itself before and after and one patched
    /// copy per changed row — each changed row with these cells rewritten and, maybe, a new
    /// start tag.
    Patch(BTreeMap<u32, RowPatch>),
    /// Written again from the model: a run of rows whose visibility the filter now splits in
    /// ways too many to copy, which the guard then judges.
    Rewrite,
}

/// One changed row of a row element.
#[derive(Default)]
struct RowPatch {
    /// Columns whose cells were written.
    cells: Vec<u32>,
    /// Whether its height was set.
    height: bool,
    /// Its `table:visibility` when that moved — `Some(None)` for visible again.
    visibility: Option<Option<&'static str>>,
}

/// What a save writes of one sheet.
struct Plan {
    /// The `source::Table` it splices into — `None` for a sheet the file did not have.
    table: Option<usize>,
    /// One per row element of that table, in order.
    rows: Vec<RowFix>,
    /// Rows past everything the file's rows covered, written from the model.
    append: Option<std::ops::Range<u32>>,
    scope: Scope,
}

impl Scope {
    fn clone_scope(&self) -> Scope {
        Scope {
            all: self.all,
            cells: self.cells.clone(),
            rows: self.rows.clone(),
            heights: self.heights.clone(),
            columns: self.columns,
            bases: self.bases.clone(),
        }
    }
}

/// The value of one attribute of a start tag as the file spelled it, by qualified name.
fn tag_attr<'t>(tag: &'t str, name: &str) -> Option<&'t str> {
    let at = tag.find(&format!(" {name}="))? + name.len() + 2;
    let quote = tag[at..].chars().next()?;
    let rest = &tag[at + 1..];
    Some(&rest[..rest.find(quote)?])
}

/// The value of the first attribute whose qualified name ends with `suffix` (`:number-columns-
/// repeated`), whatever prefix the file spelled it with.
fn suffix_attr<'t>(tag: &'t str, suffix: &str) -> Option<&'t str> {
    let at = tag.find(&format!("{suffix}="))? + suffix.len() + 1;
    let quote = tag[at..].chars().next()?;
    let rest = &tag[at + 1..];
    Some(&rest[..rest.find(quote)?])
}

/// The prefix the file binds the table namespace to — `table` in every file LibreOffice wrote.
fn table_prefix(source: &super::source::Source) -> String {
    envelope::prefix_for(&source.bytes, TABLE).unwrap_or_else(|| "table".to_owned())
}

fn plan(doc: &Document, source: &super::source::Source) -> Vec<Plan> {
    let tp = table_prefix(source);
    doc.sheets
        .iter()
        .map(|sheet| {
            let p = &sheet.origin;
            let Some((ti, t)) = p
                .table
                .and_then(|ti| source.tables.get(ti).map(|t| (ti, t)))
            else {
                return Plan {
                    table: None,
                    rows: Vec::new(),
                    append: None,
                    scope: Scope {
                        all: true,
                        ..Default::default()
                    },
                };
            };
            let visibility = Visibility::of(sheet, doc.null_date, doc.locale.as_ref());
            let mut scope = Scope {
                columns: p.columns,
                ..Default::default()
            };
            let extent = cols_or_rows_extent(sheet, doc.null_date, doc.locale.as_ref());
            let mut rows = Vec::with_capacity(t.rows.len());
            let mut covered = 0u32;
            for e in &t.rows {
                let span = e.first..e.first.saturating_add(e.repeat);
                covered = covered.max(span.end);
                let tag = std::str::from_utf8(&source.bytes[e.start.clone()]).unwrap_or("");
                let mut patches: BTreeMap<u32, RowPatch> = BTreeMap::new();
                let mut scanned: Option<Vec<super::source::Cell>> = None;
                for pos in p.cells.iter().filter(|pos| span.contains(&pos.row)) {
                    patches.entry(pos.row).or_default().cells.push(pos.col);
                    if p.looks.contains(pos) {
                        scope.cells.insert(*pos);
                        let cells = scanned.get_or_insert_with(|| scan_cells(&source.bytes, e).0);
                        let own = cells
                            .iter()
                            .find(|c| c.cols.contains(&pos.col))
                            .and_then(|c| tag_attr(&c.keep, &format!("{tp}:style-name")));
                        if let Some(base) = own.or_else(|| t.column_default(pos.col)) {
                            scope.bases.insert(*pos, unescape(base));
                        }
                    }
                }
                for row in p.rows.iter().filter(|row| span.contains(row)) {
                    patches.entry(*row).or_default().height = true;
                    scope.heights.insert(*row);
                }
                // Visibility: whenever the filter or a hand-hidden row may have moved it, every
                // row whose wanted value is not what this element says.
                if p.filter || p.rows.iter().any(|row| span.contains(row)) {
                    let said = tag_attr(tag, &format!("{tp}:visibility"));
                    let hidden_here = visibility
                        .manual
                        .range(span.clone())
                        .chain(visibility.filtered.range(span.clone()))
                        .copied()
                        .collect::<BTreeSet<u32>>();
                    let candidates: Vec<u32> = match said {
                        // Visible as written: only the rows now hidden can differ.
                        None => hidden_here.into_iter().collect(),
                        // Hidden as written: every row of it may differ, which for a long run
                        // is more copies than the element is worth.
                        Some(_) if span.len() <= 10_000 => span.clone().collect(),
                        Some(_) => {
                            rows.push(RowFix::Rewrite);
                            scope
                                .rows
                                .push(span.start..span.end.min(extent.max(span.start + 1)));
                            continue;
                        }
                    };
                    for row in candidates {
                        let wanted = visibility.value(row);
                        if wanted != said {
                            patches.entry(row).or_default().visibility = Some(wanted);
                        }
                    }
                }
                // A cell past what the file's row spells is written from the model, so the
                // pool needs its look whatever touched it.
                rows.push(match patches.is_empty() {
                    true => RowFix::Keep,
                    false => RowFix::Patch(patches),
                });
            }
            let append = (extent > covered).then_some(covered..extent);
            if let Some(range) = &append {
                scope.rows.push(range.clone());
            }
            Plan {
                table: Some(ti),
                rows,
                append,
                scope,
            }
        })
        .collect()
}

/// `bytes[range]` with `patches` (absolute ranges into `bytes`, sorted, not overlapping; an
/// empty range is an insertion) applied.
fn apply(
    bytes: &[u8],
    range: std::ops::Range<usize>,
    mut patches: Vec<(std::ops::Range<usize>, String)>,
) -> String {
    patches.sort_by_key(|(r, _)| (r.start, r.end));
    let mut out = String::with_capacity(range.len());
    let mut at = range.start;
    for (r, text) in patches {
        if r.start < at {
            continue;
        }
        out.push_str(&String::from_utf8_lossy(&bytes[at..r.start]));
        out.push_str(&text);
        at = r.end;
    }
    out.push_str(&String::from_utf8_lossy(&bytes[at..range.end]));
    out
}

/// The range of an element together with the whitespace in front of it, for removing it
/// without leaving its line behind.
fn with_leading_space(bytes: &[u8], range: std::ops::Range<usize>) -> std::ops::Range<usize> {
    let start = bytes[..range.start]
        .iter()
        .rposition(|c| !c.is_ascii_whitespace())
        .map_or(range.start, |p| p + 1);
    start..range.end
}

/// The whitespace in front of the element at `start`, back to its line's start — the file's
/// own indentation, which a new sibling written beside it borrows.
fn indent_of(bytes: &[u8], start: usize) -> String {
    let line = bytes[..start]
        .iter()
        .rposition(|c| *c == b'\n')
        .map_or(0, |p| p + 1);
    let lead = &bytes[line..start];
    match lead.iter().all(u8::is_ascii_whitespace) {
        true => String::from_utf8_lossy(lead).into_owned(),
        false => String::new(),
    }
}

/// Text this writer produced for a block-level element, re-indented to sit after `after` in the
/// file: the writer's own leading spaces dropped, the file's indentation in their place.
fn beside(bytes: &[u8], after: usize, text: &str) -> String {
    let indent = indent_of(bytes, after);
    let mut out = String::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        out.push('\n');
        out.push_str(&indent);
        out.push_str(line.trim_start());
    }
    out
}

fn origin_body(
    doc: &Document,
    source: &super::source::Source,
    plans: &[Plan],
    pool: &Pool,
    objects: &mut Objects,
) -> String {
    let Some(inner) = source.body.clone() else {
        return String::new();
    };
    let bytes = &source.bytes;
    let mut patches: Vec<(std::ops::Range<usize>, String)> = Vec::new();

    // Tables nobody has any more.
    let kept: HashSet<usize> = plans.iter().filter_map(|plan| plan.table).collect();
    for (ti, t) in source.tables.iter().enumerate() {
        if !kept.contains(&ti) {
            patches.push((with_leading_space(bytes, t.range.clone()), String::new()));
        }
    }

    // The sheets, in the model's order: each spliced back into its table, or — a sheet the
    // file did not have — written after the one before it.
    let first_table = source.tables.first().map(|t| t.range.start);
    let epilogue = source
        .parts
        .iter()
        .find(|part| part.after_tables)
        .map(|part| part.range.start);
    let mut cursor = first_table.or(epilogue).unwrap_or(inner.end);
    let mut cursor_is_end = false;
    let anchor = source
        .tables
        .first()
        .map(|t| t.range.start)
        .unwrap_or(cursor);
    for (plan, sheet) in plans.iter().zip(&doc.sheets) {
        match plan.table {
            Some(ti) => {
                let t = &source.tables[ti];
                let text = patched_table(doc, sheet, t, plan, source, pool, objects);
                patches.push((t.range.clone(), text));
                cursor = t.range.end;
                cursor_is_end = true;
            }
            None => {
                let mut text = String::new();
                table(
                    &mut text,
                    sheet,
                    doc.null_date,
                    doc.locale.as_ref(),
                    pool,
                    objects,
                );
                let placed = beside(bytes, anchor, &text);
                // After a table: a line of its own following it. Before the first: ahead of
                // it, on a line of its own, and the newline goes after.
                let placed = match cursor_is_end {
                    true => placed,
                    false => format!(
                        "{}\n{}",
                        placed.trim_start_matches('\n'),
                        indent_of(bytes, cursor)
                    ),
                };
                patches.push((cursor..cursor, placed));
            }
        }
    }
    let tables_end = source.tables.last().map(|t| t.range.end);

    // The epoch: the file's own `table:calculation-settings` unless the epoch moved.
    let part = |kind| source.parts.iter().find(|part| part.kind == kind);
    use super::source::PartKind;
    if (doc.null_date, doc.null_year) != source.epoch {
        let ours = calculation_settings(doc);
        match part(PartKind::CalculationSettings) {
            Some(existing) => patches.push((existing.range.clone(), ours.trim().to_owned())),
            None if !ours.is_empty() => {
                let at = first_table.unwrap_or(inner.end);
                patches.push((at..at, format!("{}\n{}", ours.trim(), indent_of(bytes, at))));
            }
            None => {}
        }
    }

    // Names: the file's own, globally and per table, unless somebody changed one — then the
    // model's, all of them, in one place.
    let after_tables = tables_end.unwrap_or(inner.end);
    if doc.edits.names {
        let ours = named_expressions(doc);
        match part(PartKind::NamedExpressions) {
            Some(existing) if ours.is_empty() => patches.push((
                with_leading_space(bytes, existing.range.clone()),
                String::new(),
            )),
            Some(existing) => patches.push((existing.range.clone(), ours.trim().to_owned())),
            None if !ours.is_empty() => {
                patches.push((after_tables..after_tables, beside(bytes, anchor, &ours)))
            }
            None => {}
        }
    }

    // Filters: the file's own database ranges unless a filter changed.
    if doc.sheets.iter().any(|sheet| sheet.origin.filter) {
        let mut ours = String::new();
        database_ranges(&mut ours, doc);
        match part(PartKind::DatabaseRanges) {
            Some(existing) if ours.is_empty() => patches.push((
                with_leading_space(bytes, existing.range.clone()),
                String::new(),
            )),
            Some(existing) => patches.push((existing.range.clone(), ours.trim().to_owned())),
            None if !ours.is_empty() => {
                // After the names, wherever they are.
                let at = part(PartKind::NamedExpressions).map_or(after_tables, |p| p.range.end);
                patches.push((at..at, beside(bytes, anchor, &ours)))
            }
            None => {}
        }
    }

    let text = apply(bytes, inner.clone(), patches);
    text.trim_end().to_owned()
}

#[allow(clippy::too_many_arguments)]
fn patched_table(
    doc: &Document,
    sheet: &Sheet,
    t: &super::source::Table,
    plan: &Plan,
    source: &super::source::Source,
    pool: &Pool,
    objects: &mut Objects,
) -> String {
    let bytes = &source.bytes;
    let tp = table_prefix(source);
    let mut patches: Vec<(std::ops::Range<usize>, String)> = Vec::new();
    let tag = std::str::from_utf8(&bytes[t.start.clone()]).unwrap_or("");

    // Renamed: the start tag again, its other attributes the file's own.
    if tag_attr(tag, &format!("{tp}:name")).map(unescape) != Some(sheet.name.clone()) {
        let qname = tag[1..]
            .split(|c: char| c.is_whitespace() || c == '>' || c == '/')
            .next()
            .unwrap_or("table:table");
        patches.push((
            t.start.clone(),
            format!("<{qname} {tp}:name=\"{}\"{}>", esc(&sheet.name), t.keep),
        ));
    }

    // Charts: the file's own unless one changed.
    if sheet.origin.charts {
        let mut ours = String::new();
        write_shapes(&mut ours, sheet, objects);
        match &t.shapes {
            Some(range) => patches.push((range.clone(), ours.trim().to_owned())),
            None if !ours.is_empty() => {
                let at = t.start.end;
                let first = t.columns.first().map_or(at, |r| r.start);
                patches.push((at..at, beside(bytes, first, &ours)));
            }
            None => {}
        }
    }

    // Columns: the file's own unless a width or a hidden flag changed.
    if sheet.origin.columns {
        let mut ours = String::new();
        write_columns(&mut ours, sheet, pool, Some(t));
        match (t.columns.first(), t.columns.last()) {
            (Some(first), Some(last)) => {
                let block = beside(bytes, first.start, &ours);
                patches.push((first.start..last.end, block.trim_start().to_owned()));
            }
            _ => {
                let at = t.shapes.as_ref().map_or(t.start.end, |r| r.end);
                let next = t.rows.first().map_or(at, |r| r.range.start);
                patches.push((at..at, beside(bytes, next, &ours)));
            }
        }
    }

    // Rows.
    let visibility = Visibility::of(sheet, doc.null_date, doc.locale.as_ref());
    for (e, fix) in t.rows.iter().zip(&plan.rows) {
        match fix {
            RowFix::Keep => {}
            RowFix::Patch(changed) => {
                let text = split_row(sheet, doc, e, changed, source, pool, &tp);
                patches.push((e.range.clone(), text));
            }
            RowFix::Rewrite => {
                let extent = cols_or_rows_extent(sheet, doc.null_date, doc.locale.as_ref());
                let span = e.first..e.first.saturating_add(e.repeat);
                let written = span.start..span.end.min(extent.max(span.start));
                let mut ours = String::new();
                write_rows(
                    &mut ours,
                    sheet,
                    written.clone(),
                    doc.null_date,
                    pool,
                    &visibility,
                );
                // What the element covered past the model's last row stays covered — empty.
                if span.end > written.end && written.end < MAX_ROWS {
                    let _ = writeln!(
                        ours,
                        "    <table:table-row{}><table:table-cell/></table:table-row>",
                        count(span.end - written.end, "rows")
                    );
                }
                if ours.is_empty() {
                    ours.push_str("    <table:table-row><table:table-cell/></table:table-row>\n");
                }
                let block = beside(bytes, e.start.start, &ours);
                patches.push((e.range.clone(), block.trim_start().to_owned()));
            }
        }
    }
    if let Some(range) = &plan.append {
        let mut ours = String::new();
        write_rows(
            &mut ours,
            sheet,
            range.clone(),
            doc.null_date,
            pool,
            &visibility,
        );
        let at = t
            .rows
            .last()
            .map(|r| r.range.end)
            .or(t.columns.last().map(|r| r.end))
            .unwrap_or(t.start.end);
        let like = t.rows.last().map_or(at, |r| r.range.start);
        patches.push((at..at, beside(bytes, like, &ours)));
    }

    // Sheet-local names go when the names were rewritten globally.
    if doc.edits.names
        && let Some((range, _)) = &t.names
    {
        patches.push((with_leading_space(bytes, range.clone()), String::new()));
    }

    apply(bytes, t.range.clone(), patches)
}

/// One row element of the file, split around its changed rows: copies of the element for the
/// untouched runs (its `table:number-rows-repeated` set to each run's length), and one copy per
/// changed row with that row's start tag and cells patched. An element standing for one row is
/// the degenerate case — one patched copy.
#[allow(clippy::too_many_arguments)]
fn split_row(
    sheet: &Sheet,
    doc: &Document,
    e: &super::source::RowElement,
    changed: &BTreeMap<u32, RowPatch>,
    source: &super::source::Source,
    pool: &Pool,
    tp: &str,
) -> String {
    let bytes = &source.bytes;
    let tag = String::from_utf8_lossy(&bytes[e.start.clone()]).into_owned();
    let repeated = format!("{tp}:number-rows-repeated");
    let indent = indent_of(bytes, e.range.start);
    // The element as written, standing for `n` rows.
    let copy = |n: u32| {
        let count = (n > 1).then(|| n.to_string());
        let new_tag = envelope::set_attributes(&tag, &[(repeated.as_str(), count.as_deref())]);
        let mut out = new_tag;
        out.push_str(&String::from_utf8_lossy(&bytes[e.start.end..e.range.end]));
        out
    };
    // Its cells, recorded by the reader for a row that stood for itself and read out of the
    // element's own bytes otherwise.
    let (cells, width) = scan_all_cells(bytes, e);
    let span_end = e.first.saturating_add(e.repeat);
    let visibility = Visibility::of(sheet, doc.null_date, doc.locale.as_ref());

    let mut pieces: Vec<String> = Vec::new();
    let mut cursor = e.first;
    for (row, patch) in changed {
        if *row > cursor {
            pieces.push(copy(row - cursor));
        }
        let patched = patched_row(
            sheet, doc, e, *row, patch, &tag, &cells, width, source, pool, tp,
        );
        pieces.push(match patched {
            Some(text) => text,
            // A written column the file's row cannot place — inside a merge's covered half —
            // and so the row from the model, whose losses the guard then names.
            None => {
                let mut ours = String::new();
                write_rows(
                    &mut ours,
                    sheet,
                    *row..row + 1,
                    doc.null_date,
                    pool,
                    &visibility,
                );
                ours.trim().to_owned()
            }
        });
        cursor = row + 1;
    }
    if cursor < span_end {
        pieces.push(copy(span_end - cursor));
    }
    pieces.join(&format!("\n{indent}"))
}

/// One row of a row element with its start tag and cells patched.
#[allow(clippy::too_many_arguments)]
fn patched_row(
    sheet: &Sheet,
    doc: &Document,
    e: &super::source::RowElement,
    row: u32,
    patch: &RowPatch,
    tag: &str,
    cells: &[Scanned],
    width: u32,
    source: &super::source::Source,
    pool: &Pool,
    tp: &str,
) -> Option<String> {
    let bytes = &source.bytes;
    let repeated = format!("{tp}:number-rows-repeated");
    let mut changes: Vec<(String, Option<String>)> = vec![(repeated, None)];
    if patch.height {
        let style = pool.row_attr(sheet.row_height(row));
        changes.push((
            format!("{tp}:style-name"),
            tag_attr(&style, "table:style-name").map(str::to_owned),
        ));
    }
    if let Some(wanted) = patch.visibility {
        changes.push((format!("{tp}:visibility"), wanted.map(str::to_owned)));
    }
    let refs: Vec<(&str, Option<&str>)> = changes
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_deref()))
        .collect();
    let new_tag = envelope::set_attributes(tag, &refs);

    // The cells: a restyled cell's own element with a new style name, a rewritten cell from the
    // model, everything else the file's bytes.
    let mut inside: Vec<(std::ops::Range<usize>, String)> = Vec::new();
    // The touched columns, by the element holding them.
    let mut by_element: BTreeMap<usize, Vec<u32>> = BTreeMap::new();
    for col in patch.cells.iter().filter(|col| **col < width) {
        let index = cells.iter().position(|s| s.cell.cols.contains(col))?;
        by_element.entry(index).or_default().push(*col);
    }
    for (index, cols) in by_element {
        let scanned = &cells[index];
        let element = &scanned.cell;
        let restyled_only = cols
            .iter()
            .all(|col| !sheet.origin.values.contains(&Pos::new(row, *col)));
        if restyled_only {
            // Only looks changed: the element's own bytes, split around the restyled columns
            // with the pool's style name on each — whatever it holds stays, a merge's covered
            // half included.
            inside.push((
                element.range.clone(),
                restyle(sheet, row, element, &cols, bytes, pool, tp),
            ));
            continue;
        }
        // A value in the covered half of a merge has nowhere to go but out of the merge: the row
        // from the model, whose losses the guard then names.
        if scanned.covered {
            return None;
        }
        inside.push((
            element.range.clone(),
            rewrite_cells(sheet, row, element, doc.null_date, pool),
        ));
    }
    // Columns past the last cell the file's row spells: an empty run up to the first of them,
    // then each from the model.
    let beyond: Vec<u32> = patch
        .cells
        .iter()
        .copied()
        .filter(|c| *c >= width)
        .collect();
    if let Some(last) = beyond.iter().max() {
        let mut ours = String::new();
        let mut col = width;
        while col <= *last {
            let pos = Pos::new(row, col);
            let look = pool.look(sheet, pos);
            if !carries(sheet, pos) {
                let run = (col..=*last)
                    .take_while(|c| !carries(sheet, Pos::new(row, *c)))
                    .count() as u32;
                let _ = write!(ours, "<table:table-cell{}/>", count(run, "columns"));
                col += run;
                continue;
            }
            let attr = pool.attr(look);
            cell(
                &mut ours,
                &sheet.get(pos),
                sheet.formula(pos),
                sheet.kind(pos),
                look,
                doc.null_date,
                1,
                &attr,
            );
            col += 1;
        }
        let close = bytes[e.range.clone()]
            .iter()
            .rposition(|c| *c == b'<')
            .map_or(e.range.end, |p| e.range.start + p);
        inside.push((close..close, ours));
    }
    let body = apply(bytes, e.start.end..e.range.end, inside);
    Some(format!("{new_tag}{body}"))
}

/// A cell element whose columns `restyled` only had their look changed: copies of the element
/// for the runs between them (`table:number-columns-repeated` set to each run's length) and a
/// copy per restyled column carrying the pool's style for it.
fn restyle(
    sheet: &Sheet,
    row: u32,
    element: &super::source::Cell,
    restyled: &[u32],
    bytes: &[u8],
    pool: &Pool,
    tp: &str,
) -> String {
    let tag_end = bytes[element.range.clone()]
        .iter()
        .position(|c| *c == b'>')
        .map_or(element.range.end, |p| element.range.start + p + 1);
    let tag = String::from_utf8_lossy(&bytes[element.range.start..tag_end]).into_owned();
    let rest = String::from_utf8_lossy(&bytes[tag_end..element.range.end]).into_owned();
    let repeated = format!("{tp}:number-columns-repeated");
    let styled = format!("{tp}:style-name");
    let copy = |n: u32, style: Option<Option<String>>| {
        let count = (n > 1).then(|| n.to_string());
        let mut changes: Vec<(&str, Option<&str>)> = vec![(repeated.as_str(), count.as_deref())];
        if let Some(style) = &style {
            changes.push((styled.as_str(), style.as_deref()));
        }
        format!("{}{rest}", envelope::set_attributes(&tag, &changes))
    };
    let mut out = String::new();
    let mut cursor = element.cols.start;
    let mut sorted = restyled.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    for col in sorted {
        if col > cursor {
            out.push_str(&copy(col - cursor, None));
        }
        let pos = Pos::new(row, col);
        let attr = pool.attr(pool.look(sheet, pos));
        out.push_str(&copy(
            1,
            Some(tag_attr(&attr, "table:style-name").map(str::to_owned)),
        ));
        cursor = col + 1;
    }
    if cursor < element.cols.end {
        out.push_str(&copy(element.cols.end - cursor, None));
    }
    out
}

/// The cell elements of a row element, read out of its own bytes — what the reader records only
/// for a row standing for itself (`source::Source::rows`).
/// One cell element of a row, read out of its bytes: [`super::source::Cell`] plus whether it is
/// the covered half of a merge — which holds a position and a style but no value of its own.
struct Scanned {
    cell: super::source::Cell,
    covered: bool,
}

fn scan_cells(bytes: &[u8], e: &super::source::RowElement) -> (Vec<super::source::Cell>, u32) {
    let (all, width) = scan_all_cells(bytes, e);
    (
        all.into_iter()
            .filter(|s| !s.covered)
            .map(|s| s.cell)
            .collect(),
        width,
    )
}

fn scan_all_cells(bytes: &[u8], e: &super::source::RowElement) -> (Vec<Scanned>, u32) {
    let mut cells = Vec::new();
    let end = bytes[e.range.clone()]
        .iter()
        .rposition(|c| *c == b'<')
        .map_or(e.range.end, |p| e.range.start + p);
    let mut at = e.start.end;
    let mut col = 0u32;
    while at < end {
        let Some(lt) = bytes[at..end].iter().position(|c| *c == b'<') else {
            break;
        };
        let start = at + lt;
        // The start tag, to its `>` outside any quoted value.
        let mut quote = None;
        let mut close = None;
        for (i, c) in bytes[start..end].iter().enumerate() {
            match (quote, *c) {
                (None, b'"' | b'\'') => quote = Some(*c),
                (Some(q), c) if c == q => quote = None,
                (None, b'>') => {
                    close = Some(start + i + 1);
                    break;
                }
                _ => {}
            }
        }
        let Some(tag_end) = close else { break };
        let tag = String::from_utf8_lossy(&bytes[start..tag_end]).into_owned();
        let qname = tag[1..]
            .split(|c: char| c.is_whitespace() || c == '>' || c == '/')
            .next()
            .unwrap_or("");
        let Some(range) = grind_core::odf::xml::element_extent(bytes, start..tag_end) else {
            break;
        };
        if qname.ends_with(":table-cell") || qname.ends_with(":covered-table-cell") {
            let repeat = suffix_attr(&tag, ":number-columns-repeated")
                .and_then(|n| n.trim().parse::<u32>().ok())
                .unwrap_or(1)
                .max(1);
            cells.push(Scanned {
                cell: super::source::Cell {
                    range: range.clone(),
                    cols: col..col.saturating_add(repeat),
                    keep: super::source::kept_attributes(tag.as_bytes()),
                },
                covered: qname.ends_with(":covered-table-cell"),
            });
            col = col.saturating_add(repeat);
        }
        at = range.end;
    }
    (cells, col)
}

/// XML's five entities, undone — enough to compare an attribute the file spelled with a name.
fn unescape(value: &str) -> String {
    value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// One cell element of the file, rewritten from the sheet — [`rewrite`] for a save that may
/// also have changed a cell's look: a cell whose format or style was written takes the pool's
/// style in place of the file's, and every other cell the element covers keeps the file's.
fn rewrite_cells(
    sheet: &Sheet,
    row: u32,
    at: &super::source::Cell,
    null_date: i64,
    pool: &Pool,
) -> String {
    let looks = &sheet.origin.looks;
    let restyled_keep =
        super::source::attributes(format!("<x{}>", at.keep).as_bytes(), &["table:style-name"]);
    let mut out = String::new();
    let mut col = at.cols.start;
    while col < at.cols.end {
        let pos = Pos::new(row, col);
        let (value, formula, kind) = (sheet.get(pos), sheet.formula(pos), sheet.kind(pos));
        let restyled = looks.contains(&pos);
        let look = pool.look(sheet, pos);
        let repeat = match formula.is_some() {
            true => 1,
            false => (col..at.cols.end)
                .take_while(|c| {
                    let p = Pos::new(row, *c);
                    sheet.get(p) == value
                        && sheet.formula(p).is_none()
                        && sheet.kind(p) == kind
                        && looks.contains(&p) == restyled
                        && (!restyled || pool.look(sheet, p) == look)
                })
                .count() as u32,
        };
        let attrs = match restyled {
            true => format!("{restyled_keep}{}", pool.attr(look)),
            false => at.keep.clone(),
        };
        cell(
            &mut out, &value, formula, kind, look, null_date, repeat, &attrs,
        );
        col += repeat.max(1);
    }
    out
}

/// `office:styles` — which this writer fills with exactly one thing, the document's locale on
/// the default cell style (`doc/ods-format.md` §5.2, "The document's own language"), and only
/// when the document has one (R3). `None` otherwise, so a document with no locale writes no
/// element for it at all.
fn common_styles(doc: &Document) -> Option<String> {
    let locale = doc.locale.as_ref()?;
    let country = match locale.country.is_empty() {
        true => String::new(),
        false => format!(" fo:country=\"{}\"", esc(&locale.country)),
    };
    Some(format!(
        " <office:styles><style:default-style style:family=\"table-cell\">\
         <style:text-properties fo:language=\"{}\"{country}/></style:default-style>\
         </office:styles>\n",
        esc(&locale.language)
    ))
}

/// The package form's `styles.xml`: [`common_styles`] in the part `office:styles` lives in, since
/// `content.xml`'s root does not allow one. `None` when there is nothing for it to hold.
fn common_styles_part(doc: &Document) -> Option<String> {
    let common = common_styles(doc)?;
    Some(format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document-styles \
         xmlns:office=\"{OFFICE}\" xmlns:style=\"{STYLE}\" xmlns:fo=\"{FO}\" \
         office:version=\"{VERSION}\">\n{common}</office:document-styles>\n"
    ))
}

/// `table:database-ranges` (§9.4): every sheet's autofilter, as a range plus one
/// set-of-values condition per filtered field.
///
/// `table:filter-condition` needs a `table:value` even when the set-items carry the values,
/// so it gets the first of them — which is also what LibreOffice writes.
fn database_ranges(out: &mut String, doc: &Document) {
    let filters: Vec<_> = doc
        .sheets
        .iter()
        .filter_map(|sheet| Some((sheet, sheet.filter()?)))
        .collect();
    if filters.is_empty() {
        return;
    }
    out.push_str("   <table:database-ranges>\n");
    for (sheet, filter) in filters {
        // The reference serialiser, minus its brackets: `table:target-range-address` is the
        // bracketless spelling of the same thing, dots and sheet-name quoting included, and
        // the display form is not it (it drops the second end's dot).
        let address = crate::a1::reference(Some(&sheet.name), filter.start, filter.end)
            .to_string()
            .trim_matches(['[', ']'])
            .to_owned();
        let _ = writeln!(
            out,
            "    <table:database-range table:name=\"{}\" table:target-range-address=\"{}\" \
             table:contains-header=\"{}\" table:display-filter-buttons=\"{}\">",
            esc(&filter.name),
            esc(&address),
            filter.contains_header,
            filter.buttons
        );
        if !filter.keep.is_empty() {
            out.push_str("     <table:filter>\n      <table:filter-and>\n");
            for (field, values) in &filter.keep {
                let first = values.iter().next().map(String::as_str).unwrap_or_default();
                let _ = writeln!(
                    out,
                    "       <table:filter-condition table:field-number=\"{field}\" \
                     table:operator=\"=\" table:value=\"{}\">",
                    esc(first)
                );
                for value in values {
                    let _ = writeln!(
                        out,
                        "        <table:filter-set-item table:value=\"{}\"/>",
                        esc(value)
                    );
                }
                out.push_str("       </table:filter-condition>\n");
            }
            out.push_str("      </table:filter-and>\n     </table:filter>\n");
        }
        out.push_str("    </table:database-range>\n");
    }
    out.push_str("   </table:database-ranges>\n");
}

/// The format a date cell gets when the document gives it none.
///
/// Not decoration: LibreOffice *requires* a date cell to display through a date style and
/// invents one from its own locale when a file omits it, so a document written without one
/// comes back with `M/D/YY` bolted on and the round trip is no longer an identity. Writing
/// the ISO spelling instead makes the file say what it means, in the one form that reads
/// the same in every locale (§3.4 Note 2).
static DATE_DEFAULT: LazyLock<Format> = LazyLock::new(|| numfmt::preset(Kind::Date, 0, false, ""));

/// A date that carries a time is a DateTime (§4.3.4) and needs a style that shows both,
/// since a format cannot look at the value it is given.
static DATETIME_DEFAULT: LazyLock<Format> = LazyLock::new(numfmt::datetime_preset);

/// The same for a time cell — a 24-hour clock, for the same reason.
static TIME_DEFAULT: LazyLock<Format> = LazyLock::new(|| numfmt::preset(Kind::Time, 0, false, ""));

/// The format a cell is actually written with: its own, or the default its value type
/// demands.
fn effective(sheet: &Sheet, pos: Pos) -> Option<&Format> {
    if let Some(format) = sheet.format(pos) {
        return Some(format);
    }
    match sheet.kind(pos) {
        Some(NumberKind::Date) => match sheet.get(pos) {
            CellValue::Number(n) if n.fract() != 0.0 => Some(&DATETIME_DEFAULT),
            _ => Some(&DATE_DEFAULT),
        },
        Some(NumberKind::Time) => Some(&TIME_DEFAULT),
        None => None,
    }
}

/// How one cell is written: its number format and its styling, which travel together on a
/// single `style:style` and are therefore pooled together (§5.3). A cell with neither gets
/// no `table:style-name` at all.
///
/// The third member is the cell's *base*, an index into [`Pool::bases`]: the style a restyled
/// cell had in the file, which its new style is built from so that what the model does not
/// read on it survives (`envelope::patch_style`). Part of the identity, since two cells that
/// now look the same but had different styles in the file carry different unread properties.
type Look<'a> = (Option<&'a Format>, Option<&'a CellStyle>, Option<usize>);

/// Every distinct format and every distinct look in the document, in first-seen order.
///
/// Two pools, because the file has two vocabularies: format *i* is a `number:*-style` named
/// `N{i}`, look *i* is a `table-cell` `style:style` named `ce{i}` that points at one and
/// carries the properties. Every cell that shares a look shares its name.
struct Pool<'a> {
    formats: Vec<&'a Format>,
    index: HashMap<&'a Format, usize>,
    looks: Vec<Look<'a>>,
    look_index: HashMap<Look<'a>, usize>,
    /// The distinct column widths and row heights (§5.4), pooled the same way and for the
    /// same reason: a track's size is reachable only through a `style:style` of family
    /// `table-column`/`table-row`, named `co{i}`/`ro{i}` here.
    cols: Vec<&'a str>,
    col_index: HashMap<&'a str, usize>,
    rows: Vec<&'a str>,
    row_index: HashMap<&'a str, usize>,
    /// The style names restyled cells had in the file (`Scope::bases`), each once, and by sheet
    /// name and position which of them a cell had.
    bases: Vec<String>,
    based: HashMap<&'a str, HashMap<Pos, usize>>,
    /// Each base's element, when it is one of the file's own automatic styles — what a new
    /// style is patched from. `None` for a named style, which a new one takes as its parent.
    base_elements: Vec<Option<String>>,
    /// Where each family's numbering starts — `ce`, `co`, `ro` and `N` — so that a pool written
    /// into a file that already declares `ce1`…`ce9` starts at `ce10` and never takes a name the
    /// file uses (`envelope::merge` keeps the file's own automatic styles beside these).
    offsets: [usize; 4],
}

/// What a regenerating save writes of one sheet, when it is spliced back into the file's own
/// table rather than written whole — the only cells and tracks the pool needs styles for.
#[derive(Default)]
struct Scope {
    /// The whole sheet: a sheet the file did not have.
    all: bool,
    /// Cells whose look this save writes.
    cells: BTreeSet<Pos>,
    /// Rows this save writes whole.
    rows: Vec<std::ops::Range<u32>>,
    /// Rows whose height this save writes on a start tag of the file's.
    heights: BTreeSet<u32>,
    /// Whether this save writes the column declarations.
    columns: bool,
    /// The style each restyled cell had in the file — its own `table:style-name`, or its
    /// column's default — which its new style is built from.
    bases: HashMap<Pos, String>,
}

impl<'a> Pool<'a> {
    /// The pool for exactly what a save writes: everything (`None`), or per sheet what
    /// [`Scope`] says.
    fn scoped(doc: &'a Document, scopes: Option<&[Scope]>) -> Self {
        let mut pool = Pool {
            formats: Vec::new(),
            index: HashMap::new(),
            looks: Vec::new(),
            look_index: HashMap::new(),
            cols: Vec::new(),
            col_index: HashMap::new(),
            rows: Vec::new(),
            row_index: HashMap::new(),
            bases: Vec::new(),
            based: HashMap::new(),
            base_elements: Vec::new(),
            offsets: [0; 4],
        };
        for (i, sheet) in doc.sheets.iter().enumerate() {
            let scope = scopes.and_then(|scopes| scopes.get(i));
            let all = scope.is_none_or(|scope| scope.all);
            let row_written = |row: u32| {
                all || scope.is_some_and(|scope| {
                    scope.heights.contains(&row) || scope.rows.iter().any(|r| r.contains(&row))
                })
            };
            let cell_written = |pos: Pos| {
                all || scope.is_some_and(|scope| {
                    scope.cells.contains(&pos) || scope.rows.iter().any(|r| r.contains(&pos.row))
                })
            };
            // `entry` rather than `insert`, which would overwrite the index of a size
            // already pooled and point its first tracks at a later style.
            if all || scope.is_some_and(|scope| scope.columns) {
                for (_, width) in sheet.col_widths() {
                    let next = pool.cols.len();
                    if *pool.col_index.entry(width).or_insert(next) == next {
                        pool.cols.push(width);
                    }
                }
            }
            for (row, height) in sheet.row_heights() {
                if !row_written(row) {
                    continue;
                }
                let next = pool.rows.len();
                if *pool.row_index.entry(height).or_insert(next) == next {
                    pool.rows.push(height);
                }
            }
            if let Some(scope) = scope {
                for (pos, base) in &scope.bases {
                    let at = match pool.bases.iter().position(|b| b == base) {
                        Some(at) => at,
                        None => {
                            pool.bases.push(base.clone());
                            pool.bases.len() - 1
                        }
                    };
                    pool.based
                        .entry(sheet.name.as_str())
                        .or_default()
                        .insert(*pos, at);
                }
            }
            let formatted = sheet.formats().map(|(pos, _)| pos);
            let dated = sheet.kinds().map(|(pos, _)| pos);
            let styled = sheet.styles().map(|(pos, _)| pos);
            for pos in formatted.chain(dated).chain(styled) {
                if !cell_written(pos) {
                    continue;
                }
                let look = pool.look(sheet, pos);
                if let Some(format) = look.0 {
                    pool.add(format);
                }
                if look.1.is_none() && look.0.is_none() {
                    continue;
                }
                if !pool.look_index.contains_key(&look) {
                    pool.look_index.insert(look, pool.looks.len());
                    pool.looks.push(look);
                }
            }
        }
        pool
    }

    /// How the cell at `pos` is written: its format, its styling, and the style it had in the
    /// file when this save restyled it.
    fn look(&self, sheet: &'a Sheet, pos: Pos) -> Look<'a> {
        let base = self
            .based
            .get(sheet.name.as_str())
            .and_then(|cells| cells.get(&pos))
            .copied();
        (effective(sheet, pos), sheet.style(pos), base)
    }

    /// Read each base's element out of the file this document came from.
    fn adopt_bases(&mut self, source: &[u8]) {
        // The patch spells attributes as LibreOffice and this writer both do; a file binding
        // `style:` or `fo:` to other prefixes gets styles of this writer's own instead.
        let standard = envelope::prefix_for(source, STYLE).as_deref() == Some("style")
            && envelope::prefix_for(source, FO).as_deref() == Some("fo");
        self.base_elements = self
            .bases
            .iter()
            .map(|base| {
                let range = envelope::automatic_style_range(source, "table-cell", base)?;
                let element = std::str::from_utf8(&source[range]).ok()?.to_owned();
                Some(element)
            })
            .map(|element| element.filter(|_| standard))
            .collect();
    }

    /// Pool a format, and the target of every `style:map` it carries — a branch is a style
    /// in its own right in the file, referenced by name from the map (§5.1).
    fn add(&mut self, format: &'a Format) {
        // `insert` would *overwrite* the index of a format already pooled, quietly pointing
        // its first cells at a later style.
        if self.index.contains_key(format) {
            return;
        }
        self.index.insert(format, self.formats.len());
        self.formats.push(format);
        for map in &format.maps {
            self.add(&map.format);
        }
    }

    /// Start every family's numbering past the names `reserved` already uses.
    fn avoid(&mut self, reserved: &HashSet<String>) {
        for (slot, prefix) in ["ce", "co", "ro", "N"].iter().enumerate() {
            self.offsets[slot] = reserved
                .iter()
                .filter_map(|name| name.strip_prefix(prefix)?.parse::<usize>().ok())
                .map(|n| n + 1)
                .max()
                .unwrap_or(0);
        }
    }

    fn ce(&self, i: usize) -> String {
        format!("ce{}", i + self.offsets[0])
    }

    fn co(&self, i: usize) -> String {
        format!("co{}", i + self.offsets[1])
    }

    fn ro(&self, i: usize) -> String {
        format!("ro{}", i + self.offsets[2])
    }

    fn n(&self, i: usize) -> String {
        format!("N{}", i + self.offsets[3])
    }

    /// ` table:style-name="ce3"`, or nothing when the cell has neither format nor styling.
    fn attr(&self, look: Look) -> String {
        match self.look_index.get(&look) {
            Some(i) => format!(" table:style-name=\"{}\"", self.ce(*i)),
            None => String::new(),
        }
    }

    /// ` table:style-name="co3"` for a column's width, or nothing for a default one.
    fn col_attr(&self, width: Option<&str>) -> String {
        match width.and_then(|w| self.col_index.get(w)) {
            Some(i) => format!(" table:style-name=\"{}\"", self.co(*i)),
            None => String::new(),
        }
    }

    /// The row twin of [`Pool::col_attr`].
    fn row_attr(&self, height: Option<&str>) -> String {
        match height.and_then(|h| self.row_index.get(h)) {
            Some(i) => format!(" table:style-name=\"{}\"", self.ro(*i)),
            None => String::new(),
        }
    }

    fn is_empty(&self) -> bool {
        self.formats.is_empty()
            && self.looks.is_empty()
            && self.cols.is_empty()
            && self.rows.is_empty()
    }

    /// Whether anything here spells a *cell*, which is what needs `number:` and `fo:`. A
    /// document whose only styles are track sizes uses neither (§1.4).
    fn styles_cells(&self) -> bool {
        !self.formats.is_empty() || !self.looks.is_empty()
    }

    /// `office:automatic-styles`: each format once, and one cell style per format to point
    /// at it — the indirection is mandatory, there is no way to put a format on a cell
    /// directly (§5.3).
    fn write(&self, out: &mut String) {
        out.push_str(" <office:automatic-styles>\n");
        for (i, format) in self.formats.iter().enumerate() {
            let _ = writeln!(out, "  {}", data_style(format, i, self));
        }
        for (i, (format, style, base)) in self.looks.iter().enumerate() {
            let data_name = format.and_then(|f| self.index.get(f)).map(|n| self.n(*n));
            // A restyled cell's style is its old one, renamed, with only what the model owns
            // rewritten — so a rotation, a protection flag, a parent style or a conditional
            // `style:map` the model never read is carried rather than dropped.
            if let Some(base) = base {
                let name = self.ce(i);
                let patched = self
                    .base_elements
                    .get(*base)
                    .and_then(Option::as_deref)
                    .and_then(|element| {
                        envelope::patch_style(
                            element,
                            &[
                                ("style:name", Some(name.as_str())),
                                ("style:data-style-name", data_name.as_deref()),
                            ],
                            &owned_properties(*style),
                            &PROPERTY_ORDER,
                        )
                    });
                if let Some(patched) = patched {
                    let _ = writeln!(out, "  {patched}");
                    continue;
                }
            }
            let data = match &data_name {
                Some(n) => format!(" style:data-style-name=\"{n}\""),
                None => String::new(),
            };
            // A cell that pointed at a *named* style keeps it as the new style's parent, as
            // LibreOffice writes one.
            let parent =
                match base.filter(|b| self.base_elements.get(*b).is_some_and(Option::is_none)) {
                    Some(b) => format!(" style:parent-style-name=\"{}\"", esc(&self.bases[b])),
                    None => String::new(),
                };
            let _ = write!(
                out,
                "  <style:style style:name=\"{}\" style:family=\"table-cell\"{parent}{data}",
                self.ce(i)
            );
            match style {
                Some(style) => {
                    let _ = writeln!(out, ">{}</style:style>", properties(style));
                }
                None => out.push_str("/>\n"),
            }
        }
        // §5.4. `style:use-optimal-column-width` is deliberately not written: the size here
        // is one somebody chose, and claiming it was derived would invite a reader to
        // re-derive it from text this program never measured.
        for (i, width) in self.cols.iter().enumerate() {
            let _ = writeln!(
                out,
                "  <style:style style:name=\"{}\" style:family=\"table-column\">\
                 <style:table-column-properties style:column-width=\"{}\"/></style:style>",
                self.co(i),
                esc(width)
            );
        }
        for (i, height) in self.rows.iter().enumerate() {
            let _ = writeln!(
                out,
                "  <style:style style:name=\"{}\" style:family=\"table-row\">\
                 <style:table-row-properties style:row-height=\"{}\"/></style:style>",
                self.ro(i),
                esc(height)
            );
        }
        out.push_str(" </office:automatic-styles>\n");
    }
}

/// The property elements of a cell style in the schema's order — where a patch puts one the old
/// style did not have.
const PROPERTY_ORDER: [&str; 3] = [
    "style:table-cell-properties",
    "style:paragraph-properties",
    "style:text-properties",
];

/// Every attribute [`properties`] writes, set to `style`'s value or removed — the whole of what
/// a cell style's properties mean *to the model*, and so exactly what a restyle may change on
/// the style it is built from. Must name every attribute `properties` does, or a property set
/// on the old style and cleared in the model would survive the clearing.
fn owned_properties(style: Option<&CellStyle>) -> Vec<envelope::PropertyPatch<'static>> {
    let value = |v: Option<&Option<String>>| v.and_then(|v| v.clone());
    let s = style;
    let mut cell = vec![
        (
            "fo:background-color".to_owned(),
            value(s.map(|s| &s.background)),
        ),
        (
            "style:vertical-align".to_owned(),
            value(s.map(|s| &s.vertical_align)),
        ),
        ("fo:wrap-option".to_owned(), value(s.map(|s| &s.wrap))),
    ];
    match s.and_then(|s| s.uniform_border()) {
        Some(border) => {
            cell.push(("fo:border".to_owned(), Some(border.to_owned())));
            for edge in EDGES {
                cell.push((format!("fo:border-{edge}"), None));
            }
        }
        None => {
            cell.push(("fo:border".to_owned(), None));
            for (i, edge) in EDGES.iter().enumerate() {
                cell.push((format!("fo:border-{edge}"), value(s.map(|s| &s.borders[i]))));
            }
        }
    }
    vec![
        ("style:table-cell-properties", cell),
        (
            "style:paragraph-properties",
            vec![("fo:text-align".to_owned(), value(s.map(|s| &s.align)))],
        ),
        (
            "style:text-properties",
            vec![
                (
                    "fo:font-weight".to_owned(),
                    value(s.map(|s| &s.font_weight)),
                ),
                ("fo:font-style".to_owned(), value(s.map(|s| &s.font_style))),
                ("fo:font-size".to_owned(), value(s.map(|s| &s.font_size))),
                ("fo:color".to_owned(), value(s.map(|s| &s.color))),
            ],
        ),
    ]
}

/// The property children of a cell style (§5.1), in the order the schema declares them.
///
/// Every value goes out exactly as it came in. LibreOffice will re-quantise a border width
/// and rewrite a font reference (§5.4) — that is its normalisation, not ours to anticipate.
fn properties(style: &CellStyle) -> String {
    let mut out = String::new();
    let attr = |out: &mut String, name: &str, value: &Option<String>| {
        if let Some(value) = value {
            let _ = write!(out, " {name}=\"{}\"", esc(value));
        }
    };

    let mut cell = String::new();
    attr(&mut cell, "fo:background-color", &style.background);
    attr(&mut cell, "style:vertical-align", &style.vertical_align);
    attr(&mut cell, "fo:wrap-option", &style.wrap);
    // The shorthand when every edge agrees, four attributes when they do not — the two
    // spell the same style, and emitting both would be a document contradicting itself.
    match style.uniform_border() {
        Some(border) => {
            let _ = write!(cell, " fo:border=\"{}\"", esc(border));
        }
        None => {
            for (i, edge) in EDGES.iter().enumerate() {
                attr(&mut cell, &format!("fo:border-{edge}"), &style.borders[i]);
            }
        }
    }
    if !cell.is_empty() {
        let _ = write!(out, "<style:table-cell-properties{cell}/>");
    }

    if let Some(align) = &style.align {
        let _ = write!(
            out,
            "<style:paragraph-properties fo:text-align=\"{}\"/>",
            esc(align)
        );
    }

    let mut text = String::new();
    attr(&mut text, "fo:font-weight", &style.font_weight);
    attr(&mut text, "fo:font-style", &style.font_style);
    attr(&mut text, "fo:font-size", &style.font_size);
    attr(&mut text, "fo:color", &style.color);
    if !text.is_empty() {
        let _ = write!(out, "<style:text-properties{text}/>");
    }
    out
}

/// One `number:*-style` (§5.2) — the exact inverse of what `read`'s `NumberStyle` builds.
fn data_style(format: &Format, i: usize, pool: &Pool) -> String {
    let element = match format.kind {
        Kind::Number => "number:number-style",
        Kind::Percentage => "number:percentage-style",
        Kind::Currency => "number:currency-style",
        Kind::Date => "number:date-style",
        Kind::Time => "number:time-style",
        Kind::Boolean => "number:boolean-style",
        Kind::Text => "number:text-style",
    };
    let locale = match &format.locale {
        Some(locale) if locale.country.is_empty() => {
            format!(" number:language=\"{}\"", esc(&locale.language))
        }
        Some(locale) => format!(
            " number:language=\"{}\" number:country=\"{}\"",
            esc(&locale.language),
            esc(&locale.country)
        ),
        None => String::new(),
    };
    let mut out = format!("<{element} style:name=\"{}\"{locale}>", pool.n(i));
    for part in &format.parts {
        // `number:style="long"` is the spec's spelling of "padded"; short is the default and
        // is written by omission, which is what LibreOffice does too.
        let long = |long: &bool| match long {
            true => " number:style=\"long\"",
            false => "",
        };
        match part {
            Part::Text(text) => {
                let _ = write!(out, "<number:text>{}</number:text>", esc(text));
            }
            Part::Currency(symbol) => {
                let _ = write!(
                    out,
                    "<number:currency-symbol>{}</number:currency-symbol>",
                    esc(symbol)
                );
            }
            Part::Number {
                decimals,
                min_decimals,
                min_int,
                grouping,
            } => {
                let _ = write!(
                    out,
                    "<number:number number:decimal-places=\"{decimals}\" \
                     number:min-decimal-places=\"{min_decimals}\" \
                     number:min-integer-digits=\"{min_int}\"{}/>",
                    match grouping {
                        true => " number:grouping=\"true\"",
                        false => "",
                    }
                );
            }
            Part::Year { long: l } => {
                let _ = write!(out, "<number:year{}/>", long(l));
            }
            Part::Month { long: l, textual } => {
                let _ = write!(
                    out,
                    "<number:month{}{}/>",
                    long(l),
                    match textual {
                        true => " number:textual=\"true\"",
                        false => "",
                    }
                );
            }
            Part::Day { long: l } => {
                let _ = write!(out, "<number:day{}/>", long(l));
            }
            Part::DayOfWeek { long: l } => {
                let _ = write!(out, "<number:day-of-week{}/>", long(l));
            }
            Part::Hours { long: l } => {
                let _ = write!(out, "<number:hours{}/>", long(l));
            }
            Part::Minutes { long: l } => {
                let _ = write!(out, "<number:minutes{}/>", long(l));
            }
            Part::Seconds { long: l, decimals } => {
                let _ = write!(
                    out,
                    "<number:seconds{} number:decimal-places=\"{decimals}\"/>",
                    long(l)
                );
            }
            Part::AmPm => out.push_str("<number:am-pm/>"),
            Part::Boolean => out.push_str("<number:boolean/>"),
            Part::Content => out.push_str("<number:text-content/>"),
        }
    }
    // The branches last, which is where the schema puts them, and by the name the pool gave
    // the target — a map whose target is not pooled cannot happen, since `Pool::add` walks
    // them, but a missing one is skipped rather than written as a dangling reference.
    for map in &format.maps {
        let Some(target) = pool.index.get(&map.format) else {
            continue;
        };
        let _ = write!(
            out,
            "<style:map style:condition=\"{}\" style:apply-style-name=\"{}\"/>",
            esc(&format!("value(){}{}", map.op.spelling(), map.value)),
            pool.n(*target)
        );
    }
    let _ = write!(out, "</{element}>");
    out
}

/// `table:shapes` (rng:15678) — every chart on this sheet, regenerated fresh from the model
/// every time: `doc/chart-format.md` explains why this build does not try to splice a chart's
/// own document the way a cell splices, and assigns [`crate::chart::series_color`] rather than
/// reproducing whatever colours the chart was last saved with.
fn write_shapes(out: &mut String, sheet: &Sheet, objects: &mut Objects) {
    if sheet.charts().is_empty() {
        return;
    }
    out.push_str("    <table:shapes>\n");
    for chart in sheet.charts() {
        write_chart(out, chart, objects);
    }
    out.push_str("    </table:shapes>\n");
}

/// Where a chart's own document goes, which is the one thing the two physical forms disagree
/// about (`doc/chart-format.md`, The two places a chart's own document can live).
///
/// **Flat: inline**, an `office:document` right inside `draw:object`. **Package: a
/// sub-document** beside `content.xml` — `Object 1/content.xml`, which `draw:object` points at
/// by `xlink:href`. The schema allows either in either form (rng:5541-5545), and this writer
/// used to write the inline shape into packages too, on the strength of that; measured
/// (LibreOffice 26.8.0.3, loop C), LibreOffice **drops every inline chart it finds in a
/// package**, so an `.ods` this build wrote came back with no charts at all.
struct Objects {
    form: Form,
    documents: Vec<SubDocument>,
    /// Directories the package already holds and this save keeps — an embedded object this
    /// build did not read — which a chart's own directory must never take.
    taken: HashSet<String>,
}

impl Objects {
    fn new(form: Form, taken: HashSet<String>) -> Self {
        Objects {
            form,
            documents: Vec::new(),
            taken,
        }
    }

    /// `Object 1`, `Object 2`, … skipping every name already taken.
    fn next_directory(&self) -> String {
        (1..)
            .map(|n| format!("Object {n}"))
            .find(|dir| {
                !self.taken.contains(dir) && !self.documents.iter().any(|d| d.directory == *dir)
            })
            .expect("an unbounded range has a free name")
    }
}

/// One chart, as the flat form's own inline shape (`doc/chart-format.md`) — valid ODF
/// regardless of the outer document's physical form, and always the simpler of the two
/// `draw:object` may hold (R3).
fn write_chart(out: &mut String, chart: &crate::chart::Chart, objects: &mut Objects) {
    use crate::chart::ChartKind;
    // Every `style:style style:family="chart"` this chart's document needs, one
    // [`crate::chart::effective_color`] each. Built before anything is written, so the
    // automatic-styles section — which the schema puts ahead of the body — never has to
    // forward-reference a name the body decides on later.
    //
    // **Every series names a style of its own, whatever its kind** (`doc/chart-format.md`,
    // Colour): LibreOffice ignores a data point's style under a series that names none, which
    // is how every bar and slice this build wrote used to come out in its default palette. A
    // bar or a line colours per series; a pie's series style carries its first slice's colour
    // and each slice is a data point of its own; a bar's data points are written only where a
    // bar carries an override ([`bar_points`]).
    let mut styles: Vec<(String, String)> = Vec::new();
    for (n, series) in chart.series.iter().enumerate() {
        let own = match chart.kind {
            ChartKind::Pie => crate::chart::effective_color(chart, n, Some(0)),
            ChartKind::Bar | ChartKind::Line => crate::chart::effective_color(chart, n, None),
        };
        styles.push((format!("gch{n}"), own));
        match chart.kind {
            ChartKind::Pie => {
                for point in 0..cell_count(&series.values) {
                    styles.push((
                        format!("gch{n}-{point}"),
                        crate::chart::effective_color(chart, n, Some(point)),
                    ));
                }
            }
            ChartKind::Bar => {
                for point in bar_overrides(series, cell_count(&series.values)) {
                    styles.push((
                        format!("gch{n}-{point}"),
                        crate::chart::effective_color(chart, n, Some(point)),
                    ));
                }
            }
            ChartKind::Line => {}
        }
    }
    // Each axis needs a style of its own to carry `chart:display-label` — a
    // `style:chart-properties` attribute, not an attribute of `chart:axis` itself.
    //
    // **Always written, on both axes, whichever way it goes** (`doc/chart-format.md` has the
    // measurement): LibreOffice's own importer reads an *absent* `chart:display-label` as
    // `false`, so leaving the attribute off when tick labels are shown would hand out a chart
    // that draws differently there than it does here. A chart LibreOffice itself writes states
    // it explicitly too, which is the same conclusion reached from the other direction.
    //
    // A pie's y axis is its angle axis, and its style states `chart:reverse-direction` for the
    // same reason: LibreOffice draws a pie whose file says nothing counter-clockwise
    // (`doc/chart-format.md`, Direction), so leaving it off would draw a clockwise pie the
    // other way round there.
    let direction = match chart.kind {
        ChartKind::Pie => format!(" chart:reverse-direction=\"{}\"", chart.clockwise),
        ChartKind::Bar | ChartKind::Line => String::new(),
    };
    let axis_styles: [(&str, &Axis, &str); 2] = [
        ("gchx", &chart.x_axis, ""),
        ("gchy", &chart.y_axis, &direction),
    ];

    let _ = writeln!(
        out,
        "     <draw:frame svg:x=\"{}\" svg:y=\"{}\" svg:width=\"{}\" svg:height=\"{}\">",
        esc(&chart.x),
        esc(&chart.y),
        esc(&chart.width),
        esc(&chart.height)
    );
    // The chart's own document is written into `document` and placed afterwards, inline or
    // as a sub-document ([`Objects`]); its content is the same either way.
    let whole = out;
    let mut document = String::new();
    let out = &mut document;
    out.push_str("        <office:automatic-styles>\n");
    for (name, color) in &styles {
        let _ = writeln!(
            out,
            "         <style:style style:name=\"{name}\" style:family=\"chart\">\
             <style:graphic-properties svg:stroke-color=\"{color}\" draw:fill-color=\"{color}\"/>\
             </style:style>"
        );
    }
    for (name, axis, extra) in axis_styles {
        let _ = writeln!(
            out,
            "         <style:style style:name=\"{name}\" style:family=\"chart\">\
             <style:chart-properties chart:display-label=\"{}\"{extra}/>\
             </style:style>",
            axis.tick_labels
        );
    }
    out.push_str("        </office:automatic-styles>\n");
    out.push_str("        <office:body><office:chart>\n");
    let _ = writeln!(
        out,
        "         <chart:chart svg:width=\"{}\" svg:height=\"{}\" chart:class=\"{}\">",
        esc(&chart.width),
        esc(&chart.height),
        chart.kind.class()
    );
    // The schema's order: title, (subtitle, footer), legend, then the plot area (rng:462-485).
    // No position on either — both are optional (rng:1722-1733), and LibreOffice lays out a
    // title and a legend that say nothing about where they sit.
    if let Some(title) = &chart.title {
        let _ = writeln!(
            out,
            "          <chart:title><text:p>{}</text:p></chart:title>",
            esc(title)
        );
    }
    if let Some(legend) = chart.legend {
        let _ = writeln!(
            out,
            "          <chart:legend chart:legend-position=\"{}\"/>",
            legend.token()
        );
    }
    out.push_str("          <chart:plot-area>\n");
    write_axis(out, "x", chart.categories.as_deref(), &chart.x_axis);
    write_axis(out, "y", None, &chart.y_axis);
    for (n, series) in chart.series.iter().enumerate() {
        let label = series
            .label
            .as_deref()
            .map(|l| format!(" chart:label-cell-address=\"{}\"", esc(l)))
            .unwrap_or_default();
        let _ = writeln!(
            out,
            "           <chart:series chart:class=\"{}\" \
             chart:values-cell-range-address=\"{}\"{label} chart:style-name=\"gch{n}\">",
            chart.kind.class(),
            esc(&series.values)
        );
        let count = cell_count(&series.values);
        match chart.kind {
            // A pie's slices are what a reader tells apart, so every one is its own
            // `chart:data-point` and its own colour.
            ChartKind::Pie => {
                for point in 0..count {
                    let _ = writeln!(
                        out,
                        "            <chart:data-point chart:style-name=\"gch{n}-{point}\"/>"
                    );
                }
            }
            // A bar series is its own colour; only a bar picked by hand is a point of its own,
            // and the runs between them are one `chart:repeated` each.
            ChartKind::Bar => {
                for run in bar_points(series, count) {
                    match run {
                        PointRun::Own(point) => {
                            let _ = writeln!(
                                out,
                                "            <chart:data-point chart:style-name=\"gch{n}-{point}\"/>"
                            );
                        }
                        PointRun::Plain(repeated) => {
                            let _ = writeln!(
                                out,
                                "            <chart:data-point chart:repeated=\"{repeated}\"/>"
                            );
                        }
                    }
                }
            }
            // A line series shares one colour across every point in it.
            ChartKind::Line => {
                let _ = writeln!(
                    out,
                    "            <chart:data-point chart:repeated=\"{count}\"/>"
                );
            }
        }
        out.push_str("           </chart:series>\n");
    }
    out.push_str("          </chart:plot-area>\n");
    out.push_str("         </chart:chart>\n");
    out.push_str("        </office:chart></office:body>\n");

    let out = whole;
    match objects.form {
        Form::Package => {
            let directory = objects.next_directory();
            let _ = writeln!(
                out,
                "      <draw:object xlink:href=\"./{}\" xlink:type=\"simple\" \
                 xlink:show=\"embed\" xlink:actuate=\"onLoad\"/>",
                esc(&directory)
            );
            // A part of its own, so it declares every namespace it uses rather than
            // inheriting them from an outer document it is no longer inside.
            let content = format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
                 <office:document-content xmlns:office=\"{OFFICE}\" xmlns:style=\"{STYLE}\" \
                 xmlns:text=\"{TEXT}\" xmlns:table=\"{TABLE}\" xmlns:draw=\"{DRAW}\" \
                 xmlns:chart=\"{CHART}\" xmlns:svg=\"{SVG}\" office:version=\"{VERSION}\">\n\
                 {document}</office:document-content>\n"
            );
            objects.documents.push(SubDocument {
                directory,
                mimetype: CHART_MIMETYPE,
                content,
            });
        }
        Form::Flat | Form::Projection => {
            out.push_str("      <draw:object>\n");
            let _ = writeln!(
                out,
                "       <office:document office:mimetype=\"{CHART_MIMETYPE}\" \
                 office:version=\"{VERSION}\">"
            );
            out.push_str(&document);
            out.push_str("       </office:document>\n");
            out.push_str("      </draw:object>\n");
        }
    }
    out.push_str("     </draw:frame>\n");
}

/// The points of a bar series that carry a colour of their own, in order — the only ones that
/// get a `chart:data-point` style.
fn bar_overrides(series: &crate::chart::Series, count: usize) -> impl Iterator<Item = usize> + '_ {
    (0..count).filter(|point| matches!(series.point_colors.get(*point), Some(Some(_))))
}

/// One `chart:data-point` of a bar series: a point with a colour of its own, or a run of
/// points that simply wear the series' colour.
enum PointRun {
    Own(usize),
    Plain(usize),
}

/// A bar series' data points as [`PointRun`]s covering all `count` of them — every override
/// its own element, the runs between them one `chart:repeated` each, so an untouched series is
/// a single element however long it is.
fn bar_points(series: &crate::chart::Series, count: usize) -> Vec<PointRun> {
    let mut runs = Vec::new();
    let mut plain = 0;
    for point in 0..count {
        match series.point_colors.get(point) {
            Some(Some(_)) => {
                if plain > 0 {
                    runs.push(PointRun::Plain(plain));
                    plain = 0;
                }
                runs.push(PointRun::Own(point));
            }
            _ => plain += 1,
        }
    }
    if plain > 0 {
        runs.push(PointRun::Plain(plain));
    }
    runs
}

/// One `chart:axis` (rng:423): categories (x only, `None` for y), and whatever the [`Axis`]
/// itself carries — self-closed when all of that is empty, the same as the y axis always used
/// to be before it could carry anything.
///
/// Child order is the schema's, not this build's taste: `chart:title`, then
/// `chart:categories`, then any `chart:grid` (rng:422-434). Tick labels are *not* here —
/// `chart:display-label` lives on the axis' own style, written by [`write_chart`] and
/// referenced by the `chart:style-name` below.
fn write_axis(out: &mut String, dimension: &str, categories: Option<&str>, axis: &Axis) {
    let style = format!(" chart:style-name=\"gch{dimension}\"");
    if categories.is_none() && axis.label.is_none() && !axis.gridlines {
        let _ = writeln!(
            out,
            "           <chart:axis chart:dimension=\"{dimension}\"{style}/>"
        );
        return;
    }
    let _ = writeln!(
        out,
        "           <chart:axis chart:dimension=\"{dimension}\"{style}>"
    );
    if let Some(title) = &axis.label {
        let _ = writeln!(
            out,
            "            <chart:title><text:p>{}</text:p></chart:title>",
            esc(title)
        );
    }
    if let Some(categories) = categories {
        let _ = writeln!(
            out,
            "            <chart:categories table:cell-range-address=\"{}\"/>",
            esc(categories)
        );
    }
    if axis.gridlines {
        out.push_str("            <chart:grid chart:class=\"major\"/>\n");
    }
    out.push_str("           </chart:axis>\n");
}

const CHART_MIMETYPE: &str = "application/vnd.oasis.opendocument.chart";

/// How many cells a chart's own range spans — the only reason this build parses one back out:
/// a pie's per-slice colouring needs one `chart:data-point` per cell, and a bar or line
/// series' `chart:data-point chart:repeated`  needs the same count for R2 validity (the
/// schema does not require it to add up, but a reader that draws one dot per data point
/// should see as many as the range actually has).
fn cell_count(range: &str) -> usize {
    let Ok(reference) = crate::a1::parse_bracketed(&format!("[{range}]")) else {
        return 1;
    };
    let end = reference.end.as_ref().unwrap_or(&reference.start);
    let rows = match (reference.start.row, end.row) {
        (Some(s), Some(e)) => e.index.abs_diff(s.index) + 1,
        _ => 1,
    };
    let cols = match (reference.start.col, end.col) {
        (Some(s), Some(e)) => e.index.abs_diff(s.index) + 1,
        _ => 1,
    };
    (rows * cols) as usize
}

fn table(
    out: &mut String,
    sheet: &Sheet,
    null_date: i64,
    locale: Option<&grind_core::locale::Locale>,
    pool: &Pool,
    objects: &mut Objects,
) {
    let rows = cols_or_rows_extent(sheet, null_date, locale);

    let _ = writeln!(out, "   <table:table table:name=\"{}\">", esc(&sheet.name));
    // Before the columns, which is where the schema puts it (rng:15961, ahead of
    // rng:15963-15964's `table-columns-and-groups`/`table-rows-and-groups`).
    write_shapes(out, sheet, objects);
    write_columns(out, sheet, pool, None);

    if rows == 0 {
        out.push_str("    <table:table-row><table:table-cell/></table:table-row>\n");
    }
    let visibility = Visibility::of(sheet, null_date, locale);
    write_rows(out, sheet, 0..rows, null_date, pool, &visibility);

    out.push_str("   </table:table>\n");
}

/// The column declarations: both the column block and the row block are mandatory, even for an
/// all-empty sheet (§3.2), which is why everything here has a `.max(1)` behind it. Neighbouring
/// columns of equal width, hidden state and default cell style are one declaration, which is
/// what the reader's repeat handling reads back and what keeps a sheet's declarations to a
/// handful. `defaults` is the file's own `table:default-cell-style-name` per column, when the
/// sheet is being written back into the table it was read from — the model does not carry it.
fn write_columns(
    out: &mut String,
    sheet: &Sheet,
    pool: &Pool,
    defaults: Option<&super::source::Table>,
) {
    let cols = sheet.used_cols().max(1);
    // A sized or hidden track past the last value still has to be declared, or the layout
    // is lost — widening or hiding an empty column is a perfectly ordinary thing to do.
    let mut declared = cols
        .max(last(sheet.col_widths()))
        .max(last_index(sheet.hidden_cols()));
    if let Some(table) = defaults {
        // Every column the file gave a default style keeps it — the file's own run, however
        // far past the content it reached.
        let reach = table
            .column_defaults
            .iter()
            .filter(|(_, style)| style.is_some())
            .map(|(cols, _)| cols.end)
            .max()
            .unwrap_or(0);
        declared = declared.max(reach);
    }
    let default = |col: u32| defaults.and_then(|table| table.column_default(col));
    let mut col = 0;
    while col < declared {
        let width = sheet.col_width(col);
        let hidden = sheet.col_hidden(col);
        let style = default(col);
        let run = (col..declared)
            .take_while(|c| {
                sheet.col_width(*c) == width
                    && sheet.col_hidden(*c) == hidden
                    && default(*c) == style
            })
            .count() as u32;
        let style = match style {
            Some(name) => format!(" table:default-cell-style-name=\"{}\"", esc(name)),
            None => String::new(),
        };
        let _ = writeln!(
            out,
            "    <table:table-column{}{}{}{style}/>",
            pool.col_attr(width),
            collapse(hidden),
            count(run, "columns")
        );
        col += run;
    }
}

/// What the filter hides, written out as `table:visibility="filter"` (§9.4). Derived here rather
/// than stored, so the attribute cannot drift from the conditions — see [`crate::filter`]. Rows
/// hidden by hand are stored, and write `"collapse"` instead — LibreOffice keeps the two
/// spellings apart, and a row that is both wins `"collapse"`, the more structural of the two.
struct Visibility {
    filtered: BTreeSet<u32>,
    manual: BTreeSet<u32>,
}

impl Visibility {
    fn of(sheet: &Sheet, null_date: i64, locale: Option<&grind_core::locale::Locale>) -> Self {
        Visibility {
            filtered: sheet.hidden_rows(null_date, locale).into_iter().collect(),
            manual: sheet.manually_hidden_rows().collect(),
        }
    }

    fn attr(&self, row: u32) -> &'static str {
        match self.value(row) {
            Some("collapse") => " table:visibility=\"collapse\"",
            Some(_) => " table:visibility=\"filter\"",
            None => "",
        }
    }

    /// The value of `table:visibility` for this row, `None` being the default, visible.
    fn value(&self, row: u32) -> Option<&'static str> {
        match (self.manual.contains(&row), self.filtered.contains(&row)) {
            (true, _) => Some("collapse"),
            (false, true) => Some("filter"),
            (false, false) => None,
        }
    }
}

/// The rows in `range`, each as the model has it.
fn write_rows(
    out: &mut String,
    sheet: &Sheet,
    range: std::ops::Range<u32>,
    null_date: i64,
    pool: &Pool,
    visibility: &Visibility,
) {
    let cols = sheet.used_cols().max(1);
    // Which rows carry anything, asked once of the sheet's sparse storage rather than of every
    // cell in the used rectangle — see `Sheet::rows_carrying` for the sheet that never finished.
    let carrying = sheet.rows_carrying();
    let is_blank = |row: u32| {
        let i = carrying.partition_point(|range| range.end <= row);
        carrying.get(i).is_none_or(|range| range.start > row)
    };

    let mut row = range.start;
    while row < range.end {
        let height = sheet.row_height(row);
        // Interior blank rows collapse into one repeated row (§3.3) — the main file-size
        // lever, and the difference between a 20-row sheet and 20 rows plus a megabyte of
        // nothing after one stray edit at row 50 000. A row of a different height stops the
        // run, or the height would spread down the sheet.
        // A hidden row stops the run too, for the same reason a differently sized one does.
        let blank = (row..range.end)
            .take_while(|r| {
                is_blank(*r)
                    && sheet.row_height(*r) == height
                    && visibility.attr(*r) == visibility.attr(row)
            })
            .count() as u32;
        if blank > 0 {
            let _ = writeln!(
                out,
                "    <table:table-row{}{}{}><table:table-cell/></table:table-row>",
                pool.row_attr(height),
                visibility.attr(row),
                count(blank, "rows")
            );
            row += blank;
            continue;
        }
        write_row(out, sheet, row, cols, null_date, pool, visibility.attr(row));
        row += 1;
    }
}

/// The row extent: the last used row, the last sized one, and the last one hidden by hand
/// or by the filter — any of which has to be declared even past the sheet's used content.
fn cols_or_rows_extent(
    sheet: &Sheet,
    null_date: i64,
    locale: Option<&grind_core::locale::Locale>,
) -> u32 {
    sheet
        .used_rows()
        .max(last(sheet.row_heights()))
        .max(last_index(sheet.manually_hidden_rows()))
        .max(last_index(sheet.hidden_rows(null_date, locale).into_iter()))
}

/// One past the last track a sparse size table mentions.
fn last<'a>(sizes: impl Iterator<Item = (u32, &'a str)>) -> u32 {
    sizes.map(|(i, _)| i.saturating_add(1)).max().unwrap_or(0)
}

/// One past the last track a plain index list mentions — [`last`]'s twin for hidden tracks.
fn last_index(indices: impl Iterator<Item = u32>) -> u32 {
    indices.map(|i| i.saturating_add(1)).max().unwrap_or(0)
}

/// Whether a cell carries anything the file has to spell.
///
/// **Five things, not two.** It used to be a value or a formula, and a cell holding only a
/// number format or a cell style was therefore "blank" — dropped as a blank row when its
/// whole row was, and cut off by `write_row`'s trailing-cell trim when it was at the end of one.
/// `Sheet::used_rows` has the same story and the same three ways in; a hand-written projection
/// is the one that found it, because `cell B5 "=SUM([.B2:.B4])"` with no cached value is the
/// *normal* way to write a spreadsheet you have not done the arithmetic for.
///
/// `Sheet::rows_carrying` is the same five questions asked row-wise of the sparse storage, and
/// `rows_carrying_is_carries_asked_row_by_row` holds the two to one answer.
fn carries(sheet: &Sheet, pos: Pos) -> bool {
    !sheet.get(pos).is_empty()
        || sheet.formula(pos).is_some()
        || sheet.kind(pos).is_some()
        || sheet.format(pos).is_some()
        || sheet.style(pos).is_some()
}

/// `table:visibility="collapse"` for a column hidden by hand — a column has no filter, so
/// this is the only spelling it ever needs (§5.4).
fn collapse(hidden: bool) -> &'static str {
    match hidden {
        true => " table:visibility=\"collapse\"",
        false => "",
    }
}

#[allow(clippy::too_many_arguments)]
fn write_row(
    out: &mut String,
    sheet: &Sheet,
    row: u32,
    cols: u32,
    null_date: i64,
    pool: &Pool,
    visibility: &'static str,
) {
    let _ = write!(
        out,
        "    <table:table-row{}{}>",
        pool.row_attr(sheet.row_height(row)),
        visibility
    );
    // Trailing empty cells are simply not written: unmentioned is the same as empty
    // (§3.3), and the row is known non-blank so at least one cell survives. "Empty" is
    // `carries` — a cell whose only content is a style still has to be spelled, or the style
    // is what the trim throws away.
    let last = (0..cols)
        .rposition(|col| carries(sheet, Pos::new(row, col)))
        .map_or(0, |c| c as u32);

    let mut col = 0;
    while col <= last {
        let pos = Pos::new(row, col);
        let value = sheet.get(pos);
        let formula = sheet.formula(pos);
        let look = pool.look(sheet, pos);
        // Only blank runs are compressed. Repeating a *valued* cell would need the formula
        // to repeat with it, and a formula is position-dependent — a correctness trap for
        // bytes nobody is short of. A run of blanks sharing one format compresses like any
        // other; one that does not share it stops the run, or the format would spread.
        let repeat = if value.is_empty() && formula.is_none() {
            (col..=last)
                .take_while(|c| {
                    let p = Pos::new(row, *c);
                    sheet.get(p).is_empty()
                        && sheet.formula(p).is_none()
                        && pool.look(sheet, p) == look
                })
                .count() as u32
        } else {
            1
        };
        cell(
            out,
            &value,
            formula,
            sheet.kind(pos),
            look,
            null_date,
            repeat,
            &pool.attr(look),
        );
        col += repeat;
    }
    out.push_str("</table:table-row>\n");
}

#[allow(clippy::too_many_arguments)]
fn cell(
    out: &mut String,
    value: &CellValue,
    formula: Option<&str>,
    kind: Option<NumberKind>,
    look: Look,
    null_date: i64,
    repeat: u32,
    style_attr: &str,
) {
    let format = look.0;
    let mut attrs = count(repeat, "columns");
    attrs.push_str(style_attr);
    if let Some(f) = formula {
        let _ = write!(attrs, " table:formula=\"{}\"", esc(f));
    }

    // The cached result travels with the formula, never instead of it: an omitted cached
    // value is schema-legal and renders blank until the next recalculation (§4).
    match value {
        // A bare cell is a valid, correctly typed empty cell (§3.4).
        CellValue::Empty => {
            let _ = write!(out, "<table:table-cell{attrs}/>");
        }
        CellValue::Number(n) => {
            // The reader maps a non-finite `office:value` to zero (§9); agreeing here keeps
            // read(write(d)) == d for every document the reader can produce, and xsd:double's
            // `INF`/`NaN` spellings are not worth the interop risk for a value no ODF
            // document should be carrying in the first place.
            let n = if n.is_finite() { *n } else { 0.0 };
            // A date and a time are Numbers (§4.3.3, §4.3.2) that were *written* as a
            // calendar date or a clock, and go back out the way they came in. The kind is
            // consulted only here, so a cell whose date was overwritten with text or a
            // boolean cannot carry a stale one out — no side table to keep in step.
            let typed = match kind {
                Some(NumberKind::Date) => format!(
                    "office:value-type=\"date\" office:date-value=\"{}\"",
                    date::format_date(n, null_date)
                ),
                Some(NumberKind::Time) => format!(
                    "office:value-type=\"time\" office:time-value=\"{}\"",
                    date::format_time(n)
                ),
                None => format!("office:value-type=\"float\" office:value=\"{n}\""),
            };
            display(out, &attrs, &typed, value, format, null_date);
        }
        CellValue::Bool(b) => {
            let typed = format!("office:value-type=\"boolean\" office:boolean-value=\"{b}\"");
            display(out, &attrs, &typed, value, format, null_date);
        }
        CellValue::Text(s) => {
            // Carried as paragraphs rather than `office:string-value`, which is what LO
            // itself writes and — the load-bearing reason — is the only form that survives
            // a newline. XML normalises a literal newline in an attribute value to a space,
            // so a multi-line string in `office:string-value` comes back mangled.
            let _ = write!(
                out,
                "<table:table-cell{attrs} office:value-type=\"string\">"
            );
            for line in s.split('\n') {
                if line.is_empty() {
                    out.push_str("<text:p/>");
                } else {
                    let _ = write!(out, "<text:p>{}</text:p>", paragraph(line));
                }
            }
            out.push_str("</table:table-cell>");
        }
    }
}

/// A typed cell, with the `text:p` its format renders when it has one.
///
/// The display text is redundant to us — the value and the style say everything — and is
/// written anyway because §7.2's template writes it and because a reader that does not
/// implement number formats shows *something* rather than a blank cell. It is never the
/// value: every branch here carries a typed `office:*-value` attribute beside it.
fn display(
    out: &mut String,
    attrs: &str,
    typed: &str,
    value: &CellValue,
    format: Option<&Format>,
    null_date: i64,
) {
    let Some(format) = format else {
        let _ = write!(out, "<table:table-cell{attrs} {typed}/>");
        return;
    };
    let _ = write!(
        out,
        "<table:table-cell{attrs} {typed}><text:p>{}</text:p></table:table-cell>",
        paragraph(&format.render(value, null_date))
    );
}

/// ` table:number-<axis>-repeated="n"`, or nothing at all when `n` is one.
fn count(n: u32, axis: &str) -> String {
    if n > 1 {
        format!(" table:number-{axis}-repeated=\"{n}\"")
    } else {
        String::new()
    }
}

/// One line of cell text as the body of a `text:p`.
///
/// Whitespace inside a `text:p` is **collapsed** by any conforming reader, which is the
/// entire reason `text:s` and `text:tab` exist. Writing `"a    b"` literally gets it back
/// as `"a b"`, and a leading or trailing space vanishes outright — so runs of spaces, and
/// any space at either end, are written as an explicit `text:s`. A single interior space
/// survives collapsing untouched and stays literal, which keeps ordinary prose readable in
/// the output.
fn paragraph(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while !rest.is_empty() {
        let plain = rest.find([' ', '\t']).unwrap_or(rest.len());
        if plain > 0 {
            out.push_str(&esc(&rest[..plain]));
            rest = &rest[plain..];
            continue;
        }
        if rest.starts_with('\t') {
            out.push_str("<text:tab/>");
            rest = &rest[1..];
            continue;
        }
        let spaces = rest.len() - rest.trim_start_matches(' ').len();
        let ends_the_line = spaces == rest.len();
        if spaces == 1 && !out.is_empty() && !ends_the_line {
            out.push(' ');
        } else {
            let _ = write!(out, "<text:s text:c=\"{spaces}\"/>");
        }
        rest = &rest[spaces..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(doc: &Document) -> String {
        content(doc, Form::Flat, &HashSet::new(), None).0
    }

    #[test]
    fn an_empty_sheet_still_has_a_column_and_a_row() {
        let xml = flat(&Document::default());
        // §3.2: both blocks are `oneOrMore` in the grammar. Omitting either is invalid even
        // though there is nothing to say.
        assert!(xml.contains("<table:table-column/>"), "{xml}");
        assert!(
            xml.contains("<table:table-row><table:table-cell/></table:table-row>"),
            "{xml}"
        );
    }

    /// The used rectangle here is the whole grid — seventeen billion cells — and a writer that
    /// asked each one whether it was blank never finished. Four cells in, four cells out, and
    /// the rows between are two repeats.
    #[test]
    fn the_far_corners_of_the_grid_are_four_cells_not_the_rectangle_between() {
        let mut doc = Document::default();
        let sheet = doc.sheet_mut(0).unwrap();
        let (last_row, last_col) = (crate::MAX_ROWS - 1, crate::MAX_COLS - 1);
        for (row, col) in [(0, 0), (0, last_col), (last_row, 0), (last_row, last_col)] {
            sheet.set(Pos::new(row, col), CellValue::Number(1.0));
        }
        let start = std::time::Instant::now();
        let xml = flat(&doc);
        assert!(
            xml.contains(&format!("table:number-rows-repeated=\"{}\"", last_row - 1)),
            "{xml}"
        );
        assert!(
            start.elapsed() < std::time::Duration::from_secs(10),
            "{:?}",
            start.elapsed()
        );
    }

    /// `Sheet::rows_carrying` and [`carries`] are one rule spelled twice — once row-wise over
    /// the sparse storage, once per cell — and a row the two disagreed about would be written
    /// blank with something in it, or the reverse.
    #[test]
    fn rows_carrying_is_carries_asked_row_by_row() {
        let mut doc = Document::default();
        let sheet = doc.sheet_mut(0).unwrap();
        sheet.set(Pos::new(0, 0), CellValue::Number(1.0));
        sheet.set(Pos::new(1, 2), CellValue::Text("t".into()));
        sheet.set(Pos::new(2, 2), CellValue::Number(2.0));
        sheet.set_formula(Pos::new(5, 1), "=1".into());
        sheet.set_kind(Pos::new(7, 0), crate::model::NumberKind::Date);
        sheet.set_format(Pos::new(9, 3), money());
        sheet.set_style(
            Pos::new(12, 0),
            crate::style::CellStyle {
                font_weight: Some("bold".into()),
                ..Default::default()
            },
        );
        sheet.set(Pos::new(20, 1), CellValue::Bool(true));
        sheet.set(Pos::new(21, 1), CellValue::Bool(false));

        let ranges = sheet.rows_carrying();
        let (rows, cols) = (sheet.used_rows(), sheet.used_cols());
        for row in 0..rows + 2 {
            let by_cell = (0..cols).any(|col| carries(sheet, Pos::new(row, col)));
            let by_range = ranges.iter().any(|range| range.contains(&row));
            assert_eq!(by_cell, by_range, "row {row}: {ranges:?}");
        }
    }

    #[test]
    fn blank_rows_and_cells_collapse_into_repeats() {
        let mut doc = Document::default();
        let sheet = doc.sheet_mut(0).unwrap();
        sheet.set(Pos::new(0, 0), CellValue::Number(1.0));
        sheet.set(Pos::new(500, 3), CellValue::Number(2.0));

        let xml = flat(&doc);
        // 499 blank rows between them, and 3 blank cells before the second value.
        assert!(xml.contains("table:number-rows-repeated=\"499\""), "{xml}");
        assert!(xml.contains("table:number-columns-repeated=\"3\""), "{xml}");
        // A 501-row sheet must not cost 501 row elements.
        assert!(xml.matches("<table:table-row").count() < 10, "{xml}");
    }

    #[test]
    fn a_sized_track_past_the_used_extent_is_still_declared() {
        let mut doc = Document::default();
        let sheet = doc.sheet_mut(0).unwrap();
        sheet.set(Pos::new(0, 0), CellValue::Number(1.0));
        sheet.set_col_width(4, Some("5cm".into()));

        // The declarations run to the last *sized* column, not to the last used one, or the
        // width would have no column to sit on. Loop C cannot check this — LibreOffice drops
        // a width outside its own used extent on the way back — so it is checked here.
        let xml = flat(&doc);
        assert!(
            xml.contains("<style:table-column-properties style:column-width=\"5cm\""),
            "{xml}"
        );
        assert!(
            xml.contains("<table:table-column table:number-columns-repeated=\"4\"/>"),
            "{xml}"
        );
        assert!(
            xml.contains("<table:table-column table:style-name=\"co0\"/>"),
            "{xml}"
        );
    }

    #[test]
    fn trailing_blank_cells_are_not_written_at_all() {
        let mut doc = Document::default();
        let sheet = doc.sheet_mut(0).unwrap();
        sheet.set(Pos::new(0, 9), CellValue::Number(1.0));
        sheet.set(Pos::new(1, 0), CellValue::Number(2.0));

        // Row 1 stops after its one cell rather than padding out to the sheet's width;
        // unmentioned is the same as empty (§3.3).
        let xml = flat(&doc);
        let row1 = xml
            .lines()
            .find(|l| l.contains("office:value=\"2\""))
            .unwrap();
        assert_eq!(row1.matches("<table:table-cell").count(), 1, "{row1}");
    }

    #[test]
    fn a_formula_keeps_its_cached_value() {
        let mut doc = Document::default();
        let sheet = doc.sheet_mut(0).unwrap();
        sheet.set(Pos::new(0, 0), CellValue::Number(30.0));
        sheet.set_formula(Pos::new(0, 0), "of:=SUM([.B1:.C1])".into());

        let xml = flat(&doc);
        // Both, always: an omitted cached value renders blank until recalculation (§4).
        assert!(
            xml.contains("table:formula=\"of:=SUM([.B1:.C1])\""),
            "{xml}"
        );
        assert!(xml.contains("office:value=\"30\""), "{xml}");
    }

    #[test]
    fn characters_xml_cannot_carry_are_dropped_rather_than_emitted() {
        // A vertical tab is legal in a Rust String and has no representation in XML 1.0.
        // Writing it produces a file nothing can read back, including us.
        assert_eq!(esc("a\u{b}b"), "ab");
        assert_eq!(esc("<&\">"), "&lt;&amp;&quot;&gt;");
        assert_eq!(esc("keep\tthese\n"), "keep\tthese\n");
    }

    #[test]
    fn space_runs_are_written_explicitly_because_readers_collapse_them() {
        // A single interior space survives collapsing and stays readable.
        assert_eq!(paragraph("a b"), "a b");
        // Everything else would be eaten: runs, and either end of the line.
        assert_eq!(paragraph("a    b"), "a<text:s text:c=\"4\"/>b");
        assert_eq!(paragraph(" a"), "<text:s text:c=\"1\"/>a");
        assert_eq!(paragraph("a "), "a<text:s text:c=\"1\"/>");
        assert_eq!(paragraph("a\tb"), "a<text:tab/>b");
        assert_eq!(paragraph("<&"), "&lt;&amp;");
    }

    fn money() -> Format {
        let mut f = Format::new(Kind::Currency);
        f.push(Part::Number {
            decimals: 2,
            min_decimals: 2,
            min_int: 1,
            grouping: true,
        });
        f.push(Part::Currency(" \u{20ac}".into()));
        f
    }

    /// §5.3: one style per distinct format, however many cells wear it. Two cells sharing a
    /// format must share a name — pooling that silently reindexes is worse than none, since
    /// the second cell then displays through the *other* format in the document.
    #[test]
    fn identical_formats_pool_into_one_style_and_different_ones_do_not() {
        let mut doc = Document::default();
        let mut percent = Format::new(Kind::Percentage);
        percent.push(Part::Number {
            decimals: 1,
            min_decimals: 1,
            min_int: 1,
            grouping: false,
        });
        let sheet = doc.sheet_mut(0).unwrap();
        for row in 0..3 {
            sheet.set(Pos::new(row, 0), CellValue::Number(1.0));
        }
        sheet.set_format(Pos::new(0, 0), money());
        sheet.set_format(Pos::new(1, 0), percent);
        sheet.set_format(Pos::new(2, 0), money());

        let xml = flat(&doc);
        assert_eq!(xml.matches("<style:style ").count(), 2, "{xml}");
        assert_eq!(xml.matches("table:style-name=\"ce0\"").count(), 2, "{xml}");
        assert_eq!(xml.matches("table:style-name=\"ce1\"").count(), 1, "{xml}");
        // The link from cell to format is the only construct ODF has (§5.3).
        assert!(
            xml.contains(
                "<style:style style:name=\"ce0\" style:family=\"table-cell\" \
                          style:data-style-name=\"N0\"/>"
            ),
            "{xml}"
        );
    }

    /// A formatted cell carries its display text, and the value beside it is untouched —
    /// the format is display only (§5.2).
    #[test]
    fn a_formatted_cell_carries_display_text_next_to_its_real_value() {
        let mut doc = Document::default();
        let sheet = doc.sheet_mut(0).unwrap();
        sheet.set(Pos::new(0, 0), CellValue::Number(1234.5));
        sheet.set_format(Pos::new(0, 0), money());

        let xml = flat(&doc);
        assert!(xml.contains("office:value=\"1234.5\""), "{xml}");
        assert!(xml.contains("<text:p>1,234.50 \u{20ac}</text:p>"), "{xml}");
    }

    /// A document with no formats must not gain a style section or its namespaces (§1.4).
    #[test]
    fn a_document_without_formats_declares_no_style_namespaces() {
        let xml = flat(&Document::default());
        assert!(!xml.contains("automatic-styles"), "{xml}");
        assert!(!xml.contains("xmlns:number"), "{xml}");
    }

    #[test]
    fn the_package_starts_with_an_uncompressed_mimetype_entry() {
        let bytes = write_package_with(
            MIMETYPE,
            &content(&Document::default(), Form::Package, &HashSet::new(), None).0,
            None,
            &[],
        )
        .unwrap();
        // Readers sniff this at a fixed offset without unzipping anything (§1.1): local
        // header is 30 bytes, then the name, then the raw media type. Compression method
        // (offset 8) must be 0 = stored, and the extra-field length (offset 28) zero.
        assert_eq!(&bytes[..4], b"PK\x03\x04");
        assert_eq!(
            &bytes[8..10],
            &[0, 0],
            "mimetype must be stored, not deflated"
        );
        assert_eq!(
            &bytes[28..30],
            &[0, 0],
            "mimetype entry must carry no extra field"
        );
        assert_eq!(&bytes[30..38], b"mimetype");
        assert_eq!(&bytes[38..38 + MIMETYPE.len()], MIMETYPE.as_bytes());
    }
}
