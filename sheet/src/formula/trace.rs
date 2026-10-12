// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A formula taken apart: every sub-expression, where it is in the text, and what it came to
//! **in place** — the workbench's trace (`doc/formula-workbench.md` §3.2).
//!
//! Built from two things that already existed and one that did not: the display-form parse
//! with node spans ([`display::parse_display`]), the evaluator itself — not a second one, so
//! a trace cannot disagree with what the cell would compute — and the evaluator's recorder,
//! which keeps each node's operand as it goes by (`Engine::trace`). Nothing is stored.

use std::collections::HashMap;
use std::ops::Range;

use crate::a1;
use crate::model::{Document, Pos};

use super::display::{self, DisplayError};
use super::eval::{Address, Area, Engine, Operand};
use super::parse::{Expr, Shape};
use super::value::{Value, format_number};

/// A formula, evaluated at a cell, with every node's answer.
#[derive(Clone, Debug, PartialEq)]
pub struct Trace {
    /// What the cell would hold — [`crate::App::preview`]'s answer, empty already read as 0.
    pub value: Value,
    pub root: Node,
}

/// One sub-expression of the formula.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    /// **Byte** range in the display text that was traced — what to highlight.
    pub range: Range<usize>,
    pub came_to: CameTo,
    pub kind: Kind,
    /// In the order the expression has them: a call's arguments, a sign's or a bracket's
    /// operand, a binary operator's two sides.
    pub children: Vec<Node>,
}

/// What sort of node it is — what a shell needs to draw it and [`Trace::by_line`] to place
/// its answer, without the expression tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A function call.
    Call,
    /// An operator and its operands — `A1*2`, `-A1`, `A1%`.
    Operator,
    /// Parentheses a person wrote.
    Paren,
    /// A constant, a reference, a name, or a parameter left empty.
    Leaf,
}

/// What a node came to.
#[derive(Clone, Debug, PartialEq)]
pub enum CameTo {
    /// A value — a reference to one cell is that cell's value, and an empty cell is
    /// [`Value::Empty`] here, which is the one place a person can see that it is not 0.
    Value(Value),
    /// A reference to more than one cell, left as a reference: what it *becomes* depends on
    /// what reads it (a sum, an implied intersection), and that is its parent's answer.
    Range(Area),
    /// Never evaluated — a branch not taken, an argument a function did not need, or a
    /// parameter left empty (§5.6).
    Skipped,
}

impl Trace {
    /// Every node depth-first with how deep it is — the rows a trace is read in, one per
    /// line, which is what the CLI prints and what a list-shaped shell draws.
    pub fn rows(&self) -> Vec<(usize, &Node)> {
        fn walk<'a>(node: &'a Node, depth: usize, out: &mut Vec<(usize, &'a Node)>) {
            out.push((depth, node));
            node.children
                .iter()
                .for_each(|child| walk(child, depth + 1, out));
        }
        let mut out = Vec::new();
        walk(&self.root, 0, &mut out);
        out
    }

    /// For each line of `text` — the display text that was traced, typically laid out by
    /// [`display::pretty`] — the outermost node that **starts** on it, or `None` for a line
    /// that starts none (a closing bracket). So the first line carries the whole formula's
    /// answer, `ROUND(` the call's, and `+ X` the answer of `X`.
    ///
    /// One exception: an operator broken over several lines does not claim its first line,
    /// because that line shows only its left operand — `ROUND(` over `ROUND(…) & " total"`
    /// must read as the `ROUND`. Its answer is its parent's to show. The root is exempt: the
    /// formula's own answer belongs on the line that starts with `=`.
    pub fn by_line<'a>(&'a self, text: &str) -> Vec<Option<&'a Node>> {
        let mut starts = Vec::new();
        let mut at = 0;
        for line in text.split('\n') {
            starts.push(at..at + line.len());
            at += line.len() + 1;
        }
        let rows = self.rows();
        starts
            .into_iter()
            .map(|line| {
                rows.iter()
                    .map(|(_, node)| *node)
                    .filter(|node| !node.range.is_empty() && line.contains(&node.range.start))
                    .find(|node| {
                        std::ptr::eq(*node, &self.root)
                            || node.kind != Kind::Operator
                            || node.range.end <= line.end
                    })
            })
            .collect()
    }
}

