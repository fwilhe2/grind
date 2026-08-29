// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The reference index: which cells each formula reads, and its transpose.
//!
//! `doc/view-modes.md` §4.4 is why this exists. *Is this constant referenced by anything?*,
//! *does anything read this cell?* and *what is downstream of my edit?* are all **reverse
//! dependency** questions, and until now nothing here could answer one: `formula/eval.rs`
//! recurses over the dependency graph rather than sorting it, and `doc/plan.md`'s `graph.rs`
//! was in the plan and unbuilt on purpose. View modes are the use that pays for it.
//!
//! **Scope, deliberately small.** A forward index of which areas each formula reads is one
//! walk of the ASTs the parser already produces; the reverse index is its transpose. There
//! is no cycle detection here, no topological order and no incremental maintenance — the
//! evaluator keeps doing exactly what it does, and this is built beside it rather than
//! underneath it.
//!
//! **It resolves references through [`Engine::area`]**, the same function the evaluator
//! calls. An index that resolved them its own way would be a second opinion about what a
//! document reads, and the two would drift on precisely the cases that matter: a name, a
//! whole-column reference, a sheet that does not exist.
//!
//! ponytail: built on demand and thrown away, so a caller that wants it per keystroke
//! re-walks every formula in the document. The upgrade is invalidating only what is
//! downstream of an edit, which is what this structure is *for* — but it needs an owner with
//! a lifetime, and view modes' first cut deliberately has none.

use std::collections::BTreeMap;

use crate::formula::eval::{Address, Area, Engine, intersect};
use crate::formula::lex::Op;
use crate::formula::parse::{Expr, parse};
use crate::model::Document;

/// How deep a chain of named expressions may nest before the walk gives up.
///
/// A name defined in terms of itself is the case this exists for. The evaluator handles it
/// with the same kind of counter (`MAX_DEPTH`), and here it need only be large enough for
/// any name a person writes.
const MAX_NAME_DEPTH: usize = 64;

/// How many `(cell, formula reading it)` pairs the reverse index will hold.
///
/// The forward index is bounded by the number of formulas; the reverse one is bounded by
/// the number of *cells* they read, and `=SUM([.A:.A])` in a thousand columns is a million
/// pairs on its own. Past this the index stops recording and says so
/// ([`RefIndex::truncated`]) rather than growing without limit or lying about it.
pub const MAX_EDGES: usize = 1 << 20;

