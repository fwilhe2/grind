// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The column store: run-length blocks of homogeneous type. A column is mostly empty, and where it
//! is not, long runs share one type, so it is a sequence of typed blocks: `Empty(1_000_000)` costs
//! four bytes and a million numbers one `Vec<f64>`.
//!
//! LibreOffice Calc arrived at the same design (`mdds::mtv::soa::multi_type_vector`, observed at
//! `sc/inc/mtvelements.hxx:141-153` and `sc/inc/column.hxx:164`); this was reached independently,
//! with no LibreOffice code (see CONTRIBUTING.md).
//!
//! ponytail: text and bool share one `Other` block; splitting it is a one-variant change if a
//! profile asks.
//!

use crate::model::CellValue;

/// What kind of block a value belongs in. Two values of the same kind can share a block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Empty,
    Number,
    Other,
}

fn kind_of(v: &CellValue) -> Kind {
    match v {
        CellValue::Empty => Kind::Empty,
        CellValue::Number(_) => Kind::Number,
        CellValue::Text(_) | CellValue::Bool(_) => Kind::Other,
    }
}

/// Serialised only inside a whole [`crate::Sheet`], for undoing a sheet deletion.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
enum Block {
    Empty(u32),
    Number(Vec<f64>),
    Other(Vec<CellValue>),
}

impl Block {
    fn len(&self) -> u32 {
        match self {
            Block::Empty(n) => *n,
            Block::Number(v) => v.len() as u32,
            Block::Other(v) => v.len() as u32,
        }
    }

    fn kind(&self) -> Kind {
        match self {
            Block::Empty(_) => Kind::Empty,
            Block::Number(_) => Kind::Number,
            Block::Other(_) => Kind::Other,
        }
    }

    fn get(&self, off: u32) -> CellValue {
        match self {
            Block::Empty(_) => CellValue::Empty,
            Block::Number(v) => CellValue::Number(v[off as usize]),
            Block::Other(v) => v[off as usize].clone(),
        }
    }

    /// A block holding exactly one value.
    fn one(v: CellValue) -> Block {
        match v {
            CellValue::Empty => Block::Empty(1),
            CellValue::Number(n) => Block::Number(vec![n]),
            other => Block::Other(vec![other]),
        }
    }

    /// Split at `at`, keeping `[0, at)` here and returning `[at, len)`.
    fn split_off(&mut self, at: u32) -> Block {
        match self {
            Block::Empty(n) => {
                let rest = *n - at;
                *n = at;
                Block::Empty(rest)
            }
            Block::Number(v) => Block::Number(v.split_off(at as usize)),
            Block::Other(v) => Block::Other(v.split_off(at as usize)),
        }
    }

    /// Absorb `other`, which must be the same kind.
    fn append(&mut self, other: Block) {
        match (self, other) {
            (Block::Empty(a), Block::Empty(b)) => *a += b,
            (Block::Number(a), Block::Number(b)) => a.extend(b),
            (Block::Other(a), Block::Other(b)) => a.extend(b),
            _ => unreachable!("append across kinds"),
        }
    }
}

/// One column of one sheet. A column whose type alternates every row (dates mixed with dates
/// typed as text) is a block per row, so blocks are found by binary search over where each ends.
#[derive(Debug, Default, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(from = "Stored", into = "Stored")]
pub struct Column {
    blocks: Vec<Block>,
    /// One past the last row of each block, derived from `blocks`.
    ends: Vec<u32>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Stored {
    blocks: Vec<Block>,
}

impl From<Stored> for Column {
    fn from(stored: Stored) -> Self {
        let mut column = Column::default();
        for block in stored.blocks {
            column.push(block);
        }
        column.trim();
        column
    }
}

impl From<Column> for Stored {
    fn from(column: Column) -> Self {
        Stored {
            blocks: column.blocks,
        }
    }
}

impl Column {
    /// One past the last row holding a value; trailing empties are never stored.
    pub fn len(&self) -> u32 {
        self.ends.last().copied().unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// The rows holding a value, one range per non-empty block, in order — one step per block,
    /// where a million empty rows are one block.
    pub fn occupied(&self) -> impl Iterator<Item = std::ops::Range<u32>> + '_ {
        self.blocks
            .iter()
            .enumerate()
            .filter(|(_, block)| block.kind() != Kind::Empty)
            .map(|(i, _)| self.start(i)..self.ends[i])
    }

    pub fn get(&self, row: u32) -> CellValue {
        match self.find(row) {
            Some((bi, off)) => self.blocks[bi].get(off),
            None => CellValue::Empty,
        }
    }

