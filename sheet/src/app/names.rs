// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Named expressions: defining, renaming and inlining them, each carrying every use with it.

use crate::*;

impl App {
    /// Define or redefine a named expression (§5.11). `expression` is OpenFormula without `of:=` —
    /// `[$Data.$A$1:.$B$9]` for a range, `SUM([$Data.$A$1:.$A$9])` for an expression. Parsed
    /// before it is stored: a broken name is `#NAME?` in every formula using it.
    pub fn set_name(&self, name: &str, expression: &str) -> Result<()> {
        validate_name(name)?;
        formula::parse::parse(expression).map_err(|e| Error::Formula(e.to_string()))?;
        self.mutate(|state| {
            let inverse = state
                .doc
                .apply(Action::SetName {
                    name: name.to_owned(),
                    expression: Some(expression.to_owned()),
                })
                .expect("a name addresses no sheet");
            state.undo.push(inverse);
            state.redo.clear();
            Ok(())
        })
    }

    /// Rename a named expression and every formula and name that uses it, as one undo step
    /// (`doc/dsl.md` §6.5). An AST substitution (`formula::rename::rename_name`): `rate` inside
    /// `rate_2`, `"rate"` or `RATE(…)` is untouched, and a formula that will not parse is left for
    /// `grind lint`. Cached values are kept. Returns how many formulas and names were rewritten.
    ///
    /// Refused when `from` is undefined, when `to` is not a valid name, and when `to` already
    /// exists. Changing only the case is allowed.
    pub fn rename_name(&self, from: &str, to: &str) -> Result<usize> {
        validate_name(to)?;
        self.mutate(|state| {
            let doc = &state.doc;
            let Some(expression) = doc.names.get(&from.to_lowercase()).cloned() else {
                return Err(Error::Formula(format!("no such name: {from}")));
            };
            if !from.eq_ignore_ascii_case(to) && doc.names.contains_key(&to.to_lowercase()) {
                return Err(Error::Formula(format!(
                    "{to} is already a name; delete it first, or pick another"
                )));
            }
            let mut actions = vec![
                Action::SetName {
                    name: from.to_owned(),
                    expression: None,
                },
                Action::SetName {
                    name: to.to_owned(),
                    // A name that mentions itself is a cycle `grind lint` reports; renamed all
                    // the same, so the definition keeps meaning what it meant.
                    expression: Some(
                        formula::rename::rename_name_in_formula(&expression, from, to)
                            .unwrap_or(expression),
                    ),
                },
            ];
            for (index, sheet) in doc.sheets.iter().enumerate() {
                for (pos, text) in sheet.formulas() {
                    if let Some(formula) = formula::rename::rename_name_in_formula(text, from, to) {
                        actions.push(Action::SetFormula {
                            sheet: index,
                            pos,
                            formula: Some(formula),
                            value: sheet.get(pos),
                        });
                    }
                }
            }
            for (name, other) in &doc.names {
                if name.eq_ignore_ascii_case(from) {
                    continue;
                }
                if let Some(rewritten) = formula::rename::rename_name_in_formula(other, from, to) {
                    actions.push(Action::SetName {
                        name: name.clone(),
                        expression: Some(rewritten),
                    });
                }
            }
            for (index, sheet) in doc.sheets.iter().enumerate() {
                if let Some(rules) = rules_rewritten(sheet, |condition| {
                    formula::rename::rename_name_in_formula(condition, from, to)
                }) {
                    actions.push(Action::SetRules {
                        sheet: index,
                        rules,
                    });
                }
            }
            let rewritten = actions.len() - 2;
            let inverse = state
                .doc
                .apply(Action::Batch(actions))
                .expect("a name addresses no sheet");
            state.undo.push(inverse);
            state.redo.clear();
            Ok(rewritten)
        })
    }