impl CameTo {
    /// The answer as a formula would spell it, so that text and a number never read alike
    /// — `"3"` and `3` are the difference `doc/ods-format.md` §4.2 is about. A range is its
    /// address, with its sheet when that is not `here`; `sheets` are the document's names.
    pub fn spelled(&self, sheets: &[String], here: usize) -> String {
        match self {
            CameTo::Value(Value::Empty) => "empty".to_owned(),
            CameTo::Value(Value::Number(n)) => format_number(*n),
            CameTo::Value(Value::Text(text)) => format!("\"{}\"", text.replace('"', "\"\"")),
            CameTo::Value(Value::Bool(b)) => if *b { "TRUE" } else { "FALSE" }.to_owned(),
            CameTo::Value(Value::Error(e)) => e.name().to_owned(),
            CameTo::Range(area) => {
                let sheet = (area.sheet != here)
                    .then(|| sheets.get(area.sheet))
                    .flatten();
                let start = Pos::new(area.rows.start, area.cols.start);
                let end = Pos::new(area.rows.end - 1, area.cols.end - 1);
                format!(
                    "{}:{} ({} cells)",
                    a1::format(sheet.map(String::as_str), start),
                    a1::format(None, end),
                    area.rows.len() * area.cols.len()
                )
            }
            CameTo::Skipped => "not evaluated".to_owned(),
        }
    }
}

/// Cells that hold something else for one trace — the workbench's *what if*
/// (`doc/formula-workbench.md` §3.3). Never written anywhere: it lives as long as a shell
/// keeps it, and the document never hears of it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Overlay {
    values: HashMap<Address, Value>,
}

impl Overlay {
    /// Hold `value` in a cell, for this overlay only. An empty value is an override too —
    /// "what if this cell were blank" — rather than the absence of one; [`Overlay::clear`]
    /// is that.
    pub fn set(&mut self, sheet: usize, pos: Pos, value: Value) {
        self.values.insert(Address::new(sheet, pos), value);
    }

    /// Let a cell hold what it really holds again.
    pub fn clear(&mut self, sheet: usize, pos: Pos) {
        self.values.remove(&Address::new(sheet, pos));
    }

