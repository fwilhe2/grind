// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The ODS content model: one context per element read (`doc/ods-format.md` §3). **\[ODS\]**
//! Everything else falls to `context::Ignore`. Contexts never talk to each other; all mutation
//! goes through the shared [`Builder`].

use std::collections::{BTreeSet, HashMap};

use crate::filter::Filter;
use crate::formula::date;
use crate::locale::Locale;
use crate::model::{CellValue, Document, NumberKind, Pos, Sheet};
use crate::numfmt::{self, Format, Kind, Map, Op, Part};
use crate::style::{CellStyle, EDGES};

use super::context::{Attrs, Context};
use super::names::{Name, Ns};
use super::source;

/// The sheet limits, which are also the clamp for every repeat count (§9).
use crate::{MAX_COLS, MAX_ROWS};

/// Ceiling on non-empty cells written across the whole document. Clamping each repeat count (§9)
/// bounds the counts but not their product: a row repeated 1 048 576 times holding a cell repeated
/// 16 384 times is legal under both clamps and asks for seventeen billion writes.
const MAX_MATERIALISED_CELLS: u64 = 4_000_000;

/// How long a run of equally sized columns or rows is still *layout* rather than the
/// sheet's background width. Shared with the setting side — see [`crate::MAX_TRACK_RUN`].
use crate::MAX_TRACK_RUN;

/// Everything the contexts share: the document under construction and the cursors into it.
pub struct Builder {
    pub doc: Document,
    sheet: usize,
    /// Next row to write in the current sheet.
    row: u32,
    /// Next column to write in the current row.
    col: u32,
    /// Cells of the row being read, held back until the row ends because
    /// `table:number-rows-repeated` may ask for the whole row again.
    row_cells: Vec<RowCell>,
    /// `table:number-rows-repeated` of the row being read. One element there stands for many
    /// rows, so none of its cells can be spliced — see [`crate::odf::source`].
    row_repeat: u32,
    /// Where each cell element of the row being read sits in the file (R6) — one entry per
    /// element, where `row_cells` has one per address.
    row_spans: Vec<super::source::Cell>,
    /// Text accumulated by the paragraph contexts under the current cell.
    text: String,
    /// Cells left in the materialisation budget. See [`MAX_MATERIALISED_CELLS`].
    budget: u64,

    // --- styles (§5) ---
    /// `number:*-style` by `style:name`.
    number_styles: HashMap<String, Format>,
    /// A `table-cell` `style:style`'s name to its `style:data-style-name` — the one link
    /// between "how it looks" and "how the number renders" (§5.1).
    cell_styles: HashMap<String, String>,
    /// The same styles' own properties: fonts, colours, borders, alignment.
    style_props: HashMap<String, CellStyle>,
    /// Each `table-cell` style's `style:parent-style-name`. A style's look is its parent's with
    /// its own properties laid over it (§5.1), so every lookup walks this chain.
    style_parents: HashMap<String, String>,
    /// The properties a style sets to their *nothing* spelling — `fo:border="none"`, a zero
    /// rotation — which the model stores as an absent value and which must still override a
    /// parent that sets one.
    style_resets: HashMap<String, Resets>,
    /// The parts of the `number:*-style` being read, and the text its literal pieces collect —
    /// here because a child has no channel back to its parent, which takes these at its `end`.
    parts: Vec<Part>,
    style_text: String,
    /// `style:map` branches whose target has only been *named* so far — the style that owns
    /// the map, the condition, and the name it applies. A target may be declared after the
    /// style that points at it, so resolution waits until the section ends.
    pending_maps: Vec<(String, Op, String, String)>,
    /// Each `table-cell` style's conditional-format `style:map`s, in order — `(condition,
    /// applied style, base cell address)`, verbatim (`doc/ods-format.md` §3.6).
    cell_maps: HashMap<String, Vec<(String, String, String)>>,
    /// The cells of the current sheet whose style carries a map, as rectangles `[first row, end
    /// row, first column, end column)` by style name, and the order those styles were first met —
    /// rectangles, since a rule over a whole column is a million cells.
    rule_rects: HashMap<String, Vec<[u32; 4]>>,
    rule_styles: Vec<String>,
    /// `table:default-cell-style-name` per column of the current sheet, and for the current row. A
    /// cell's own style wins over the row's, which wins over the column's (§5.1).
    col_styles: Vec<Option<String>>,
    row_style: Option<String>,
    /// Next column to be claimed by a `table:table-column` declaration.
    col_decl: u32,
    /// A `table-column` or `table-row` `style:style`'s name to the one property this model
    /// keeps of it: `style:column-width` or `style:row-height`, verbatim. One map for both
    /// families, because a `style:name` is unique across the document (§5.1).
    track_sizes: HashMap<String, String>,
    /// `style:row-height` of the row being read, applied when the row ends because
    /// `table:number-rows-repeated` decides how many rows it is for.
    row_size: Option<String>,
    /// `table:visibility="collapse"` on the row being read: hidden by hand. `"filter"` is the
    /// filter's, which [`crate::filter`] re-derives rather than trusts.
    row_hidden: bool,

    // --- the autofilter (§9.4) ---
    /// The `table:database-range` being read and the sheet it covers, until its element
    /// ends. Here rather than in the context because its conditions are two levels down.
    filter: Option<(usize, Filter)>,
    /// The `table:filter-condition` being read: its field number and the values so far.
    filter_values: (u32, BTreeSet<String>),
    /// The conditions of a `table:filter-or` being read, held until it ends: kept only when
    /// every one names the same column. `None` outside one.
    filter_or: Option<Vec<(u32, BTreeSet<String>)>>,

    // --- charts (`doc/chart-format.md`) ---
    /// The chart being assembled out of the `draw:frame`/`draw:object` currently open, if any.
    pending_chart: Option<PendingChart>,
    /// The checkboxes of the table being read, by the identifier its cells' `draw:control`s
    /// name — `office:forms` comes before the rows (rng:15958), so every one is known by the
    /// time a cell points at it.
    controls: HashMap<String, crate::model::Checkbox>,
    /// Checkboxes drawn in the table being read, placed when it ends: `(cell, how far below
    /// its top the control's middle is, in mm, the checkbox)`. Placed then rather than at once
    /// because a control's offset can carry it into the next row, whose height is not known
    /// yet (`Sheet::row_holding`).
    placed: Vec<(Pos, f64, String, crate::model::Checkbox)>,
    /// The original bytes when they are a package, for resolving a chart's `xlink:href` to its own
    /// part (`Object 1/content.xml`).
    package: Option<Vec<u8>>,
    /// The package directories of the charts read into the model (`Object 1`, …), which a
    /// regenerating save replaces; every other directory stays.
    pub chart_parts: Vec<String>,
}

/// One chart (`draw:frame`/`draw:object`) being read: position from the frame, everything else
/// from its own document, however that is reached (`doc/chart-format.md`). Becomes a
/// [`crate::chart::Chart`] when `draw:frame` closes, or is dropped if a required piece never
/// arrived (§9).
struct PendingChart {
    kind: Option<crate::chart::ChartKind>,
    categories: Option<String>,
    series: Vec<crate::chart::Series>,
    x: Option<String>,
    y: Option<String>,
    width: Option<String>,
    height: Option<String>,
    x_axis: crate::chart::Axis,
    y_axis: crate::chart::Axis,
    /// The chart's own `style:family="chart"` styles, name → `draw:fill-color`, collected before
    /// `chart:plot-area` (the schema puts automatic styles first) so a series' or data point's
    /// `chart:style-name` resolves when seen.
    styles: HashMap<String, String>,
    /// Name → `chart:display-label`, which an axis' `chart:style-name` resolves against. A
    /// separate table because it lives on `style:chart-properties`, not
    /// `style:graphic-properties`.
    display_labels: HashMap<String, bool>,
    /// Name → `chart:reverse-direction` (rng:10064): a pie's direction (`doc/chart-format.md`,
    /// Direction).
    reverse_directions: HashMap<String, bool>,
    /// What the y axis' own style said about its direction, if anything — `None` being "said
    /// nothing", which for a pie is LibreOffice's counter-clockwise.
    y_reversed: Option<bool>,
    /// `chart:chart`'s own title and legend (rng:466, rng:475).
    title: Option<String>,
    legend: Option<crate::chart::Legend>,
    /// The package directory the chart's document was read from, when it was a separate part.
    part: Option<String>,
}

impl Default for PendingChart {
    fn default() -> Self {
        PendingChart {
            kind: None,
            categories: None,
            series: Vec::new(),
            x: None,
            y: None,
            width: None,
            height: None,
            // `Axis::bare`, not `Axis::default`: an axis a file says nothing about differs from
            // one a user said nothing about (see `crate::chart::Axis::bare`).
            x_axis: crate::chart::Axis::bare(),
            y_axis: crate::chart::Axis::bare(),
            styles: HashMap::new(),
            display_labels: HashMap::new(),
            reverse_directions: HashMap::new(),
            y_reversed: None,
            title: None,
            legend: None,
            part: None,
        }
    }
}

/// One cell of the buffered row.
struct RowCell {
    col: u32,
    value: CellValue,
    formula: Option<String>,
    kind: Option<NumberKind>,
    format: Option<Format>,
    style: Option<CellStyle>,
}

impl Builder {
    pub fn new() -> Self {
        Self {
            // Sheets come from the file: a document declaring none stays empty rather than getting
            // "Sheet1".
            doc: Document {
                sheets: Vec::new(),
                names: Default::default(),
                null_date: date::DEFAULT_NULL_DATE,
                null_year: date::DEFAULT_NULL_YEAR,
                locale: None,
                source: None,
                projection_source: None,
                edits: Default::default(),
            },
            sheet: 0,
            row: 0,
            col: 0,
            row_cells: Vec::new(),
            row_repeat: 1,
            row_spans: Vec::new(),
            text: String::new(),
            budget: MAX_MATERIALISED_CELLS,
            number_styles: HashMap::new(),
            cell_styles: HashMap::new(),
            style_props: HashMap::new(),
            style_parents: HashMap::new(),
            style_resets: HashMap::new(),
            parts: Vec::new(),
            style_text: String::new(),
            pending_maps: Vec::new(),
            cell_maps: HashMap::new(),
            rule_rects: HashMap::new(),
            rule_styles: Vec::new(),
            col_styles: Vec::new(),
            row_style: None,
            col_decl: 0,
            track_sizes: HashMap::new(),
            row_size: None,
            row_hidden: false,
            filter: None,
            filter_values: Default::default(),
            filter_or: None,
            pending_chart: None,
            controls: HashMap::new(),
            placed: Vec::new(),
            package: None,
            chart_parts: Vec::new(),
        }
    }

    /// The whole extent of an element of the source, given its start tag's span.
    fn extent(&self, start_tag: std::ops::Range<usize>) -> Option<std::ops::Range<usize>> {
        let source = self.doc.source.as_deref()?;
        grind_core::odf::xml::element_extent(&source.bytes, start_tag)
    }

