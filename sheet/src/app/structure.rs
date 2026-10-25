// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later
//! What a sheet has besides its cells' values: an autofilter, merges, checkboxes, conditional
//! formats, and tables.

use crate::*;

impl App {
    /// Set — or with `None`, remove — a sheet's autofilter (§9.4), one per sheet, replaced rather
    /// than merged. The hidden rows are derived, not stored ([`crate::filter`]), so a value edit
    /// changes them at once.
    pub fn set_filter(&self, sheet: usize, filter: Option<Filter>) -> Result<()> {
        self.mutate(|state| {
            let inverse = state
                .doc
                .apply(Action::SetFilter {
                    sheet,
                    filter: filter.map(Box::new),
                })
                .ok_or(Error::NoSuchSheet(sheet))?;
            state.undo.push(inverse);
            state.redo.clear();
            Ok(())
        })
    }

    /// Merge the rectangle `start..=end` into one cell drawn with the top-left cell's value and
    /// look (rng:16102). Merges it overlaps are taken away in the same undo step. The other cells
    /// keep what they hold, out of sight, as LibreOffice keeps it, and unmerging shows it again.
    /// Returns whether anything changed; one cell is refused.
    pub fn merge(&self, sheet: usize, start: Pos, end: Pos) -> Result<bool> {
        let (anchor, span) = model::Span::between(start, end);
        if span.is_single() {
            return Err(Error::BadSheet("one cell is no merge".to_owned()));
        }
        let last = span.end(anchor);
        if last.row >= MAX_ROWS || last.col >= MAX_COLS {
            return Err(Error::BadSheet(
                "a merge may not reach past the sheet".to_owned(),
            ));
        }
        self.mutate(|state| {
            let s = state.doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
            let replaced = s.merges_within(anchor, last);
            if replaced == [(anchor, span)] {
                return Ok(false);
            }
            let mut actions: Vec<Action> = replaced
                .iter()
                .map(|(other, _)| Action::SetMerge {
                    sheet,
                    anchor: *other,
                    span: None,
                })
                .collect();
            actions.push(Action::SetMerge {
                sheet,
                anchor,
                span: Some(span),
            });
            self::apply_batch(state, sheet, actions)?;
            Ok(true)
        })
    }

    /// Take away every merge overlapping `start..=end`, as one undo step; returns how many (zero
    /// is fine).
    pub fn unmerge(&self, sheet: usize, start: Pos, end: Pos) -> Result<usize> {
        let (anchor, span) = model::Span::between(start, end);
        self.mutate(|state| {
            let s = state.doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
            let actions = s
                .merges_within(anchor, span.end(anchor))
                .into_iter()
                .map(|(other, _)| Action::SetMerge {
                    sheet,
                    anchor: other,
                    span: None,
                })
                .collect();
            self::apply_batch(state, sheet, actions)
        })
    }

    /// Every merged range on a sheet, as its top-left and bottom-right cells.
    pub fn merges(&self, sheet: usize) -> Result<Vec<(Pos, Pos)>> {
        let state = self.state.read().unwrap();
        let s = state.doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
        Ok(s.merges().map(|(a, span)| (a, span.end(a))).collect())
    }

    /// The merged range `pos` is part of, as its top-left and bottom-right cells.
    pub fn merge_at(&self, sheet: usize, pos: Pos) -> Result<Option<(Pos, Pos)>> {
        let state = self.state.read().unwrap();
        let s = state.doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
        Ok(s.merge_at(pos).map(|(a, span)| (a, span.end(a))))
    }

    /// Put a checkbox in `pos`, or with `None` remove it (`doc/ods-format.md` §3.5), as one undo
    /// step. The linked cell is not touched.
    pub fn set_checkbox(&self, sheet: usize, pos: Pos, checkbox: Option<Checkbox>) -> Result<()> {
        self.mutate(|state| {
            let s = state.doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
            if s.checkbox(pos) == checkbox.as_ref() {
                return Ok(());
            }
            self::apply_batch(
                state,
                sheet,
                vec![Action::SetCheckbox {
                    sheet,
                    pos,
                    checkbox: checkbox.map(Box::new),
                }],
            )
            .map(|_| ())
        })
    }

    /// A sheet's conditional-format rules, first-holding-wins order (`doc/conditional-format.md`).
    pub fn rules(&self, sheet: usize) -> Result<Vec<rule::Rule>> {
        let state = self.state.read().unwrap();
        let s = state.doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
        Ok(s.rules().to_vec())
    }

