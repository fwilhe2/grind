// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Grammar: the checker a shell attaches, and checking against it (`doc/grammar.md`).

use crate::*;

impl App {
    /// Check grammar with `checker` from now on, or with `None` stop. Observers are not told, as
    /// for [`App::set_lexicon`]; the caller redraws.
    pub fn set_checker(&self, checker: Option<Arc<dyn grammar::Checker>>) {
        *self.checker.write().unwrap() = checker;
    }

    /// Every grammar finding in `blocks`, in document order; empty without a checker. Each block
    /// is checked in the language the document states, or with none for the checker to guess.
    pub fn grammar(&self, blocks: Range<usize>) -> Vec<grammar::Flag> {
        let Some(checker) = self.checker.read().unwrap().clone() else {
            return Vec::new();
        };
        let language = self.language();
        let state = self.state.read().unwrap();
        let end = blocks.end.min(state.doc.blocks.len());
        let start = blocks.start.min(end);
        state.doc.blocks[start..end]
            .iter()
            .enumerate()
            .flat_map(|(i, block)| {
                grammar::check_block(block, start + i, language.as_deref(), checker.as_ref())
            })
            .collect()
    }
}
