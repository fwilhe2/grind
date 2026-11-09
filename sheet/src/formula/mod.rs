// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The OpenFormula engine (ODF 1.4 Part 4). [`value`] is the value model everything else inherits
//! its correctness from.

pub mod array;
pub mod assist;
pub mod date;
pub mod display;
pub mod eval;
pub mod friendly;
pub mod funcs;
pub mod lex;
pub mod parse;
pub mod rename;
pub mod serialize;
pub mod shift;
pub mod trace;
pub mod value;
pub mod wildcard;

/// What typing a formula refuses, with what to write instead: a formula that does not parse, a
/// cell address without its brackets, which would parse as a name `defined` does not know, and
/// Excel's name for a function ODF spells `COM.MICROSOFT.…` — the last two would read `#NAME?`.
pub fn check(input: &str, defined: impl Fn(&str) -> bool) -> Result<(), String> {
    if !input.starts_with('=') {
        return Ok(());
    }
    let expr = parse::parse(input).map_err(|e| e.to_string())?;
    let implemented = funcs::implemented();
    for name in funcs::used(input).unwrap_or_default() {
        let odf = format!("COM.MICROSOFT.{name}");
        if !implemented.contains(&name.as_str()) && implemented.contains(&odf.as_str()) {
            return Err(format!(
                "{name} is Excel's name: an ODF formula writes it {odf}"
            ));
        }
    }
    match unbracketed(&defined, &expr) {
        Some(written) => Err(format!(
            "{written} is not a reference in an ODF formula, which brackets every reference: \
             write {}",
            bracketed(&written)
        )),
        None => Ok(()),
    }
}

/// The first cell-shaped name in `expr` that is not `defined`, as written: `S2`,
/// `Data.S2`, or a range of two of them, `S2:S3`.
fn unbracketed(defined: &impl Fn(&str) -> bool, expr: &parse::Expr) -> Option<String> {
    use parse::Expr;
    let cell = |e: &Expr| match e {
        Expr::Name(n) if display::reads_as_reference(n) && !defined(n) => Some(n.clone()),
        _ => None,
    };
    match expr {
        Expr::Binary(lex::Op::Range, a, b) => match (cell(a), cell(b)) {
            (Some(a), Some(b)) => Some(format!("{a}:{b}")),
            _ => unbracketed(defined, a).or_else(|| unbracketed(defined, b)),
        },
        Expr::Binary(_, a, b) => unbracketed(defined, a).or_else(|| unbracketed(defined, b)),
        Expr::Name(_) => cell(expr),
        Expr::Call { args, .. } => args.iter().find_map(|arg| unbracketed(defined, arg)),
        Expr::Prefix(_, e) | Expr::Postfix(_, e) | Expr::Paren(e) => unbracketed(defined, e),
        _ => None,
    }
}

/// `S2` → `[.S2]`, `Data.s2` → `[Data.S2]`, `S2:S3` → `[.S2:.S3]`.
fn bracketed(written: &str) -> String {
    let end = |e: &str| match e.rsplit_once('.') {
        Some((sheet, cell)) => format!("{sheet}.{}", cell.to_uppercase()),
        None => format!(".{}", e.to_uppercase()),
    };
    let ends: Vec<String> = written.split(':').map(end).collect();
    format!("[{}]", ends.join(":"))
}
