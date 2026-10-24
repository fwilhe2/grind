// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The codec between a rectangle of cells and the tab-separated text a clipboard carries — the
//! shape every spreadsheet reads, so a copy round-trips through Excel and LibreOffice Calc. What a
//! clipboard is (`CF_UNICODETEXT`, a `gdk::Clipboard`, `navigator.clipboard`, a vi register, an
//! `NSPasteboard`) stays a shell's.

use crate::formula::display;
use crate::{App, Pos, Result};

/// Every cell in a rectangle, tab-separated, one row per line joined by `eol` (`"\r\n"` on
/// Windows), read through `get`. For a copy `get` is [`App::input_text`] — the raw number or the
/// formula in display form, which pastes back exactly — and for a copy of values
/// [`App::value_text`]. A cell `get` cannot read is empty.
///
/// ponytail: tabs and newlines inside a cell become spaces; quoting, here and nowhere else, is the
/// upgrade.
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

/// Clipboard text back to the rows [`App::enter_range`] wants, display syntax converted to
/// canonical cell by cell. A formula that will not parse passes through as typed. `\r\n` and `\n`
/// read alike, and a trailing line ending is not an extra row.
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

    /// A copied formula pastes back as a formula, not `#NAME?`: display syntax is converted on the
    /// way in.
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