    /// Record the package this document is being read from, so a chart's `xlink:href` can
    /// later be resolved against it. Called from `odf::read` before parsing starts.
    pub fn set_package(&mut self, bytes: Vec<u8>) {
        self.package = Some(bytes);
    }

    /// A part of the package this document was read from, by path — `None` for the flat form
    /// and for anything the archive does not actually hold.
    fn resolve_part(&self, path: &str) -> Option<Vec<u8>> {
        super::package::part(self.package.as_deref()?, path)
    }

    fn start_sheet(&mut self, name: String) {
        self.doc.sheets.push(Sheet::new(name));
        self.sheet = self.doc.sheets.len() - 1;
        self.row = 0;
        self.col_styles.clear();
        self.col_decl = 0;
    }

    /// The number format a cell displays through, by §5.1's indirection: the cell style names a
    /// data style. A style naming none, or one the document does not define, leaves the cell
    /// unformatted (§9).
    fn resolve_format(&self, cell_style: Option<&str>, col: u32) -> Option<Format> {
        let style = self.style_name(cell_style, col)?;
        // The nearest style in the parent chain that names a data style decides it.
        let data_style = self
            .lineage(style)
            .into_iter()
            .find_map(|name| self.cell_styles.get(name))?;
        self.number_styles.get(data_style).cloned()
    }

    /// The cell's own styling, through the same chain.
    fn resolve_style(&self, cell_style: Option<&str>, col: u32) -> Option<CellStyle> {
        let style = self.style_name(cell_style, col)?;
        self.inherited(style)
    }

    /// A style and its ancestors, nearest first. A chain that loops or runs deeper than any
    /// real document's is cut short rather than followed (§9).
    fn lineage<'a>(&'a self, name: &'a str) -> Vec<&'a str> {
        let mut chain = Vec::new();
        let mut at = Some(name);
        while let Some(name) = at {
            if chain.len() == 32 || chain.contains(&name) {
                break;
            }
            chain.push(name);
            at = self.style_parents.get(name).map(String::as_str);
        }
        chain
    }

    /// What a cell styled `name` looks like: the farthest ancestor first, each nearer style
    /// laid over it. `None` when nothing in the chain sets anything.
    fn inherited(&self, name: &str) -> Option<CellStyle> {
        let mut look = CellStyle::default();
        for name in self.lineage(name).into_iter().rev() {
            if let Some(own) = self.style_props.get(name) {
                look = own.over(Some(&look));
            }
            if let Some(resets) = self.style_resets.get(name) {
                resets.apply(&mut look);
            }
        }
        (!look.is_plain()).then_some(look)
    }

    /// Which `style:style` applies: the cell's own, else the row's default, else the column's —
    /// consulted only for cells that exist, so a styled column costs its cells, not its million
    /// rows.
    fn style_name<'a>(&'a self, cell_style: Option<&'a str>, col: u32) -> Option<&'a str> {
        cell_style
            .or(self.row_style.as_deref())
            .or_else(|| self.col_styles.get(col as usize)?.as_deref())
    }

    /// Attach every `style:map` collected in this section to the style that declared it — at the
    /// section's end, since a map may name a later style, and against a snapshot so a branch never
    /// gains branches of its own (one level is what LibreOffice writes and [`Format::render`]
    /// follows).
    fn resolve_maps(&mut self) {
        let targets = self.number_styles.clone();
        for (owner, op, value, target) in std::mem::take(&mut self.pending_maps) {
            let (Some(format), Some(owner)) =
                (targets.get(&target), self.number_styles.get_mut(&owner))
            else {
                continue; // A map naming a style the document does not define is dropped.
            };
            let mut format = format.clone();
            // A branch that names no locale of its own follows the style that points at it,
            // or a mapped negative would print with different separators from the positive.
            if format.locale.is_none() {
                format.locale.clone_from(&owner.locale);
            }
            owner.maps.push(Map { op, value, format });
        }
    }

    /// Note the cells `start..start + repeat` of the row being read as carrying their style's
    /// conditional-format maps, if it has any.
    fn note_rule_cells(&mut self, own: Option<&str>, start: u32, repeat: u32) {
        if self.cell_maps.is_empty() {
            return;
        }
        let Some(style) = self.style_name(own, start) else {
            return;
        };
        if !self.cell_maps.contains_key(style) {
            return;
        }
        let style = style.to_owned();
        let rows = self.row.min(MAX_ROWS)..self.row.saturating_add(self.row_repeat).min(MAX_ROWS);
        let cols = start.min(MAX_COLS)..start.saturating_add(repeat).min(MAX_COLS);
        if rows.is_empty() || cols.is_empty() {
            return;
        }
        let rects = self.rule_rects.entry(style.clone()).or_default();
        if rects.is_empty() {
            self.rule_styles.push(style);
        }
        rects.push([rows.start, rows.end, cols.start, cols.end]);
    }

    /// The current sheet's rules, out of the cells that carried a map (`doc/ods-format.md` §3.6):
    /// one rule per distinct (condition, applied style, base cell), LibreOffice's identity for
    /// one, over every cell whose style named it. Ordered by each style's map order, then by first
    /// cell met.
    fn finish_rules(&mut self) {
        let rects = std::mem::take(&mut self.rule_rects);
        let styles = std::mem::take(&mut self.rule_styles);
        let Some(own) = self.doc.sheets.get(self.sheet).map(|s| s.name.clone()) else {
            return;
        };
        let mut keys: Vec<(String, String, Pos)> = Vec::new();
        let mut cells: Vec<Vec<(Pos, Pos)>> = Vec::new();
        let mut before: Vec<(usize, usize)> = Vec::new();
        for style in &styles {
            let mut chain = Vec::new();
            for (condition, apply, base) in self.cell_maps.get(style).into_iter().flatten() {
                let Some(base) = crate::model::Link::parse(base, &own)
                    .filter(|link| link.sheet.is_none())
                    .map(|link| link.pos)
                else {
                    continue;
                };
                let Some(condition) = crate::rule::from_odf_condition(condition, base) else {
                    continue;
                };
                let key = (condition, apply.clone(), base);
                let at = match keys.iter().position(|k| *k == key) {
                    Some(at) => at,
                    None => {
                        keys.push(key);
                        cells.push(Vec::new());
                        keys.len() - 1
                    }
                };
                cells[at].extend(
                    rects[style]
                        .iter()
                        .map(|[r0, r1, c0, c1]| (Pos::new(*r0, *c0), Pos::new(*r1 - 1, *c1 - 1))),
                );
                chain.push(at);
            }
            before.extend(chain.windows(2).map(|w| (w[0], w[1])));
        }
        let order = crate::rule::priority(keys.len(), &before);
        let rules = order
            .into_iter()
            .map(|i| {
                let (condition, apply, base) = &keys[i];
                crate::rule::Rule {
                    ranges: crate::rule::rectangles(std::mem::take(&mut cells[i])),
                    base: *base,
                    condition: condition.clone(),
                    style: self.inherited(apply).unwrap_or_default(),
                }
            })
            .collect();
        if let Some(sheet) = self.doc.sheets.get_mut(self.sheet) {
            sheet.set_rules(rules);
        }
    }

    /// Claim `repeat` columns for a `table:table-column` declaration, recording the default
    /// cell style it gives them.
    fn declare_columns(
        &mut self,
        style: Option<String>,
        width: Option<&str>,
        hidden: bool,
        repeat: u32,
    ) {
        let end = self.col_decl.saturating_add(repeat).min(MAX_COLS);
        // Each column's default cell style as the file named it, for a save that rewrites the
        // column declarations (`source::Table::column_defaults`).
        if let Some(table) = self
            .doc
            .source
            .as_deref_mut()
            .and_then(|source| source.tables.last_mut())
        {
            table
                .column_defaults
                .push((self.col_decl..end, style.clone()));
        }
        // A column that names no default style still has to occupy its slots, or every
        // later declaration lands on the wrong column.
        if style.is_some() {
            self.col_styles.resize(end as usize, None);
            for slot in &mut self.col_styles[self.col_decl as usize..end as usize] {
                slot.clone_from(&style);
            }
        }
        if repeat <= MAX_TRACK_RUN
            && let Some(sheet) = self.doc.sheets.get_mut(self.sheet)
        {
            for col in self.col_decl..end {
                if let Some(width) = width {
                    sheet.set_col_width(col, Some(width.to_owned()));
                }
                if hidden {
                    sheet.set_col_hidden(col, true);
                }
            }
        }
        self.col_decl = end;
    }

    /// `table:number-columns-spanned`/`table:number-rows-spanned` on the cell starting at `col` of
    /// the row being read (rng:16102) — a merge, anchored there.
    ///
    /// A span past the sheet is cut at its edge, and one overlapping an earlier merge replaces it.
    /// A repeated row holding a one-row merge repeats the merge, under a repeated row's bound; a
    /// taller merge is anchored on the first row only.
    fn merge(&mut self, col: u32, attrs: &Attrs) {
        let cols = attrs.count(Ns::Table, "number-columns-spanned", MAX_COLS);
        let rows = attrs.count(Ns::Table, "number-rows-spanned", MAX_ROWS);
        if cols <= 1 && rows <= 1 {
            return;
        }
        let first = self.row;
        let Some(sheet) = self.doc.sheets.get_mut(self.sheet) else {
            return;
        };
        let repeat = match rows {
            1 if self.row_repeat <= MAX_TRACK_RUN => self.row_repeat,
            _ => 1,
        };
        for r in 0..repeat {
            let row = first.saturating_add(r);
            if row >= MAX_ROWS {
                break;
            }
            let span = crate::model::Span {
                cols: cols.min(MAX_COLS - col),
                rows: rows.min(MAX_ROWS - row),
            };
            let anchor = Pos::new(row, col);
            for (other, _) in sheet.merges_within(anchor, span.end(anchor)) {
                sheet.set_merge(other, None);
            }
            sheet.set_merge(anchor, Some(span));
        }
    }

