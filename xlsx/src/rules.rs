// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `<conditionalFormatting>` — Excel's conditional formats, as the model's one rule type
//! (`doc/conditional-format.md`, `doc/xlsx-format.md` §4.12).
//!
//! `<conditionalFormatting sqref="B5:C7 B10:D14"><cfRule type="expression" dxfId="0"
//! priority="1"><formula>$E5=TRUE</formula></cfRule></conditionalFormatting>` is a
//! [`grind_sheet::rule::Rule`] over both ranges, written from the top-left of the whole `sqref`
//! (measured), drawing the `dxf`'s partial style.
//!
//! **Every rule that has a formula is one.** `expression` is the formula itself; `containsText`,
//! `beginsWith`, `containsBlanks`, `containsErrors`, `timePeriod` and their negations each carry
//! the formula that decides them, which is the rule — `doc/conditional-format.md` §1 keeps no
//! second vocabulary for what a formula can already say. `cellIs` compares the cell itself, so
//! it is *written* as one: `[.B1]>5`, with the cell a relative reference to the base.
//!
//! What is left — a colour scale, a data bar, an icon set, top-N, above-average, duplicate and
//! unique values — is a second rule engine, and is counted as `Dropped::ConditionalFormat`; so
//! is a rule whose formula this filter cannot translate, or whose `dxf` draws nothing the model
//! can hold. A piece of a `dxf` the model has no slot for is an [`Appearance`](crate::styles::Appearance), once per rule.
//!
//! **Priority** is `priority`, lowest first, across the whole sheet. Excel draws every true
//! rule with the higher winning a property both set; the model and LibreOffice draw only the
//! first true one. That divergence is the format's and is written down (§4.12) rather than
//! approximated.

use grind_sheet::formula::parse::{Expr, parse};
use grind_sheet::model::{Pos, Sheet};
use grind_sheet::rule::Rule;

use crate::address;
use crate::formula;
use crate::report::{Dropped, Report};
use crate::sheet::Context;
use crate::xml::{Attrs, Handled, Reader, Spreadsheet as _};

/// The rules of one sheet as they are read, each with its `priority`, ordered when the sheet
/// ends ([`Pending::finish`]).
#[derive(Default)]
pub struct Pending {
    rules: Vec<(u64, usize, Rule)>,
}

impl Pending {
    /// Hand the sheet its rules, in priority order — a tie in the order the file listed them.
    pub fn finish(mut self, sheet: &mut Sheet) {
        if self.rules.is_empty() {
            return;
        }
        self.rules
            .sort_by_key(|(priority, seen, _)| (*priority, *seen));
        sheet.set_rules(self.rules.into_iter().map(|(_, _, rule)| rule).collect());
    }
}

/// One `<conditionalFormatting>`, its `<cfRule>`s read into `pending`.
pub fn read(
    reader: &mut Reader<'_>,
    attrs: &Attrs,
    context: &Context<'_>,
    index: usize,
    pending: &mut Pending,
    report: &mut Report,
) -> crate::Result<()> {
    let ranges = sqref(attrs.plain("sqref").unwrap_or_default());
    reader.children(|reader, name, attrs| {
        if !name.is("cfRule") {
            return Ok(Handled::No);
        }
        let kind = attrs.plain("type").unwrap_or_default().to_owned();
        let operator = attrs.plain("operator").map(str::to_owned);
        let dxf = attrs
            .plain("dxfId")
            .and_then(|v| v.trim().parse::<usize>().ok());
        let priority = attrs
            .plain("priority")
            .and_then(|v| v.trim().parse::<u64>().ok())
            .unwrap_or(u64::MAX);
        let mut formulas = Vec::new();
        reader.children(|reader, name, _| {
            if !name.is("formula") {
                return Ok(Handled::No);
            }
            formulas.push(reader.text()?);
            Ok(Handled::Yes)
        })?;
        match rule(
            &ranges,
            &kind,
            operator.as_deref(),
            dxf,
            &formulas,
            context,
            index,
            report,
        ) {
            Some(rule) => {
                let seen = pending.rules.len();
                pending.rules.push((priority, seen, rule));
                report.rules += 1;
            }
            None => report.drop_one(Dropped::ConditionalFormat),
        }
        Ok(Handled::Yes)
    })?;
    Ok(())
}

