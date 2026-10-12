// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! AST → formula text (ODF 1.4 Part 4 §5), as [`Display`](std::fmt::Display).
//!
//! Two rules the round-trip rests on:
//!
//! **Parenthesise by precedence, not by memory.** An [`Expr::Paren`] the user wrote is kept
//! (§5.5 Note 3), but a tree that never came from a parse — one the CLI or a reference
//! rewrite built — still has to print as itself. So every child is bracketed when its
//! binding power is looser than its parent's, which is what makes `print` total rather than
//! "correct for ASTs we happen to have parsed".
//!
//! **Write only the forms §5 says to write.** Numbers as `StandardNumber` with a leading
//! digit (§5.3), parameters separated by `;` (§5.6), and the sheet of a range's second end
//! omitted when it is inherited anyway (§5.8) — which is the spelling LO uses, so a
//! formula we write back is the formula a user recognises.
//!
//! One printer, two spellings. [`Bare`] is the same walk with the brackets left off a
//! reference — the *display form* a person types and reads (`SUM(B2:B4)`), which
//! `formula::display` turns back into the canonical one. A second serialiser would be a
//! second set of precedence rules to keep in step, so the difference is one flag threaded
//! through this file and nothing else.

use std::fmt;

use super::lex::{Axis, CellRef, Op, Reference, column_name};
use super::parse::{Expr, POSTFIX_BP, PREFIX_BP, infix_bp};
use super::value::format_number;

/// An expression printed in **display form**: references without their `[…]`.
///
/// Everything else is spelled exactly as the canonical form spells it, `;` separators
/// included, because that is the syntax this project stores and there is deliberately no
/// translation from another spreadsheet's.
pub struct Bare<'a>(pub &'a Expr);

impl fmt::Display for Bare<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        expr(f, self.0, true)
    }
}

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        expr(f, self, false)
    }
}

fn expr(f: &mut fmt::Formatter<'_>, e: &Expr, bare: bool) -> fmt::Result {
    match e {
        Expr::Number(n) => f.write_str(&format_number(*n)),
        // §5.4: a literal `"` is written doubled.
        Expr::Text(s) => write!(f, "\"{}\"", s.replace('"', "\"\"")),
        Expr::Error(e) => f.write_str(e.name()),
        Expr::Ref(r) => reference(f, r, bare),
        Expr::Name(name) => write!(f, "{}", name_text(name)),
        Expr::Call { name, args } => {
            write!(f, "{name}(")?;
            for (i, arg) in args.iter().enumerate() {
                if i > 0 {
                    f.write_str(";")?;
                }
                expr(f, arg, bare)?;
            }
            f.write_str(")")
        }
        Expr::Prefix(op, operand) => {
            f.write_str(op.text())?;
            child(f, operand, PREFIX_BP, bare)
        }
        Expr::Postfix(op, operand) => {
            child(f, operand, POSTFIX_BP, bare)?;
            f.write_str(op.text())
        }
        Expr::Binary(op, lhs, rhs) => {
            // `[Sheet2.C22]:[.C33]` — the range *operator* between two references — is not
            // the single reference `[Sheet2.C22:.C33]`: the second end there inherits the
            // first's sheet, and here it does not. Display form spells both `Sheet2.C22:C33`,
            // so this one keeps its brackets, the same honesty an external reference gets.
            let bare = bare && *op != Op::Range;
            let bp = infix_bp(*op);
            child(f, lhs, bp, bare)?;
            f.write_str(op.text())?;
            // Every infix operator in §5.5 Table 1 is left-associative, so a right
            // child of equal precedence needs brackets: `1-(2-3)` is not `1-2-3`.
            child(f, rhs, bp + 1, bare)
        }
        Expr::Paren(inner) => {
            f.write_str("(")?;
            expr(f, inner, bare)?;
            f.write_str(")")
        }
        Expr::Empty => Ok(()),
    }
}

/// Write `e` as the child of something binding at `parent_bp`, bracketing if it would
/// otherwise re-associate.
fn child(f: &mut fmt::Formatter<'_>, e: &Expr, parent_bp: u8, bare: bool) -> fmt::Result {
    if binding_power(e) < parent_bp {
        f.write_str("(")?;
        expr(f, e, bare)?;
        f.write_str(")")
    } else {
        expr(f, e, bare)
    }
}

