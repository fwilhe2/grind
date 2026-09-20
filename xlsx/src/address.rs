// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Excel's spelling of a cell address — `B2`, `XFD1048576` — as a 0-based [`Pos`].
//!
//! `grind_sheet::a1` is the workspace's one 0↔1 conversion for **ODF's** spelling, and it gets
//! there by wrapping the address in `[…]` and lexing it as a formula reference. This is the
//! same arithmetic for **Excel's** spelling, which is this crate's business rather than that
//! one's (R1: nothing Excel-shaped in `grind-sheet`), and which is asked half a million times
//! by `scale/large-sheet.xlsx` — once per `<c r="…">` — where a format-and-lex per cell would be
//! the most expensive thing the importer does. X2's formula lexer is the second caller.
//!
//! The two cannot disagree about what `B2` means: a test holds this function to `a1::parse`
//! over every boundary the grid has.

use grind_sheet::model::Pos;
use grind_sheet::{MAX_COLS, MAX_ROWS};

/// `B2` → row 1, column 1. `None` for anything that is not a plain relative address inside
/// the grid — a `$`, a sheet name, a range, lower-case letters past the spec, or a position
/// past `XFD1048576`.
///
/// Lower-case letters are accepted: ECMA-376's `ST_CellRef` is upper-case, and a producer that
/// writes `b2` means B2 rather than nothing.
pub fn cell(r: &str) -> Option<Pos> {
    let split = r.find(|c: char| !c.is_ascii_alphabetic())?;
    let (letters, digits) = r.split_at(split);
    if letters.is_empty() || letters.len() > 3 {
        return None;
    }
    let mut col: u32 = 0;
    for c in letters.bytes() {
        col = col * 26 + u32::from(c.to_ascii_uppercase() - b'A' + 1);
    }
    if !digits.bytes().all(|b| b.is_ascii_digit()) || digits.starts_with('0') {
        return None;
    }
    let row: u32 = digits.parse().ok()?;
    if col > MAX_COLS || row == 0 || row > MAX_ROWS {
        return None;
    }
    Some(Pos::new(row - 1, col - 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_addresses() {
        assert_eq!(cell("A1"), Some(Pos::new(0, 0)));
        assert_eq!(cell("B2"), Some(Pos::new(1, 1)));
        assert_eq!(cell("Z10"), Some(Pos::new(9, 25)));
        assert_eq!(cell("AA100"), Some(Pos::new(99, 26)));
        assert_eq!(cell("b2"), Some(Pos::new(1, 1)));
    }

    /// The last row and column are legal and a reader that rejects them rejects a legal file
    /// (`scale/wide-and-sparse.xlsx`); one past either is not.
    #[test]
    fn the_edges_of_the_grid() {
        assert_eq!(cell("XFD1048576"), Some(Pos::new(1_048_575, 16_383)));
        assert_eq!(cell("XFE1"), None);
        assert_eq!(cell("A1048577"), None);
        assert_eq!(cell("A0"), None);
    }

    #[test]
    fn anything_else_is_not_an_address() {
        for bad in [
            "",
            "A",
            "1",
            "$A$1",
            "A01",
            "A1:B2",
            "Sheet1!A1",
            "AAAA1",
            "A1 ",
            "Ä1",
        ] {
            assert_eq!(cell(bad), None, "{bad:?}");
        }
    }

    /// The property this file exists under: it is `a1`'s arithmetic, spelled Excel's way.
    #[test]
    fn it_agrees_with_the_workspaces_own_addressing() {
        for addr in [
            "A1",
            "B2",
            "Z1",
            "AA1",
            "AZ9",
            "BA10",
            "ZZ99",
            "AAA1",
            "XFD1048576",
        ] {
            let ours = cell(addr).expect(addr);
            let start = grind_sheet::a1::parse(addr).expect(addr).start;
            let theirs = (start.row.expect(addr).index, start.col.expect(addr).index);
            assert_eq!((ours.row, ours.col), theirs, "{addr}");
        }
    }
}