    /// Write the buffered row `repeat` times, then advance the row cursor. A trailing
    /// `<table:table-row table:number-rows-repeated="1048576">` with one empty cell bounds most
    /// real sheets (§3.3); with nothing buffered it costs one addition, not a million iterations.
    fn finish_row(&mut self, repeat: u32) {
        // R6: hand this row's cell elements to the source only when the row stands for itself.
        let spans = std::mem::take(&mut self.row_spans);
        if repeat == 1
            && !spans.is_empty()
            && let Some(source) = self.doc.source.as_mut()
        {
            source.rows.insert((self.sheet, self.row), spans);
        }

        // A height or hidden flag applies whether or not the row holds anything.
        let height = self.row_size.take();
        let hidden = std::mem::take(&mut self.row_hidden);
        if (height.is_some() || hidden)
            && repeat <= MAX_TRACK_RUN
            && let Some(sheet) = self.doc.sheets.get_mut(self.sheet)
        {
            for r in 0..repeat {
                let row = self.row.saturating_add(r);
                if row >= MAX_ROWS {
                    break;
                }
                if let Some(height) = &height {
                    sheet.set_row_height(row, Some(height.clone()));
                }
                if hidden {
                    sheet.set_row_hidden(row, true);
                }
            }
        }

        if self.row_cells.is_empty() {
            self.row = self.row.saturating_add(repeat).min(MAX_ROWS);
            return;
        }
        let Some(sheet) = self.doc.sheets.get_mut(self.sheet) else {
            self.row_cells.clear();
            return;
        };

        // How many copies of this row the budget allows. The cursor still advances by the full
        // repeat, so the rest of the sheet keeps its addresses.
        let per_row = self.row_cells.len() as u64;
        let affordable = (self.budget / per_row.max(1)).min(u64::from(repeat)) as u32;

        for r in 0..affordable {
            let row = self.row.saturating_add(r);
            if row >= MAX_ROWS {
                break;
            }
            for cell in &self.row_cells {
                let pos = Pos::new(row, cell.col);
                if !cell.value.is_empty() {
                    sheet.set(pos, cell.value.clone());
                }
                if let Some(f) = &cell.formula {
                    sheet.set_formula(pos, f.clone());
                }
                if let Some(k) = cell.kind {
                    sheet.set_kind(pos, k);
                }
                if let Some(f) = &cell.format {
                    sheet.set_format(pos, f.clone());
                }
                if let Some(s) = &cell.style {
                    sheet.set_style(pos, s.clone());
                }
            }
            self.budget = self.budget.saturating_sub(per_row);
        }

        self.row_cells.clear();
        self.row = self.row.saturating_add(repeat).min(MAX_ROWS);
    }
}

impl Default for Builder {
    fn default() -> Self {
        Self::new()
    }
}

type Ctx = Box<dyn Context<Builder>>;

/// The document root: `office:document` (flat) and `office:document-content` (a package's
/// `content.xml`) alike (§1.2).
pub struct Root;

impl Context<Builder> for Root {
    fn start_child(&mut self, name: &Name, _a: &Attrs, _b: &mut Builder) -> Option<Ctx> {
        match (name.ns, name.local.as_str()) {
            // `office:document-styles` is a package's `styles.xml` root, read with the same stack
            // (§1.1).
            (Ns::Office, "document" | "document-content" | "document-styles") => {
                Some(Box::new(Root))
            }
            (Ns::Office, "body") => Some(Box::new(Body)),
            // Both style sections hold the same elements and differ only in whether a human
            // named them (§5.1), so one context reads both.
            (Ns::Office, "automatic-styles" | "styles") => Some(Box::new(Styles)),
            _ => None,
        }
    }
}

struct Body;

impl Context<Builder> for Body {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        if !name.is(Ns::Office, "spreadsheet") {
            return None;
        }
        // Where the body's content is, for a save that splices into it (`source::Source::body`).
        if let Some(range) = b.extent(attrs.span())
            && let Some(source) = b.doc.source.as_deref_mut()
        {
            let close = source.bytes[range.clone()]
                .iter()
                .rposition(|c| *c == b'<')
                .map_or(range.end, |p| range.start + p);
            if attrs.span() != range {
                source.body = Some(attrs.span().end..close);
            }
        }
        Some(Box::new(Spreadsheet) as Ctx)
    }
}

struct Spreadsheet;

impl Context<Builder> for Spreadsheet {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        if !name.is(Ns::Table, "table") {
            // Every other child of the body, recorded with what it is, so a save that
            // regenerates the body carries it through (`source::Part`).
            let kind = match name.local.as_str() {
                _ if name.ns != Ns::Table => source::PartKind::Other,
                "calculation-settings" => source::PartKind::CalculationSettings,
                "named-expressions" => source::PartKind::NamedExpressions,
                "database-ranges" => source::PartKind::DatabaseRanges,
                _ => source::PartKind::Other,
            };
            if let Some(range) = b.extent(attrs.span())
                && let Some(source) = b.doc.source.as_deref_mut()
            {
                let after_tables = !source.tables.is_empty();
                source.parts.push(source::Part {
                    kind,
                    range,
                    after_tables,
                });
            }
        }
        if name.is(Ns::Table, "named-expressions") {
            return Some(Box::new(NamedExpressions { local: None }));
        }
        if name.is(Ns::Table, "database-ranges") {
            return Some(Box::new(DatabaseRanges));
        }
        if name.is(Ns::Table, "calculation-settings") {
            // `table:null-year` is an attribute here; `table:null-date` is a child element.
            if let Some(year) = attrs.get(Ns::Table, "null-year")
                && let Ok(year) = year.trim().parse::<i64>()
            {
                b.doc.null_year = year;
            }
            return Some(Box::new(CalculationSettings));
        }
        if !name.is(Ns::Table, "table") {
            return None;
        }
        let sheet_name = attrs.get(Ns::Table, "name").unwrap_or("Sheet").to_owned();
        b.start_sheet(sheet_name);
        // R6 one level up: where this table sits and what its start tag says, so a save that
        // regenerates the sheet splices it back into this element (`source::Table`).
        let range = b.extent(attrs.span());
        if let Some(source) = b.doc.source.as_deref_mut()
            && let Some(range) = range
        {
            let keep = source
                .bytes
                .get(attrs.span())
                .map(|tag| source::attributes(tag, &["table:name"]))
                .unwrap_or_default();
            let index = source.tables.len();
            source.tables.push(source::Table {
                range,
                start: attrs.span(),
                keep,
                ..Default::default()
            });
            if let Some(sheet) = b.doc.sheets.last_mut() {
                sheet.origin.table = Some(index);
            }
        }
        Some(Box::new(Table { top: true }))
    }
}

/// `table:named-expressions` (§5.11), which appears both here and inside a `table:table`
/// for sheet-local names. Both land in the same map — see [`Document::names`].
struct NamedExpressions {
    /// The `source::Table` this element sits in, for sheet-local names.
    local: Option<usize>,
}

impl Context<Builder> for NamedExpressions {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        let Some(key) = attrs.get(Ns::Table, "name") else {
            return Some(Box::new(super::context::Ignore));
        };
        if let Some(table) = self.local
            && let Some(source) = b.doc.source.as_deref_mut()
            && let Some((_, names)) = source.tables.get_mut(table).and_then(|t| t.names.as_mut())
        {
            names.push(key.to_lowercase());
        }
        // A named range carries a bare cell-range address and a named expression a formula;
        // storing the range as its bracketed reference gives the evaluator one kind of thing to
        // parse.
        let expression = if name.is(Ns::Table, "named-range") {
            attrs
                .get(Ns::Table, "cell-range-address")
                .map(|address| format!("[{address}]"))
        } else if name.is(Ns::Table, "named-expression") {
            attrs.get(Ns::Table, "expression").map(str::to_owned)
        } else {
            None
        };
        if let Some(expression) = expression {
            b.doc.names.insert(key.to_lowercase(), expression);
        }
        Some(Box::new(super::context::Ignore))
    }
}

/// `office:forms` and its `form:form`s: every `form:checkbox`, by identifier, into
/// [`Builder::controls`] (`doc/ods-format.md` §3.5). Other controls are kept in the file by R6,
/// not in the model.
struct Forms;

impl Context<Builder> for Forms {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        if name.is(Ns::Form, "form") {
            // R6: where a new checkbox can go — before the first form's end tag.
            if let Some(range) = b.extent(attrs.span())
                && let Some(source) = b.doc.source.as_deref_mut()
                && let Some(table) = source.tables.last_mut()
                && table.form_end.is_none()
            {
                let close = source.bytes[range.clone()]
                    .iter()
                    .rposition(|c| *c == b'<')
                    .map(|at| range.start + at);
                // A self-closed `<form:form/>` has no end tag to put anything before.
                table.form_end = close.filter(|at| *at > attrs.span().end - 1);
            }
            return Some(Box::new(Forms));
        }
        if !name.is(Ns::Form, "checkbox") {
            return None;
        }
        let id = attrs
            .get(Ns::Xml, "id")
            .or_else(|| attrs.get(Ns::Form, "id"))?;
        let own = b
            .doc
            .sheets
            .get(b.sheet)
            .map(|s| s.name.clone())
            .unwrap_or_default();
        let link = attrs
            .get(Ns::Form, "linked-cell")
            .and_then(|address| crate::model::Link::parse(address, &own));
        let checkbox = crate::model::Checkbox {
            // A linked checkbox's state is its cell's; LibreOffice mirrors it into
            // `form:current-state`, which says nothing the cell does not (§3.5).
            checked: link.is_none() && attrs.get(Ns::Form, "current-state") == Some("checked"),
            link,
            name: attrs.get(Ns::Form, "name").map(str::to_owned),
            label: attrs.get(Ns::Form, "label").map(str::to_owned),
        };
        if let Some(range) = b.extent(attrs.span())
            && let Some(table) = b
                .doc
                .source
                .as_deref_mut()
                .and_then(|source| source.tables.last_mut())
        {
            table.checkbox_elements.insert(id.to_owned(), range);
        }
        b.controls.insert(id.to_owned(), checkbox);
        None
    }
}

/// `table:shapes` (rng:15678) — a sheet's charts, and nothing else this build reads out of a
/// shape (`doc/chart-format.md`).
struct Shapes;

impl Context<Builder> for Shapes {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        if !name.is(Ns::Draw, "frame") {
            return None;
        }
        b.pending_chart = Some(PendingChart {
            x: attrs.get(Ns::Svg, "x").map(str::to_owned),
            y: attrs.get(Ns::Svg, "y").map(str::to_owned),
            width: attrs.get(Ns::Svg, "width").map(str::to_owned),
            height: attrs.get(Ns::Svg, "height").map(str::to_owned),
            ..Default::default()
        });
        Some(Box::new(Frame))
    }
}

/// `draw:frame` (rng:5088) — a chart's position. A frame that is not a chart (a picture, a text
/// box) gets no [`crate::chart::ChartKind`] and is dropped when it closes.
struct Frame;

