// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What changed between two text documents, block by block and, inside a changed block, word by
//! word (`grind diff`, `doc/ideas.md` §1).
//!
//! Blocks are matched by kind and text along a longest common subsequence, so an insertion above
//! does not make every later block a change. What is left between two matches is paired by
//! similarity into changed blocks; a block that went from one place and arrived unchanged at
//! another is a move. A matched block whose runs or style differ was reformatted.

use crate::loc;
use crate::model::{Block, BlockKind, Document};

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Diff {
    pub blocks: Vec<BlockChange>,
}

impl Diff {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct BlockChange {
    pub change: Change,
    pub before: Option<Side>,
    pub after: Option<Side>,
    /// For [`Change::Changed`], the text as runs of kept, removed and added words.
    pub words: Vec<Piece>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Added,
    Removed,
    Changed,
    Moved,
    Reformatted,
}

impl Change {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Change::Added => "added",
            Change::Removed => "removed",
            Change::Changed => "changed",
            Change::Moved => "moved",
            Change::Reformatted => "reformatted",
        }
    }
}

/// One block on one side: its address there (`p12`), what kind it is and its text.
#[derive(Debug, PartialEq, Eq)]
pub struct Side {
    pub at: String,
    pub kind: String,
    pub text: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Piece {
    Same(String),
    Removed(String),
    Added(String),
}

/// How alike two blocks' words must be for one to be the other changed rather than one removed
/// and another added.
const ALIKE: f64 = 0.5;

/// Past this many cells a dynamic-programming table is too big, and blocks that occur once on
/// each side anchor the match instead.
const MAX_TABLE: usize = 1 << 22;

#[must_use]
pub fn diff(before: &Document, after: &Document) -> Diff {
    let keys = |doc: &Document| -> Vec<(String, String)> {
        doc.blocks.iter().map(|b| (kind(b), b.text())).collect()
    };
    let (old, new) = (keys(before), keys(after));
    let matched = common(&old, &new);

    let mut changes: Vec<(usize, usize, BlockChange)> = Vec::new();
    let mut removed = Vec::new();
    let mut added = Vec::new();
    let (mut i, mut j) = (0, 0);
    for &(a, b) in matched
        .iter()
        .chain(std::iter::once(&(old.len(), new.len())))
    {
        pair_up(
            before,
            after,
            (i..a).collect(),
            (j..b).collect(),
            &mut changes,
            &mut removed,
            &mut added,
        );
        if a < old.len() {
            let (x, y) = (&before.blocks[a], &after.blocks[b]);
            if x.runs != y.runs || x.style != y.style || x.cell != y.cell {
                changes.push((
                    a,
                    b,
                    change(Change::Reformatted, Some((x, a)), Some((y, b))),
                ));
            }
        }
        (i, j) = (a + 1, b + 1);
    }

    // A block that left one place and arrived, word for word, at another.
    let mut moved = Vec::new();
    removed.retain(|&a| {
        match added
            .iter()
            .position(|&b: &usize| old[a] == new[b] && !moved.iter().any(|&(_, m)| m == b))
        {
            Some(at) => {
                moved.push((a, added[at]));
                false
            }
            None => true,
        }
    });
    added.retain(|b| !moved.iter().any(|(_, m)| m == b));
    for (a, b) in moved {
        let (x, y) = (&before.blocks[a], &after.blocks[b]);
        changes.push((a, b, change(Change::Moved, Some((x, a)), Some((y, b)))));
    }
    for b in added {
        changes.push((
            usize::MAX,
            b,
            change(Change::Added, None, Some((&after.blocks[b], b))),
        ));
    }
    for a in removed {
        // Listed where it was, among the blocks that are still there.
        let at = matched
            .iter()
            .take_while(|&&(x, _)| x < a)
            .last()
            .map_or(0, |&(_, y)| y + 1);
        changes.push((
            a,
            at,
            change(Change::Removed, Some((&before.blocks[a], a)), None),
        ));
    }
    changes.sort_by_key(|&(a, b, _)| (b, a));
    Diff {
        blocks: changes.into_iter().map(|(_, _, change)| change).collect(),
    }
}

/// The blocks between two matches: each removed one paired, in order, with the first added one
/// alike enough; what is left is removed or added.
fn pair_up(
    before: &Document,
    after: &Document,
    gone: Vec<usize>,
    came: Vec<usize>,
    changes: &mut Vec<(usize, usize, BlockChange)>,
    removed: &mut Vec<usize>,
    added: &mut Vec<usize>,
) {
    let mut from = 0;
    for a in gone {
        let x = &before.blocks[a];
        let text = x.text();
        let old = words(&text);
        let found = came[from..].iter().position(|&b| {
            let text = after.blocks[b].text();
            similarity(&old, &words(&text)) >= ALIKE
        });
        match found {
            Some(offset) => {
                added.extend_from_slice(&came[from..from + offset]);
                let b = came[from + offset];
                from += offset + 1;
                let y = &after.blocks[b];
                let mut changed = change(Change::Changed, Some((x, a)), Some((y, b)));
                let text = y.text();
                changed.words = word_diff(&old, &words(&text));
                changes.push((a, b, changed));
            }
            None => removed.push(a),
        }
    }
    added.extend_from_slice(&came[from..]);
}

fn change(
    change: Change,
    before: Option<(&Block, usize)>,
    after: Option<(&Block, usize)>,
) -> BlockChange {
    let side = |(block, index): (&Block, usize)| Side {
        at: loc::format(index),
        kind: kind(block),
        text: block.text(),
    };
    BlockChange {
        change,
        before: before.map(side),
        after: after.map(side),
        words: Vec::new(),
    }
}

/// `paragraph`, `heading 2`, `list item 3`, with `in Table1` for a block in a table.
fn kind(block: &Block) -> String {
    let kind = match block.kind {
        BlockKind::Paragraph => "paragraph".to_owned(),
        BlockKind::Heading { level } => format!("heading {level}"),
        BlockKind::ListItem { depth } => format!("list item {depth}"),
    };
    match &block.cell {
        Some(cell) => format!("{kind} in {}", cell.table),
        None => kind,
    }
}

/// Words and the space between them, so the pieces put back together are the text.
fn words(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut space = None;
    for (at, c) in text.char_indices() {
        let is_space = c.is_whitespace();
        if space.is_some_and(|s| s != is_space) {
            out.push(&text[start..at]);
            start = at;
        }
        space = Some(is_space);
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

/// The share of words two blocks have in common, in order; 1 for two empty blocks.
fn similarity(a: &[&str], b: &[&str]) -> f64 {
    let solid = |w: &&&str| !w.trim().is_empty();
    let (a, b): (Vec<&str>, Vec<&str>) = (
        a.iter().filter(solid).copied().collect(),
        b.iter().filter(solid).copied().collect(),
    );
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    2.0 * common(&a, &b).len() as f64 / (a.len() + b.len()) as f64
}

fn word_diff(old: &[&str], new: &[&str]) -> Vec<Piece> {
    let mut out: Vec<Piece> = Vec::new();
    let mut push = |piece: Piece| match (out.last_mut(), piece) {
        (Some(Piece::Same(a)), Piece::Same(b))
        | (Some(Piece::Removed(a)), Piece::Removed(b))
        | (Some(Piece::Added(a)), Piece::Added(b)) => a.push_str(&b),
        (_, piece) => out.push(piece),
    };
    let (mut i, mut j) = (0, 0);
    for (a, b) in common(old, new)
        .into_iter()
        .chain(std::iter::once((old.len(), new.len())))
    {
        old[i..a]
            .iter()
            .for_each(|w| push(Piece::Removed((*w).to_owned())));
        new[j..b]
            .iter()
            .for_each(|w| push(Piece::Added((*w).to_owned())));
        if a < old.len() {
            push(Piece::Same(old[a].to_owned()));
        }
        (i, j) = (a + 1, b + 1);
    }
    out
}

/// A longest common subsequence of `a` and `b`, as index pairs in order. The common ends are
/// matched first; what is left is a table when it fits and anchored on items that occur once on
/// each side when it does not.
fn common<T: PartialEq + std::hash::Hash + Eq>(a: &[T], b: &[T]) -> Vec<(usize, usize)> {
    let head = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let tail = a[head..]
        .iter()
        .rev()
        .zip(b[head..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let (mid_a, mid_b) = (&a[head..a.len() - tail], &b[head..b.len() - tail]);
    let mut out: Vec<(usize, usize)> = (0..head).map(|i| (i, i)).collect();
    out.extend(
        middle(mid_a, mid_b)
            .into_iter()
            .map(|(i, j)| (i + head, j + head)),
    );
    out.extend((0..tail).map(|k| (a.len() - tail + k, b.len() - tail + k)));
    out
}

fn middle<T: PartialEq + std::hash::Hash + Eq>(a: &[T], b: &[T]) -> Vec<(usize, usize)> {
    if a.is_empty() || b.is_empty() {
        return Vec::new();
    }
    if a.len().saturating_mul(b.len()) <= MAX_TABLE {
        return table(a, b);
    }
    // Patience: items unique on both sides, kept in an order both agree on, then the gaps.
    use std::collections::HashMap;
    let mut counts: HashMap<&T, (usize, usize, usize)> = HashMap::new();
    for (i, x) in a.iter().enumerate() {
        let entry = counts.entry(x).or_insert((0, 0, 0));
        entry.0 += 1;
        entry.2 = i;
    }
    let mut anchors = Vec::new();
    let mut seen_b: HashMap<&T, usize> = HashMap::new();
    for y in b {
        *seen_b.entry(y).or_default() += 1;
    }
    for (j, y) in b.iter().enumerate() {
        if let Some(&(1, _, i)) = counts.get(y)
            && seen_b[y] == 1
        {
            anchors.push((i, j));
        }
    }
    let anchors = increasing(&anchors);
    if anchors.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    for &(x, y) in anchors.iter().chain(std::iter::once(&(a.len(), b.len()))) {
        out.extend(
            common(&a[i..x], &b[j..y])
                .into_iter()
                .map(|(p, q)| (p + i, q + j)),
        );
        if x < a.len() {
            out.push((x, y));
        }
        (i, j) = (x + 1, y + 1);
    }
    out
}

/// The longest run of `pairs` (in order of the second index) whose first indices increase.
fn increasing(pairs: &[(usize, usize)]) -> Vec<(usize, usize)> {
    let mut tails: Vec<usize> = Vec::new();
    let mut back: Vec<Option<usize>> = vec![None; pairs.len()];
    for (k, &(i, _)) in pairs.iter().enumerate() {
        let at = tails.partition_point(|&t| pairs[t].0 < i);
        back[k] = at.checked_sub(1).map(|p| tails[p]);
        match tails.get_mut(at) {
            Some(slot) => *slot = k,
            None => tails.push(k),
        }
    }
    let mut out = Vec::new();
    let mut k = tails.last().copied();
    while let Some(at) = k {
        out.push(pairs[at]);
        k = back[at];
    }
    out.reverse();
    out
}

fn table<T: PartialEq>(a: &[T], b: &[T]) -> Vec<(usize, usize)> {
    let width = b.len() + 1;
    let mut lengths = vec![0u32; (a.len() + 1) * width];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            lengths[i * width + j] = if a[i] == b[j] {
                lengths[(i + 1) * width + j + 1] + 1
            } else {
                lengths[(i + 1) * width + j].max(lengths[i * width + j + 1])
            };
        }
    }
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if a[i] == b[j] {
            out.push((i, j));
            (i, j) = (i + 1, j + 1);
        } else if lengths[(i + 1) * width + j] >= lengths[i * width + j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{BlockId, Run};

    fn doc(blocks: &[(BlockKind, &str)]) -> Document {
        let mut doc = Document::empty();
        for (n, (kind, text)) in blocks.iter().enumerate() {
            let mut block = Block::new(BlockId(n as u64), kind.clone());
            block.runs.push(Run::plain(*text));
            doc.blocks.push(block);
        }
        doc
    }

    fn paragraphs(texts: &[&str]) -> Document {
        let blocks: Vec<(BlockKind, &str)> =
            texts.iter().map(|t| (BlockKind::Paragraph, *t)).collect();
        doc(&blocks)
    }

    fn summary(diff: &Diff) -> Vec<String> {
        diff.blocks
            .iter()
            .map(|c| {
                let at = c.after.as_ref().or(c.before.as_ref()).unwrap();
                format!("{} {}", c.change.label(), at.at)
            })
            .collect()
    }

    #[test]
    fn a_document_against_itself_is_no_change() {
        let d = paragraphs(&["One.", "Two.", "Three."]);
        assert!(diff(&d, &d).is_empty());
    }

    #[test]
    fn an_insertion_above_leaves_the_rest_alone() {
        let before = paragraphs(&["One.", "Two.", "Three."]);
        let after = paragraphs(&["Zero.", "One.", "Two.", "Three."]);
        assert_eq!(summary(&diff(&before, &after)), ["added p1"]);
    }

    #[test]
    fn a_reworded_paragraph_is_changed_word_by_word() {
        let before = paragraphs(&["Intro.", "The quick brown fox jumps.", "End."]);
        let after = paragraphs(&["Intro.", "The slow brown fox jumps.", "End."]);
        let found = diff(&before, &after);
        assert_eq!(summary(&found), ["changed p2"]);
        assert_eq!(
            found.blocks[0].words,
            [
                Piece::Same("The ".to_owned()),
                Piece::Removed("quick".to_owned()),
                Piece::Added("slow".to_owned()),
                Piece::Same(" brown fox jumps.".to_owned()),
            ]
        );
    }

    #[test]
    fn a_paragraph_with_nothing_in_common_is_removed_and_another_added() {
        let before = paragraphs(&["Intro.", "Apples are red.", "End."]);
        let after = paragraphs(&["Intro.", "Totally different words here.", "End."]);
        assert_eq!(summary(&diff(&before, &after)), ["removed p2", "added p2"]);
    }

    #[test]
    fn a_paragraph_moved_elsewhere_is_a_move() {
        let before = paragraphs(&["A first.", "B second.", "C third.", "D fourth."]);
        let after = paragraphs(&["B second.", "C third.", "D fourth.", "A first."]);
        let found = diff(&before, &after);
        assert_eq!(summary(&found), ["moved p4"]);
        assert_eq!(found.blocks[0].before.as_ref().unwrap().at, "p1");
    }

    #[test]
    fn a_heading_made_a_paragraph_is_a_change_of_kind() {
        let before = doc(&[(BlockKind::Heading { level: 2 }, "Results")]);
        let after = doc(&[(BlockKind::Paragraph, "Results")]);
        let found = diff(&before, &after);
        assert_eq!(summary(&found), ["changed p1"]);
        assert_eq!(found.blocks[0].before.as_ref().unwrap().kind, "heading 2");
    }

    #[test]
    fn anchors_carry_a_document_too_big_for_one_table() {
        let texts: Vec<String> = (0..3000).map(|n| format!("Block {n}.")).collect();
        let before: Vec<&str> = texts.iter().map(String::as_str).collect();
        let mut after = before.clone();
        after[1500] = "Block 1500. Rewritten.";
        after.insert(10, "New.");
        // Original p2900, the 2,900th block: one was inserted above it.
        after.remove(2900);
        let found = diff(&paragraphs(&before), &paragraphs(&after));
        assert_eq!(
            summary(&found),
            ["added p11", "changed p1502", "removed p2900"]
        );
    }
}
