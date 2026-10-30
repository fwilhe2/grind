// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Grammar in a text document (`doc/grammar.md`): a checker handed in, each paragraph checked
//! with its language and without what is not prose, and the findings addressed by block.

use std::sync::{Arc, Mutex};

use grind_core::grammar::{Checker, Finding, Kind, Paragraph};
use grind_text::{App, Caret};

/// Flags every `xx`, and remembers the language each paragraph was handed with.
#[derive(Default)]
struct Doubles {
    languages: Mutex<Vec<Option<String>>>,
}

impl Checker for Doubles {
    fn check(&self, paragraph: &Paragraph<'_>) -> Vec<Finding> {
        self.languages
            .lock()
            .unwrap()
            .push(paragraph.language.map(str::to_owned));
        let chars: Vec<char> = paragraph.text.chars().collect();
        (0..chars.len().saturating_sub(1))
            .filter(|&i| chars[i] == 'x' && chars[i + 1] == 'x')
            .map(|i| Finding {
                range: i..i + 2,
                rule: "test:xx".into(),
                kind: Kind::Grammar,
                message: "two x".into(),
                replacements: vec!["x".into()],
                url: None,
            })
            .collect()
    }
}

fn fodt(styles: &str, body: &str) -> App {
    let bytes = format!(
        r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" xmlns:xlink="http://www.w3.org/1999/xlink" office:mimetype="application/vnd.oasis.opendocument.text">{styles}<office:body><office:text>{body}</office:text></office:body></office:document>"#
    );
    let app = App::new();
    app.open_bytes("g.fodt", bytes.as_bytes()).unwrap();
    app
}

#[test]
fn no_checker_finds_nothing() {
    let app = fodt("", "<text:p>a xx b</text:p>");
    assert!(app.grammar(0..1).is_empty());
}

#[test]
fn findings_are_addressed_by_block_and_a_links_text_is_not_checked() {
    let app = fodt(
        "",
        r##"<text:p>fine</text:p><text:p>one xx two <text:a xlink:href="#x">xx</text:a></text:p>"##,
    );
    let checker = Arc::new(Doubles::default());
    app.set_checker(Some(checker.clone()));
    let flags = app.grammar(0..app.block_count());
    assert_eq!(flags.len(), 1, "the link's xx is not prose: {flags:?}");
    assert_eq!(flags[0].block, 1);
    assert_eq!(flags[0].finding.range, 4..6);
    assert_eq!(flags[0].address(), "p2+4");
    assert_eq!(
        flags[0].caret(),
        Caret {
            block: 1,
            offset: 4
        }
    );
    // Correcting one is spelling's correction, over the finding's span.
    app.correct(flags[0].caret(), "xx", "x").unwrap();
    assert!(app.grammar(0..app.block_count()).is_empty());
}

#[test]
fn each_paragraph_is_checked_in_the_language_the_document_states() {
    let stated = r#"<office:styles><style:style style:name="Standard" style:family="paragraph"><style:text-properties fo:language="de" fo:country="DE"/></style:style></office:styles>"#;
    let app = fodt(stated, "<text:p>a</text:p><text:p>b</text:p>");
    let checker = Arc::new(Doubles::default());
    app.set_checker(Some(checker.clone()));
    app.grammar(0..2);
    assert_eq!(
        *checker.languages.lock().unwrap(),
        [Some("de-DE".to_owned()), Some("de-DE".to_owned())]
    );
    let unstated = fodt("", "<text:p>a</text:p>");
    let checker = Arc::new(Doubles::default());
    unstated.set_checker(Some(checker.clone()));
    unstated.grammar(0..1);
    assert_eq!(
        *checker.languages.lock().unwrap(),
        [None],
        "the checker guesses"
    );
}

/// Knows no word at all, so every word spelling checks is reported.
struct Nothing;

impl grind_text::spell::Lexicon for Nothing {
    fn knows(&self, _: &str) -> bool {
        false
    }
    fn suggest(&self, _: &str) -> Vec<String> {
        Vec::new()
    }
    fn language(&self) -> &str {
        "en-US"
    }
}

/// A fenced code block is not prose: neither grammar nor spelling reads it, as neither reads a
/// run of code (`doc/spelling.md` decision 3).
#[test]
fn a_code_block_is_not_checked() {
    let app = App::new();
    app.import_markdown(
        0,
        "Some xx prose.\n\n```\nlet xx = foo;\n```\n",
        &grind_text::commonmark::nowhere,
    )
    .unwrap();
    let code = (0..app.block_count())
        .find(|&i| {
            app.get_viewport(i..i + 1)
                .iter()
                .any(|v| v.text.contains("let"))
        })
        .expect("the code block");
    app.set_checker(Some(Arc::new(Doubles::default())));
    let flags = app.grammar(0..app.block_count());
    assert_eq!(flags.len(), 1, "only the prose: {flags:?}");
    assert_ne!(flags[0].block, code);
    app.set_lexicon(Some(Arc::new(Nothing)));
    let words: Vec<_> = app
        .misspellings(0..app.block_count())
        .into_iter()
        .filter(|m| m.block == code)
        .collect();
    assert!(words.is_empty(), "{words:?}");
}
