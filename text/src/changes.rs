// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Which blocks changed since a revision, for a reader that set the document then and would set
//! again only what changed ([`crate::App::changed_since`], `doc/page-model.md`).

use std::collections::VecDeque;
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};

/// How many changes are remembered; a reader further behind sets the document again whole.
const KEPT: usize = 256;

/// The blocks that replaced others: `blocks` in the document as it is now, where it had
/// `before` blocks from `blocks.start`. Everything before `blocks.start` is as it was, and
/// everything after `blocks.end` is what came after those `before` blocks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Changed {
    pub blocks: Range<usize>,
    pub before: usize,
}

impl Changed {
    /// Nothing changed.
    #[must_use]
    pub fn none(&self) -> bool {
        self.blocks.is_empty() && self.before == 0
    }

    /// The block at `index` replaced by another.
    pub(crate) fn set(index: usize) -> Self {
        Changed {
            blocks: index..index + 1,
            before: 1,
        }
    }

    /// This change, then `next` made over its result, as one change.
    fn then(self, next: &Changed) -> Changed {
        if self.none() {
            return next.clone();
        }
        let start = self.blocks.start.min(next.blocks.start);
        // The end of what either touched, in the document between the two.
        let between = self.blocks.end.max(next.blocks.start + next.before);
        let end = next.blocks.end + between - (next.blocks.start + next.before);
        let old_end = self.blocks.start + self.before + between - self.blocks.end;
        Changed {
            blocks: start..end,
            before: old_end - start,
        }
    }
}

/// A document's recent changes, each under the revision it made. Revisions are numbered across
/// every document of the process, so one from another document is never mistaken for this one's.
#[derive(Clone, Debug)]
pub struct Changes {
    /// The revision the document began at: nothing before it is known.
    since: u64,
    log: VecDeque<(u64, Changed)>,
}

impl Default for Changes {
    fn default() -> Self {
        Changes {
            since: next(),
            log: VecDeque::new(),
        }
    }
}

impl Changes {
    /// The revision the document is at.
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.log
            .back()
            .map_or(self.since, |(revision, _)| *revision)
    }

    pub(crate) fn record(&mut self, changed: Changed) {
        if self.log.len() == KEPT
            && let Some((revision, _)) = self.log.pop_front()
        {
            self.since = revision;
        }
        self.log.push_back((next(), changed));
    }

    /// Everything changed: no earlier revision can be followed from here.
    pub(crate) fn forget(&mut self) {
        self.log.clear();
        self.since = next();
    }

    /// Every change since `revision` as one; `None` when that revision is not this document's
    /// or too far back.
    #[must_use]
    pub fn since(&self, revision: u64) -> Option<Changed> {
        if revision < self.since || revision > self.revision() {
            return None;
        }
        Some(
            self.log
                .iter()
                .filter(|(at, _)| *at > revision)
                .fold(Changed::default(), |all, (_, changed)| all.then(changed)),
        )
    }
}

fn next() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn changed(blocks: Range<usize>, before: usize) -> Changed {
        Changed { blocks, before }
    }

    /// Each change applied to a list of numbers, and the one change they add up to, agree.
    #[test]
    fn changes_one_after_another_are_the_span_they_cover() {
        let apply = |list: &mut Vec<u32>, change: &Changed, fresh: &mut u32| {
            let new: Vec<u32> = change
                .blocks
                .clone()
                .map(|_| {
                    *fresh += 1;
                    *fresh
                })
                .collect();
            list.splice(
                change.blocks.start..change.blocks.start + change.before,
                new,
            );
        };
        let cases = [
            vec![changed(3..4, 1), changed(3..4, 1)],
            vec![changed(3..4, 1), changed(5..6, 0)],
            vec![changed(5..5, 1), changed(2..3, 1)],
            vec![changed(4..6, 1), changed(0..0, 2), changed(7..8, 1)],
            vec![changed(1..1, 3), changed(1..2, 0)],
        ];
        for case in cases {
            let original: Vec<u32> = (100..112).collect();
            let mut list = original.clone();
            let mut fresh = 0;
            let mut all = Changed::default();
            for change in &case {
                apply(&mut list, change, &mut fresh);
                all = all.then(change);
            }
            let tail = original.len() - (all.blocks.start + all.before);
            assert_eq!(
                list[..all.blocks.start],
                original[..all.blocks.start],
                "{case:?}"
            );
            assert_eq!(
                list[list.len() - tail..],
                original[original.len() - tail..],
                "{case:?}"
            );
            assert_eq!(list.len() - tail, all.blocks.end, "{case:?}");
        }
    }

    #[test]
    fn a_revision_of_another_document_or_too_long_ago_is_not_followed() {
        let mut changes = Changes::default();
        let start = changes.revision();
        changes.record(Changed::set(2));
        assert_eq!(changes.since(start), Some(changed(2..3, 1)));
        assert_eq!(changes.since(changes.revision()), Some(Changed::default()));
        assert_eq!(Changes::default().since(start), None);
        for _ in 0..KEPT {
            changes.record(Changed::set(0));
        }
        assert_eq!(changes.since(start), None);
        changes.forget();
        assert_eq!(changes.since(start), None);
    }
}