    pub fn set(&mut self, row: u32, value: CellValue) {
        let Some((bi, off)) = self.find(row) else {
            // Past the end: clearing writes nothing; otherwise pad with one empty run and append.
            if value.is_empty() {
                return;
            }
            let pad = row - self.len();
            if pad > 0 {
                self.push(Block::Empty(pad));
            }
            self.push(Block::one(value));
            return;
        };

        // Same kind as the block it lands in: overwrite in place, no reshaping.
        if self.blocks[bi].kind() == kind_of(&value) {
            match (&mut self.blocks[bi], value) {
                (Block::Empty(_), _) => {}
                (Block::Number(v), CellValue::Number(n)) => v[off as usize] = n,
                (Block::Other(v), other) => v[off as usize] = other,
                _ => unreachable!("kind matched but variants did not"),
            }
            return;
        }

        // Different kind: split the block around the target and splice the new one in. Every
        // other block keeps its rows, so only the neighbourhood needs normalising.
        let start = self.start(bi);
        let mut head = self.blocks.remove(bi);
        self.ends.remove(bi);
        let tail = head.split_off(off + 1); // rows after the target
        let _target = head.split_off(off); // the target itself, discarded
        let mut at = bi;
        let mut end = start;
        for block in [head, Block::one(value), tail] {
            if block.len() > 0 {
                end += block.len();
                self.blocks.insert(at, block);
                self.ends.insert(at, end);
                at += 1;
            }
        }
        let mut i = bi.saturating_sub(1);
        while i + 1 < self.blocks.len() && i <= at {
            if self.blocks[i].kind() == self.blocks[i + 1].kind() {
                let next = self.blocks.remove(i + 1);
                self.blocks[i].append(next);
                self.ends.remove(i);
                at -= 1;
            } else {
                i += 1;
            }
        }
        self.trim();
    }

    /// The first row of block `i`.
    fn start(&self, i: usize) -> u32 {
        i.checked_sub(1).map_or(0, |before| self.ends[before])
    }

    /// Locate `row`: which block holds it, and how far into that block it sits.
    fn find(&self, row: u32) -> Option<(usize, u32)> {
        let i = self.ends.partition_point(|&end| end <= row);
        (i < self.blocks.len()).then(|| (i, row - self.start(i)))
    }

    /// Add a block at the end, into the last one when they are the same kind.
    fn push(&mut self, block: Block) {
        if block.len() == 0 {
            return;
        }
        let end = self.len() + block.len();
        match self.blocks.last_mut() {
            Some(last) if last.kind() == block.kind() => {
                last.append(block);
                *self.ends.last_mut().unwrap() = end;
            }
            _ => {
                self.blocks.push(block);
                self.ends.push(end);
            }
        }
    }

