// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The reference index: which cells each formula reads, and its transpose — the reverse-dependency
//! questions view modes ask (`doc/view-modes.md` §4.4): is this constant referenced, does anything
//! read this cell, what is downstream of an edit.
//!
//! A forward index from one walk of the parsed formulas, and its transpose. No cycle detection,
//! ordering or incremental maintenance; the evaluator does not use it. References resolve through
//! [`Engine::area`], the evaluator's own function, so the index cannot disagree with it about a
//! name, a whole column or a missing sheet.
//!
//! ponytail: built on demand and discarded, so asking per keystroke re-walks every formula.
//! Invalidating only what is downstream of an edit needs an owner with a lifetime.

use std::collections::BTreeMap;

use crate::formula::eval::{Address, Area, Engine, intersect};
use crate::formula::lex::Op;
use crate::formula::parse::{Expr, parse};
use crate::model::Document;

/// How deep a chain of named expressions may nest before the walk gives up — for a name defined in
/// terms of itself.
const MAX_NAME_DEPTH: usize = 64;

/// How many `(cell, formula reading it)` pairs the reverse index holds. `=SUM([.A:.A])` in a
/// thousand columns is a million pairs alone; past this the index stops and says so
/// ([`RefIndex::truncated`]).
pub const MAX_EDGES: usize = 1 << 20;

/// Which cells every formula in a document reads, and which formulas read every cell — valid only
/// for the document it was built from.
#[derive(Clone, Debug, Default)]
pub struct RefIndex {
    /// Every formula cell that reads anything → the areas it reads, deduplicated, in the
    /// order they appear in the formula.
    reads: BTreeMap<Address, Vec<Area>>,
    /// Every cell read by at least one formula → the formula cells reading it, in address
    /// order.
    read_by: BTreeMap<Address, Vec<Address>>,
    /// How many distinct formulas read each cell on its own — through a single-cell reference
    /// rather than a range ([`RefIndex::singled_out_by`], `doc/view-modes.md` §4.2).
    read_singly: BTreeMap<Address, usize>,
    /// Every cell a conditional-format rule's condition reads at a cell it covers
    /// (`doc/conditional-format.md` §2), apart from `read_by` since a rule is not a formula cell.
    read_by_rules: std::collections::BTreeSet<Address>,
    truncated: bool,
}

impl RefIndex {
    /// Walk every formula in the document and index what it reads.
    pub fn build(doc: &Document) -> Self {
        let engine = Engine::new(doc);
        let mut index = RefIndex::default();
        let mut edges = 0usize;
        for (sheet, cells) in doc.sheets.iter().enumerate() {
            for (pos, formula) in cells.formulas() {
                let at = Address::new(sheet, pos);
                let Ok(expr) = parse(formula) else {
                    // A formula that will not parse reads nothing nameable; the evaluator answers
                    // `#NAME?` for it.
                    continue;
                };
                let mut areas = Vec::new();
                collect(doc, &engine, &expr, at, 0, &mut areas);
                if areas.is_empty() {
                    continue;
                }
                for area in &areas {
                    if area.rows.len() == 1 && area.cols.len() == 1 {
                        // Areas are deduplicated per formula, so this counts formulas:
                        // `=[.C2]+[.C2]` is one reader.
                        for cell in area.cells() {
                            *index.read_singly.entry(cell).or_default() += 1;
                        }
                    }
                    for cell in area.cells() {
                        if edges >= MAX_EDGES {
                            index.truncated = true;
                            break;
                        }
                        let readers = index.read_by.entry(cell).or_default();
                        // One formula's areas are appended together, so a cell it reads twice
                        // already has `at` last.
                        if readers.last() != Some(&at) {
                            readers.push(at);
                            edges += 1;
                        }
                    }
                    if index.truncated {
                        break;
                    }
                }
                index.reads.insert(at, areas);
            }
            // What each rule reads, moved to every cell it covers: a relative `[.$E5]` reads a
            // different row on each.
            for rule in cells.rules() {
                let Ok(expr) = parse(&rule.condition) else {
                    continue;
                };
                for (start, end) in &rule.ranges {
                    for row in start.row..=end.row {
                        for col in start.col..=end.col {
                            if edges >= MAX_EDGES {
                                index.truncated = true;
                                return index;
                            }
                            let pos = crate::model::Pos::new(row, col);
                            let moved = crate::formula::shift::shift(
                                &expr,
                                i64::from(row) - i64::from(rule.base.row),
                                i64::from(col) - i64::from(rule.base.col),
                            );
                            let mut areas = Vec::new();
                            collect(
                                doc,
                                &engine,
                                &moved,
                                Address::new(sheet, pos),
                                0,
                                &mut areas,
                            );
                            for cell in areas.iter().flat_map(Area::cells) {
                                if index.read_by_rules.insert(cell) {
                                    edges += 1;
                                }
                            }
                            edges += 1;
                        }
                    }
                }
            }
        }
        index
    }

