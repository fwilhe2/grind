// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Paragraphs into requests, and a server's answer back into findings.

use grind_core::grammar::{Finding, Kind, Paragraph};
use grind_spell::Language;
use serde_json::{Value, json};

/// What every request says besides the text.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Options {
    /// `level=picky`: the style rules too.
    pub picky: bool,
    /// Rule ids the person ignores, LanguageTool's own (without `lt:`).
    pub disabled_rules: Vec<String>,
    /// The most text one request may carry, in characters: 20,000 on the public server.
    pub max_chars: usize,
}

/// One request's worth of paragraphs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Batch {
    /// Which of the paragraphs handed to [`batches`] this carries, in order.
    pub paragraphs: Vec<usize>,
    /// The form fields to `POST` to `/v2/check`.
    pub form: Vec<(String, String)>,
    /// Each paragraph sent.
    sent: Vec<Sent>,
}

/// One paragraph of a request: its text, where it starts and ends in what was sent in UTF-16
/// units, and its ranges sent as markup, in characters.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Sent {
    text: String,
    from: usize,
    to: usize,
    markup: Vec<std::ops::Range<usize>>,
}

/// Between two paragraphs of one request: markup the server reads as a paragraph break.
const SEPARATOR: &str = "\n\n";

/// The language a paragraph is sent in: the one stated, else the one its common words name, else
/// `auto` for the server to detect.
fn language_of(paragraph: &Paragraph<'_>) -> String {
    match paragraph.language {
        Some(tag) => tag.to_owned(),
        None => grind_spell::paragraph_language(paragraph.text)
            .map_or_else(|| "auto".to_owned(), |l| Language::tag(l).to_owned()),
    }
}

/// The requests that check `paragraphs`: grouped by language and cut to `options.max_chars`.
pub fn batches(paragraphs: &[Paragraph<'_>], options: &Options) -> Vec<Batch> {
    let mut languages: Vec<(String, Vec<usize>)> = Vec::new();
    for (at, paragraph) in paragraphs.iter().enumerate() {
        let language = language_of(paragraph);
        match languages.iter_mut().find(|(l, _)| *l == language) {
            Some((_, members)) => members.push(at),
            None => languages.push((language, vec![at])),
        }
    }
    let mut out = Vec::new();
    for (language, members) in languages {
        let mut group: Vec<usize> = Vec::new();
        let mut chars = 0;
        for at in members {
            let len = paragraphs[at].text.chars().count();
            if !group.is_empty() && chars + len > options.max_chars {
                out.push(batch(paragraphs, &group, &language, options));
                group.clear();
                chars = 0;
            }
            group.push(at);
            chars += len + SEPARATOR.len();
        }
        if !group.is_empty() {
            out.push(batch(paragraphs, &group, &language, options));
        }
    }
    out
}

fn batch(
    paragraphs: &[Paragraph<'_>],
    members: &[usize],
    language: &str,
    options: &Options,
) -> Batch {
    let units = |text: &str| text.encode_utf16().count();
    let mut annotation = Vec::new();
    let mut sent = Vec::new();
    let mut at = 0;
    for (n, &index) in members.iter().enumerate() {
        if n > 0 {
            annotation.push(json!({"markup": SEPARATOR, "interpretAs": SEPARATOR}));
            at += units(SEPARATOR);
        }
        let paragraph = &paragraphs[index];
        let chars: Vec<char> = paragraph.text.chars().collect();
        let mut from = 0;
        let mut pieces: Vec<(bool, String)> = Vec::new();
        let mut skip = paragraph.skip.to_vec();
        skip.sort_by_key(|r| r.start);
        for range in skip {
            let (start, end) = (range.start.min(chars.len()), range.end.min(chars.len()));
            if start > from {
                pieces.push((false, chars[from..start].iter().collect()));
            }
            if end > start.max(from) {
                pieces.push((true, chars[start.max(from)..end].iter().collect()));
            }
            from = from.max(end);
        }
        if from < chars.len() {
            pieces.push((false, chars[from..].iter().collect()));
        }
        for (markup, piece) in pieces {
            annotation.push(match markup {
                true => json!({ "markup": piece }),
                false => json!({ "text": piece }),
            });
        }
        let len = units(paragraph.text);
        sent.push(Sent {
            text: paragraph.text.to_owned(),
            from: at,
            to: at + len,
            markup: paragraph.skip.to_vec(),
        });
        at += len;
    }
    let mut form = vec![
        (
            "data".to_owned(),
            json!({ "annotation": annotation }).to_string(),
        ),
        ("language".to_owned(), language.to_owned()),
        ("disabledCategories".to_owned(), "TYPOS".to_owned()),
    ];
    if language == "auto" {
        form.push(("preferredVariants".to_owned(), "en-US,de-DE".to_owned()));
    }
    if options.picky {
        form.push(("level".to_owned(), "picky".to_owned()));
    }
    if !options.disabled_rules.is_empty() {
        form.push(("disabledRules".to_owned(), options.disabled_rules.join(",")));
    }
    Batch {
        paragraphs: members.to_vec(),
        form,
        sent,
    }
}

/// Our four kinds from LanguageTool's category and issue type.
fn kind(category: &str, issue: &str) -> Kind {
    match (category, issue) {
        ("TYPOGRAPHY", _) | (_, "typographical") => Kind::Typography,
        ("PUNCTUATION", _) => Kind::Punctuation,
        ("STYLE" | "REDUNDANCY" | "PLAIN_ENGLISH" | "COLLOQUIALISMS", _) | (_, "style") => {
            Kind::Style
        }
        _ => Kind::Grammar,
    }
}

impl Batch {
    /// The server's answer to this batch, as `(index into the paragraphs handed to [`batches`],
    /// finding)` in characters. A match that crosses a paragraph's end or lies in what was sent
    /// as markup is dropped; an answer that is not LanguageTool's JSON is an error.
    pub fn findings(&self, answer: &str) -> Result<Vec<(usize, Finding)>, String> {
        let answer: Value = serde_json::from_str(answer)
            .map_err(|e| format!("not an answer from LanguageTool: {e}"))?;
        let matches = answer["matches"]
            .as_array()
            .ok_or("an answer from LanguageTool with no matches")?;
        let mut out = Vec::new();
        for found in matches {
            let (Some(offset), Some(length)) = (found["offset"].as_u64(), found["length"].as_u64())
            else {
                continue;
            };
            let (start, end) = (offset as usize, (offset + length) as usize);
            let Some(n) = self
                .sent
                .iter()
                .position(|sent| sent.from <= start && end <= sent.to)
            else {
                continue;
            };
            let sent = &self.sent[n];
            let char_at = |unit: usize| {
                let byte = grind_core::utf16::byte_of(&sent.text, unit - sent.from);
                sent.text[..byte].chars().count()
            };
            let range = char_at(start)..char_at(end);
            if sent
                .markup
                .iter()
                .any(|m| m.start < range.end && range.start < m.end)
            {
                continue;
            }
            let rule = &found["rule"];
            let replacements = found["replacements"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|r| r["value"].as_str().map(str::to_owned))
                .take(5)
                .collect();
            out.push((
                self.paragraphs[n],
                Finding {
                    range,
                    rule: format!("lt:{}", rule["id"].as_str().unwrap_or("?")),
                    kind: kind(
                        rule["category"]["id"].as_str().unwrap_or(""),
                        rule["issueType"].as_str().unwrap_or(""),
                    ),
                    message: found["message"].as_str().unwrap_or_default().to_owned(),
                    replacements,
                    url: rule["urls"][0]["value"].as_str().map(str::to_owned),
                },
            ));
        }
        Ok(out)
    }
}