impl Context<Builder> for Frame {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        if !name.is(Ns::Draw, "object") {
            return None;
        }
        // A package points at the chart's own part (`Object 1/content.xml`, rng:1621); the flat
        // form embeds an `office:document` inline (rng:5541).
        match attrs.get(Ns::Xlink, "href") {
            Some(href) => {
                let directory = href.trim_start_matches("./").trim_end_matches('/');
                let path = format!("{directory}/content.xml");
                if let Some(main) = &mut b.pending_chart {
                    main.part = Some(directory.to_owned());
                }
                if let Some(bytes) = b.resolve_part(&path) {
                    let mut sub = Builder::new();
                    sub.pending_chart = Some(PendingChart::default());
                    let _ = super::context::parse(
                        std::io::Cursor::new(bytes),
                        Box::new(ChartPartFile),
                        &mut sub,
                    );
                    if let (Some(main), Some(found)) = (&mut b.pending_chart, sub.pending_chart) {
                        main.kind = found.kind;
                        main.categories = found.categories;
                        main.series = found.series;
                        main.x_axis = found.x_axis;
                        main.y_axis = found.y_axis;
                        main.styles = found.styles;
                        main.display_labels = found.display_labels;
                        main.reverse_directions = found.reverse_directions;
                        main.y_reversed = found.y_reversed;
                        main.title = found.title;
                        main.legend = found.legend;
                    }
                }
                Some(Box::new(super::context::Ignore))
            }
            // The flat form's own shape: the chart's document lives right here.
            None => Some(Box::new(DrawObject)),
        }
    }

    fn end(&mut self, b: &mut Builder) {
        let Some(pending) = b.pending_chart.take() else {
            return;
        };
        let (Some(kind), Some(width), Some(height)) = (pending.kind, pending.width, pending.height)
        else {
            return;
        };
        if pending.series.is_empty() {
            return;
        }
        let chart = crate::chart::Chart {
            kind,
            categories: pending.categories,
            series: pending.series,
            x: pending.x.unwrap_or_else(|| "0cm".to_owned()),
            y: pending.y.unwrap_or_else(|| "0cm".to_owned()),
            width,
            height,
            x_axis: pending.x_axis,
            y_axis: pending.y_axis,
            // A pie runs as its file says, and a file that says nothing is LibreOffice's
            // counter-clockwise (`doc/chart-format.md`, Direction). A bar or line keeps a new
            // chart's direction.
            clockwise: match kind {
                crate::chart::ChartKind::Pie => pending.y_reversed == Some(true),
                _ => true,
            },
            title: pending.title,
            legend: pending.legend,
        };
        if let Some(part) = pending.part {
            if let Some(table) = b
                .doc
                .source
                .as_deref_mut()
                .and_then(|source| source.tables.last_mut())
            {
                table.charts.push(part.clone());
            }
            b.chart_parts.push(part);
        }
        let sheet = &mut b.doc.sheets[b.sheet];
        let index = sheet.charts().len();
        sheet.insert_chart(index, chart);
    }
}

/// The chart's *own* document, one level in — reached as `office:document`'s child when the
/// flat form embeds it right here, inline (`doc/chart-format.md`).
struct DrawObject;

impl Context<Builder> for DrawObject {
    fn start_child(&mut self, name: &Name, _attrs: &Attrs, _b: &mut Builder) -> Option<Ctx> {
        name.is(Ns::Office, "document")
            .then(|| Box::new(ChartPartRoot) as Ctx)
    }
}

/// The chart document's `office:body`, one level below its root in both forms — reached from
/// [`DrawObject`] (flat) and [`ChartPartFile`] (package).
struct ChartPartRoot;

impl Context<Builder> for ChartPartRoot {
    fn start_child(&mut self, name: &Name, _attrs: &Attrs, _b: &mut Builder) -> Option<Ctx> {
        match (name.ns, name.local.as_str()) {
            (Ns::Office, "body") => Some(Box::new(OfficeBody) as Ctx),
            (Ns::Office, "automatic-styles") => Some(Box::new(ChartAutomaticStyles) as Ctx),
            _ => None,
        }
    }
}

/// `office:automatic-styles`, inside a chart's own document — `style:style
/// style:family="chart"` entries only, collected into [`PendingChart::styles`].
struct ChartAutomaticStyles;

impl Context<Builder> for ChartAutomaticStyles {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, _b: &mut Builder) -> Option<Ctx> {
        if !name.is(Ns::Style, "style") || attrs.get(Ns::Style, "family") != Some("chart") {
            return None;
        }
        let name = attrs.get(Ns::Style, "name")?.to_owned();
        Some(Box::new(ChartStyleStyle { name }) as Ctx)
    }
}

/// One `style:style style:family="chart"`: a mark's `draw:fill-color` (rng:10941) and an axis'
/// `chart:display-label` (rng:10069) are read; the rest is read past.
struct ChartStyleStyle {
    name: String,
}

impl Context<Builder> for ChartStyleStyle {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        let Some(pending) = &mut b.pending_chart else {
            return None;
        };
        if name.is(Ns::Style, "graphic-properties")
            && let Some(color) = attrs.get(Ns::Draw, "fill-color")
        {
            pending.styles.insert(self.name.clone(), color.to_owned());
        }
        if name.is(Ns::Style, "chart-properties") {
            if let Some(display) = attrs.get(Ns::Chart, "display-label") {
                pending
                    .display_labels
                    .insert(self.name.clone(), display == "true");
            }
            if let Some(reverse) = attrs.get(Ns::Chart, "reverse-direction") {
                pending
                    .reverse_directions
                    .insert(self.name.clone(), reverse == "true");
            }
        }
        None
    }
}

/// `office:body` (rng:7687) — one recognised child, `office:chart` (rng:7717).
struct OfficeBody;

impl Context<Builder> for OfficeBody {
    fn start_child(&mut self, name: &Name, _attrs: &Attrs, _b: &mut Builder) -> Option<Ctx> {
        name.is(Ns::Office, "chart")
            .then(|| Box::new(OfficeChart) as Ctx)
    }
}

/// The root of the standalone parse over a package's chart part (`Object 1/content.xml`), which
/// starts at `office:document-content`.
struct ChartPartFile;

impl Context<Builder> for ChartPartFile {
    fn start_child(&mut self, name: &Name, _attrs: &Attrs, _b: &mut Builder) -> Option<Ctx> {
        name.is(Ns::Office, "document-content")
            .then(|| Box::new(ChartPartRoot) as Ctx)
    }
}

/// `office:chart` (rng:7717) — one child, `chart:chart`, and nothing this build reads directly
/// on `office:chart` itself.
struct OfficeChart;

impl Context<Builder> for OfficeChart {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        if !name.is(Ns::Chart, "chart") {
            return None;
        }
        if let Some(pending) = &mut b.pending_chart {
            pending.kind = attrs
                .get(Ns::Chart, "class")
                .and_then(crate::chart::ChartKind::from_class);
        }
        Some(Box::new(ChartChart))
    }
}

/// `chart:chart` (rng:463): the title and legend are read (`doc/chart-format.md`); a subtitle and
/// a footer are read past.
struct ChartChart;

impl Context<Builder> for ChartChart {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        match (name.ns, name.local.as_str()) {
            (Ns::Chart, "plot-area") => Some(Box::new(ChartPlotArea) as Ctx),
            (Ns::Chart, "title") => Some(Box::new(ChartTitle) as Ctx),
            (Ns::Chart, "legend") => {
                if let Some(pending) = &mut b.pending_chart {
                    pending.legend = Some(crate::chart::Legend::from_token(
                        attrs.get(Ns::Chart, "legend-position").unwrap_or("end"),
                    ));
                }
                None
            }
            _ => None,
        }
    }
}

/// `chart:chart`'s own `chart:title` (rng:934) — the same element an axis' title is, one level
/// up, and read the same way ([`ChartAxisTitle`]).
struct ChartTitle;

impl Context<Builder> for ChartTitle {
    fn start_child(&mut self, name: &Name, _attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        if !name.is(Ns::Text, "p") {
            return None;
        }
        b.text.clear();
        Some(Box::new(Paragraph::CHART) as Ctx)
    }

    fn end(&mut self, b: &mut Builder) {
        let text = std::mem::take(&mut b.text);
        let text = text.trim_end();
        if let Some(pending) = &mut b.pending_chart
            && !text.is_empty()
        {
            pending.title = Some(text.to_owned());
        }
    }
}

/// `chart:plot-area` (rng:776) — the x and y axes, and every series.
struct ChartPlotArea;

impl Context<Builder> for ChartPlotArea {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        match (name.ns, name.local.as_str()) {
            (Ns::Chart, "axis") => {
                let dim = match attrs.get(Ns::Chart, "dimension") {
                    Some("x") => AxisDim::X,
                    Some("y") => AxisDim::Y,
                    _ => return None,
                };
                // The axis' own style says whether its tick labels are drawn; saying nothing keeps
                // `crate::chart::Axis::default`'s answer.
                if let Some(display) = attrs
                    .get(Ns::Chart, "style-name")
                    .and_then(|name| b.pending_chart.as_ref()?.display_labels.get(name).copied())
                    && let Some(pending) = &mut b.pending_chart
                {
                    dim.axis(pending).tick_labels = display;
                }
                if let AxisDim::Y = dim
                    && let Some(pending) = &mut b.pending_chart
                {
                    pending.y_reversed = attrs
                        .get(Ns::Chart, "style-name")
                        .and_then(|name| pending.reverse_directions.get(name).copied());
                }
                Some(Box::new(ChartAxis { dim }) as Ctx)
            }
            (Ns::Chart, "series") => {
                let values = attrs
                    .get(Ns::Chart, "values-cell-range-address")?
                    .to_owned();
                let label = attrs
                    .get(Ns::Chart, "label-cell-address")
                    .map(str::to_owned);
                let series_style = attrs.get(Ns::Chart, "style-name").map(str::to_owned);
                let (index, kind) = {
                    let pending = b.pending_chart.as_ref()?;
                    (pending.series.len(), pending.kind)
                };
                // A bar or line has one colour per series, on the series' own style
                // (`doc/chart-format.md`, Colour); a pie's are its slices', read from each
                // `chart:data-point`. A colour equal to the default cycle at its position counts
                // as unset ([`crate::chart::effective_color`]).
                let own = series_style.as_deref().and_then(|name| {
                    let pending = b.pending_chart.as_ref()?;
                    pending.styles.get(name).cloned()
                });
                let color = match kind {
                    Some(crate::chart::ChartKind::Bar | crate::chart::ChartKind::Line) => own
                        .as_deref()
                        .filter(|hex| *hex != crate::chart::series_color(index))
                        .map(str::to_owned),
                    _ => None,
                };
                // What a data point's colour is compared against to tell an override from a
                // default. A pie's slices, and a bar under a series naming no style, cycle per
                // point — the latter is how bars were written before colours belonged to series,
                // and such a file must not read every old default as a chosen colour.
                let points = match (kind, &own) {
                    (Some(crate::chart::ChartKind::Pie), _)
                    | (Some(crate::chart::ChartKind::Bar), None) => PointDefault::Cycle,
                    (Some(crate::chart::ChartKind::Bar), Some(_)) => PointDefault::Series(
                        color
                            .clone()
                            .unwrap_or_else(|| crate::chart::series_color(index).to_owned()),
                    ),
                    _ => PointDefault::NoPoints,
                };
                let pending = b.pending_chart.as_mut()?;
                pending.series.push(crate::chart::Series {
                    values,
                    label,
                    color,
                    point_colors: Vec::new(),
                });
                Some(Box::new(ChartSeries {
                    series_index: index,
                    point: 0,
                    points,
                }) as Ctx)
            }
            _ => None,
        }
    }
}

