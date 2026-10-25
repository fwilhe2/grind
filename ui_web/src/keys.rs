// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! *Keyboard shortcuts* (F1): every key a pane answers, in a `<dialog>`. The moves are listed
//! here, as each pane's `keymap.rs` answers them; the rest are the command tables' own `keys`, so
//! a shortcut added to a row is listed without being written twice.

use crate::about::escape;
use crate::command::Command;

/// The keys the palette itself answers, over either pane.
const CHROME: &[(&str, &str)] = &[
    ("Ctrl+K", "Commands, cells and places — type to search"),
    ("Ctrl+F", "The same list, to find text"),
    ("F1", "These shortcuts"),
];

const GRID: &[(&str, &str)] = &[
    ("Arrows", "Move one cell; with Shift, extend the selection"),
    ("Ctrl+Arrows", "Jump to the edge of the data"),
    ("Home, End", "Start or end of the row"),
    ("Ctrl+Home, Ctrl+End", "Start or end of the sheet"),
    ("Page Up, Page Down", "Move a screen"),
    (
        "Enter, Tab",
        "Commit and move down or right; with Shift, up or left",
    ),
    ("F2", "Edit the cell"),
    ("Escape", "Cancel the edit"),
    ("Delete", "Clear the selection"),
    ("F3, Shift+F3", "Next or previous match"),
    ("F9", "Recalculate"),
    ("Ctrl+D, Ctrl+R", "Fill down, fill right"),
    ("Ctrl+Shift+L", "Filter rows"),
    (
        "Ctrl+Plus, Ctrl+Minus, Ctrl+0",
        "Zoom in, out, back to 100%",
    ),
];

const PAGE: &[(&str, &str)] = &[
    (
        "Arrows",
        "Move by character or line; with Shift, extend the selection",
    ),
    ("Home, End", "Start or end of the line"),
    ("Ctrl+Home, Ctrl+End", "Start or end of the document"),
    ("Page Up, Page Down", "Move a screen"),
    (
        "Tab, Shift+Tab",
        "A list item one level deeper or shallower; elsewhere Tab is a tab",
    ),
    ("F3, Shift+F3", "Next or previous match"),
    ("Ctrl+0 … Ctrl+3", "Paragraph, Heading 1 to 3"),
];

/// The dialog's content for a pane: its moves (`grid`), then the table's rows that have a key.
pub fn html(table: &[Command], grid: bool) -> String {
    let moves = match grid {
        true => GRID,
        false => PAGE,
    };
    let mut out = String::from("<h2>Keyboard shortcuts</h2><table class=\"keys\">");
    let rows = CHROME.iter().chain(moves).copied().chain(
        table
            .iter()
            .filter(|command| !command.keys.is_empty())
            .map(|command| (command.keys, command.title)),
    );
    let mut seen = Vec::new();
    for (keys, what) in rows {
        if seen.contains(&keys) {
            continue;
        }
        seen.push(keys);
        out.push_str(&format!(
            "<tr><th><kbd>{}</kbd></th><td>{}</td></tr>",
            escape(keys),
            escape(what)
        ));
    }
    out.push_str("</table>");
    out.push_str(
        "<form method=\"dialog\"><button class=\"about-close\" type=\"submit\">Close</button></form>",
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{SHEET, TEXT};

    #[test]
    fn each_pane_lists_its_moves_and_its_rows_keys_once() {
        let sheet = html(SHEET, true);
        assert!(sheet.contains("Ctrl+Arrows") && sheet.contains("Ctrl+S"));
        assert_eq!(sheet.matches("<kbd>Ctrl+H</kbd>").count(), 1);
        let text = html(TEXT, false);
        assert!(text.contains("Ctrl+Shift+E"), "from the table's own row");
        assert!(!text.contains("Ctrl+Arrows"));
    }
}
