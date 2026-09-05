// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The codec between a rectangle of cells and the tab-separated text a clipboard carries.
//!
//! **Portable, and tested on any host**, like every other file in this module: it takes an
//! `&App` and two positions and returns a `String`, or the reverse, and knows nothing about
//! `CF_UNICODETEXT` or any other Windows type — `crate::clipboard` is where those live.
//! `doc/windows-shell.md` decision 6 is what this exists for: TSV is the shape every other
//! spreadsheet reads, so a rectangle copied here has to look the same as one `ui_sheet_gtk`
//! copies, and both have to round-trip through Excel and LibreOffice Calc.

use grind_sheet::formula::display;
use grind_sheet::{App, Pos, Result};

/// Every cell in a rectangle, tab- and newline-separated, read through `get` —
/// `App::input_text`, the raw number or the formula in display form rather than what the cell
/// *displays*: pasted back here it reproduces the cells exactly, and pasted into another
/// spreadsheet `1234.5` is a number where `1,234.50 €` is a guess about that program's locale.
///
/// ponytail: a cell holding a tab or a newline has them replaced with a space, so the
/// rectangle survives. The upgrade is quoting, in a codec no shell may grow a private dialect
/// of (`doc/plan.md` rule 4).
pub fn rect_text(
    app: &App,
    sheet: usize,
    start: Pos,
    end: Pos,
    get: impl Fn(&App, usize, Pos) -> Result<String>,
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
        .join("\r\n")
}

/// Clipboard text back to the rows `App::enter_range` wants — display syntax back to canonical,
/// cell by cell, the same step a single typed cell goes through. A formula that will not parse
/// is passed through as typed, which `enter_range` then stores verbatim rather than losing.
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
    use grind_sheet::RecalcMode;

    fn sample() -> App {
        let app = App::new();
        app.enter(0, Pos::new(0, 0), "1", RecalcMode::Document)
            .unwrap();
        app.enter(0, Pos::new(0, 1), "=A1*2", RecalcMode::Document)
            .unwrap();
        app.enter(0, Pos::new(1, 0), "hi\tthere", RecalcMode::Document)
            .unwrap();
        app.enter(0, Pos::new(1, 1), "2", RecalcMode::Document)
            .unwrap();
        app
    }

    #[test]
    fn a_rectangle_becomes_tab_and_crlf_separated_text() {
        let app = sample();
        let text = rect_text(&app, 0, Pos::new(0, 0), Pos::new(1, 1), App::input_text);
        assert_eq!(text, "1\t=A1*2\r\nhi there\t2");
    }

    #[test]
    fn pasted_text_round_trips_through_enter_range() {
        let app = sample();
        let text = rect_text(&app, 0, Pos::new(0, 0), Pos::new(1, 1), App::input_text);
        let rows = parse_rows(&text);
        app.enter_range(0, Pos::new(3, 0), &rows, RecalcMode::Document)
            .unwrap();
        assert_eq!(app.input_text(0, Pos::new(3, 0)).unwrap(), "1");
        assert_eq!(app.input_text(0, Pos::new(3, 1)).unwrap(), "=A1*2");
    }

    /// A clipboard from another application arrives with CRLF line endings; `str::lines`
    /// already treats a lone `\n` and a `\r\n` alike, so both have to parse to the same rows.
    #[test]
    fn crlf_and_lf_parse_the_same() {
        assert_eq!(parse_rows("1\t2\r\n3\t4"), parse_rows("1\t2\n3\t4"));
    }
}
