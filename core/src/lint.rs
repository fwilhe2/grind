// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What a diagnostic is. **\[GENERIC\]**
//!
//! `doc/dsl.md` §4.3. The rules live with each application (R8); what is shared is the shape of an
//! answer — a rule id, a severity, an address and a sentence — so a shell's squiggle, `--format
//! json` and a terminal's text render one list. The address is the application's own spelling
//! (`Sheet1.B12`, `p12`), a string here.

use std::fmt;

use crate::projection::{Location, Source};

/// How loud a diagnostic is. Three levels, so that a dangling bookmark (wrong) and a colour off
/// the shell's palette (merely unlike the defaults) need not be the same.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// The document contradicts itself, and something a reader sees is wrong.
    Error,
    /// The document loses something, or says something it probably did not mean.
    Warning,
    /// A house-style remark. Off unless [`Options::hints`] asks for it.
    Hint,
}

impl Severity {
    pub fn label(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Hint => "hint",
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// One rule, as data: turned off by id, listed by `grind lint --rules`, and held to the table in
/// `doc/dsl.md` §4.3 by a test in both directions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rule {
    /// Kebab-case, unique across both applications: it is what a user types after `--off` and
    /// what a JSON consumer matches on.
    pub id: &'static str,
    pub severity: Severity,
    /// One line, lower case, no full stop — the same register as a compiler's rule list.
    pub what: &'static str,
}

/// One finding.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Diagnostic {
    /// The [`Rule`] that produced it.
    pub rule: &'static str,
    pub severity: Severity,
    /// Where, in the application's own addressing — `Sheet1.B12`, `p12`, `#intro`. Empty for a
    /// finding about the document as a whole.
    pub at: String,
    /// What is wrong, as a sentence fragment a shell can put after the address.
    pub message: String,
    /// Where [`Diagnostic::at`] is in the file, when the document was read from a projection that
    /// spelled it. Filled by [`Report::locate_in`], never by a rule.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<Location>,
}

impl Diagnostic {
    pub fn new(rule: &Rule, at: impl Into<String>, message: impl Into<String>) -> Self {
        Diagnostic {
            rule: rule.id,
            severity: rule.severity,
            at: at.into(),
            message: message.into(),
            location: None,
        }
    }
}

impl fmt::Display for Diagnostic {
    /// `Sheet1.B12: warning: reads the empty cell Sheet1.C4 [empty-reference]` — the shape an
    /// editor's error parser already understands.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.at.is_empty() {
            write!(f, "{}: ", self.at)?;
        }
        write!(f, "{}: {} [{}]", self.severity, self.message, self.rule)
    }
}

/// What to check: every rule, optionally the house-style hints, less the ids in `off`.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// Report [`Severity::Hint`] rules too. Off by default (`doc/dsl.md` §4.3).
    pub hints: bool,
    /// Rule ids to skip.
    pub off: Vec<String>,
}

impl Options {
    /// Whether a rule should run at all. Checked once per rule rather than per finding, so a
    /// silenced rule costs nothing.
    pub fn wants(&self, rule: &Rule) -> bool {
        (self.hints || rule.severity != Severity::Hint)
            && !self.off.iter().any(|off| off == rule.id)
    }
}

/// How many findings one lint will report before it stops looking; reaching it sets
/// [`Report::truncated`] rather than returning a silently short list.
pub const MAX_DIAGNOSTICS: usize = 1000;

/// A lint's whole answer: what it found, and whether it stopped early.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct Report {
    pub diagnostics: Vec<Diagnostic>,
    /// Whether [`MAX_DIAGNOSTICS`] was reached. A caller that says "clean" must check it.
    pub truncated: bool,
}

impl Report {
    /// Push one finding, unless the cap is reached. `false` once it is, which is a caller's
    /// signal to stop generating them.
    pub fn push(&mut self, diagnostic: Diagnostic) -> bool {
        if self.diagnostics.len() >= MAX_DIAGNOSTICS {
            self.truncated = true;
            return false;
        }
        self.diagnostics.push(diagnostic);
        true
    }

    /// Sort by severity, then by rule, then by address — the order a person reads, and stable
    /// across runs so two lints of one document can be diffed.
    pub fn sort(&mut self) {
        self.diagnostics
            .sort_by(|a, b| (a.severity, a.rule, &a.at).cmp(&(b.severity, b.rule, &b.at)));
    }

    /// Give every finding the line and column its address has in `source`, the projection the
    /// document was read from. An address the file never spelled gets no location.
    pub fn locate_in(&mut self, source: &Source) {
        for diagnostic in &mut self.diagnostics {
            diagnostic.location = source.locate(&diagnostic.at);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.diagnostics.is_empty()
    }

    pub fn len(&self) -> usize {
        self.diagnostics.len()
    }

    /// How many findings are at least this loud — what an exit status is made of.
    pub fn count(&self, severity: Severity) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.severity <= severity)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RULE: Rule = Rule {
        id: "empty-reference",
        severity: Severity::Warning,
        what: "a formula reads a cell that is empty",
    };

    const HINT: Rule = Rule {
        id: "off-palette",
        severity: Severity::Hint,
        what: "a colour outside the default palette",
    };

    #[test]
    fn a_hint_runs_only_when_it_is_asked_for() {
        let quiet = Options::default();
        assert!(quiet.wants(&RULE));
        assert!(!quiet.wants(&HINT), "hints are off by default (§4.3)");

        let loud = Options {
            hints: true,
            off: vec!["empty-reference".to_owned()],
        };
        assert!(loud.wants(&HINT));
        assert!(!loud.wants(&RULE), "and any rule can be silenced by id");
    }

    #[test]
    fn a_diagnostic_prints_the_way_a_compiler_does() {
        let d = Diagnostic::new(&RULE, "Sheet1.B12", "reads the empty cell Sheet1.C4");
        assert_eq!(
            d.to_string(),
            "Sheet1.B12: warning: reads the empty cell Sheet1.C4 [empty-reference]"
        );
        // A finding about the whole document has no address and does not print an empty one.
        let d = Diagnostic::new(&RULE, "", "something about the document");
        assert_eq!(
            d.to_string(),
            "warning: something about the document [empty-reference]"
        );
    }

    #[test]
    fn the_cap_is_reported_rather_than_silently_applied() {
        let mut report = Report::default();
        for i in 0..MAX_DIAGNOSTICS + 10 {
            report.push(Diagnostic::new(&RULE, format!("A{i}"), "no"));
        }
        assert_eq!(report.len(), MAX_DIAGNOSTICS);
        assert!(report.truncated);
    }

    #[test]
    fn sorting_puts_the_loudest_first_and_is_stable_across_runs() {
        let mut report = Report::default();
        report.push(Diagnostic::new(&HINT, "B1", "a colour"));
        report.push(Diagnostic::new(&RULE, "A2", "empty"));
        report.push(Diagnostic::new(&RULE, "A1", "empty"));
        report.sort();
        let order: Vec<_> = report.diagnostics.iter().map(|d| d.at.as_str()).collect();
        assert_eq!(order, ["A1", "A2", "B1"]);
        assert_eq!(report.count(Severity::Warning), 2, "hints are quieter");
    }
}