/// A `chart:series`' `chart:data-point` children (rng:552) — colour only, into
/// [`crate::chart::Series::point_colors`] for `Bar`/`Pie`, each compared with its
/// [`PointDefault`].
struct ChartSeries {
    series_index: usize,
    point: usize,
    points: PointDefault,
}

/// What a data point of one series is coloured when nobody picked it a colour.
enum PointDefault {
    /// The default cycle at the point's own position — a pie's slice, or a bar written before
    /// a colour was a series.
    Cycle,
    /// The series' own colour — a bar.
    Series(String),
    /// A line's points carry no colour of their own.
    NoPoints,
}

impl Context<Builder> for ChartSeries {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        if !name.is(Ns::Chart, "data-point") {
            return None;
        }
        let repeated: usize = attrs
            .get(Ns::Chart, "repeated")
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(1)
            .max(1);
        let default = match &self.points {
            PointDefault::Cycle => Some(crate::chart::series_color(self.point)),
            PointDefault::Series(hex) => Some(hex.as_str()),
            PointDefault::NoPoints => None,
        };
        if let Some(default) = default
            && let Some(style_name) = attrs.get(Ns::Chart, "style-name")
            && let Some(pending) = &mut b.pending_chart
            && let Some(hex) = pending.styles.get(style_name).cloned()
            && hex != default
            && let Some(series) = pending.series.get_mut(self.series_index)
        {
            if series.point_colors.len() <= self.point {
                series.point_colors.resize(self.point + 1, None);
            }
            series.point_colors[self.point] = Some(hex);
        }
        self.point += repeated;
        None
    }
}

/// Which axis a [`ChartAxis`]/[`ChartAxisTitle`] belongs to.
#[derive(Clone, Copy)]
enum AxisDim {
    X,
    Y,
}

impl AxisDim {
    /// The half of the chart being read that this dimension writes into.
    fn axis(self, pending: &mut PendingChart) -> &mut crate::chart::Axis {
        match self {
            AxisDim::X => &mut pending.x_axis,
            AxisDim::Y => &mut pending.y_axis,
        }
    }
}

/// `chart:axis` (rng:423) — categories (x only), a title, and gridlines (either axis).
struct ChartAxis {
    dim: AxisDim,
}

impl Context<Builder> for ChartAxis {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        match (name.ns, name.local.as_str(), self.dim) {
            (Ns::Chart, "categories", AxisDim::X) => {
                if let Some(pending) = &mut b.pending_chart {
                    pending.categories = attrs
                        .get(Ns::Table, "cell-range-address")
                        .map(str::to_owned);
                }
                None
            }
            (Ns::Chart, "title", _) => Some(Box::new(ChartAxisTitle { dim: self.dim }) as Ctx),
            // A minor grid is read as a grid: one set of gridlines is drawn, and a save writes the
            // major one.
            (Ns::Chart, "grid", _) => {
                if let Some(pending) = &mut b.pending_chart {
                    self.dim.axis(pending).gridlines = true;
                }
                None
            }
            _ => None,
        }
    }
}

/// An axis' own `chart:title` (rng:934) — plain text via [`Paragraph`]. Distinct from
/// `chart:chart`'s own title ([`ChartTitle`]), though the element is the same.
struct ChartAxisTitle {
    dim: AxisDim,
}

impl Context<Builder> for ChartAxisTitle {
    fn start_child(&mut self, name: &Name, _attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        if !name.is(Ns::Text, "p") {
            return None;
        }
        b.text.clear();
        Some(Box::new(Paragraph::CHART) as Ctx)
    }

    fn end(&mut self, b: &mut Builder) {
        let text = std::mem::take(&mut b.text);
        // The trailing half of the white-space rule `Paragraph::CHART` applies.
        let text = text.trim_end();
        if text.is_empty() {
            return;
        }
        if let Some(pending) = &mut b.pending_chart {
            self.dim.axis(pending).label = Some(text.to_owned());
        }
    }
}

/// `table:calculation-settings`, for `table:null-date`, the epoch (Part 4 §3.4 item 8). The schema
/// puts it before the tables. The rest (iteration, case sensitivity, wildcards) stays in the file,
/// unread.
struct CalculationSettings;

impl Context<Builder> for CalculationSettings {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        if name.is(Ns::Table, "null-date")
            && let Some(value) = attrs.get(Ns::Table, "date-value")
            // Parsed against 1970-01-01 rather than against the epoch being defined, which
            // would be circular. An unparseable date leaves the default in place (§9).
            && let Some(days) = date::parse_date(value, 0)
        {
            b.doc.null_date = days as i64;
        }
        Some(Box::new(super::context::Ignore))
    }
}

/// `table:database-ranges` (§9.4) — the autofilters, one per sheet at most, addressed by name
/// (`Sheet1.A1:Sheet1.F12`) and parsed with [`crate::a1`]. The schema puts it after the tables.
struct DatabaseRanges;

impl Context<Builder> for DatabaseRanges {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        if !name.is(Ns::Table, "database-range") {
            return None;
        }
        let address = attrs.get(Ns::Table, "target-range-address")?;
        let reference = crate::a1::parse(address).ok()?;
        let end = reference.end.clone().unwrap_or(reference.start.clone());
        // A whole column or row as a filter range is not a shape LibreOffice writes, and
        // guessing an extent for one would be inventing the filter's meaning.
        let (start, end) = (cell_pos(&reference.start)?, cell_pos(&end)?);
        let sheet = reference.start.sheet.as_deref()?;
        let sheet = b
            .doc
            .sheets
            .iter()
            .position(|s| s.name.eq_ignore_ascii_case(sheet))?;
        let mut filter = Filter::new(attrs.get(Ns::Table, "name").unwrap_or_default(), start, end);
        // Both default to true (§9.4).
        filter.contains_header = attrs.get(Ns::Table, "contains-header") != Some("false");
        filter.buttons = attrs.get(Ns::Table, "display-filter-buttons") != Some("false");
        b.filter = Some((sheet, filter));
        Some(Box::new(DatabaseRange))
    }
}

/// A `CellRef` as a position, when it names both axes.
fn cell_pos(cell: &crate::formula::lex::CellRef) -> Option<Pos> {
    Some(Pos::new(cell.row?.index, cell.col?.index))
}

struct DatabaseRange;

impl Context<Builder> for DatabaseRange {
    fn start_child(&mut self, name: &Name, _a: &Attrs, _b: &mut Builder) -> Option<Ctx> {
        // `table:filter` and `table:filter-and` only wrap the conditions; `table:filter-or` counts
        // only when all its conditions name one column ([`FilterOr`]).
        name.is(Ns::Table, "filter")
            .then(|| Box::new(FilterAnd) as Ctx)
    }

    fn end(&mut self, b: &mut Builder) {
        if let Some((sheet, filter)) = b.filter.take()
            && let Some(sheet) = b.doc.sheets.get_mut(sheet)
        {
            sheet.set_filter(Some(filter));
        }
    }
}

struct FilterAnd;

impl Context<Builder> for FilterAnd {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        match (name.ns, name.local.as_str()) {
            (Ns::Table, "filter-and") => Some(Box::new(FilterAnd)),
            (Ns::Table, "filter-or") if b.filter_or.is_none() => {
                b.filter_or = Some(Vec::new());
                Some(Box::new(FilterOr { readable: true }))
            }
            (Ns::Table, "filter-condition") => {
                // Only a set of values. Anything else — `<`, `begins-with`, top-10 — is
                // dropped rather than half-applied (see [`crate::filter`]).
                if attrs.get(Ns::Table, "operator") != Some("=") {
                    return None;
                }
                // A `data-type` outside ODF's namespace is LibreOffice's filter by colour
                // (`doc/ods-format.md` §6): read as a value set, `#e8f2a1` would hide every row.
                if attrs
                    .names()
                    .any(|n| n.ns != Ns::Table && n.local == "data-type")
                {
                    return None;
                }
                let field = attrs.get(Ns::Table, "field-number")?.parse().ok()?;
                b.filter_values = (
                    field,
                    // A condition with no `filter-set-item` children is a single value,
                    // and that is what `table:value` holds.
                    [attrs.get(Ns::Table, "value").unwrap_or_default().to_owned()].into(),
                );
                Some(Box::new(FilterCondition { items: false }))
            }
            _ => None,
        }
    }
}

struct FilterCondition {
    /// Whether any `filter-set-item` has been seen — the first one replaces the condition's
    /// own `table:value` rather than joining it.
    items: bool,
}

impl Context<Builder> for FilterCondition {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        if name.is(Ns::Table, "filter-set-item") {
            if !std::mem::replace(&mut self.items, true) {
                b.filter_values.1.clear();
            }
            b.filter_values
                .1
                .insert(attrs.get(Ns::Table, "value").unwrap_or_default().to_owned());
        }
        Some(Box::new(super::context::Ignore))
    }

    fn end(&mut self, b: &mut Builder) {
        let (field, values) = std::mem::take(&mut b.filter_values);
        if let Some(or) = b.filter_or.as_mut() {
            or.push((field, values));
        } else if let Some((_, filter)) = b.filter.as_mut() {
            filter.keep.insert(field, values);
        }
    }
}

/// `table:filter-or`. LibreOffice saves a one-column autofilter as an `or` of one condition, and
/// an `or` of values in one column is a set of values. One across columns, or holding anything
/// else (another operator, a nested `and`), is dropped whole: dropping one branch would hide rows
/// the file shows.
struct FilterOr {
    readable: bool,
}

impl Context<Builder> for FilterOr {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        let child = name
            .is(Ns::Table, "filter-condition")
            .then(|| FilterAnd.start_child(name, attrs, b))
            .flatten();
        if child.is_none() {
            self.readable = false;
        }
        child
    }

    fn end(&mut self, b: &mut Builder) {
        let conditions = b.filter_or.take().unwrap_or_default();
        let Some((field, _)) = conditions.first() else {
            return;
        };
        let field = *field;
        if !self.readable || conditions.iter().any(|(f, _)| *f != field) {
            return;
        }
        let values = conditions.into_iter().flat_map(|(_, v)| v).collect();
        if let Some((_, filter)) = b.filter.as_mut() {
            filter.keep.insert(field, values);
        }
    }
}

struct Table {
    /// Whether this is the `table:table` itself rather than a row or column group inside it —
    /// only its own children are the table's columns, charts and names (`source::Table`).
    top: bool,
}

