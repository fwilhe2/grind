// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Grammar's generic half (`doc/grammar.md`): what a finding is and the [`Checker`] a checker is
//! handed in through, as spelling's [`crate::spell::Lexicon`] is. An application decides what a
//! paragraph is and which of its ranges are not prose; `grind-grammar` and `grind-languagetool`
//! hold the checkers.

use std::ops::Range;

/// What kind of finding it is, which decides how it is underlined (`doc/grammar.md` decision 2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Grammar,
    Punctuation,
    Typography,
    Style,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Grammar => "grammar",
            Kind::Punctuation => "punctuation",
            Kind::Typography => "typography",
            Kind::Style => "style",
        }
    }
}

/// One thing a checker found in a paragraph. `range` counts characters into the paragraph's text;
/// an empty replacement means "delete it".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    pub range: Range<usize>,
    /// Stable enough to be ignored by name: `builtin:repeated-word`, `lt:EN_A_VS_AN`.
    pub rule: String,
    pub kind: Kind,
    pub message: String,
    pub replacements: Vec<String>,
    /// Where the rule is explained, when the checker says.
    pub url: Option<String>,
}

/// One paragraph to check: its text, the character ranges that are not prose (a link, a run of
/// code), and its language as a BCP 47 tag when known.
#[derive(Clone, Copy, Debug)]
pub struct Paragraph<'a> {
    pub text: &'a str,
    pub skip: &'a [Range<usize>],
    pub language: Option<&'a str>,
}

/// A grammar checker that answers at once. `Send + Sync` because an `App` holds one behind its
/// lock.
pub trait Checker: Send + Sync {
    /// Every finding in `paragraph`, in any order.
    fn check(&self, paragraph: &Paragraph<'_>) -> Vec<Finding>;

    /// Every finding in each of `paragraphs`, in their order — one at a time unless a checker
    /// does better, as one that asks a server does with one request.
    fn check_all(&self, paragraphs: &[Paragraph<'_>]) -> Vec<Vec<Finding>> {
        paragraphs.iter().map(|p| self.check(p)).collect()
    }
}

/// Several checkers as one: each paragraph checked by all of them, and where two findings
/// overlap the later checker's wins — so the built-in checker's quick answer gives way to
/// LanguageTool's where both have something to say (`doc/grammar.md` §5).
pub struct Both(pub Vec<std::sync::Arc<dyn Checker>>);

impl Checker for Both {
    fn check(&self, paragraph: &Paragraph<'_>) -> Vec<Finding> {
        self.check_all(std::slice::from_ref(paragraph))
            .pop()
            .unwrap_or_default()
    }

    fn check_all(&self, paragraphs: &[Paragraph<'_>]) -> Vec<Vec<Finding>> {
        let mut out: Vec<Vec<Finding>> = vec![Vec::new(); paragraphs.len()];
        for checker in &self.0 {
            for (mine, later) in out.iter_mut().zip(checker.check_all(paragraphs)) {
                mine.retain(|f| {
                    !later
                        .iter()
                        .any(|l| f.range.start < l.range.end && l.range.start < f.range.end)
                });
                mine.extend(later);
            }
        }
        for findings in &mut out {
            findings.sort_by_key(|f| (f.range.start, f.range.end));
        }
        out
    }
}

/// `findings` less any that touch a range of `skip`, sorted by where they start.
pub fn outside(mut findings: Vec<Finding>, skip: &[Range<usize>]) -> Vec<Finding> {
    findings.retain(|f| {
        !skip
            .iter()
            .any(|s| s.start < f.range.end && f.range.start < s.end)
    });
    findings.sort_by_key(|f| (f.range.start, f.range.end));
    findings
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(range: Range<usize>) -> Finding {
        Finding {
            range,
            rule: "builtin:test".into(),
            kind: Kind::Grammar,
            message: String::new(),
            replacements: Vec::new(),
            url: None,
        }
    }

    struct Fixed(Vec<Range<usize>>, &'static str);

    impl Checker for Fixed {
        fn check(&self, _: &Paragraph<'_>) -> Vec<Finding> {
            self.0
                .iter()
                .map(|r| Finding {
                    rule: self.1.into(),
                    ..at(r.clone())
                })
                .collect()
        }
    }

    #[test]
    fn of_two_checkers_the_later_wins_where_their_findings_overlap() {
        use std::sync::Arc;
        let both = Both(vec![
            Arc::new(Fixed(vec![0..3, 10..12], "first")),
            Arc::new(Fixed(vec![2..5, 20..22], "second")),
        ]);
        let paragraph = Paragraph {
            text: "",
            skip: &[],
            language: None,
        };
        let found: Vec<(Range<usize>, String)> = both
            .check_all(&[paragraph])
            .remove(0)
            .into_iter()
            .map(|f| (f.range, f.rule))
            .collect();
        assert_eq!(
            found,
            [
                (2..5, "second".to_owned()),
                (10..12, "first".to_owned()),
                (20..22, "second".to_owned())
            ]
        );
    }

    #[test]
    fn a_finding_touching_a_skipped_range_is_dropped_and_the_rest_sorted() {
        let skip: Vec<Range<usize>> = std::iter::once(5..9).collect();
        let kept = outside(vec![at(10..12), at(0..3), at(4..7)], &skip);
        let ranges: Vec<_> = kept.into_iter().map(|f| f.range).collect();
        assert_eq!(ranges, [0..3, 10..12], "4..7 overlaps 5..9");
        // An empty finding (a missing space) at a skipped range's edge is outside it.
        let edge = outside(vec![at(9..9)], &skip);
        assert_eq!(edge.len(), 1);
    }
}