/// `sqref`'s ranges — `B5:C7 B10` — each top-left and bottom-right.
fn sqref(text: &str) -> Vec<(Pos, Pos)> {
    text.split_whitespace()
        .filter_map(|r| {
            let (a, b) = r.split_once(':').unwrap_or((r, r));
            let (a, b) = (address::cell(a)?, address::cell(b)?);
            Some((
                Pos::new(a.row.min(b.row), a.col.min(b.col)),
                Pos::new(a.row.max(b.row), a.col.max(b.col)),
            ))
        })
        .collect()
}

/// One `<cfRule>` as a rule, or `None` when it is not one the model can hold.
#[allow(clippy::too_many_arguments)]
fn rule(
    ranges: &[(Pos, Pos)],
    kind: &str,
    operator: Option<&str>,
    dxf: Option<usize>,
    formulas: &[String],
    context: &Context<'_>,
    index: usize,
    report: &mut Report,
) -> Option<Rule> {
    // Written from the top-left of the whole `sqref`, not of its first range (§4.12).
    let base = Pos::new(
        ranges.iter().map(|(a, _)| a.row).min()?,
        ranges.iter().map(|(a, _)| a.col).min()?,
    );
    let look = context.styles.dxf(dxf?)?;
    let style = look.style.clone()?;
    let translate = |text: &str| -> Option<Expr> {
        let scope = crate::tables::Scope {
            tables: context.tables,
            sheet: index,
            sheet_names: context.sheet_names,
            row: base.row,
        };
        let expr = formula::translate_in(text, Some(&scope)).ok()?;
        if crate::sheet::names_in(&expr).any(|name| context.names.is_local(index, name)) {
            return None;
        }
        Some(crate::workbook::renamed(&expr, context.renames))
    };
    let condition = match kind {
        "expression" | "containsText" | "notContainsText" | "beginsWith" | "endsWith"
        | "containsBlanks" | "notContainsBlanks" | "containsErrors" | "notContainsErrors"
        | "timePeriod" => translate(formulas.first()?)?.to_string(),
        "cellIs" => {
            let me = format!("[.{}]", grind_sheet::a1::format(None, base));
            let operand = |i: usize| -> Option<String> {
                let expr = translate(formulas.get(i)?)?;
                Some(match expr {
                    Expr::Binary(..) | Expr::Prefix(..) | Expr::Postfix(..) => {
                        format!("({expr})")
                    }
                    other => other.to_string(),
                })
            };
            match operator.unwrap_or("equal") {
                "between" => format!("AND({me}>={};{me}<={})", operand(0)?, operand(1)?),
                "notBetween" => format!("OR({me}<{};{me}>{})", operand(0)?, operand(1)?),
                op => {
                    let op = match op {
                        "lessThan" => "<",
                        "lessThanOrEqual" => "<=",
                        "equal" => "=",
                        "notEqual" => "<>",
                        "greaterThanOrEqual" => ">=",
                        "greaterThan" => ">",
                        _ => return None,
                    };
                    format!("{me}{op}{}", operand(0)?)
                }
            }
        }
        // A colour scale, a data bar, an icon set, top-N, above-average, duplicate and unique
        // values: a second rule engine each.
        _ => return None,
    };
    // Built as text, so asked of the core's own parser once more: a rule this filter writes is
    // one the model could have been given by hand.
    parse(&condition).ok()?;
    for name in grind_sheet::formula::funcs::used(&condition).unwrap_or_default() {
        if !grind_sheet::formula::funcs::implemented().contains(&name.as_str()) {
            report.unknown_functions.insert(name);
        }
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
    Some(Rule {
        ranges: ranges.to_vec(),
        base,
        condition,
        style,
    })
}