    /// Write a named expression into every formula and name that uses it, and delete it, as one
    /// undo step (`doc/dsl.md` §6.5). The same substitution as [`App::rename_name`] with the
    /// definition's tree, bracketed by precedence; cached values are kept. Returns how many were
    /// rewritten.
    ///
    /// Refused rather than leave a `#NAME?`: when the name is undefined, its definition does not
    /// parse or uses itself, or a formula or name that will not parse spells it (a textual check,
    /// erring towards refusing).
    pub fn inline_name(&self, name: &str) -> Result<usize> {
        self.mutate(|state| {
            let doc = &state.doc;
            let key = name.to_lowercase();
            let Some(expression) = doc.names.get(&key).cloned() else {
                return Err(Error::Formula(format!("no such name: {name}")));
            };
            let definition = formula::parse::parse(&expression).map_err(|e| {
                Error::Formula(format!(
                    "{name} does not parse, so it cannot be inlined: {e}"
                ))
            })?;
            if formula::rename::uses_name(&definition, name) {
                return Err(Error::Formula(format!(
                    "{name} uses itself, so it cannot be written out of existence"
                )));
            }
            let unreadable = |text: &str| {
                formula::parse::parse(text).is_err() && text.to_lowercase().contains(&key)
            };
            let mut actions = vec![Action::SetName {
                name: name.to_owned(),
                expression: None,
            }];
            for (index, sheet) in doc.sheets.iter().enumerate() {
                for (pos, text) in sheet.formulas() {
                    if unreadable(text) {
                        return Err(Error::Formula(format!(
                            "{} may use {name} and does not parse; fix it before inlining",
                            a1::format(Some(&sheet.name), pos)
                        )));
                    }
                    if let Some(formula) =
                        formula::rename::inline_name_in_formula(text, name, &definition)
                    {
                        actions.push(Action::SetFormula {
                            sheet: index,
                            pos,
                            formula: Some(formula),
                            value: sheet.get(pos),
                        });
                    }
                }
            }
            for (other, text) in &doc.names {
                if *other == key {
                    continue;
                }
                if unreadable(text) {
                    return Err(Error::Formula(format!(
                        "the name {other} may use {name} and does not parse; fix it before inlining"
                    )));
                }
                if let Some(rewritten) =
                    formula::rename::inline_name_in_formula(text, name, &definition)
                {
                    actions.push(Action::SetName {
                        name: other.clone(),
                        expression: Some(rewritten),
                    });
                }
            }
            for (index, sheet) in doc.sheets.iter().enumerate() {
                if let Some(rule) = sheet.rules().iter().find(|r| unreadable(&r.condition)) {
                    return Err(Error::Formula(format!(
                        "a rule at {} may use {name} and does not parse; fix it before inlining",
                        a1::format(Some(&sheet.name), rule.base)
                    )));
                }
                if let Some(rules) = rules_rewritten(sheet, |condition| {
                    formula::rename::inline_name_in_formula(condition, name, &definition)
                }) {
                    actions.push(Action::SetRules {
                        sheet: index,
                        rules,
                    });
                }
            }
            let rewritten = actions.len() - 1;
            let inverse = state
                .doc
                .apply(Action::Batch(actions))
                .expect("a name addresses no sheet");
            state.undo.push(inverse);
            state.redo.clear();
            Ok(rewritten)
        })
    }

    /// Delete a named expression; `false` if there was none. Formulas still using it become
    /// `#NAME?` at the next recalculation, as in LibreOffice.
    pub fn clear_name(&self, name: &str) -> bool {
        self.mutate(|state| {
            if !state.doc.names.contains_key(&name.to_lowercase()) {
                return Err(Unchanged);
            }
            let inverse = state
                .doc
                .apply(Action::SetName {
                    name: name.to_owned(),
                    expression: None,
                })
                .expect("a name addresses no sheet");
            state.undo.push(inverse);
            state.redo.clear();
            Ok(())
        })
        .is_ok()
    }