/// How tightly an expression holds together. Leaves and anything already bracketed by its
/// own syntax — a call, a parenthesised group — never need help.
fn binding_power(expr: &Expr) -> u8 {
    match expr {
        Expr::Binary(op, _, _) => infix_bp(*op),
        Expr::Postfix(_, _) => POSTFIX_BP,
        Expr::Prefix(_, _) => PREFIX_BP,
        _ => u8::MAX,
    }
}

/// §5.11: a name that is not a plain `Identifier` is written in the `$$'…'` form, which is
/// the only spelling that can carry spaces and punctuation.
fn name_text(name: &str) -> String {
    let plain = !name.is_empty()
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_alphanumeric() || c == '_');
    if plain {
        name.to_owned()
    } else {
        format!("$$'{}'", name.replace('\'', "''"))
    }
}

impl fmt::Display for Reference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        reference(f, self, false)
    }
}

fn reference(f: &mut fmt::Formatter<'_>, r: &Reference, bare: bool) -> fmt::Result {
    // An external-source reference keeps its brackets in display form too: nothing
    // evaluates one, the syntax has no unbracketed spelling for the `'…'#` part, and a
    // rare construct shown honestly beats one shown wrong.
    let bare = bare && r.source.is_none();
    if !bare {
        f.write_str("[")?;
        if let Some(source) = &r.source {
            write!(f, "'{}'#", source.replace('\'', "''"))?;
        }
    }
    cell_ref(f, &r.start, bare)?;
    if let Some(end) = &r.end {
        // §5.8 lets the second end inherit the first's sheet, and omitting it there is
        // both shorter and what the reader will reconstruct.
        let end = if end.sheet == r.start.sheet {
            CellRef {
                sheet: None,
                sheet_absolute: false,
                ..end.clone()
            }
        } else {
            end.clone()
        };
        f.write_str(":")?;
        cell_ref(f, &end, bare)?;
    }
    if !bare {
        f.write_str("]")?;
    }
    Ok(())
}

impl fmt::Display for CellRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        cell_ref(f, self, false)
    }
}

fn cell_ref(f: &mut fmt::Formatter<'_>, c: &CellRef, bare: bool) -> fmt::Result {
    if let Some(sheet) = &c.sheet {
        if c.sheet_absolute {
            f.write_str("$")?;
        }
        // SheetName ::= '$'? [^\].#$']+ — anything outside that set forces the quoted
        // form, and a literal quote inside it is doubled (§5.8). The set is the same in
        // display form, where a `.` in an unquoted name would read as the separator.
        if sheet.contains([']', '.', '#', '$', '\'', ' ']) || sheet.is_empty() {
            write!(f, "'{}'", sheet.replace('\'', "''"))?;
        } else {
            f.write_str(sheet)?;
        }
    }
    // The dot before the cell is what the brackets make unambiguous; display form drops it
    // when there is no sheet to separate (`B2`), and keeps it when there is (`Data.B2`).
    if !bare || c.sheet.is_some() {
        f.write_str(".")?;
    }
    if let Some(Axis { index, absolute }) = c.col {
        write!(
            f,
            "{}{}",
            if absolute { "$" } else { "" },
            column_name(index)
        )?;
    }
    if let Some(Axis { index, absolute }) = c.row {
        write!(f, "{}{}", if absolute { "$" } else { "" }, index + 1)?;
    }
    Ok(())
}

/// How far each level of a laid-out formula is indented ([`pretty`]).
pub const INDENT: usize = 4;

/// What is never broken, however narrow the width: `B2*C2` over two lines is harder to read
/// than over one, and a deep enough formula would otherwise be broken down to its leaves.
pub const UNBROKEN: usize = 16;