impl Context<Builder> for Table {
    /// Place the checkboxes, each in the row its middle falls in, now that every row's height is
    /// known. Of two in one cell, the first stays.
    fn end(&mut self, b: &mut Builder) {
        if !self.top {
            return;
        }
        b.finish_rules();
        let placed = std::mem::take(&mut b.placed);
        let Some(sheet) = b.doc.sheets.get_mut(b.sheet) else {
            return;
        };
        let mut read = std::collections::BTreeMap::new();
        for (pos, middle, id, checkbox) in placed {
            let at = Pos::new(sheet.row_holding(pos.row, middle), pos.col);
            if sheet.checkbox(at).is_none() {
                sheet.set_checkbox(at, Some(checkbox.clone()));
                read.insert(at, (id, checkbox));
            }
        }
        if let Some(table) = b
            .doc
            .source
            .as_deref_mut()
            .and_then(|source| source.tables.last_mut())
        {
            table.checkboxes = read;
        }
    }

    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        self.record(name, attrs, b);
        match (name.ns, name.local.as_str()) {
            (Ns::Table, "table-row") => {
                b.col = 0;
                b.row_cells.clear();
                b.row_style = attrs
                    .get(Ns::Table, "default-cell-style-name")
                    .map(str::to_owned);
                // §5.4: the row's own `style:style` is where its height lives.
                b.row_size = attrs
                    .get(Ns::Table, "style-name")
                    .and_then(|name| b.track_sizes.get(name))
                    .cloned();
                // Only `"collapse"` is hidden by hand; `"filter"` is recomputed by
                // [`crate::filter`].
                b.row_hidden = attrs.get(Ns::Table, "visibility") == Some("collapse");
                let repeat = attrs.count(Ns::Table, "number-rows-repeated", MAX_ROWS);
                b.row_repeat = repeat;
                Some(Box::new(Row { repeat }))
            }
            // Columns carry the default cell style for everything below them, which is how
            // a whole formatted column is written (§3.3).
            (Ns::Table, "table-column") => {
                let style = attrs
                    .get(Ns::Table, "default-cell-style-name")
                    .map(str::to_owned);
                let width = attrs
                    .get(Ns::Table, "style-name")
                    .and_then(|name| b.track_sizes.get(name))
                    .cloned();
                // A column has no filter, so `table:visibility` on one is always by hand.
                let hidden = attrs.get(Ns::Table, "visibility") == Some("collapse");
                let repeat = attrs.count(Ns::Table, "number-columns-repeated", MAX_COLS);
                b.declare_columns(style, width.as_deref(), hidden, repeat);
                Some(Box::new(super::context::Ignore))
            }
            (Ns::Table, "table-column-group" | "table-header-columns" | "table-columns") => {
                Some(Box::new(Table { top: false }))
            }
            // Row and column groups nest real rows inside a wrapper. Recursing with the
            // same context keeps the cursor continuous instead of losing the contents.
            (Ns::Table, "table-row-group" | "table-header-rows" | "table-rows") => {
                Some(Box::new(Table { top: false }))
            }
            // Sheet-local names (§5.11).
            (Ns::Table, "named-expressions") => Some(Box::new(NamedExpressions {
                local: b
                    .doc
                    .source
                    .as_deref()
                    .and_then(|s| s.tables.len().checked_sub(1)),
            })),
            // A sheet's charts (rng:15678) — `doc/chart-format.md`.
            (Ns::Table, "shapes") => Some(Box::new(Shapes)),
            // The table's form controls, of which a checkbox is read (§3.5).
            (Ns::Office, "forms") if self.top => {
                b.controls.clear();
                Some(Box::new(Forms))
            }
            _ => None,
        }
    }
}

struct Row {
    repeat: u32,
}

impl Context<Builder> for Row {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        let covered = name.is(Ns::Table, "covered-table-cell");
        if !covered && !name.is(Ns::Table, "table-cell") {
            return None;
        }

        // Claim this cell's columns now, so the context does not need to report back.
        let start = b.col;
        let repeat = attrs.count(Ns::Table, "number-columns-repeated", MAX_COLS);
        b.col = b.col.saturating_add(repeat).min(MAX_COLS);
        b.text.clear();

        // A covered cell is the hidden half of a merge (rng:14298). Its content is read like any
        // other — LibreOffice keeps it, out of sight — but only an uncovered cell anchors a merge.
        if !covered {
            b.merge(start, attrs);
        }
        b.note_rule_cells(attrs.get(Ns::Table, "style-name"), start, repeat);

        // R6: where this element sits, for a splice — a repeated cell too (it is split), but not
        // inside a repeated row, and never a covered cell, which a value splice would uncover.
        // `Attrs::span` is the whole `<table:table-cell/>` or just the start tag, which
        // `Cell::end` finishes.
        let span = (!covered && b.row_repeat == 1 && b.doc.source.is_some()).then(|| attrs.span());

        Some(Box::new(Cell {
            start,
            repeat,
            span,
            value_type: attrs
                .get_any(&[(Ns::Office, "value-type"), (Ns::Calcext, "value-type")])
                .map(str::to_owned),
            value: attrs.get(Ns::Office, "value").map(str::to_owned),
            string_value: attrs.get(Ns::Office, "string-value").map(str::to_owned),
            boolean_value: attrs.get(Ns::Office, "boolean-value").map(str::to_owned),
            date_value: attrs.get(Ns::Office, "date-value").map(str::to_owned),
            time_value: attrs.get(Ns::Office, "time-value").map(str::to_owned),
            formula: attrs.get(Ns::Table, "formula").map(str::to_owned),
            style: attrs.get(Ns::Table, "style-name").map(str::to_owned),
            cached_error: attrs.get(Ns::Calcext, "value-type") == Some("error"),
            saw_paragraph: false,
        }))
    }

    fn end(&mut self, b: &mut Builder) {
        b.finish_row(self.repeat);
    }
}

impl Table {
    /// Where this child of a table sits, for a save that splices the sheet back into the file
    /// (`source::Table`): every row element at any depth, and the table's own columns, charts
    /// and names.
    fn record(&self, name: &Name, attrs: &Attrs, b: &mut Builder) {
        if self.top && name.is(Ns::Office, "forms") {
            if let Some(range) = b.extent(attrs.span())
                && let Some(table) = b
                    .doc
                    .source
                    .as_deref_mut()
                    .and_then(|source| source.tables.last_mut())
            {
                table.forms = Some(range);
            }
            return;
        }
        if self.top && name.is(Ns::Calcext, "conditional-formats") {
            if let Some(range) = b.extent(attrs.span())
                && let Some(table) = b
                    .doc
                    .source
                    .as_deref_mut()
                    .and_then(|source| source.tables.last_mut())
            {
                table.conditional_formats = Some(range);
            }
            return;
        }
        if name.ns != Ns::Table {
            return;
        }
        let local = name.local.as_str();
        let interesting = local == "table-row"
            || (self.top
                && matches!(
                    local,
                    "table-column"
                        | "table-column-group"
                        | "table-header-columns"
                        | "table-columns"
                        | "shapes"
                        | "named-expressions"
                ));
        if !interesting {
            return;
        }
        let Some(range) = b.extent(attrs.span()) else {
            return;
        };
        let first = b.row;
        let repeat = attrs.count(Ns::Table, "number-rows-repeated", MAX_ROWS);
        let Some(table) = b
            .doc
            .source
            .as_deref_mut()
            .and_then(|source| source.tables.last_mut())
        else {
            return;
        };
        match local {
            "table-row" => table.rows.push(source::RowElement {
                range,
                start: attrs.span(),
                first,
                repeat,
            }),
            "shapes" => table.shapes = Some(range),
            "named-expressions" => table.names = Some((range, Vec::new())),
            _ => table.columns.push(range),
        }
    }
}

/// Where a cell element ends, given its start tag's span — the whole element for
/// `<table:table-cell/>`, otherwise found by scanning for the close tag.
///
/// A cell may contain a subtable and so other cells, so an opening `<table:table-cell` before the
/// close makes the cell unreplaceable: a rare document loses its small diff, never its
/// correctness.
fn cell_extent(bytes: &[u8], start_tag: std::ops::Range<usize>) -> Option<std::ops::Range<usize>> {
    const OPEN: &[u8] = b"<table:table-cell";
    const CLOSE: &[u8] = b"</table:table-cell>";
    if bytes.get(start_tag.end.checked_sub(2)?..start_tag.end)? == b"/>" {
        return Some(start_tag);
    }
    let rest = bytes.get(start_tag.end..)?;
    let close = rest
        .windows(CLOSE.len())
        .position(|w| w == CLOSE)
        .filter(|at| !rest[..*at].windows(OPEN.len()).any(|w| w == OPEN))?;
    Some(start_tag.start..start_tag.end + close + CLOSE.len())
}

struct Cell {
    start: u32,
    repeat: u32,
    /// The start tag's extent, when this cell is replaceable. See [`cell_extent`].
    span: Option<std::ops::Range<usize>>,
    value_type: Option<String>,
    value: Option<String>,
    string_value: Option<String>,
    boolean_value: Option<String>,
    date_value: Option<String>,
    time_value: Option<String>,
    formula: Option<String>,
    /// `table:style-name` — resolved to a number format at the cell's end, when the styles
    /// section has certainly been read (the schema puts it before the body).
    style: Option<String>,
    /// `calcext:value-type="error"` — the formula's cached result is an error.
    cached_error: bool,
    /// Whether a `text:p` has been seen, so the second starts a new line — counted, so
    /// `<text:p/><text:p/>` is one blank line rather than nothing.
    saw_paragraph: bool,
}

impl Cell {
    /// Resolve the cell's value from its attributes and its display text, every failure scoped to
    /// this cell (§9): a malformed number is 0, a numeric type without `office:value` falls back
    /// to the text, and a cell with no value type but some text is a string. The [`NumberKind`] is
    /// `Some` exactly when a date or time converted.
    fn resolve(&self, text: &str, null_date: i64) -> (CellValue, Option<NumberKind>) {
        let number = |raw: &Option<String>| -> CellValue {
            raw.as_deref()
                .or(Some(text))
                .and_then(|s| s.trim().parse::<f64>().ok())
                .filter(|f| f.is_finite())
                .map_or(CellValue::Number(0.0), CellValue::Number)
        };

        let value = match self.value_type.as_deref() {
            Some("float" | "percentage" | "currency") => number(&self.value),
            Some("boolean") => {
                let raw = self.boolean_value.as_deref().unwrap_or(text).trim();
                CellValue::Bool(raw.eq_ignore_ascii_case("true") || raw == "1")
            }
            // A date or time is a Number (Part 4 §4.3.3, §4.3.2) counted from `null_date`; the
            // kind lets the writer spell it back as a date. Unparseable keeps the ISO text (§9).
            Some("date") => {
                let parsed = self
                    .date_value
                    .as_deref()
                    .and_then(|s| date::parse_date(s, null_date));
                match parsed {
                    Some(serial) => return (CellValue::Number(serial), Some(NumberKind::Date)),
                    None => CellValue::Text(self.date_value.as_deref().unwrap_or(text).to_owned()),
                }
            }
            Some("time") => {
                let parsed = self.time_value.as_deref().and_then(date::parse_time);
                match parsed {
                    Some(fraction) => {
                        return (CellValue::Number(fraction), Some(NumberKind::Time));
                    }
                    None => CellValue::Text(self.time_value.as_deref().unwrap_or(text).to_owned()),
                }
            }
            Some("string" | "error") => {
                // LibreOffice writes an error result as an empty `office:string-value` with the
                // name in the display text and `calcext:value-type="error"` (`doc/ods-format.md`
                // §6), so for those the paragraph is the value.
                if self.cached_error {
                    return (CellValue::Text(text.to_owned()), None);
                }
                CellValue::Text(self.string_value.clone().unwrap_or_else(|| text.to_owned()))
            }
            // An unknown value-type is not a reason to lose the cell: keep what is visible.
            Some(_) | None => {
                if let Some(s) = &self.string_value {
                    CellValue::Text(s.clone())
                } else if text.is_empty() {
                    CellValue::Empty
                } else {
                    CellValue::Text(text.to_owned())
                }
            }
        };
        (value, None)
    }
}

