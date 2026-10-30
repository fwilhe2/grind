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

    /// Every grammar finding in `blocks`, in document order: the attached checker's, and a late
    /// checker's answers ([`App::grammar_found`]) where they are for the text as it is now —
    /// winning where both found something. Each block is checked in the language the document
    /// states, or with none for the checker to guess.
    pub fn grammar(&self, blocks: Range<usize>) -> Vec<grammar::Flag> {
        let checker = self.checker.read().unwrap().clone();
        let language = self.language();
        let background = self.background.read().unwrap();
        let state = self.state.read().unwrap();
        let end = blocks.end.min(state.doc.blocks.len());
        let start = blocks.start.min(end);
        let blocks: Vec<(usize, &Block)> = state.doc.blocks[start..end]
            .iter()
            .enumerate()
            .map(|(i, block)| (start + i, block))
            .collect();
        let mut flags = match &checker {
            Some(checker) => grammar::check_blocks(
                blocks.iter().copied(),
                language.as_deref(),
                checker.as_ref(),
            ),
            None => Vec::new(),
        };
        if let Some(cache) = background.as_ref() {
            for (index, block) in blocks {
                let (key, _, skip) = grammar::key(block, language.as_deref());
                let Some(late) = cache.get(&key) else {
                    continue;
                };
                let late = grind_core::grammar::outside(late.clone(), &skip);
                flags.retain(|flag| {
                    flag.block != index
                        || !late.iter().any(|l| {
                            flag.finding.range.start < l.range.end
                                && l.range.start < flag.finding.range.end
                        })
                });
                flags.extend(late.into_iter().map(|finding| grammar::Flag {
                    block: index,
                    finding,
                }));
            }
        }
        flags.sort_by_key(|f| (f.block, f.finding.range.start, f.finding.range.end));
        flags
    }

    /// Keep answers from a checker that answers later (`doc/grammar.md` decision 10) — or with
    /// `false`, stop, forgetting every answer kept.
    pub fn set_background_grammar(&self, on: bool) {
        *self.background.write().unwrap() = on.then(std::collections::HashMap::new);
    }

    /// The paragraphs of `blocks` the late checker has not answered for, one per distinct text;
    /// empty when there is none ([`App::set_background_grammar`]).
    pub fn grammar_pending(&self, blocks: Range<usize>) -> Vec<grammar::Pending> {
        let language = self.language();
        let background = self.background.read().unwrap();
        let Some(cache) = background.as_ref() else {
            return Vec::new();
        };
        let state = self.state.read().unwrap();
        let end = blocks.end.min(state.doc.blocks.len());
        let start = blocks.start.min(end);
        let mut out: Vec<grammar::Pending> = Vec::new();
        for block in &state.doc.blocks[start..end] {
            let (key, text, skip) = grammar::key(block, language.as_deref());
            if !cache.contains_key(&key) && !out.iter().any(|p| p.key == key) {
                out.push(grammar::Pending {
                    key,
                    text,
                    skip,
                    language: language.clone(),
                });
            }
        }
        out
    }

    /// The late checker's answer for the paragraph [`App::grammar_pending`] gave `key`. Kept
    /// while it is wanted; an answer for text that has since changed is never shown. Observers
    /// are not told — the document has not changed — so the caller redraws.
    pub fn grammar_found(&self, key: u64, findings: Vec<grammar::Finding>) {
        /// Past this many answers, every answer is forgotten and asked for again.
        const KEPT: usize = 20_000;
        if let Some(cache) = self.background.write().unwrap().as_mut() {
            if cache.len() >= KEPT {
                cache.clear();
            }
            cache.insert(key, findings);
        }
    }
}