    /// The areas the formula in `at` reads; empty for no formula, a formula reading nothing
    /// (`=1+1`), or one that will not parse.
    pub fn reads(&self, at: Address) -> &[Area] {
        self.reads.get(&at).map_or(&[], Vec::as_slice)
    }

    /// The formula cells that read `at`, in address order.
    pub fn dependents(&self, at: Address) -> &[Address] {
        self.read_by.get(&at).map_or(&[], Vec::as_slice)
    }

    /// Whether any formula or conditional-format rule reads this cell.
    pub fn is_referenced(&self, at: Address) -> bool {
        self.read_by.contains_key(&at) || self.read_by_rules.contains(&at)
    }

    /// Whether a conditional-format rule reads this cell at some cell it covers.
    pub fn read_by_a_rule(&self, at: Address) -> bool {
        self.read_by_rules.contains(&at)
    }

    /// How many distinct formulas single this cell out — read it through a one-cell reference
    /// rather than a range. §4.2's "a lone `0.2` that three formulas multiply by" needs both
    /// halves: counting range coverage flagged every value under a `SUM`, and counting any single
    /// reader flagged every value read by the formula beside it. A parameter is read from several
    /// places.
    pub fn singled_out_by(&self, at: Address) -> usize {
        self.read_singly.get(&at).copied().unwrap_or(0)
    }

    /// Every formula cell that reads something, in address order.
    pub fn formula_cells(&self) -> impl Iterator<Item = Address> + '_ {
        self.reads.keys().copied()
    }

    /// Whether the reverse index hit [`MAX_EDGES`] and stopped; past it
    /// [`RefIndex::is_referenced`] can be false for a cell that is read.
    pub fn truncated(&self) -> bool {
        self.truncated
    }
}

/// Collect the areas an expression reads into `out`, deduplicated. A reference subtree is taken
/// whole ([`area_of`]), so `[.A1]:[.B2]` is the rectangle it denotes.
fn collect(
    doc: &Document,
    engine: &Engine<'_>,
    expr: &Expr,
    at: Address,
    depth: usize,
    out: &mut Vec<Area>,
) {
    if let Some(area) = area_of(doc, engine, expr, at, depth) {
        if !out.contains(&area) {
            out.push(area);
        }
        return;
    }
    match expr {
        Expr::Call { args, .. } => {
            for arg in args {
                collect(doc, engine, arg, at, depth, out);
            }
        }
        Expr::Prefix(_, inner) | Expr::Postfix(_, inner) | Expr::Paren(inner) => {
            collect(doc, engine, inner, at, depth, out)
        }
        Expr::Binary(_, lhs, rhs) => {
            collect(doc, engine, lhs, at, depth, out);
            collect(doc, engine, rhs, at, depth, out);
        }
        // A name that is not a plain reference denotes no place; its body reads the cells,
        // evaluated at the using cell (§5.11), so `at` is passed on.
        Expr::Name(name) if depth < MAX_NAME_DEPTH => {
            if let Some(expression) = doc.name(name).map(str::to_owned)
                && let Ok(parsed) = parse(&expression)
            {
                collect(doc, engine, &parsed, at, depth + 1, out);
            }
        }
        Expr::Number(_)
        | Expr::Text(_)
        | Expr::Error(_)
        | Expr::Empty
        | Expr::Ref(_)
        | Expr::Name(_) => {}
    }
}