impl Context<Builder> for Cell {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        // A checkbox drawn in this cell, when it names one `office:forms` declared — only for a
        // cell standing for itself.
        if name.is(Ns::Draw, "control") {
            if self.repeat == 1
                && b.row_repeat == 1
                && let Some(id) = attrs.get(Ns::Draw, "control")
                && let Some(checkbox) = b.controls.get(id).cloned()
            {
                let mm = |local: &str| {
                    attrs
                        .get(Ns::Svg, local)
                        .and_then(grind_core::style::length_mm)
                        .unwrap_or(0.0)
                };
                let middle = mm("y") + mm("height") / 2.0;
                b.placed
                    .push((Pos::new(b.row, self.start), middle, id.to_owned(), checkbox));
            }
            return None;
        }
        if !name.is(Ns::Text, "p") {
            return None;
        }
        // A cell may hold several paragraphs; they are separate lines, not one run-on
        // string.
        if self.saw_paragraph {
            b.text.push('\n');
        }
        self.saw_paragraph = true;
        Some(Box::new(Paragraph::CELL))
    }

    fn end(&mut self, b: &mut Builder) {
        let text = std::mem::take(&mut b.text);
        let (value, kind) = self.resolve(&text, b.doc.null_date);

        let formula = self.formula.clone();

        // R6: the element's full extent, known now that it is closed. Empty cells are recorded too
        // — writing a value into `<table:table-cell/>` is the case R6 is named for.
        if let Some((range, keep)) =
            self.span
                .take()
                .zip(b.doc.source.as_ref())
                .and_then(|(tag, source)| {
                    let keep = super::source::kept_attributes(source.bytes.get(tag.clone())?);
                    Some((cell_extent(&source.bytes, tag)?, keep))
                })
        {
            b.row_spans.push(super::source::Cell {
                range,
                cols: self.start..self.start.saturating_add(self.repeat).min(MAX_COLS),
                keep,
            });
        }

        // An empty cell is kept only when it carries a style of its own, within the column and row
        // repeat bounds (`MAX_TRACK_RUN`). A column default on a blank is still dropped: over
        // `sc/qa/unit/data` blanks reached by their own style are about 1.1 million addresses, but
        // those reached by a column default are 125.9 billion (17.2 billion in
        // `ods/value-in-column-2000.ods` alone). Carrying column defaults needs a per-column side
        // table, with its own writer and projection spelling.
        let own_style =
            self.style.is_some() && self.repeat <= MAX_TRACK_RUN && b.row_repeat <= MAX_TRACK_RUN;
        if value.is_empty() && formula.is_none() && !own_style {
            return; // Nothing to write; the columns were claimed at start.
        }

        // §5.1's indirection, resolved once per cell rather than once per repeat.
        let format = b.resolve_format(self.style.as_deref(), self.start);
        let style = b.resolve_style(self.style.as_deref(), self.start);
        // A blank whose own style says nothing — LibreOffice's plain `Default` — is no cell.
        if value.is_empty() && formula.is_none() && style.is_none() && format.is_none() {
            return;
        }
        for i in 0..self.repeat {
            let col = self.start.saturating_add(i);
            if col >= MAX_COLS {
                break;
            }
            b.row_cells.push(RowCell {
                col,
                value: value.clone(),
                formula: formula.clone(),
                kind,
                format: format.clone(),
                style: style.clone(),
            });
        }
    }
}

// --- styles (§5) -----------------------------------------------------------------------

/// `style:default-style style:family="table-cell"`: only the document's locale is read
/// (`fo:language`/`fo:country`, verbatim). Its font size is read past — see [`Styles`].
struct DefaultCellStyle;

impl Context<Builder> for DefaultCellStyle {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        if name.is(Ns::Style, "text-properties")
            && let Some(language) = attrs.get(Ns::Fo, "language")
        {
            b.doc.locale = Some(Locale::new(
                language,
                attrs.get(Ns::Fo, "country").unwrap_or_default(),
            ));
        }
        None
    }
}

/// `office:automatic-styles` and `office:styles`. Read: a `number:*-style` (a format), a
/// `table-cell` `style:style` (the link to a format and the cell's look), and a
/// `table-column`/`table-row` style's size (§5.4). Everything else is dropped rather than
/// half-kept.
///
/// ponytail: the default cell style's `fo:font-size="10pt"` is dropped, though LibreOffice states
/// it there and again on each styled cell, so unstyled cells carry no size and get the shell's
/// font. `ui_sheet_gtk` draws sizes relative to 10pt to cover for it, wrong for a document whose
/// default differs. Reading it costs a writer and a projection node too (loops C and F).
struct Styles;

impl Context<Builder> for Styles {
    fn end(&mut self, b: &mut Builder) {
        b.resolve_maps();
    }

    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        // The document's own locale lives on the default *cell* style (`doc/ods-format.md`
        // §5.2); a default style of any other family says nothing this model holds.
        if name.is(Ns::Style, "default-style") {
            return match attrs.get(Ns::Style, "family") {
                Some("table-cell") => Some(Box::new(DefaultCellStyle)),
                _ => Some(Box::new(super::context::Ignore)),
            };
        }
        if name.is(Ns::Style, "style") {
            let family = attrs.get(Ns::Style, "family").unwrap_or_default();
            let Some(name) = attrs.get(Ns::Style, "name").map(str::to_owned) else {
                return Some(Box::new(super::context::Ignore));
            };
            if let "table-column" | "table-row" = family {
                return Some(Box::new(TrackStyleProps { name }));
            }
            // Anything else — a `text` style inside a paragraph, a `table` style — has
            // nothing this model holds.
            if family != "table-cell" {
                return Some(Box::new(super::context::Ignore));
            }
            if let Some(parent) = attrs.get(Ns::Style, "parent-style-name") {
                b.style_parents.insert(name.clone(), parent.to_owned());
            }
            if let Some(data) = attrs.get(Ns::Style, "data-style-name") {
                b.cell_styles.insert(name.clone(), data.to_owned());
            }
            b.style_props.insert(name.clone(), CellStyle::default());
            return Some(Box::new(CellStyleProps { name }));
        }
        if name.ns != Ns::Number {
            return None;
        }
        let kind = match name.local.as_str() {
            "number-style" => Kind::Number,
            "percentage-style" => Kind::Percentage,
            "currency-style" => Kind::Currency,
            "date-style" => Kind::Date,
            "time-style" => Kind::Time,
            "boolean-style" => Kind::Boolean,
            "text-style" => Kind::Text,
            _ => return None,
        };
        b.parts.clear();
        // §5.2: the locale sits on the style, and decides the decimal and grouping
        // characters every `number:number` beneath it prints with.
        let locale = attrs.get(Ns::Number, "language").map(|language| {
            Locale::new(
                language,
                attrs.get(Ns::Number, "country").unwrap_or_default(),
            )
        });
        Some(Box::new(NumberStyle {
            name: attrs.get(Ns::Style, "name").unwrap_or_default().to_owned(),
            kind,
            locale,
            color: None,
        }))
    }
}

/// The property children of a `table-cell` `style:style` (§5.1), written straight into the style
/// being built through the [`Builder`].
struct CellStyleProps {
    name: String,
}

/// The properties a style sets to the spelling of nothing (`none`, `0`), stored as absent but
/// still overriding a parent ([`Builder::inherited`]).
#[derive(Clone, Copy, Debug, Default)]
struct Resets {
    borders: [bool; 4],
    rotation: bool,
    underline: bool,
    line_through: bool,
    indent: bool,
    /// `style:use-window-font-color="true"` without `fo:color`: ODF's automatic text colour,
    /// stated, so a parent's colour does not show through.
    color: bool,
}

impl Resets {
    fn apply(&self, look: &mut CellStyle) {
        for (edge, cleared) in look.borders.iter_mut().zip(self.borders) {
            if cleared {
                *edge = None;
            }
        }
        let clear = |field: &mut Option<String>, cleared: bool| {
            if cleared {
                *field = None;
            }
        };
        clear(&mut look.rotation, self.rotation);
        clear(&mut look.underline, self.underline);
        clear(&mut look.line_through, self.line_through);
        clear(&mut look.indent, self.indent);
        clear(&mut look.color, self.color);
    }
}