    /// Drop a trailing empty run, which is never stored.
    fn trim(&mut self) {
        while matches!(self.blocks.last(), Some(Block::Empty(_))) {
            self.blocks.pop();
            self.ends.pop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every invariant a column keeps after a write.
    #[track_caller]
    fn check(c: &Column) {
        let mut end = 0;
        let ends: Vec<u32> = c
            .blocks
            .iter()
            .map(|b| {
                end += b.len();
                end
            })
            .collect();
        assert_eq!(c.ends, ends, "stale ends for {:?}", c.blocks);
        assert!(
            c.blocks.iter().all(|b| b.len() > 0),
            "zero-length block: {:?}",
            c.blocks
        );
        assert!(
            c.blocks.windows(2).all(|w| w[0].kind() != w[1].kind()),
            "adjacent blocks of the same kind: {:?}",
            c.blocks
        );
        assert!(
            !matches!(c.blocks.last(), Some(Block::Empty(_))),
            "trailing empty run: {:?}",
            c.blocks
        );
    }

    fn block_count(c: &Column) -> usize {
        c.blocks.len()
    }

    #[test]
    fn empty_column_reads_empty_everywhere() {
        let c = Column::default();
        assert_eq!(c.len(), 0);
        assert!(c.is_empty());
        assert_eq!(c.get(0), CellValue::Empty);
        assert_eq!(c.get(999_999), CellValue::Empty);
        check(&c);
    }

    #[test]
    fn a_far_cell_costs_one_empty_run_not_a_million_cells() {
        let mut c = Column::default();
        c.set(1_000_000, 42.0.into());
        assert_eq!(c.len(), 1_000_001);
        assert_eq!(c.get(1_000_000), CellValue::Number(42.0));
        assert_eq!(c.get(0), CellValue::Empty);
        // One padding run plus one value block — not a million entries.
        assert_eq!(block_count(&c), 2);
        check(&c);
    }

    #[test]
    fn a_run_of_one_type_stays_one_block() {
        let mut c = Column::default();
        for row in 0..1000 {
            c.set(row, f64::from(row).into());
        }
        assert_eq!(block_count(&c), 1);
        assert_eq!(c.len(), 1000);
        assert_eq!(c.get(500), CellValue::Number(500.0));
        check(&c);
    }

    #[test]
    fn overwriting_within_a_block_does_not_reshape_it() {
        let mut c = Column::default();
        for row in 0..100 {
            c.set(row, 1.0.into());
        }
        assert_eq!(block_count(&c), 1);
        c.set(50, 2.0.into());
        assert_eq!(block_count(&c), 1, "same-kind overwrite must not split");
        assert_eq!(c.get(50), CellValue::Number(2.0));
        check(&c);
    }

    #[test]
    fn a_foreign_type_splits_a_block_in_three() {
        let mut c = Column::default();
        for row in 0..100 {
            c.set(row, 1.0.into());
        }
        c.set(50, "hello".into());
        assert_eq!(block_count(&c), 3, "expected number | other | number");
        assert_eq!(c.get(49), CellValue::Number(1.0));
        assert_eq!(c.get(50), CellValue::Text("hello".into()));
        assert_eq!(c.get(51), CellValue::Number(1.0));
        assert_eq!(c.len(), 100);
        check(&c);
    }

    #[test]
    fn splitting_at_a_block_edge_makes_two_blocks_not_three() {
        let mut c = Column::default();
        for row in 0..10 {
            c.set(row, 1.0.into());
        }
        c.set(0, "first".into());
        assert_eq!(block_count(&c), 2);
        check(&c);

        let mut c = Column::default();
        for row in 0..10 {
            c.set(row, 1.0.into());
        }
        c.set(9, "last".into());
        assert_eq!(block_count(&c), 2);
        check(&c);
    }

    #[test]
    fn healing_a_split_merges_the_neighbours_back() {
        let mut c = Column::default();
        for row in 0..100 {
            c.set(row, 1.0.into());
        }
        c.set(50, "hole".into());
        assert_eq!(block_count(&c), 3);
        c.set(50, 7.0.into());
        assert_eq!(
            block_count(&c),
            1,
            "the three blocks must merge back into one"
        );
        assert_eq!(c.get(50), CellValue::Number(7.0));
        check(&c);
    }

    #[test]
    fn clearing_the_last_cell_shrinks_the_column() {
        let mut c = Column::default();
        c.set(0, 1.0.into());
        c.set(5, 2.0.into());
        assert_eq!(c.len(), 6);
        c.set(5, CellValue::Empty);
        assert_eq!(c.len(), 1, "trailing empties must not be stored");
        check(&c);
        c.set(0, CellValue::Empty);
        assert!(c.is_empty());
        check(&c);
    }

    #[test]
    fn clearing_a_middle_cell_leaves_a_hole_not_a_shift() {
        let mut c = Column::default();
        for row in 0..10 {
            c.set(row, 1.0.into());
        }
        c.set(4, CellValue::Empty);
        assert_eq!(c.get(3), CellValue::Number(1.0));
        assert_eq!(c.get(4), CellValue::Empty);
        assert_eq!(c.get(5), CellValue::Number(1.0));
        assert_eq!(c.len(), 10, "clearing must not move other cells");
        check(&c);
    }

    #[test]
    fn clearing_a_cell_that_was_never_set_is_a_no_op() {
        let mut c = Column::default();
        c.set(100, CellValue::Empty);
        assert!(c.is_empty());
        check(&c);
    }

    #[test]
    fn text_and_bool_share_a_block_but_numbers_do_not() {
        let mut c = Column::default();
        c.set(0, "a".into());
        c.set(1, true.into());
        assert_eq!(block_count(&c), 1);
        c.set(2, 1.0.into());
        assert_eq!(block_count(&c), 2);
        assert_eq!(c.get(0), CellValue::Text("a".into()));
        assert_eq!(c.get(1), CellValue::Bool(true));
        assert_eq!(c.get(2), CellValue::Number(1.0));
        check(&c);
    }

    /// A deterministic pseudo-random write sequence, checking the invariants and the values
    /// against a reference model after every step.
    #[test]
    fn random_writes_agree_with_a_dumb_reference_model() {
        const ROWS: u32 = 40;
        let mut c = Column::default();
        let mut reference = vec![CellValue::Empty; ROWS as usize];
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };

        for step in 0..4000 {
            let row = (next() % u64::from(ROWS)) as u32;
            let value = match next() % 4 {
                0 => CellValue::Empty,
                1 => CellValue::Number((next() % 100) as f64),
                2 => CellValue::Text(format!("t{}", next() % 10)),
                _ => CellValue::Bool(next() % 2 == 0),
            };
            c.set(row, value.clone());
            reference[row as usize] = value;

            check(&c);
            if step % 500 == 0 {
                let json = serde_json::to_string(&c).unwrap();
                let back: Column = serde_json::from_str(&json).unwrap();
                check(&back);
                assert_eq!(back, c, "serde round trip at step {step}");
            }
            for (row, want) in reference.iter().enumerate() {
                assert_eq!(
                    &c.get(row as u32),
                    want,
                    "row {row} diverged at step {step}"
                );
            }
            let want_len = reference
                .iter()
                .rposition(|v| !v.is_empty())
                .map_or(0, |i| i as u32 + 1);
            assert_eq!(c.len(), want_len, "len diverged at step {step}");
        }
    }
}
