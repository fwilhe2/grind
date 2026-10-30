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

/// A paragraph a checker that answers later has not answered yet: its text, what is not prose,
/// and its language, with the key its answer is kept under ([`crate::App::grammar_found`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pending {
    pub key: u64,
    pub text: String,
    pub skip: Vec<std::ops::Range<usize>>,
    pub language: Option<String>,
}

impl Pending {
    pub fn paragraph(&self) -> Paragraph<'_> {
        Paragraph {
            text: &self.text,
            skip: &self.skip,
            language: self.language.as_deref(),
        }
    }
}

/// The key an answer for `block` is kept under: its text, what is not prose, and its language.
pub fn key(block: &Block, language: Option<&str>) -> (u64, String, Vec<std::ops::Range<usize>>) {
    use std::hash::{Hash, Hasher};
    let text = block.text();
    let skip = crate::spell::skipped(block);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (&text, &skip, language).hash(&mut hasher);
    (hasher.finish(), text, skip)
}
