// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The "Check Document" dialog — D6, `doc/dsl.md` §4.3. **Portable, and tested on any host**:
//! turning a [`Report`] into the strings a listbox can show is arithmetic over data, the same
//! split every other file in this crate makes.
//!
//! There is no drawn pane here, unlike `ui_tui`'s `problems.rs` or the two GTK windows' own
//! dialogs: `win.rs`'s `check_document` builds this list and hands it to `dialog::choose`, the
//! same generic chooser popup `text_outline` and `text_block_kind_dialog` already use, because a
//! second list widget for one more list would be a second way of showing what is really the same
//! question — "pick one of these rows" — those two already answer. Every row is a jump: picking
//! one hands `win.rs` back the [`Diagnostic`] it came from, and `at` is an address either pane's
//! own go-to machinery already understands.

use grind_core::lint::{Report, Severity};

/// One line per finding: a severity mark, then the [`Diagnostic`]'s own `Display`, which already
/// reads `{at}: {severity}: {message} [{rule}]` — the shape every compiler in the world prints.
///
/// In the same order `Report`'s own `diagnostics` holds them (already sorted, worst first), so a
/// row's position in this list is the index `dialog::choose` hands back.
pub fn rows(report: &Report) -> Vec<String> {
    report
        .diagnostics
        .iter()
        .map(|diagnostic| format!("{} {diagnostic}", mark(diagnostic.severity)))
        .collect()
}

/// The same three glyphs `ui_tui::problems::mark` draws — chosen independently, for the same
/// reason: a shell that could draw colour would use it, and a `LISTBOX` cannot.
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
