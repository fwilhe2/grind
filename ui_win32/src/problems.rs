// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The Check Document dialog (`doc/dsl.md` §4.3) — portable: a [`Report`] turned into the strings
//! a listbox shows. `win.rs`'s `check_document` hands the list to `dialog::choose`, the chooser
//! the outline uses, rather than a second list widget. Every row is a jump: picking one hands back
//! the [`Diagnostic`](grind_core::lint::Diagnostic) it came from, whose `at` either pane's go-to
//! reads.

use grind_core::lint::{Report, Severity};

/// One line per finding: a severity mark, then the [`Diagnostic`](grind_core::lint::Diagnostic)'s
/// `Display` — `{at}: {severity}: {message} [{rule}]`, a compiler's shape. In `Report`'s own
/// order, worst first, so a row's position is the index `dialog::choose` returns.
pub fn rows(report: &Report) -> Vec<String> {
    report
        .diagnostics
        .iter()
        .map(|diagnostic| format!("{} {diagnostic}", mark(diagnostic.severity)))
        .collect()
}

/// One glyph per severity, the terminal's three, since a `LISTBOX` cannot draw colour.
pub fn mark(severity: Severity) -> char {
    match severity {
        Severity::Error => '!',
        Severity::Warning => '▲',
        Severity::Hint => '·',
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grind_core::lint::{Diagnostic, Rule};

    const RULE: Rule = Rule {
        id: "test-rule",
        severity: Severity::Warning,
        what: "a rule made up for this test",
    };

    #[test]
    fn a_row_carries_its_marker_and_the_diagnostics_own_sentence() {
        let mut report = Report::default();
        report.push(Diagnostic::new(&RULE, "Sheet1.B12", "reads an empty cell"));
        let rows = rows(&report);
        assert_eq!(rows.len(), 1);
        assert!(rows[0].starts_with('▲'), "{}", rows[0]);
        assert!(rows[0].contains("Sheet1.B12"), "{}", rows[0]);
        assert!(rows[0].contains("reads an empty cell"), "{}", rows[0]);
        assert!(rows[0].contains("test-rule"), "{}", rows[0]);
    }

    #[test]
    fn an_empty_report_is_an_empty_list() {
        assert!(rows(&Report::default()).is_empty());
    }

    #[test]
    fn every_severity_has_a_mark_of_its_own() {
        let marks = [Severity::Error, Severity::Warning, Severity::Hint].map(mark);
        assert_eq!(marks.len(), 3);
        assert!(marks[0] != marks[1] && marks[1] != marks[2] && marks[0] != marks[2]);
    }
}
