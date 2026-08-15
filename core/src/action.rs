// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Actions: the only way the document changes, and the whole of undo/redo.
//!
//! Applying an action returns its inverse. That is the entire mechanism — the undo stack
//! is a stack of inverses, redo is the same trick run the other way, and no shell ever
//! implements history of its own (doc/plan.md, rule 2).

use crate::model::{CellValue, Document, Pos};

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    SetCell {
        sheet: usize,
        pos: Pos,
        value: CellValue,
    },
}

impl Action {
    pub fn sheet(&self) -> usize {
        match self {
            Action::SetCell { sheet, .. } => *sheet,
        }
    }
}

impl Document {
    /// Apply `action`, returning the action that undoes it.
    ///
    /// Returns `None` if the action names a sheet that does not exist, so a bad index from
    /// a shell is an error rather than a panic.
    #[must_use]
    pub fn apply(&mut self, action: Action) -> Option<Action> {
        match action {
            Action::SetCell { sheet, pos, value } => {
                let s = self.sheet_mut(sheet)?;
                let previous = s.get(pos);
                s.set(pos, value);
                Some(Action::SetCell {
                    sheet,
                    pos,
                    value: previous,
                })
            }
        }
    }
}
