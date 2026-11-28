// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A fill-handle drag: the selection extended into the cells dragged over, as a series where its
//! lines define one ([`crate::series`]).

use crate::series::{self, Seed};
use crate::*;

impl App {
    /// Extend the selection `start..=end` towards `to`, along the one axis
    /// [`nav::fill_target`] picks: each line of the selection goes on as a series where it
    /// defines one (`1, 2` → `3, 4`; month-ends stay month-ends), and is otherwise repeated with
    /// a formula's relative references shifted. Each new cell takes the number format and style of
    /// the cell it continues. One undo step; nothing happens with `to` inside the selection.
    pub fn extend(
        &self,
        sheet: usize,
        start: Pos,
        end: Pos,
        to: Pos,
        recalc: RecalcMode,
    ) -> Result<EnterOutcome> {
        let nothing = EnterOutcome {
            kind: Entered::Cleared,
            cells: 0,
            recalc: None,
            reading: None,
            readings: 0,
        };
        let Some((dir, line)) = nav::fill_target(start, end, to) else {
            return Ok(nothing);
        };
        let (a, b) = nav::fill_rect(start, end, dir, line);
        // The rectangle with the selection in it bounds the targets as well.
        drop(self.rectangle(a, b)?);
        let down = matches!(dir, nav::Dir::Down | nav::Dir::Up);
        let targets: Vec<u32> = match dir {
            nav::Dir::Down => (end.row + 1..=line).collect(),
            nav::Dir::Up => (line..start.row).collect(),
            nav::Dir::Right => (end.col + 1..=line).collect(),
            nav::Dir::Left => (line..start.col).collect(),
        };
        let lines = if down {
            start.col..=end.col
        } else {
            start.row..=end.row
        };
        self.mutate(|state| {
            let doc = &state.doc;
            let s = doc.sheet(sheet).ok_or(Error::NoSuchSheet(sheet))?;
            let mut edits = Vec::new();
            let mut kind = None;
            for across in lines.clone() {
                let at = |along: u32| {
                    if down {
                        Pos::new(along, across)
                    } else {
                        Pos::new(across, along)
                    }
                };
                let (first, last) = if down {
                    (start.row, end.row)
                } else {
                    (start.col, end.col)
                };
                let seeds: Vec<Pos> = (first..=last).map(at).collect();
                let continues = series::of(
                    &seeds
                        .iter()
                        .map(|pos| Seed {
                            number: match (s.formula(*pos), s.get(*pos)) {
                                (None, CellValue::Number(n)) => Some(n),
                                _ => None,
                            },
                            date: date_kind(s, *pos) == Some(model::NumberKind::Date),
                        })
                        .collect::<Vec<_>>(),
                    doc.null_date,
                );
                for &along in &targets {
                    let pos = at(along);
                    let k = i64::from(along) - i64::from(first);
                    let n = i64::try_from(seeds.len()).unwrap_or(i64::MAX);
                    let source = seeds[usize::try_from(k.rem_euclid(n)).unwrap_or(0)];
                    let (formula, value) = match series::at(continues, k, doc.null_date) {
                        Some(number) => (None, CellValue::Number(number)),
                        None => continued(doc, sheet, s, source, pos),
                    };
                    kind.get_or_insert(match formula {
                        Some(_) => Entered::Formula,
                        None => entered_kind(&value),
                    });
                    edits.push(Action::SetFormula {
                        sheet,
                        pos,
                        formula,
                        value,
                    });
                    if s.format(pos) != s.format(source) {
                        edits.push(Action::SetFormat {
                            sheet,
                            pos,
                            format: s.format(source).cloned().map(Box::new),
                        });
                    }
                    if s.style(pos) != s.style(source) {
                        edits.push(Action::SetStyle {
                            sheet,
                            pos,
                            style: s.style(source).cloned().map(Box::new),
                        });
                    }
                }
            }
            let cells = targets.len() * lines.clone().count();
            let recalc = commit(state, sheet, edits, recalc)?;
            Ok(EnterOutcome {
                kind: kind.unwrap_or(Entered::Cleared),
                cells,
                recalc,
                reading: None,
                readings: 0,
            })
        })
    }
}

/// What `source` becomes at `pos` when a series does not continue it: its formula with relative
/// references shifted, evaluated there, or its plain value.
fn continued(
    doc: &Document,
    sheet: usize,
    s: &Sheet,
    source: Pos,
    pos: Pos,
) -> (Option<String>, CellValue) {
    let Some(text) = s.formula(source) else {
        return (None, s.get(source));
    };
    // A formula that will not parse is copied verbatim.
    let text = formula::parse::parse(text).map_or_else(
        |_| text.to_owned(),
        |expr| {
            let shifted = formula::shift::shift(
                &expr,
                i64::from(pos.row) - i64::from(source.row),
                i64::from(pos.col) - i64::from(source.col),
            );
            format!("={shifted}")
        },
    );
    let value = formula::eval::to_cell(
        formula::eval::Engine::new(doc).eval(&text, formula::eval::Address::new(sheet, pos)),
    );
    (Some(text), value)
}
