// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The built-in grammar and style checker (`doc/grammar.md` §4): rules written here, each a
//! function over a paragraph's tokens. A rule that is not right nine times in ten does not run
//! by default.

mod english;
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
        let language = language(paragraph);
        // Exceptions (French spacing, a language's own doubles and abbreviations) only ever
        // take a finding away, so they may trust the stated language without evidence.
        let lenient = language.or_else(|| paragraph.language.and_then(Language::from_tag));
        let tokens = tokens::tokens(paragraph.text, paragraph.skip);
        let mut out = Vec::new();
        neutral::repeated_word(&tokens, lenient, &mut out);
        neutral::space_before_punctuation(&tokens, lenient, &mut out);
        neutral::double_space(&tokens, self.picky, &mut out);
        neutral::repeated_punctuation(&tokens, &mut out);
        neutral::missing_space_after(&tokens, &mut out);
        neutral::sentence_start_lowercase(&tokens, lenient, &mut out);
        if self.picky {
            neutral::unpaired_bracket(&tokens, &mut out);
        }
        if language == Some(Language::English) {
            english::a_an(&tokens, &mut out);
            english::could_of(&tokens, &mut out);
            english::than_then(&tokens, &mut out);
            english::its(&tokens, &mut out);
            english::your_welcome(&tokens, &mut out);
            english::their_is(&tokens, &mut out);
            english::lowercase_i(&tokens, &mut out);
        }
        out
    }
}

/// The language whose rules run on `paragraph` (`doc/grammar.md` decision 4): the one its own
/// common words name, else the stated one if its common words allow it, else none.
fn language(paragraph: &Paragraph<'_>) -> Option<Language> {
    if let Some(guessed) = grind_spell::paragraph_language(paragraph.text) {
        return Some(guessed);
    }
    let stated = Language::from_tag(paragraph.language?)?;
    grind_spell::paragraph_supports(paragraph.text, stated).then_some(stated)
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
