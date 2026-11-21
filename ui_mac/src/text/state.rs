// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What the page's own dialogs read. The caret, the selection and every edit at them are
//! `grind_text::caret::Editor`'s; the Mac's reading of the keys is `keys.rs`.

/// A table's size as a person types it for Insert ▸ Table… — `3x2`, `3 × 2`, `3,2` — columns
/// first, as every word processor's dialog asks: `(columns, rows)`, each at least one.
pub fn table_size(text: &str) -> Option<(u32, u32)> {
    let mut parts = text.split(['x', 'X', '×', ',', '*']).map(str::trim);
    let columns = parts.next()?.parse::<u32>().ok().filter(|n| *n > 0)?;
    let rows = parts.next()?.parse::<u32>().ok().filter(|n| *n > 0)?;
    parts.next().is_none().then_some((columns, rows))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_table_size_is_columns_by_rows() {
        assert_eq!(table_size("3x2"), Some((3, 2)));
        assert_eq!(table_size(" 4 × 1 "), Some((4, 1)));
        assert_eq!(table_size("2,5"), Some((2, 5)));
        assert_eq!(table_size("0x2"), None, "a table has a column");
        assert_eq!(table_size("3"), None);
        assert_eq!(table_size("1x2x3"), None);
    }
}