/// An expression in display form laid over several lines, nesting indented, for a person to
/// read — the workbench's layout (`doc/formula-workbench.md` §3.4).
///
/// The result is still **one formula**: §5.14 allows whitespace, line breaks included,
/// between any two tokens, so these lines read back (through `formula::display`) to exactly
/// the tree they were printed from, and a trace of them gives each line its node. Nothing
/// here is ever stored; a cell holds the one-line form.
///
/// The rule, applied from the outside in: whatever fits in `width` columns at its indent is
/// printed flat by [`Bare`]. A call that does not fit puts each argument on a line of its
/// own, one level in, with the closing bracket back out; an operator chain breaks *before*
/// each operator, so a line that starts with `+` is visibly a continuation; brackets open
/// and close on lines of their own. A reference range is never broken, and neither is
/// anything with nothing inside it to break at.
pub fn pretty(e: &Expr, width: usize) -> String {
    lines(e, width).join("\n")
}

fn lines(e: &Expr, width: usize) -> Vec<String> {
    let flat = Bare(e).to_string();
    if flat.chars().count() <= width.max(UNBROKEN) {
        return vec![flat];
    }
    let inner = width.saturating_sub(INDENT);
    match e {
        Expr::Call { name, args } if !args.is_empty() => {
            let mut out = vec![format!("{name}(")];
            for (i, arg) in args.iter().enumerate() {
                let mut arg = lines(arg, inner);
                if i + 1 < args.len() {
                    arg.last_mut().expect("never empty").push(';');
                }
                out.extend(indented(arg, INDENT));
            }
            out.push(")".to_owned());
            out
        }
        Expr::Paren(inner_expr) => bracketed(lines(inner_expr, inner)),
        // The range operator keeps its operands' brackets in display form (see `expr`), so
        // it is printed whole rather than taken apart.
        Expr::Binary(op, lhs, rhs) if *op != Op::Range => {
            let bp = infix_bp(*op);
            let mut out = child_lines(lhs, bp, width);
            let lead = format!("{} ", op.text());
            let rhs = child_lines(rhs, bp + 1, width.saturating_sub(lead.len()));
            out.extend(led(&lead, rhs));
            out
        }
        Expr::Prefix(op, operand) => led(op.text(), child_lines(operand, PREFIX_BP, width)),
        Expr::Postfix(op, operand) => {
            let mut out = child_lines(operand, POSTFIX_BP, width);
            out.last_mut().expect("never empty").push_str(op.text());
            out
        }
        _ => vec![flat],
    }
}

/// [`lines`] for the child of something binding at `parent_bp`, bracketed by [`child`]'s
/// rule — the one rule, so a laid-out formula cannot re-associate where a flat one would not.
fn child_lines(e: &Expr, parent_bp: u8, width: usize) -> Vec<String> {
    match binding_power(e) < parent_bp {
        true => {
            let flat = format!("({})", Bare(e));
            match flat.chars().count() <= width.max(UNBROKEN) {
                true => vec![flat],
                false => bracketed(lines(e, width.saturating_sub(INDENT))),
            }
        }
        false => lines(e, width),
    }
}

fn bracketed(inner: Vec<String>) -> Vec<String> {
    let mut out = vec!["(".to_owned()];
    out.extend(indented(inner, INDENT));
    out.push(")".to_owned());
    out
}

/// `lead` in front of the first line, and the rest lined up under what follows it.
fn led(lead: &str, lines: Vec<String>) -> Vec<String> {
    let pad = lead.chars().count();
    let mut lines = lines.into_iter();
    let first = format!("{lead}{}", lines.next().unwrap_or_default());
    std::iter::once(first)
        .chain(indented(lines.collect(), pad))
        .collect()
}

fn indented(lines: Vec<String>, by: usize) -> impl Iterator<Item = String> {
    lines
        .into_iter()
        .map(move |line| format!("{}{line}", " ".repeat(by)))
}

#[cfg(test)]
mod tests {
    use super::super::lex::Op;
    use super::super::parse::parse;
    use super::*;

    fn round_trip(formula: &str) -> String {
        parse(formula).expect(formula).to_string()
    }

    #[test]
    fn references_print_in_the_form_they_were_read_in() {
        for formula in [
            "[.A1]",
            "[.$B$10]",
            "[Sheet2.A1:.B2]",
            "[Sheet1.A1:Sheet3.B2]",
            "[$'It''s'.A1]",
            "[.A:.C]",
            "[.1:.3]",
            "['file:///tmp/other.ods'#Sheet1.A1]",
            "[.AMJ1048576]",
        ] {
            assert_eq!(round_trip(formula), formula);
        }
    }

