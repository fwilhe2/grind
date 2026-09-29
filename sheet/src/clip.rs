// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The codec between a rectangle of cells and the tab-separated text a clipboard carries.
//!
//! TSV is the shape every other spreadsheet reads, so a rectangle copied in one shell has to
//! look the same as one copied in another, and both have to round-trip through Excel and
//! LibreOffice Calc. There were four copies of this — `ui_win32/src/sheet/clip.rs`,
//! `ui_sheet_gtk/src/grid.rs`, `ui_web/src/sheet/mod.rs` and `ui_tui/src/sheet/app.rs` — and
//! they had already drifted: the browser's and the terminal's never turned a pasted formula
//! from display syntax back into ODF's, so `=A1*2` copied and pasted there came back as
//! `#NAME?`. Hoisted when the macOS shell would have been the fifth (`doc/macos-shell.md`, M1).
//!
//! What a clipboard *is* stays a shell's: `CF_UNICODETEXT`, a `gdk::Clipboard`, the browser's
//! `navigator.clipboard`, a vi register, an `NSPasteboard`. This is only the text.

use crate::formula::display;
use crate::{App, Pos, Result};

/// Every cell in a rectangle, tab-separated, one line per row joined by `eol`, read through
/// `get`.
///
/// `get` is [`App::input_text`] for a copy — the raw number or the formula in display form
/// rather than what the cell *displays*: pasted back it reproduces the cells exactly, and pasted
/// into another spreadsheet `1234.5` is a number where `1,234.50 €` is a guess about that
/// program's locale — and [`App::value_text`] for a copy of values, the one place that guess is
/// exactly the point. A cell `get` cannot read is empty.
///
/// `eol` is the platform's: Windows' clipboard convention is `"\r\n"`, everybody else's `"\n"`.
///
/// ponytail: a cell holding a tab or a newline has them replaced with a space, so the rectangle
/// survives. The upgrade is quoting, here and nowhere else — a shell with a private dialect of
/// TSV is exactly what this module exists to prevent.
pub fn rect_text(
    app: &App,
    sheet: usize,
    start: Pos,
    end: Pos,
    get: impl Fn(&App, usize, Pos) -> Result<String>,
    eol: &str,
) -> String {
    (start.row..=end.row)
        .map(|row| {
            (start.col..=end.col)
                .map(|col| {
                    get(app, sheet, Pos::new(row, col))
                        .unwrap_or_default()
                        .replace(['\t', '\n', '\r'], " ")
                })
                .collect::<Vec<_>>()
                .join("\t")
        })
        .collect::<Vec<_>>()
        .join(eol)
}

/// Clipboard text back to the rows [`App::enter_range`] wants — display syntax back to
/// canonical, cell by cell, the same step a single typed cell goes through.
///
/// A formula that will not parse is passed through as typed, which `enter_range` then stores
/// verbatim rather than losing. `\r\n` and `\n` read alike, and a trailing line ending (which
/// every spreadsheet puts after the last row) is not an extra empty row.
pub fn parse_rows(text: &str) -> Vec<Vec<String>> {
    text.lines()
        .map(|line| {
            line.split('\t')
                .map(|cell| match cell.starts_with('=') {
                    true => display::from_display(cell).unwrap_or_else(|_| cell.to_owned()),
                    false => cell.to_owned(),
                })
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RecalcMode;

    fn sample() -> App {
        let app = App::new();
        app.enter(0, Pos::new(0, 0), "1", RecalcMode::Document)
            .unwrap();
        app.enter(0, Pos::new(0, 1), "=[.A1]*2", RecalcMode::Document)
            .unwrap();
        app.enter(0, Pos::new(1, 0), "hi\tthere", RecalcMode::Document)
            .unwrap();
        app.enter(0, Pos::new(1, 1), "2", RecalcMode::Document)
            .unwrap();
        app
    }

    #[test]
    fn a_rectangle_becomes_tab_and_line_separated_text() {
        let app = sample();
        let (a, b) = (Pos::new(0, 0), Pos::new(1, 1));
        assert_eq!(
            rect_text(&app, 0, a, b, App::input_text, "\r\n"),
            "1\t=A1*2\r\nhi there\t2"
        );
        assert_eq!(
            rect_text(&app, 0, a, b, App::input_text, "\n"),
            "1\t=A1*2\nhi there\t2"
        );
        assert_eq!(
            rect_text(&app, 0, a, b, App::value_text, "\n"),
            "1\t2\nhi there\t2"
        );
    }

    /// The bug the hoist fixed in two shells: a formula copied comes back a formula, not
    /// `#NAME?`, because display syntax is turned back into ODF's on the way in.
    #[test]
    fn a_pasted_formula_is_a_formula_again() {
        let app = sample();
        let text = rect_text(
            &app,
            0,
            Pos::new(0, 0),
            Pos::new(1, 1),
            App::input_text,
            "\n",
        );
        app.enter_range(0, Pos::new(3, 0), &parse_rows(&text), RecalcMode::Document)
            .unwrap();
        assert_eq!(app.input_text(0, Pos::new(3, 1)).unwrap(), "=A1*2");
        assert_eq!(app.value_text(0, Pos::new(3, 1)).unwrap(), "2");
    }

    /// A clipboard from another application arrives with CRLF line endings, and usually a
    /// trailing one.
    #[test]
    fn crlf_lf_and_a_trailing_line_end_parse_the_same() {
        let rows = parse_rows("1\t2\n3\t4");
        assert_eq!(parse_rows("1\t2\r\n3\t4"), rows);
        assert_eq!(parse_rows("1\t2\r\n3\t4\r\n"), rows);
        assert_eq!(rows.len(), 2);
    }

    /// Not a formula at all, or one this build cannot read: kept as typed.
    #[test]
    fn what_will_not_parse_is_passed_through() {
        assert_eq!(parse_rows("=(\tplain"), vec![vec!["=(", "plain"]]);
    }
}
