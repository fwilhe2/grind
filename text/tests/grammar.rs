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

/// Flags `xx` as grammar and `yy` as style.
struct Kinds;

impl Checker for Kinds {
    fn check(&self, paragraph: &Paragraph<'_>) -> Vec<Finding> {
        let mut out = Vec::new();
        for (needle, kind) in [("xx", Kind::Grammar), ("yy", Kind::Style)] {
            if let Some(at) = paragraph.text.find(needle) {
                let start = paragraph.text[..at].chars().count();
                out.push(Finding {
                    range: start..start + 2,
                    rule: format!("test:{needle}"),
                    kind,
                    message: format!("{needle} found"),
                    replacements: Vec::new(),
                    url: None,
                });
            }
        }
        out
    }
}

/// `grind lint` reports grammar as a warning and style as a hint, only with a checker attached.
#[test]
fn lint_reports_grammar_as_a_warning_and_style_as_a_hint() {
    let app = fodt("", "<text:p>one xx two yy</text:p>");
    let options = grind_core::lint::Options {
        hints: true,
        off: Vec::new(),
    };
    let ids = |app: &App| -> Vec<(String, String)> {
        app.lint(&options)
            .diagnostics
            .iter()
            .map(|d| (d.rule.to_owned(), d.at.clone()))
            .filter(|(rule, _)| rule == "grammar" || rule == "style")
            .collect()
    };
    assert!(ids(&app).is_empty(), "no checker, no findings");
    app.set_checker(Some(Arc::new(Kinds)));
    assert_eq!(
        ids(&app),
        [
            ("grammar".to_owned(), "p1+4".to_owned()),
            ("style".to_owned(), "p1+11".to_owned())
        ]
    );
    let quiet = grind_core::lint::Options {
        hints: false,
        off: Vec::new(),
    };
    let warnings: Vec<_> = app
        .lint(&quiet)
        .diagnostics
        .iter()
        .map(|d| d.rule.to_owned())
        .filter(|rule| rule == "grammar" || rule == "style")
        .collect();
    assert_eq!(warnings, ["grammar"], "style is a hint");
}

/// LanguageTool answers later (`doc/grammar.md` decision 10): the app lists the paragraphs not yet
/// answered, keeps each answer by the text it was for, and shows it beside the built-in
/// checker's — winning where both found something — until that text changes.
#[test]
fn a_late_checkers_answers_are_kept_by_the_text_they_were_for() {
    let app = fodt("", "<text:p>one xx two</text:p><text:p>plain</text:p>");
    app.set_checker(Some(Arc::new(Doubles::default())));
    assert!(
        app.grammar_pending(0..2).is_empty(),
        "nothing asked for yet"
    );
    app.set_background_grammar(true);
    let pending = app.grammar_pending(0..2);
    assert_eq!(pending.len(), 2);
    assert_eq!(pending[0].text, "one xx two");
    let late = |range: std::ops::Range<usize>, rule: &str| Finding {
        range,
        rule: rule.into(),
        kind: Kind::Grammar,
        message: "late".into(),
        replacements: Vec::new(),
        url: None,
    };
    // An answer overlapping the built-in `xx` (4..6) wins; one elsewhere joins it.
    app.grammar_found(pending[0].key, vec![late(4..7, "lt:A"), late(0..3, "lt:B")]);
    app.grammar_found(pending[1].key, Vec::new());
    assert!(app.grammar_pending(0..2).is_empty(), "both answered");
    let rules: Vec<String> = app
        .grammar(0..2)
        .into_iter()
        .map(|f| f.finding.rule)
        .collect();
    assert_eq!(rules, ["lt:B", "lt:A"]);
    // An edit makes the paragraph pending again, and its old answer is not shown for new text.
    app.correct(
        Caret {
            block: 0,
            offset: 0,
        },
        "one",
        "1",
    )
    .unwrap();
    let pending = app.grammar_pending(0..2);
    assert_eq!(pending.len(), 1);
    let rules: Vec<String> = app
        .grammar(0..2)
        .into_iter()
        .map(|f| f.finding.rule)
        .collect();
    assert_eq!(rules, ["test:xx"], "the built-in one only");
    app.set_background_grammar(false);
    assert!(app.grammar_pending(0..2).is_empty());
}