    #[test]
    fn a_tree_nobody_parsed_still_prints_as_itself() {
        // The reason parenthesisation is computed rather than remembered: this AST has no
        // Paren node, and printing it as `1+2*3` would change what it means.
        let sum = Expr::Binary(
            Op::Add,
            Box::new(Expr::Number(1.0)),
            Box::new(Expr::Number(2.0)),
        );
        let product = Expr::Binary(Op::Mul, Box::new(sum), Box::new(Expr::Number(3.0)));
        assert_eq!(product.to_string(), "(1+2)*3");
    }

    #[test]
    fn a_right_hand_child_of_equal_precedence_keeps_its_brackets() {
        let inner = Expr::Binary(
            Op::Sub,
            Box::new(Expr::Number(2.0)),
            Box::new(Expr::Number(3.0)),
        );
        let outer = Expr::Binary(Op::Sub, Box::new(Expr::Number(1.0)), Box::new(inner));
        assert_eq!(outer.to_string(), "1-(2-3)");
    }

    #[test]
    fn names_that_are_not_identifiers_use_the_dollar_form() {
        assert_eq!(Expr::Name("Total".into()).to_string(), "Total");
        assert_eq!(Expr::Name("year end".into()).to_string(), "$$'year end'");
        assert_eq!(round_trip("=$$'year end'"), "$$'year end'");
    }

    fn laid_out(formula: &str, width: usize) -> String {
        pretty(&parse(formula).expect(formula), width)
    }

    #[test]
    fn what_fits_stays_on_one_line() {
        assert_eq!(laid_out("=SUM([.A1:.A3])*2", 40), "SUM(A1:A3)*2");
    }

    #[test]
    fn a_call_that_does_not_fit_puts_each_argument_on_its_own_line() {
        assert_eq!(
            laid_out("=IF([.B2]>0;ROUND([.B2]*[.C2];2);0)", 20),
            "IF(\n    B2>0;\n    ROUND(B2*C2;2);\n    0\n)"
        );
        assert_eq!(
            laid_out("=IF([.B2]>0;ROUND([.B2]*[.C2]*[.D2]*1.19;2);0)", 20),
            "IF(\n    B2>0;\n    ROUND(\n        B2*C2*D2*1.19;\n        2\n    );\n    0\n)"
        );
    }

    #[test]
    fn a_chain_breaks_before_its_operators() {
        assert_eq!(
            laid_out("=SUM([.A1:.A9])+AVERAGE([.B1:.B9])-MAX([.C1:.C9])", 20),
            "SUM(A1:A9)\n+ AVERAGE(B1:B9)\n- MAX(C1:C9)"
        );
    }

    #[test]
    fn brackets_the_printer_would_add_are_added_here_too() {
        // A tree nobody parsed: (x+2)*3 has no Paren node, and must not print as x+2*3.
        let call = |name: &str| Expr::Call {
            name: name.into(),
            args: vec![Expr::Number(1.0)],
        };
        let sum = Expr::Binary(
            Op::Add,
            Box::new(call("SOMEWHATLONGNAME")),
            Box::new(call("ANOTHERLONGNAME")),
        );
        let product = Expr::Binary(Op::Mul, Box::new(sum), Box::new(Expr::Number(3.0)));
        assert_eq!(
            pretty(&product, 26),
            "(\n    SOMEWHATLONGNAME(1)\n    + ANOTHERLONGNAME(1)\n)\n* 3"
        );
    }

    #[test]
    fn a_short_node_a_range_and_a_leaf_are_never_broken() {
        assert_eq!(laid_out("=ROUND([.B2]*[.C2];2)", 1), "ROUND(B2*C2;2)");
        assert_eq!(laid_out("=[Data.A1:.B2]", 3), "Data.A1:B2");
        assert_eq!(
            laid_out("=\"a long piece of text\"", 3),
            "\"a long piece of text\""
        );
    }

    #[test]
    fn numbers_are_written_in_standard_form() {
        // §5.3: never a leading `.`, even though reading one is allowed.
        assert_eq!(round_trip("=.5"), "0.5");
        assert_eq!(round_trip("=1E+20"), "1E+20");
    }
}
