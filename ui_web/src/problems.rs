// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The **problems pane** — `grind lint`'s findings, in a browser tab (`doc/dsl.md` §4.3, D6).
//!
//! **Shared by both panes**, like [`crate::code`] and for the same reason, which is stronger
//! here than it is there: a diagnostic is document-type-neutral *by construction*
//! (`grind_core::lint`), so a spreadsheet's findings and a text document's are one list and one
//! stylesheet. What differs is what an address means, and neither this file nor its markup has
//! an opinion about that — the row carries the string and the pane resolves it through the
//! `select_projected` it already has for the code view.
//!
//! The HTML is a string rather than `Document::create_element` calls, exactly as
//! [`crate::code`], `chart::svg` and `text::runs` already are: one `set_inner_html` per render,
//! and one escaping function every path goes through.

use grind_core::lint::{Report, Severity};

/// The findings as markup: a header with the tally, then one row per finding.
///
/// Each row carries `data-address`, which is what a click resolves — the same trick
/// `code::html` plays with `data-line`, and for the same reason: the pane is rebuilt whole on
/// every render, so a listener per row would be a hundred closures to leak.
pub fn html(report: &Report) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "<div class=\"problems-summary\">{}</div>",
        escape(&summary(report))
    ));
    for diagnostic in &report.diagnostics {
        out.push_str(&format!(
            "<div class=\"problem problem-{severity}\" data-address=\"{address}\" \
             role=\"button\" tabindex=\"0\">\
             <span class=\"problem-mark\" aria-hidden=\"true\">{mark}</span>\
             <span class=\"problem-at\">{at}</span>\
             <span class=\"problem-message\">{message}</span>\
             <span class=\"problem-rule\">{rule}</span></div>",
            severity = diagnostic.severity.label(),
            address = escape(&diagnostic.at),
            mark = mark(diagnostic.severity),
            at = escape(&diagnostic.at),
            message = escape(&diagnostic.message),
            rule = escape(diagnostic.rule),
        ));
    }
    out
}

/// The header line: what was found, or that nothing was.
pub fn summary(report: &Report) -> String {
    if report.is_empty() {
        return "No problems found".to_owned();
    }
    let count = |severity| {
        report
            .diagnostics
            .iter()
            .filter(|d| d.severity == severity)
            .count()
    };
    let mut parts = Vec::new();
    for (severity, one, many) in [
        (Severity::Error, "error", "errors"),
        (Severity::Warning, "warning", "warnings"),
        (Severity::Hint, "hint", "hints"),
    ] {
        match count(severity) {
            0 => {}
            1 => parts.push(format!("1 {one}")),
            n => parts.push(format!("{n} {many}")),
        }
    }
    let mut summary = parts.join(", ");
    if report.truncated {
        summary.push_str(" (stopped counting)");
    }
    summary
}

/// One character per severity. A page has no icon theme either — `ui_tui`'s three, so the two
/// shells that draw with characters draw the same ones.
fn mark(severity: Severity) -> char {
    match severity {
        Severity::Error => '!',
        Severity::Warning => '▲',
        Severity::Hint => '·',
    }
}

/// The four that matter inside an element and inside a double-quoted attribute — the same
/// function `code.rs` has, kept here so this file has no dependency on the other's internals.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use grind_core::lint::{Diagnostic, Rule};

    const RULE: Rule = Rule {
        id: "stale-value",
        severity: Severity::Warning,
        what: "a cell whose cached value disagrees with its formula",
    };

    const ERROR: Rule = Rule {
        id: "missing-sheet",
        severity: Severity::Error,
        what: "a formula referencing a sheet that does not exist",
    };

    #[test]
    fn nothing_found_says_so_and_draws_no_rows() {
        let report = Report::default();
        let html = html(&report);
        assert!(html.contains("No problems found"), "{html}");
        assert!(!html.contains("class=\"problem "), "{html}");
    }

    #[test]
    fn a_row_carries_the_address_a_click_resolves() {
        let mut report = Report::default();
        report.push(Diagnostic::new(&ERROR, "Sheet1.B12", "names a ghost"));
        let html = html(&report);
        assert!(html.contains("data-address=\"Sheet1.B12\""), "{html}");
        assert!(
            html.contains("problem-error"),
            "the severity is a class, so the stylesheet colours it: {html}"
        );
        assert!(html.contains("missing-sheet"), "and the rule is nameable");
    }

    /// A document's own text reaches this markup — a sheet called `<b>` or a message quoting a
    /// cell's contents — so every field goes through the escaper, attributes included.
    #[test]
    fn everything_a_document_supplied_is_escaped() {
        let mut report = Report::default();
        report.push(Diagnostic::new(
            &RULE,
            "'<b>\"'.A1",
            "holds \"<script>\" & more",
        ));
        let html = html(&report);
        assert!(!html.contains("<script>"), "{html}");
        assert!(!html.contains("<b>"), "{html}");
        assert!(html.contains("&amp; more"), "{html}");
        assert!(
            html.contains("data-address=\"&#x27;&lt;b&gt;&quot;&#x27;.A1\"")
                || html.contains("data-address=\"'&lt;b&gt;&quot;'.A1\""),
            "the attribute cannot be broken out of: {html}"
        );
    }

    #[test]
    fn the_summary_counts_each_severity() {
        let mut report = Report::default();
        report.push(Diagnostic::new(&ERROR, "A1", "x"));
        report.push(Diagnostic::new(&RULE, "A2", "x"));
        report.push(Diagnostic::new(&RULE, "A3", "x"));
        assert_eq!(summary(&report), "1 error, 2 warnings");
    }
}