impl Context<Builder> for CellStyleProps {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        if name.ns != Ns::Style {
            return None;
        }
        // A conditional format (`doc/ods-format.md` §3.6): kept by name, read into a rule
        // once the cells carrying this style are known.
        if name.local == "map" {
            if let (Some(condition), Some(apply)) = (
                attrs.get(Ns::Style, "condition"),
                attrs.get(Ns::Style, "apply-style-name"),
            ) {
                let base = attrs
                    .get(Ns::Style, "base-cell-address")
                    .unwrap_or_default();
                b.cell_maps.entry(self.name.clone()).or_default().push((
                    condition.to_owned(),
                    apply.to_owned(),
                    base.to_owned(),
                ));
            }
            return Some(Box::new(super::context::Ignore));
        }
        let style = b.style_props.get_mut(&self.name)?;
        // Most of a style is `fo:`, but a few properties are `style:` — `vertical-align` is
        // the one here. Both spellings are asked for by meaning, never by prefix (§8.1).
        let fo = |local: &str| {
            attrs
                .get_any(&[(Ns::Fo, local), (Ns::Style, local)])
                .map(str::to_owned)
        };
        match name.local.as_str() {
            "table-cell-properties" => {
                style.background = fo("background-color");
                // `0` is no rotation, as LibreOffice writes it on every cell style.
                style.rotation =
                    fo("rotation-angle").filter(|v| v.trim().parse::<f64>() != Ok(0.0));
                style.vertical_align = fo("vertical-align");
                style.wrap = fo("wrap-option");
                // The shorthand first, then each edge, so a document that writes both — and
                // LibreOffice does — ends with the specific one winning.
                style.set_border(fo("border"));
                for (i, edge) in EDGES.iter().enumerate() {
                    if let Some(value) = fo(&format!("border-{edge}")) {
                        style.borders[i] = Some(value);
                    }
                }
                // ODF's explicit `none` is an absent attribute in the model, so equal styles stay
                // equal; it is remembered beside it because it still hides a parent's line.
                let mut cleared = [false; 4];
                for (edge, cleared) in style.borders.iter_mut().zip(&mut cleared) {
                    if edge.as_deref() == Some("none") {
                        *edge = None;
                        *cleared = true;
                    }
                }
                let rotation = fo("rotation-angle").is_some() && style.rotation.is_none();
                let resets = b.style_resets.entry(self.name.clone()).or_default();
                resets.borders = cleared;
                resets.rotation = rotation;
            }
            "text-properties" => {
                style.font_weight = fo("font-weight");
                style.font_style = fo("font-style");
                style.font_size = fo("font-size");
                style.color = fo("color");
                // `none` is the absence of a line, which the model spells as no value — LibreOffice
                // writes it on every cell style it saves, as it writes `fo:border="none"`.
                let line = |local: &str| fo(local).filter(|v| v != "none");
                style.underline = line("text-underline-style");
                style.line_through = line("text-line-through-style");
                let underline = fo("text-underline-style").is_some() && style.underline.is_none();
                let through =
                    fo("text-line-through-style").is_some() && style.line_through.is_none();
                let automatic = style.color.is_none()
                    && attrs.get(Ns::Style, "use-window-font-color") == Some("true");
                let resets = b.style_resets.entry(self.name.clone()).or_default();
                resets.underline = underline;
                resets.line_through = through;
                resets.color = automatic;
            }
            "paragraph-properties" => {
                style.align = fo("text-align");
                // A zero margin is no indent, which LibreOffice states on every cell style.
                style.indent = fo("margin-left")
                    .filter(|v| crate::style::length_mm(v).is_none_or(|mm| mm != 0.0));
                let indent = fo("margin-left").is_some() && style.indent.is_none();
                b.style_resets.entry(self.name.clone()).or_default().indent = indent;
            }
            _ => return None,
        }
        Some(Box::new(super::context::Ignore))
    }

    fn end(&mut self, b: &mut Builder) {
        // A style that sets nothing is no style; otherwise every cell in a LibreOffice document
        // would carry an empty one.
        if b.style_props
            .get(&self.name)
            .is_some_and(CellStyle::is_plain)
        {
            b.style_props.remove(&self.name);
        }
    }
}

/// The property child of a `table-column` or `table-row` `style:style` (§5.4):
/// `style:column-width` or `style:row-height`, verbatim. `style:use-optimal-*` is dropped —
/// re-deriving a size needs text measurement — and the explicit size beside it carries the layout.
struct TrackStyleProps {
    name: String,
}

impl Context<Builder> for TrackStyleProps {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        let size = match (name.ns, name.local.as_str()) {
            (Ns::Style, "table-column-properties") => attrs.get(Ns::Style, "column-width"),
            (Ns::Style, "table-row-properties") => attrs.get(Ns::Style, "row-height"),
            _ => return None,
        };
        if let Some(size) = size {
            b.track_sizes.insert(self.name.clone(), size.to_owned());
        }
        Some(Box::new(super::context::Ignore))
    }
}

/// One `number:*-style`: an ordered sequence of pieces (§5.2).
struct NumberStyle {
    name: String,
    kind: Kind,
    locale: Option<Locale>,
    /// `fo:color` on its `style:text-properties` — a red negative's red (§5.2).
    color: Option<String>,
}

impl Context<Builder> for NumberStyle {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        // The conditional branch of a two-branch format (§5.1), and the only `style:`
        // element inside a data style.
        if name.is(Ns::Style, "map") {
            if let (Some(condition), Some(target)) = (
                attrs.get(Ns::Style, "condition"),
                attrs.get(Ns::Style, "apply-style-name"),
            ) && let Some((op, value)) = numfmt::parse_condition(condition)
            {
                b.pending_maps
                    .push((self.name.clone(), op, value, target.to_owned()));
            }
            return Some(Box::new(super::context::Ignore));
        }
        if name.is(Ns::Style, "text-properties") {
            self.color = attrs.get(Ns::Fo, "color").map(str::to_owned);
            return Some(Box::new(super::context::Ignore));
        }
        if name.ns != Ns::Number {
            return None;
        }
        let long = attrs.get(Ns::Number, "style") == Some("long");
        // The two pieces whose content is character data need a context of their own; every
        // other piece is entirely described by its attributes and is pushed right here.
        let part = match name.local.as_str() {
            "text" => return Some(Box::new(StyleText { currency: false })),
            "currency-symbol" => return Some(Box::new(StyleText { currency: true })),
            // An absent minimum is the decimals themselves (`doc/ods-format.md` §5.2): the
            // attribute is ODF 1.3's, and a 1.2 file's `decimal-places="2"` means `1.00`.
            "number" => Part::Number {
                decimals: digits(attrs, "decimal-places", 2),
                min_decimals: digits(
                    attrs,
                    "min-decimal-places",
                    digits(attrs, "decimal-places", 2),
                ),
                min_int: digits(attrs, "min-integer-digits", 1),
                grouping: attrs.get(Ns::Number, "grouping") == Some("true"),
                embedded: Vec::new(),
            },
            "year" => Part::Year { long },
            "month" => Part::Month {
                long,
                textual: attrs.get(Ns::Number, "textual") == Some("true"),
            },
            "day" => Part::Day { long },
            "day-of-week" => Part::DayOfWeek { long },
            "hours" => Part::Hours { long },
            "minutes" => Part::Minutes { long },
            "seconds" => Part::Seconds {
                long,
                decimals: digits(attrs, "decimal-places", 0),
            },
            "am-pm" => Part::AmPm,
            "boolean" => Part::Boolean,
            "text-content" => Part::Content,
            _ => return None,
        };
        let number = matches!(part, Part::Number { .. });
        b.parts.push(part);
        match number {
            // Its `number:embedded-text` children are read onto it.
            true => Some(Box::new(NumberPiece)),
            false => Some(Box::new(super::context::Ignore)),
        }
    }

    fn end(&mut self, b: &mut Builder) {
        let format = Format {
            kind: self.kind,
            parts: std::mem::take(&mut b.parts),
            locale: self.locale.take(),
            maps: Vec::new(),
            color: self.color.take(),
        };
        // The name is *not* taken: a `style:map` collected under this style refers to it by
        // name, and resolution happens after the section ends.
        b.number_styles.insert(self.name.clone(), format);
    }
}

/// A digit-count attribute. Not [`Attrs::count`], which clamps to at least one: zero
/// decimal places is both legal and common.
fn digits(attrs: &Attrs, local: &str, default: u8) -> u8 {
    attrs
        .get(Ns::Number, local)
        .and_then(|s| s.trim().parse::<u8>().ok())
        .unwrap_or(default)
}

/// `number:text` and `number:currency-symbol` — the two pieces that *are* their content.
struct StyleText {
    currency: bool,
}

/// A `number:number`, for its `number:embedded-text` children (rng:7242): each is added to the
/// piece just pushed, with its position.
struct NumberPiece;

impl Context<Builder> for NumberPiece {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        if !name.is(Ns::Number, "embedded-text") {
            return None;
        }
        let position = attrs.get(Ns::Number, "position")?.trim().parse().ok()?;
        b.style_text.clear();
        Some(Box::new(EmbeddedText { position }))
    }
}

/// One `number:embedded-text`: its character data is the text.
struct EmbeddedText {
    position: i32,
}

impl Context<Builder> for EmbeddedText {
    fn text(&mut self, text: &str, b: &mut Builder) {
        b.style_text.push_str(text);
    }

    fn end(&mut self, b: &mut Builder) {
        let text = std::mem::take(&mut b.style_text);
        if let Some(Part::Number { embedded, .. }) = b.parts.last_mut()
            && embedded.len() < 64
        {
            embedded.push(numfmt::Embedded {
                position: self.position,
                text,
            });
        }
    }
}

impl Context<Builder> for StyleText {
    fn text(&mut self, text: &str, b: &mut Builder) {
        b.style_text.push_str(text);
    }

    fn end(&mut self, b: &mut Builder) {
        let text = std::mem::take(&mut b.style_text);
        // An empty `number:text` says nothing; keeping it would make a format's identity depend on
        // whether it was written at all.
        if text.is_empty() && !self.currency {
            return;
        }
        b.parts.push(match self.currency {
            true => Part::Currency(text),
            false => Part::Text(text),
        });
    }
}

/// `text:p` and the runs beneath it — the only contexts that collect character data.
struct Paragraph {
    /// Whether character data follows ODF's white-space rule (Part 3 §6.1.2: a run of whitespace
    /// is one space, none opening a paragraph) or is taken verbatim, as a cell's always is. Chart
    /// titles need the rule: LibreOffice pretty-prints a chart document, and read verbatim an axis
    /// title came back as `"\n               Euro"`.
    collapse: bool,
}

impl Paragraph {
    const CELL: Paragraph = Paragraph { collapse: false };
    const CHART: Paragraph = Paragraph { collapse: true };
}

impl Context<Builder> for Paragraph {
    fn start_child(&mut self, name: &Name, attrs: &Attrs, b: &mut Builder) -> Option<Ctx> {
        match (name.ns, name.local.as_str()) {
            // A span is styled text; its characters still belong to the cell.
            (Ns::Text, "span" | "a") => Some(Box::new(Paragraph {
                collapse: self.collapse,
            })),
            // `text:s` carries a count: `"a    b"` is written as one space plus `<text:s
            // text:c="3"/>`.
            (Ns::Text, "s") => {
                let n = attrs.count(Ns::Text, "c", MAX_COLS);
                for _ in 0..n {
                    b.text.push(' ');
                }
                Some(Box::new(super::context::Ignore))
            }
            (Ns::Text, "tab") => {
                b.text.push('\t');
                Some(Box::new(super::context::Ignore))
            }
            (Ns::Text, "line-break") => {
                b.text.push('\n');
                Some(Box::new(super::context::Ignore))
            }
            _ => None,
        }
    }

    fn text(&mut self, text: &str, b: &mut Builder) {
        if !self.collapse {
            b.text.push_str(text);
            return;
        }
        for c in text.chars() {
            match c {
                ' ' | '\t' | '\r' | '\n' => {
                    if !b.text.is_empty() && !b.text.ends_with(' ') {
                        b.text.push(' ');
                    }
                }
                c => b.text.push(c),
            }
        }
    }
}