/// Which cells every formula in a document reads, and which formulas read every cell.
///
/// Built by [`RefIndex::build`] from a document, and valid only for the document it was
/// built from — nothing here is maintained across an edit.
#[derive(Clone, Debug, Default)]
pub struct RefIndex {
    /// Every formula cell that reads anything → the areas it reads, deduplicated, in the
    /// order they appear in the formula.
    reads: BTreeMap<Address, Vec<Area>>,
    /// Every cell read by at least one formula → the formula cells reading it, in address
    /// order.
    read_by: BTreeMap<Address, Vec<Address>>,
    /// How many distinct formulas read each cell **on its own** — through a single-cell
    /// reference rather than by falling inside a range. See
    /// [`RefIndex::singled_out_by`], which is the distinction `doc/view-modes.md` §4.2
    /// turns on.
    read_singly: BTreeMap<Address, usize>,
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
                    // A formula that is not OpenFormula reads nothing this can name. The
                    // evaluator answers `#NAME?` for the same input, so agreeing with it
                    // means recording no reads rather than guessing at the text.
                    continue;
                };
                let mut areas = Vec::new();
                collect(doc, &engine, &expr, at, 0, &mut areas);
                if areas.is_empty() {
                    continue;
                }
                for area in &areas {
                    if area.rows.len() == 1 && area.cols.len() == 1 {
                        // Areas are deduplicated per formula, so this counts *formulas*
                        // rather than mentions: `=[.C2]+[.C2]` is one reader.
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
                        // One formula's areas are appended together, so a cell this formula
                        // reads twice — `[.A1]+[.A1:.B2]` — has `at` last already.
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
        }
        index
    }

    /// The areas the formula in `at` reads. Empty for a cell holding no formula, for one
    /// whose formula reads nothing (`=1+1`), and for one whose formula does not parse.
    pub fn reads(&self, at: Address) -> &[Area] {
        self.reads.get(&at).map_or(&[], Vec::as_slice)
    }

    /// The formula cells that read `at`, in address order.
    pub fn dependents(&self, at: Address) -> &[Address] {
        self.read_by.get(&at).map_or(&[], Vec::as_slice)
    }

    /// Whether any formula reads this cell, by any reference at all.
    pub fn is_referenced(&self, at: Address) -> bool {
        self.read_by.contains_key(&at)
    }

    /// How many distinct formulas single this cell out — read it through a **one-cell**
    /// reference rather than by covering it with a range.
    ///
    /// `doc/view-modes.md` §4.2 is stated in exactly these terms — *a lone `0.2` that three
    /// formulas multiply by* — and both halves of it are load-bearing, which
    /// `examples/sample-sheet.sh` proved twice over. Counting every reference at all made
    /// each of the six actuals under `=SUM([.C2:.C7])` a magic constant; counting single-cell
    /// references but not *how many* still caught each actual, because the difference column
    /// beside it reads its own row's cell by address. A column of data is read once, by the
    /// formula next to it. A parameter is read from several places, which is what makes not
    /// naming it a problem.
    pub fn singled_out_by(&self, at: Address) -> usize {
        self.read_singly.get(&at).copied().unwrap_or(0)
    }

    /// Every formula cell that reads something, in address order.
    pub fn formula_cells(&self) -> impl Iterator<Item = Address> + '_ {
        self.reads.keys().copied()
    }

    /// Whether the reverse index hit [`MAX_EDGES`] and stopped recording.
    ///
    /// A caller that must not report *nothing reads this cell* wrongly has to check it: past
    /// the cap, [`RefIndex::is_referenced`] can be false for a cell that really is read.
    pub fn truncated(&self) -> bool {
        self.truncated
    }
}

/// Collect the areas an expression reads into `out`, deduplicated.
///
/// A subtree that *is* a reference is taken whole and not descended into — see [`area_of`],
/// which is where `[.A1]:[.B2]` becomes the rectangle it denotes rather than its two corners.
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
        // A name that is not a plain reference — `[.A1]*2` — denotes no place, and its
        // *body* is what reads cells. Resolving it here is what the evaluator does too
        // (§5.11: evaluated at the cell that used the name, which is why `at` is passed on
        // unchanged).
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

/// The area a reference-valued expression denotes, when it is one.
///
/// The two reference operators are here rather than in [`collect`] because they do not read
/// their operands: §6.4.11's `[.A1]:[.B2]` reads the whole rectangle — B1 and A2 included,
/// and neither appears anywhere in the text — and §6.4.12's `!` reads only the overlap.
/// Treating either as "whatever its two sides mention" would be wrong in both directions.
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
                    // A formula cell still needs a value, or `used_rows` does not count it
                    // and a whole-column reference stops short of it.
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

    /// `at(0, "A3")`. `a1::parse` needs an `App` to resolve a sheet name against, and these
    /// tests have a bare `Document` — so the two lines of arithmetic are spelled here rather
    /// than a whole application being built to avoid them.
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

    /// Which cells an address reads, spelled as sorted `Sheet!A1` strings — the shape an
    /// assertion can be read out of.
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
        // §6.4.11. B1 and A2 appear nowhere in the text and are read all the same — the one
        // case a walk that only collected `Expr::Ref` nodes would get wrong.
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
        // §6.4.12, and the mirror of the case above: reading either operand whole would
        // report cells the formula never touches.
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
        // The evaluator bounds the open axis by `used_rows`, so the index does too — this is
        // the reason it resolves through `Engine::area` rather than reading the text.
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
