// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The built-in grammar and style checker (`doc/grammar.md` §4): rules written here, each a
//! function over a paragraph's tokens. A rule that is not right nine times in ten does not run
//! by default.

mod neutral;
mod tokens;

use grind_core::grammar::{Checker, Finding, Kind, Paragraph};
pub use grind_spell::Language;

use tokens::Token;

/// The built-in checker. `picky` adds the style rules, which are off by default.
#[derive(Clone, Copy, Debug, Default)]
pub struct Builtin {
    pub picky: bool,
}

impl Checker for Builtin {
    fn check(&self, paragraph: &Paragraph<'_>) -> Vec<Finding> {
        let language = match paragraph.language {
            Some(tag) => Language::from_tag(tag),
            None => grind_spell::paragraph_language(paragraph.text),
        };
        let tokens = tokens::tokens(paragraph.text);
        let mut out = Vec::new();
        neutral::repeated_word(&tokens, language, &mut out);
        neutral::space_before_punctuation(&tokens, language, &mut out);
        neutral::double_space(&tokens, self.picky, &mut out);
        neutral::repeated_punctuation(&tokens, &mut out);
        neutral::missing_space_after(&tokens, &mut out);
        neutral::sentence_start_lowercase(&tokens, language, &mut out);
        if self.picky {
            neutral::unpaired_bracket(&tokens, &mut out);
        }
        out
    }
}

/// A finding of rule `rule` from the start of `from` to the end of `to`.
fn finding(
    rule: &str,
    kind: Kind,
    (from, to): (&Token, &Token),
    message: String,
    replacements: Vec<String>,
) -> Finding {
    Finding {
        range: from.range.start..to.range.end,
        rule: format!("builtin:{rule}"),
        kind,
        message,
        replacements,
        url: None,
    }
}