    pub fn get(&self, sheet: usize, pos: Pos) -> Option<&Value> {
        self.values.get(&Address::new(sheet, pos))
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

/// Trace `formula`, written in display syntax with or without its `=`, as if it sat at `at`,
/// with `overlay`'s cells holding what it says.
pub fn trace(
    doc: &Document,
    sheet: usize,
    at: Pos,
    formula: &str,
    overlay: &Overlay,
) -> Result<Trace, DisplayError> {
    let (expr, shape) = display::parse_display(formula)?;
    let at = Address::new(sheet, at);
    let mut engine = Engine::with_overlay(doc, &overlay.values);
    let (value, seen) = engine.trace(&expr, at);
    let root = node(&mut engine, &expr, &shape, &seen);
    Ok(Trace { value, root })
}

fn node(
    engine: &mut Engine,
    expr: &Expr,
    shape: &Shape,
    seen: &HashMap<*const Expr, Operand>,
) -> Node {
    let came_to = match seen.get(&(expr as *const Expr)) {
        None => CameTo::Skipped,
        Some(Operand::Value(value)) => CameTo::Value(value.clone()),
        Some(Operand::Area(area)) => match single(area) {
            // Read through the same engine, whose memo already holds it.
            Some(cell) => CameTo::Value(engine.value(cell)),
            None => CameTo::Range(area.clone()),
        },
    };
    let kind = match expr {
        Expr::Call { .. } => Kind::Call,
        Expr::Prefix(..) | Expr::Postfix(..) | Expr::Binary(..) => Kind::Operator,
        Expr::Paren(_) => Kind::Paren,
        _ => Kind::Leaf,
    };
    let children = children(expr)
        .into_iter()
        .zip(&shape.children)
        .map(|(child, shape)| node(engine, child, shape, seen))
        .collect();
    Node {
        range: shape.range.clone(),
        came_to,
        kind,
        children,
    }
}

fn single(area: &Area) -> Option<Address> {
    (area.rows.len() == 1 && area.cols.len() == 1)
        .then(|| Address::new(area.sheet, Pos::new(area.rows.start, area.cols.start)))
}

/// An expression's children, in [`Shape`]'s order.
fn children(expr: &Expr) -> Vec<&Expr> {
    match expr {
        Expr::Call { args, .. } => args.iter().collect(),
        Expr::Prefix(_, inner) | Expr::Postfix(_, inner) | Expr::Paren(inner) => vec![inner],
        Expr::Binary(_, left, right) => vec![left, right],
        Expr::Number(_)
        | Expr::Text(_)
        | Expr::Error(_)
        | Expr::Ref(_)
        | Expr::Name(_)
        | Expr::Empty => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formula::value::FormulaError;
    use crate::model::{CellValue, Sheet};

    /// A1 = 4, A2 = 0, A3 empty, A4 = `=A1*2` (a formula, so its value is computed), and
    /// B1:B3 = 1, 2, 3.
    fn document() -> Document {
        let mut sheet = Sheet::new("Sheet1");
        sheet.set(Pos::new(0, 0), CellValue::Number(4.0));
        sheet.set(Pos::new(1, 0), CellValue::Number(0.0));
        sheet.set_formula(Pos::new(3, 0), "of:=[.A1]*2".into());
        for row in 0..3 {
            sheet.set(Pos::new(row, 1), CellValue::Number(f64::from(row + 1)));
        }
        Document {
            sheets: vec![sheet],
            ..Default::default()
        }
    }

    fn traced(formula: &str) -> Trace {
        trace(&document(), 0, Pos::new(9, 9), formula, &Overlay::default()).expect(formula)
    }

    /// Every node as `text → answer`, depth-first — the shape the CLI prints.
    fn flat(formula: &str) -> Vec<(String, CameTo)> {
        traced(formula)
            .rows()
            .into_iter()
            .map(|(_, node)| (formula[node.range.clone()].to_owned(), node.came_to.clone()))
            .collect()
    }

    fn n(x: f64) -> CameTo {
        CameTo::Value(Value::Number(x))
    }

    #[test]
    fn every_node_has_its_text_and_its_answer() {
        assert_eq!(
            flat("=A1*2+1"),
            vec![
                ("A1*2+1".into(), n(9.0)),
                ("A1*2".into(), n(8.0)),
                ("A1".into(), n(4.0)),
                ("2".into(), n(2.0)),
                ("1".into(), n(1.0)),
            ]
        );
    }

    #[test]
    fn the_root_answer_is_what_preview_would_store() {
        let t = traced("=SUM(B1:B3)/A1");
        assert_eq!(t.value, Value::Number(1.5));
        assert_eq!(t.root.came_to, n(1.5));
    }

    #[test]
    fn a_branch_not_taken_is_never_evaluated() {
        // The guard is the point: tracing each argument on its own would show #DIV/0!.
        let nodes = flat("=IF(A2=0;0;A1/A2)");
        assert_eq!(nodes[0], ("IF(A2=0;0;A1/A2)".into(), n(0.0)));
        let division = nodes.iter().find(|(text, _)| text == "A1/A2").unwrap();
        assert_eq!(division.1, CameTo::Skipped);
        // And its children are not evaluated either.
        assert!(
            nodes
                .iter()
                .filter(|(text, _)| text == "A1" || text == "A2")
                .any(|(_, came)| *came == CameTo::Skipped)
        );
    }

    #[test]
    fn a_range_stays_a_range_and_its_parent_says_what_it_became() {
        let nodes = flat("=SUM(B1:B3)");
        assert_eq!(nodes[0].1, n(6.0));
        assert_eq!(nodes[1].0, "B1:B3");
        assert!(matches!(nodes[1].1, CameTo::Range(ref a) if a.rows == (0..3)));
    }

    #[test]
    fn an_empty_cell_is_empty_in_the_trace_and_zero_in_the_cell() {
        let t = traced("=A3");
        assert_eq!(t.root.came_to, CameTo::Value(Value::Empty));
        assert_eq!(t.value, Value::Number(0.0));
    }

    #[test]
    fn another_formulas_nodes_are_not_recorded_as_this_ones() {
        // A4's own `[.A1]*2` is evaluated at a deeper level; the trace holds A4's value only.
        let nodes = flat("=A4+1");
        assert_eq!(
            nodes,
            vec![
                ("A4+1".into(), n(9.0)),
                ("A4".into(), n(8.0)),
                ("1".into(), n(1.0))
            ]
        );
    }

    #[test]
    fn an_omitted_parameter_is_an_empty_span_and_skipped() {
        let formula = "=IF(TRUE;;2)";
        let t = traced(formula);
        let omitted = &t.root.children[1];
        assert!(omitted.range.is_empty());
        assert_eq!(omitted.range.start, formula.find(";;").unwrap() + 1);
        assert_eq!(omitted.came_to, CameTo::Skipped);
    }

    #[test]
    fn a_respelled_run_covers_what_was_typed() {
        // `TRUE` is `TRUE()` and `b1` is `[.B1]` underneath; the spans are the typed text.
        assert_eq!(
            flat("=AND(true;b1)")
                .into_iter()
                .map(|(text, _)| text)
                .collect::<Vec<_>>(),
            ["AND(true;b1)", "true", "b1"]
        );
    }

    #[test]
    fn spans_survive_whitespace_and_non_ascii() {
        let formula = "= LEN( \"äöü\" ) + ( 1 )";
        let texts: Vec<String> = flat(formula).into_iter().map(|(t, _)| t).collect();
        assert_eq!(
            texts,
            [
                "LEN( \"äöü\" ) + ( 1 )",
                "LEN( \"äöü\" )",
                "\"äöü\"",
                "( 1 )",
                "1"
            ]
        );
        assert_eq!(traced(formula).value, Value::Number(4.0));
    }

    #[test]
    fn an_error_is_an_answer_like_any_other() {
        let nodes = flat("=IFERROR(1/0;5)");
        assert_eq!(nodes[0].1, n(5.0));
        assert_eq!(
            nodes[1].1,
            CameTo::Value(Value::Error(FormulaError::DivZero))
        );
    }

    #[test]
    fn a_formula_that_reads_its_own_cell_sees_what_the_cell_holds_now() {
        // As `App::preview` does: the formula is not in the cell yet, so reading the cell is
        // reading what is there. Applying it would make a cycle the trace does not show —
        // `doc/formula-workbench.md` §9 names that.
        let mut doc = document();
        doc.sheets[0].set(Pos::new(0, 3), CellValue::Number(41.0));
        let t = trace(&doc, 0, Pos::new(0, 3), "=D1+1", &Overlay::default()).unwrap();
        assert_eq!(t.value, Value::Number(42.0));
    }

    #[test]
    fn an_answer_is_spelled_so_text_and_numbers_differ() {
        let names = ["Sheet1".to_owned(), "Data".to_owned()];
        let spell = |c: CameTo| c.spelled(&names, 0);
        assert_eq!(spell(n(3.0)), "3");
        assert_eq!(spell(CameTo::Value(Value::Text("3".into()))), "\"3\"");
        assert_eq!(
            spell(CameTo::Value(Value::Text("a\"b".into()))),
            "\"a\"\"b\""
        );
        assert_eq!(spell(CameTo::Value(Value::Empty)), "empty");
        let area = |sheet| Area {
            sheet,
            rows: 0..3,
            cols: 1..2,
        };
        assert_eq!(spell(CameTo::Range(area(0))), "B1:B3 (3 cells)");
        assert_eq!(spell(CameTo::Range(area(1))), "Data.B1:B3 (3 cells)");
    }

    fn traced_with(formula: &str, overlay: &Overlay) -> Trace {
        trace(&document(), 0, Pos::new(9, 9), formula, overlay).expect(formula)
    }

    #[test]
    fn an_override_reaches_a_dependent_two_formulas_away() {
        // A4 is `=A1*2` in the document; C1 below is `=A4+1`, so A1 → A4 → C1 → the trace.
        let mut doc = document();
        doc.sheets[0].set_formula(Pos::new(0, 2), "of:=[.A4]+1".into());
        let mut overlay = Overlay::default();
        overlay.set(0, Pos::new(0, 0), Value::Number(10.0));
        let t = trace(&doc, 0, Pos::new(9, 9), "=C1*10", &overlay).unwrap();
        assert_eq!(t.value, Value::Number(210.0), "(10*2+1)*10");
        let real = trace(&doc, 0, Pos::new(9, 9), "=C1*10", &Overlay::default()).unwrap();
        assert_eq!(real.value, Value::Number(90.0), "(4*2+1)*10");
    }

    #[test]
    fn a_formula_cell_can_be_overridden_and_a_cell_made_blank() {
        let mut overlay = Overlay::default();
        overlay.set(0, Pos::new(3, 0), Value::Number(0.0)); // A4, a formula
        overlay.set(0, Pos::new(0, 0), Value::Empty); // A1, a number
        let t = traced_with("=A4+ISBLANK(A1)", &overlay);
        assert_eq!(t.value, Value::Number(1.0));
        assert_eq!(t.root.children[0].came_to, n(0.0));
    }

    #[test]
    fn an_override_reaches_into_a_range() {
        let mut overlay = Overlay::default();
        overlay.set(0, Pos::new(1, 1), Value::Number(20.0)); // B2 of B1:B3
        assert_eq!(
            traced_with("=SUM(B1:B3)", &overlay).value,
            Value::Number(24.0)
        );
        overlay.clear(0, Pos::new(1, 1));
        assert!(overlay.is_empty());
        assert_eq!(
            traced_with("=SUM(B1:B3)", &overlay).value,
            Value::Number(6.0)
        );
    }

    #[test]
    fn a_laid_out_formula_gives_each_line_its_answer() {
        let text = display::pretty("=IF(A2=0;ROUND(SUM(B1:B3)*A1*1.19;2);A1/A2)", 28).unwrap();
        assert_eq!(
            text,
            "=IF(\n    A2=0;\n    ROUND(\n        SUM(B1:B3)*A1*1.19;\n        2\n    );\n    A1/A2\n)"
        );
        let t = traced(&text);
        let answers: Vec<String> = t
            .by_line(&text)
            .into_iter()
            .map(|node| node.map_or(String::new(), |n| n.came_to.spelled(&[], 0)))
            .collect();
        assert_eq!(
            answers,
            [
                "28.56",
                "TRUE",
                "28.56",
                "28.56",
                "2",
                "",
                "not evaluated",
                ""
            ]
        );
    }

    #[test]
    fn a_broken_operator_leaves_its_line_to_the_operand_on_it() {
        let text = display::pretty("=IF(1;ROUND(SUM(B1:B3)*A1*1.19;2)&\" total\";0)", 28).unwrap();
        assert_eq!(
            text,
            "=IF(\n    1;\n    ROUND(\n        SUM(B1:B3)*A1*1.19;\n        2\n    )\n    & \" total\";\n    0\n)"
        );
        let t = traced(&text);
        let lines = t.by_line(&text);
        assert_eq!(lines[2].unwrap().kind, Kind::Call, "ROUND( is the ROUND");
        assert_eq!(lines[2].unwrap().came_to, n(28.56));
        assert_eq!(
            lines[0].unwrap().came_to,
            CameTo::Value(Value::Text("28.56 total".into()))
        );
    }

    #[test]
    fn a_syntax_error_says_where() {
        let e = trace(
            &document(),
            0,
            Pos::new(9, 9),
            "=SUM(A1",
            &Overlay::default(),
        )
        .unwrap_err();
        assert_eq!(e.at, 7);
    }
}