    /// Move a number out of the formula at `pos` into the empty cell `target` on the same sheet,
    /// name that cell, and have the formula read the name (`doc/calc.md` §4.2). `literal` is the
    /// number in formula syntax, `%` allowed; `nth` picks one of several. One undo step.
    ///
    /// Refused when the target holds anything, the name is taken or not a name, the number is not
    /// in the formula, or the formula's answer would change (`ISREF` sees a name where it saw a
    /// number).
    pub fn extract_constant(
        &self,
        sheet: usize,
        pos: Pos,
        literal: &str,
        nth: Option<usize>,
        target: Pos,
        name: &str,
    ) -> Result<()> {
        validate_name(name)?;
        let typed = literal.trim();
        let value: f64 = typed
            .trim_end_matches('%')
            .trim()
            .parse()
            .map_err(|_| Error::Formula(format!("{typed} is not a number")))?;
        self.mutate(|state| {
            let doc = &state.doc;
            let s = doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
            let here = a1::format(None, pos);
            let Some(text) = s.formula(pos) else {
                return Err(Error::Formula(format!("{here} holds no formula")));
            };
            if !s.get(target).is_empty() || s.formula(target).is_some() {
                return Err(Error::Formula(format!(
                    "{} is not empty, and extracting never overwrites",
                    a1::format(None, target)
                )));
            }
            if doc.names.contains_key(&name.to_lowercase()) {
                return Err(Error::Formula(format!("{name} is already a name")));
            }
            let expr = formula::parse::parse(text).map_err(|e| Error::Formula(e.to_string()))?;
            let (rewritten, found) = formula::list::replace_literal(
                &expr,
                value,
                nth,
                &formula::parse::Expr::Name(name.to_owned()),
            )
            .map_err(|why| Error::Formula(format!("{here}: {why}")))?;
            let at = formula::eval::Address::new(sheet, pos);
            let before = formula::eval::Engine::new(doc).value(at);
            let reference = formula::lex::Reference {
                source: None,
                start: formula::lex::CellRef {
                    sheet: Some(s.name.clone()),
                    sheet_absolute: true,
                    col: Some(formula::lex::Axis {
                        index: target.col,
                        absolute: true,
                    }),
                    row: Some(formula::lex::Axis {
                        index: target.row,
                        absolute: true,
                    }),
                },
                end: None,
            };
            let stored = if found.percent { value / 100.0 } else { value };
            let actions = Action::Batch(vec![
                Action::SetCell {
                    sheet,
                    pos: target,
                    value: CellValue::Number(stored),
                },
                Action::SetName {
                    name: name.to_owned(),
                    expression: Some(reference.to_string()),
                },
                Action::SetFormula {
                    sheet,
                    pos,
                    formula: Some(format!("of:={rewritten}")),
                    value: s.get(pos),
                },
            ]);
            let inverse = state.doc.apply(actions).ok_or(Error::NoSuchSheet(sheet))?;
            let after = formula::eval::Engine::new(&state.doc).value(at);
            if after != before {
                let _ = state.doc.apply(inverse);
                return Err(Error::Formula(format!(
                    "{here} would change from {} to {}, so the number stays",
                    shown(&before),
                    shown(&after)
                )));
            }
            state.undo.push(inverse);
            state.redo.clear();
            Ok(())
        })
    }

    /// Every named expression, name and expression alike (§5.11).
    pub fn names(&self) -> Vec<(String, String)> {
        let state = self.state.read().unwrap();
        let doc = &state.doc;
        doc.names
            .iter()
            .map(|(name, expr)| (doc.name_spelling(name).to_owned(), expr.clone()))
            .collect()
    }
}

fn shown(value: &formula::value::Value) -> String {
    match value {
        formula::value::Value::Error(e) => e.name().to_owned(),
        formula::value::Value::Text(t) => format!("{t:?}"),
        formula::value::Value::Number(n) => n.to_string(),
        formula::value::Value::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_owned(),
        formula::value::Value::Empty => "nothing".to_owned(),
    }
}