/// The area a reference-valued expression denotes, when it is one. The reference operators do not
/// read their operands: §6.4.11's `[.A1]:[.B2]` reads the whole rectangle, B1 and A2 included, and
/// §6.4.12's `!` only the overlap.
fn area_of(
    doc: &Document,
    engine: &Engine<'_>,
    expr: &Expr,
    at: Address,
    depth: usize,
) -> Option<Area> {
    match expr {
        Expr::Ref(reference) => engine.area(reference, at),
        Expr::Paren(inner) => area_of(doc, engine, inner, at, depth),
        Expr::Name(name) if depth < MAX_NAME_DEPTH => {
            let expression = doc.name(name)?.to_owned();
            area_of(doc, engine, &parse(&expression).ok()?, at, depth + 1)
        }
        Expr::Binary(Op::Range, lhs, rhs) => {
            let a = area_of(doc, engine, lhs, at, depth)?;
            let b = area_of(doc, engine, rhs, at, depth)?;
            // A cuboid across sheets is `#REF!` to the evaluator and nothing to this.
            (a.sheet == b.sheet).then(|| Area {
                sheet: a.sheet,
                rows: a.rows.start.min(b.rows.start)..a.rows.end.max(b.rows.end),
                cols: a.cols.start.min(b.cols.start)..a.cols.end.max(b.cols.end),
            })
        }
        Expr::Binary(Op::Intersect, lhs, rhs) => {
            let a = area_of(doc, engine, lhs, at, depth)?;
            let b = area_of(doc, engine, rhs, at, depth)?;
            intersect(&a, &b)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CellValue, Pos, Sheet};

    /// One sheet's cells: row, column, and either a literal number or a formula.
    type Cells<'a> = &'a [(u32, u32, &'a str)];

    /// A document of literals and formulas, one or two sheets.
    fn doc(sheets: &[(&str, Cells<'_>)], names: &[(&str, &str)]) -> Document {
        let mut document = Document {
            sheets: Vec::new(),
            ..Default::default()
        };
        for (name, cells) in sheets {
            let mut sheet = Sheet::new(*name);
            for (row, col, text) in *cells {
                let pos = Pos::new(*row, *col);
                match text.starts_with('=') {
                    // A formula cell needs a value, or `used_rows` does not count it.
                    true => {
                        sheet.set(pos, CellValue::Number(0.0));
                        sheet.set_formula(pos, (*text).to_owned());
                    }
                    false => sheet.set(pos, CellValue::Number(text.parse().unwrap())),
                }
            }
            document.sheets.push(sheet);
        }
        for (name, expression) in names {
            document
                .names
                .insert(name.to_lowercase(), (*expression).to_owned());
        }
        document
    }

    /// `at(0, "A3")`, without an `App` for `a1::parse` to resolve against.
    fn at(sheet: usize, cell: &str) -> Address {
        let split = cell.find(|c: char| c.is_ascii_digit()).unwrap();
        let col = cell[..split]
            .bytes()
            .fold(0u32, |n, b| n * 26 + u32::from(b - b'A') + 1);
        Address::new(
            sheet,
            Pos::new(cell[split..].parse::<u32>().unwrap() - 1, col - 1),
        )
    }

    /// Which cells an address reads, as sorted `Sheet!A1` strings.
    fn reads(index: &RefIndex, cell: Address) -> Vec<String> {
        let mut cells: Vec<String> = index
            .reads(cell)
            .iter()
            .flat_map(Area::cells)
            .map(|a| format!("{}!{}", a.sheet, crate::a1::format(None, a.pos)))
            .collect();
        cells.sort();
        cells.dedup();
        cells
    }

    #[test]
    fn a_formula_reads_the_cells_its_references_name() {
        let d = doc(
            &[(
                "Sheet1",
                &[(0, 0, "1"), (1, 0, "2"), (2, 0, "=[.A1]+[.A2]")],
            )],
            &[],
        );
        let index = RefIndex::build(&d);
        assert_eq!(reads(&index, at(0, "A3")), ["0!A1", "0!A2"]);
        assert!(index.is_referenced(at(0, "A1")));
        assert!(!index.is_referenced(at(0, "A3")));
        assert_eq!(index.dependents(at(0, "A1")), [at(0, "A3")]);
    }

    #[test]
    fn a_range_is_every_cell_in_it_and_a_function_argument_counts() {
        let d = doc(
            &[(
                "Sheet1",
                &[
                    (0, 0, "1"),
                    (1, 0, "2"),
                    (2, 0, "3"),
                    (3, 0, "=SUM([.A1:.A3])"),
                ],
            )],
            &[],
        );
        let index = RefIndex::build(&d);
        assert_eq!(reads(&index, at(0, "A4")), ["0!A1", "0!A2", "0!A3"]);
    }

    #[test]
    fn the_range_operator_reads_the_rectangle_rather_than_its_corners() {
        // §6.4.11: B1 and A2 appear nowhere in the text and are read all the same.
        let d = doc(
            &[(
                "Sheet1",
                &[
                    (0, 0, "1"),
                    (0, 1, "2"),
                    (1, 0, "3"),
                    (1, 1, "4"),
                    (4, 4, "=SUM([.A1]:[.B2])"),
                ],
            )],
            &[],
        );
        let index = RefIndex::build(&d);
        assert_eq!(reads(&index, at(0, "E5")), ["0!A1", "0!A2", "0!B1", "0!B2"]);
    }

    #[test]
    fn the_intersection_operator_reads_only_the_overlap() {
        // §6.4.12: reading either operand whole would report cells the formula never touches.
        let d = doc(
            &[(
                "Sheet1",
                &[
                    (0, 0, "1"),
                    (0, 1, "2"),
                    (1, 0, "3"),
                    (1, 1, "4"),
                    (4, 4, "=[.A1:.B2]![.B1:.B9]"),
                ],
            )],
            &[],
        );
        let index = RefIndex::build(&d);
        assert_eq!(reads(&index, at(0, "E5")), ["0!B1", "0!B2"]);
    }

    #[test]
    fn a_name_is_resolved_and_both_kinds_of_name_are_walked() {
        let d = doc(
            &[(
                "Sheet1",
                &[(0, 0, "1"), (1, 0, "2"), (5, 0, "=rate+doubled")],
            )],
            &[("rate", "[.A1]"), ("doubled", "[.A2]*2")],
        );
        let index = RefIndex::build(&d);
        // A plain reference and a computed name alike end at the cells they read.
        assert_eq!(reads(&index, at(0, "A6")), ["0!A1", "0!A2"]);
    }

    #[test]
    fn a_name_defined_in_terms_of_itself_terminates() {
        let d = doc(&[("Sheet1", &[(0, 0, "=loop")])], &[("loop", "loop+1")]);
        let index = RefIndex::build(&d);
        assert!(reads(&index, at(0, "A1")).is_empty());
    }

    #[test]
    fn a_cross_sheet_reference_is_indexed_on_the_sheet_it_names() {
        let d = doc(
            &[
                ("Sheet1", &[(0, 0, "=[Rates.A1]*2")]),
                ("Rates", &[(0, 0, "7")]),
            ],
            &[],
        );
        let index = RefIndex::build(&d);
        assert_eq!(reads(&index, at(0, "A1")), ["1!A1"]);
        assert_eq!(index.dependents(at(1, "A1")), [at(0, "A1")]);
    }

    #[test]
    fn a_whole_column_reference_stops_at_what_the_sheet_uses() {
        // The evaluator bounds the open axis by `cell_extent`, and so does the index, through
        // `Engine::area`.
        let d = doc(
            &[(
                "Sheet1",
                &[(0, 0, "1"), (1, 0, "2"), (0, 1, "=SUM([.A:.A])")],
            )],
            &[],
        );
        let index = RefIndex::build(&d);
        assert_eq!(reads(&index, at(0, "B1")), ["0!A1", "0!A2"]);
    }

    #[test]
    fn a_formula_that_reads_nothing_and_one_that_does_not_parse_index_nothing() {
        let d = doc(&[("Sheet1", &[(0, 0, "=1+1"), (1, 0, "={1;2}")])], &[]);
        let index = RefIndex::build(&d);
        assert!(index.reads(at(0, "A1")).is_empty());
        assert!(index.reads(at(0, "A2")).is_empty());
        assert_eq!(index.formula_cells().count(), 0);
    }

    #[test]
    fn a_reference_to_a_sheet_that_does_not_exist_indexes_nothing() {
        let d = doc(&[("Sheet1", &[(0, 0, "=[Nope.A1]")])], &[]);
        let index = RefIndex::build(&d);
        assert!(index.reads(at(0, "A1")).is_empty());
    }

    #[test]
    fn a_cell_read_twice_by_one_formula_lists_that_formula_once() {
        let d = doc(
            &[(
                "Sheet1",
                &[(0, 0, "1"), (1, 0, "2"), (2, 0, "=[.A1]+SUM([.A1:.A2])")],
            )],
            &[],
        );
        let index = RefIndex::build(&d);
        assert_eq!(index.dependents(at(0, "A1")), [at(0, "A3")]);
    }
}
