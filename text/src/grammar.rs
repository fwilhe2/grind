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

/// Every finding `checker` makes in `blocks` — `(index, block)` — in `language` when the document
/// states one, asked for all of them at once ([`Checker::check_all`]).
pub fn check_blocks<'a>(
    blocks: impl IntoIterator<Item = (usize, &'a Block)>,
    language: Option<&str>,
    checker: &dyn Checker,
) -> Vec<Flag> {
    let blocks: Vec<(usize, String, Vec<std::ops::Range<usize>>)> = blocks
        .into_iter()
        .map(|(index, block)| (index, block.text(), crate::spell::skipped(block)))
        .collect();
    let paragraphs: Vec<Paragraph<'_>> = blocks
        .iter()
        .map(|(_, text, skip)| Paragraph {
            text,
            skip,
            language,
        })
        .collect();
    checker
        .check_all(&paragraphs)
        .into_iter()
        .zip(&blocks)
        .flat_map(|(findings, (index, _, skip))| {
            grind_core::grammar::outside(findings, skip)
                .into_iter()
                .map(|finding| Flag {
                    block: *index,
                    finding,
                })
        })
        .collect()
}