    /// Replace a sheet's rules, the whole list, as one undo step. A condition that is not
    /// OpenFormula, or a rule with no range, is refused.
    pub fn set_rules(&self, sheet: usize, rules: Vec<rule::Rule>) -> Result<()> {
        for r in &rules {
            if r.ranges.is_empty() {
                return Err(Error::Formula(
                    "a rule needs a range to apply to".to_owned(),
                ));
            }
            if let Err(e) = formula::parse::parse(&r.condition) {
                return Err(Error::Formula(format!(
                    "{:?} is not a formula: {e}",
                    r.condition
                )));
            }
        }
        self.mutate(|state| {
            let s = state.doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
            if s.rules() == rules.as_slice() {
                return Ok(());
            }
            self::apply_batch(state, sheet, vec![Action::SetRules { sheet, rules }]).map(|_| ())
        })
    }

    /// Add a rule after every rule the sheet has — lowest priority. Returns its index.
    pub fn add_rule(&self, sheet: usize, rule: rule::Rule) -> Result<usize> {
        let mut rules = self.rules(sheet)?;
        rules.push(rule);
        let index = rules.len() - 1;
        self.set_rules(sheet, rules)?;
        Ok(index)
    }

    /// Take away the rule at `index`.
    pub fn remove_rule(&self, sheet: usize, index: usize) -> Result<rule::Rule> {
        let mut rules = self.rules(sheet)?;
        if index >= rules.len() {
            return Err(Error::BadSheet(format!("no rule {}", index + 1)));
        }
        let removed = rules.remove(index);
        self.set_rules(sheet, rules)?;
        Ok(removed)
    }

    /// Every checkbox on a sheet, by the cell it is drawn in.
    pub fn checkboxes(&self, sheet: usize) -> Result<Vec<(Pos, Checkbox)>> {
        let state = self.state.read().unwrap();
        let s = state.doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
        Ok(s.checkboxes().map(|(pos, c)| (pos, c.clone())).collect())
    }

    /// Tick or untick the checkbox in `pos`, returning whether it is ticked now. A linked
    /// checkbox's state is its linked cell: `TRUE`/`FALSE`, or `1`/`0` where the cell holds a
    /// number as LibreOffice stores one, recalculated in the same undo step. An unlinked one flips
    /// its own state.
    pub fn toggle_checkbox(&self, sheet: usize, pos: Pos) -> Result<bool> {
        self.mutate(|state| {
            let s = state.doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
            let checkbox = s
                .checkbox(pos)
                .cloned()
                .ok_or_else(|| Error::BadSheet("no checkbox in that cell".to_owned()))?;
            let now = !checkbox_state(&state.doc, sheet, &checkbox);
            let edit = match &checkbox.link {
                Some(link) => {
                    let target = match &link.sheet {
                        Some(name) => state
                            .doc
                            .sheets
                            .iter()
                            .position(|s| &s.name == name)
                            .ok_or_else(|| Error::BadSheet(format!("no such sheet: {name}")))?,
                        None => sheet,
                    };
                    // In the cell's own spelling: ticks kept as `1`/`0` stay numbers, so a sum
                    // over them still works.
                    let value = match state.doc.sheet(target).map(|s| s.get(link.pos)) {
                        Some(CellValue::Number(_)) => CellValue::Number(f64::from(u8::from(now))),
                        _ => CellValue::Bool(now),
                    };
                    Action::SetCell {
                        sheet: target,
                        pos: link.pos,
                        value,
                    }
                }
                None => Action::SetCheckbox {
                    sheet,
                    pos,
                    checkbox: Some(Box::new(Checkbox {
                        checked: now,
                        ..checkbox
                    })),
                },
            };
            commit(state, sheet, vec![edit], RecalcMode::Document)?;
            Ok(now)
        })
    }

    /// A sheet's autofilter, if it has one.
    pub fn filter(&self, sheet: usize) -> Result<Option<Filter>> {
        let state = self.state.read().unwrap();
        let s = state.doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
        Ok(s.filter().cloned())
    }

    /// The rows the filter hides, in order; empty without a filter.
    pub fn hidden_rows(&self, sheet: usize) -> Result<Vec<u32>> {
        let state = self.state.read().unwrap();
        let s = state.doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
        Ok(s.hidden_rows(state.doc.null_date, state.doc.locale.as_ref()))
    }

    /// Format a rectangle as a table: an autofilter, alternating row shading, an optional totals
    /// row and a named range, as one undo step; returns the name. ODF has no table object, so
    /// these are ordinary edits ([`table_format`]) and undoing them is clearing each. Bounded by
    /// [`MAX_FORMATTED_CELLS`].
    pub fn format_table(
        &self,
        sheet: usize,
        start: Pos,
        end: Pos,
        options: table_format::TableOptions,
    ) -> Result<String> {
        let _ = self.rectangle(start, end)?;
        self.mutate(|state| {
            let s = state.doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
            let plan = table_format::plan(sheet, start, end, &options, s, &state.doc.names)?;
            let inverse = state
                .doc
                .apply(Action::Batch(plan.actions))
                .ok_or(Error::NoSuchSheet(sheet))?;
            state.undo.push(inverse);
            state.redo.clear();
            Ok(plan.name)
        })
    }
}
