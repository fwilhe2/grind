// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Grammar in a text document (`doc/grammar.md`): each block checked as one paragraph, without
//! the runs spelling also leaves alone, and each finding addressed by block.

pub use grind_core::grammar::{Checker, Finding, Kind, Paragraph};

use crate::model::Block;

/// One finding in one block. The finding's range counts characters into the block's text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Flag {
    pub block: usize,
    pub finding: Finding,
}

impl Flag {
    /// `p12+40` — where the finding starts, in the address every verb takes.
    pub fn address(&self) -> String {
        format!(
            "{}+{}",
            crate::loc::format(self.block),
            self.finding.range.start
        )
    }

    pub fn caret(&self) -> crate::Caret {
        crate::Caret {
            block: self.block,
            offset: self.finding.range.start,
        }
    }
}

/// Every finding `checker` makes in `block`, in `language` when the document states one.
pub fn check_block(
    block: &Block,
    index: usize,
    language: Option<&str>,
    checker: &dyn Checker,
) -> Vec<Flag> {
    let text = block.text();
    let skip = crate::spell::skipped(block);
    let paragraph = Paragraph {
        text: &text,
        skip: &skip,
        language,
    };
    grind_core::grammar::outside(checker.check(&paragraph), &skip)
        .into_iter()
        .map(|finding| Flag {
            block: index,
            finding,
        })
        .collect()
}
