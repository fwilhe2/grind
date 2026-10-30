// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Requests and answers, against what LanguageTool 6.8 was measured to send back
//! (`doc/grammar.md` §5).

use std::ops::Range;

use grind_core::grammar::{Kind, Paragraph};
use grind_languagetool::{Options, batches};

fn paragraph<'a>(
    text: &'a str,
    skip: &'a [Range<usize>],
    language: Option<&'a str>,
) -> Paragraph<'a> {
    Paragraph {
        text,
        skip,
        language,
    }
}

fn field<'a>(form: &'a [(String, String)], name: &str) -> Option<&'a str> {
    form.iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
}

fn options() -> Options {
    Options {
        max_chars: 20_000,
        ..Options::default()
    }
}

#[test]
fn a_request_never_checks_spelling_and_sends_skipped_text_as_markup() {
    let skip: Vec<Range<usize>> = std::iter::once(7..12).collect();
    let paragraphs = [paragraph("Run it `cat` now.", &skip, Some("en-GB"))];
    let batches = batches(&paragraphs, &options());
    assert_eq!(batches.len(), 1);
    let form = &batches[0].form;
    assert_eq!(field(form, "language"), Some("en-GB"));
    assert_eq!(field(form, "disabledCategories"), Some("TYPOS"));
    assert_eq!(field(form, "level"), None);
    let data: serde_json::Value = serde_json::from_str(field(form, "data").unwrap()).unwrap();
    assert_eq!(
        data,
        serde_json::json!({"annotation": [
            {"text": "Run it "}, {"markup": "`cat`"}, {"text": " now."}
        ]})
    );
}

#[test]
fn paragraphs_go_together_by_language_and_apart_past_the_limit() {
    let none: Vec<Range<usize>> = Vec::new();
    let paragraphs = [
        paragraph("The cat is on the mat and it is happy.", &none, None),
        paragraph("Der Hund ist nicht hier und die Katze auch.", &none, None),
        paragraph("This is the second one, and it is English.", &none, None),
    ];
    let all = batches(&paragraphs, &options());
    let languages: Vec<(Option<&str>, Vec<usize>)> = all
        .iter()
        .map(|b| (field(&b.form, "language"), b.paragraphs.clone()))
        .collect();
    assert_eq!(
        languages,
        [(Some("en-US"), vec![0, 2]), (Some("de-DE"), vec![1])]
    );
    let small = Options {
        max_chars: 50,
        ..options()
    };
    assert_eq!(
        batches(&paragraphs, &small).len(),
        3,
        "one paragraph a request"
    );
    let picky = Options {
        picky: true,
        disabled_rules: vec!["EN_A_VS_AN".into(), "COMMA".into()],
        ..options()
    };
    let form = &batches(&paragraphs[..1], &picky)[0].form;
    assert_eq!(field(form, "level"), Some("picky"));
    assert_eq!(field(form, "disabledRules"), Some("EN_A_VS_AN,COMMA"));
}

#[test]
fn a_paragraph_of_no_known_language_is_sent_for_the_server_to_detect() {
    let none: Vec<Range<usize>> = Vec::new();
    let paragraphs = [paragraph("Quarterly report", &none, None)];
    let form = &batches(&paragraphs, &options())[0].form;
    assert_eq!(field(form, "language"), Some("auto"));
    assert_eq!(field(form, "preferredVariants"), Some("en-US,de-DE"));
}

/// Offsets count UTF-16 units over everything sent, markup included (measured against
/// LanguageTool 6.8: `go` after an emoji came back at 6, where it is the fifth character).
#[test]
fn an_answer_comes_back_per_paragraph_in_characters() {
    let none: Vec<Range<usize>> = Vec::new();
    let paragraphs = [
        paragraph("😀 He go home and it is fine.", &none, Some("en-US")),
        paragraph("She go too, and it is the end.", &none, Some("en-US")),
    ];
    let batch = &batches(&paragraphs, &options())[0];
    // `go` in the first at UTF-16 6; the second starts after 29 units and two of separator.
    let answer = r#"{"software":{"name":"LanguageTool","version":"6.8","buildDate":"x","apiVersion":1},
      "language":{"name":"English (US)","code":"en-US"},
      "matches":[
        {"message":"Agreement","shortMessage":"","offset":6,"length":2,
         "replacements":[{"value":"goes"},{"value":"went"}],
         "context":{"text":"","offset":0,"length":0},"sentence":"",
         "rule":{"id":"HE_VERB_AGR","description":"d","issueType":"grammar",
                 "category":{"id":"GRAMMAR","name":"Grammar"},
                 "urls":[{"value":"https://example.org/rule"}]}},
        {"message":"Style","offset":35,"length":2,"replacements":[],
         "context":{"text":"","offset":0,"length":0},"sentence":"",
         "rule":{"id":"SOME_STYLE","description":"d","issueType":"style",
                 "category":{"id":"STYLE","name":"Style"}}}
      ]}"#;
    let found = batch.findings(answer).unwrap();
    assert_eq!(found.len(), 2, "{found:?}");
    let (first, f) = &found[0];
    assert_eq!(*first, 0);
    assert_eq!(f.range, 5..7);
    assert_eq!(f.rule, "lt:HE_VERB_AGR");
    assert_eq!(f.kind, Kind::Grammar);
    assert_eq!(f.replacements, ["goes", "went"]);
    assert_eq!(f.url.as_deref(), Some("https://example.org/rule"));
    let (second, g) = &found[1];
    assert_eq!(*second, 1);
    assert_eq!(g.range, 4..6, "`go` in the second paragraph");
    assert_eq!(g.kind, Kind::Style);
}

#[test]
fn a_match_across_paragraphs_or_inside_markup_is_dropped_and_garbage_is_an_error() {
    let skip: Vec<Range<usize>> = std::iter::once(0..5).collect();
    let paragraphs = [paragraph("`cat` is it and the end.", &skip, Some("en-US"))];
    let batch = &batches(&paragraphs, &options())[0];
    let answer = r#"{"matches":[{"message":"m","offset":1,"length":2,"replacements":[],
        "rule":{"id":"X","description":"d","category":{"id":"MISC","name":"m"}}}]}"#;
    assert!(batch.findings(answer).unwrap().is_empty());
    assert!(batch.findings("<html>busy</html>").is_err());
}
