// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

use grind_core::grammar::{Checker, Paragraph};
use grind_grammar::Builtin;

/// Every finding in `text`, as `rule:covered` — the rule without its prefix, and the text it
/// covers — in order.
pub fn found(text: &str, language: Option<&str>) -> Vec<String> {
    let paragraph = Paragraph {
        text,
        skip: &[],
        language,
    };
    let chars: Vec<char> = text.chars().collect();
    let mut findings = Builtin::default().check(&paragraph);
    findings.sort_by_key(|f| f.range.start);
    findings
        .into_iter()
        .map(|f| {
            let rule = f
                .rule
                .strip_prefix("builtin:")
                .unwrap_or(&f.rule)
                .to_owned();
            let covered: String = chars[f.range].iter().collect();
            format!("{rule}:{covered}")
        })
        .collect()
}

/// That `text` draws no finding at all.
#[track_caller]
pub fn nothing(text: &str, language: Option<&str>) {
    let found = found(text, language);
    assert!(found.is_empty(), "{text:?} drew {found:?}");
}
