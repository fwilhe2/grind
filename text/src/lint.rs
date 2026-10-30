// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The word processor's lint rules (`doc/dsl.md` §4.3); [`grind_core::lint`] holds what a
//! diagnostic is (R8). [`RULES`] is checked against §4.3's table by `cli/tests/lint.rs`. Addresses
//! are `loc.rs`'s, so each can be typed back at `grind text get`.

/// The diagnostic vocabulary, `grind_core`'s, under this crate's path.
pub use grind_core::lint::{Diagnostic, Options, Report, Rule, Severity};

use grind_core::style::PALETTE;

use crate::loc;
use crate::model::{Block, Document, Run};
use crate::spell::{self, Lexicon};
use crate::style::CharStyle;

/// A heading more than one level past the heading before it (1 → 3). Only after a first heading: a
/// document starting at level 2 is as often a chapter file as a mistake.
pub const HEADING_SKIP: Rule = Rule {
    id: "heading-skip",
    severity: Severity::Warning,
    what: "a heading level skipped — 1 followed by 3",
};

/// A link to `#name` where no `text:bookmark` of that name exists. An error: the link does
/// nothing.
pub const UNKNOWN_BOOKMARK: Rule = Rule {
    id: "unknown-bookmark",
    severity: Severity::Error,
    what: "a bookmark referenced and never declared",
};

/// A `text:style-name` naming a style the document does not declare. A document this build created
/// declares no styles, so every name in it is reported — the file does refer to something it does
/// not define.
pub const UNDECLARED_STYLE: Rule = Rule {
    id: "undeclared-style",
    severity: Severity::Warning,
    what: "a style name used and never declared",
};

/// A colour that is not one of `grind_core::style::PALETTE`'s — a hint, off unless asked for,
/// since the palette is a default and never a limit.
pub const OFF_PALETTE: Rule = Rule {
    id: "off-palette",
    severity: Severity::Hint,
    what: "a colour outside the default palette",
};

/// Something the projection cannot spell — images (`doc/projection-text.md`) — so converting to a
/// `.grind` would lose exactly what this lists.
pub const UNSPELLABLE: Rule = Rule {
    id: "unspellable",
    severity: Severity::Warning,
    what: "a construct the projection cannot spell",
};

/// A word the dictionary does not know (`doc/spelling.md`). Runs only with a dictionary attached
/// ([`lint_with`]). A warning, never a reason for `grind lint` to fail.
pub const MISSPELT: Rule = Rule {
    id: "misspelt",
    severity: Severity::Warning,
    what: "a word the spelling dictionary does not know",
};

/// What a grammar checker finds that is not a matter of style (`doc/grammar.md`). Runs only with
/// a checker attached ([`grammar`]); a warning, like [`MISSPELT`].
pub const GRAMMAR: Rule = Rule {
    id: "grammar",
    severity: Severity::Warning,
    what: "a grammar, punctuation or typography finding",
};

/// What a grammar checker suggests as a matter of style — a hint, so only when asked for.
pub const STYLE: Rule = Rule {
    id: "style",
    severity: Severity::Hint,
    what: "a style suggestion",
};

/// Every rule this application has. The order is the order `grind text lint --rules` prints.
pub const RULES: [Rule; 8] = [
    HEADING_SKIP,
    UNKNOWN_BOOKMARK,
    UNDECLARED_STYLE,
    OFF_PALETTE,
    UNSPELLABLE,
    MISSPELT,
    GRAMMAR,
    STYLE,
];

/// Check a document against every rule `options` wants — all but [`MISSPELT`], which needs a
/// dictionary ([`lint_with`]).
pub fn lint(doc: &Document, options: &Options) -> Report {
    lint_with(doc, options, None)
}

/// [`lint`], and spelling as well when there is a `lexicon` to check it against.
pub fn lint_with(doc: &Document, options: &Options, lexicon: Option<&dyn Lexicon>) -> Report {
    let mut report = Report::default();
    if options.wants(&HEADING_SKIP) {
        heading_skips(doc, &mut report);
    }
    if options.wants(&UNKNOWN_BOOKMARK) {
        unknown_bookmarks(doc, &mut report);
    }
    if options.wants(&UNDECLARED_STYLE) {
        undeclared_styles(doc, &mut report);
    }
    if options.wants(&OFF_PALETTE) {
        off_palette(doc, &mut report);
    }
    if options.wants(&UNSPELLABLE) {
        unspellable(doc, &mut report);
    }
    if let Some(lexicon) = lexicon
        && options.wants(&MISSPELT)
    {
        misspelt(doc, lexicon, &mut report);
    }
    report.sort();
    report
}

/// Add what `checker` finds to `report`: [`STYLE`] for style, [`GRAMMAR`] for the rest. Each block
/// is checked in `language`, the one the document states, or none for the checker to guess.
pub fn grammar(
    doc: &Document,
    options: &Options,
    checker: &dyn crate::grammar::Checker,
    language: Option<&str>,
    report: &mut Report,
) {
    for (index, block) in doc.blocks.iter().enumerate() {
        for flag in crate::grammar::check_block(block, index, language, checker) {
            let rule = match flag.finding.kind {
                crate::grammar::Kind::Style => &STYLE,
                _ => &GRAMMAR,
            };
            if options.wants(rule)
                && !report.push(Diagnostic::new(rule, flag.address(), flag.finding.message))
            {
                return;
            }
        }
    }
    report.sort();
}

/// `p12` — the address of a block, through `loc::format`, the one 0↔1 conversion.
fn at(index: usize) -> String {
    loc::format(index)
}

fn heading_skips(doc: &Document, report: &mut Report) {
    let mut previous: Option<u32> = None;
    for (index, level) in doc.outline() {
        if let Some(before) = previous
            && level > before + 1
        {
            let ok = report.push(Diagnostic::new(
                &HEADING_SKIP,
                at(index),
                format!("heading level {level} follows level {before}"),
            ));
            if !ok {
                return;
            }
        }
        previous = Some(level);
    }
}

fn unknown_bookmarks(doc: &Document, report: &mut Report) {
    for (index, block) in doc.blocks.iter().enumerate() {
        for run in &block.runs {
            let Run::Text {
                href: Some(href), ..
            } = run
            else {
                continue;
            };
            let Some(target) = internal_target(href) else {
                continue;
            };
            if doc.bookmarks.contains_key(target) {
                continue;
            }
            if !report.push(Diagnostic::new(
                &UNKNOWN_BOOKMARK,
                at(index),
                format!("links to #{target}, which nothing in this document declares"),
            )) {
                return;
            }
        }
    }
}

/// The bookmark an `xlink:href` points at inside this document. LibreOffice writes
/// `#intro|outline` and `#intro|region`, and the part after the bar is not the name. A link into
/// another document answers `None`.
fn internal_target(href: &str) -> Option<&str> {
    let target = href.strip_prefix('#')?;
    let target = target.split('|').next().unwrap_or(target);
    (!target.is_empty()).then_some(target)
}

fn undeclared_styles(doc: &Document, report: &mut Report) {
    for (index, block) in doc.blocks.iter().enumerate() {
        for name in used_styles(block) {
            if declares(doc, name) {
                continue;
            }
            if !report.push(Diagnostic::new(
                &UNDECLARED_STYLE,
                at(index),
                format!("uses the style {name:?}, which the document does not declare"),
            )) {
                return;
            }
        }
    }
}

/// Every style name a block mentions — its own and its runs' — deduplicated in document order, so
/// forty runs in one style are one diagnostic.
fn used_styles(block: &Block) -> Vec<&str> {
    let mut names: Vec<&str> = Vec::new();
    let runs = block.runs.iter().map(|run| match run {
        Run::Text { style, .. } => style.as_deref(),
        _ => None,
    });
    for name in std::iter::once(block.style.as_deref()).chain(runs) {
        if let Some(name) = name.map(str::trim).filter(|n| !n.is_empty())
            && !names.contains(&name)
        {
            names.push(name);
        }
    }
    names
}

/// Whether the document declares this style name. A run's name may be a composition of nested
/// spans joined with spaces, so the whole string is tried before its pieces: `"List Paragraph"` is
/// one name.
fn declares(doc: &Document, name: &str) -> bool {
    doc.styles.contains(name)
        || (name.split_whitespace().count() > 1
            && name
                .split_whitespace()
                .all(|part| doc.styles.contains(part)))
}

fn off_palette(doc: &Document, report: &mut Report) {
    for (index, block) in doc.blocks.iter().enumerate() {
        for run in &block.runs {
            let Some(props) = run.props() else { continue };
            for colour in colours(props) {
                if !report.push(Diagnostic::new(
                    &OFF_PALETTE,
                    at(index),
                    format!("{colour} is not one of the palette's colours"),
                )) {
                    return;
                }
            }
        }
    }
}

/// The colours a character style sets that the palette lacks; `transparent` is no background.
fn colours(props: &CharStyle) -> Vec<&str> {
    [props.color.as_deref(), props.background.as_deref()]
        .into_iter()
        .flatten()
        .filter(|colour| *colour != "transparent")
        .filter(|colour| {
            !PALETTE
                .iter()
                .any(|(_, hex)| hex.eq_ignore_ascii_case(colour))
        })
        .collect()
}

fn unspellable(doc: &Document, report: &mut Report) {
    for (index, block) in doc.blocks.iter().enumerate() {
        for run in &block.runs {
            let Run::Image { mime, data, .. } = run else {
                continue;
            };
            if !report.push(Diagnostic::new(
                &UNSPELLABLE,
                at(index),
                format!(
                    "holds a {} image of {} bytes, which the projection has no node for",
                    match mime.is_empty() {
                        true => "(untyped)",
                        false => mime.as_str(),
                    },
                    data.len()
                ),
            )) {
                return;
            }
        }
    }
}

fn misspelt(doc: &Document, lexicon: &dyn Lexicon, report: &mut Report) {
    for (index, block) in doc.blocks.iter().enumerate() {
        for wrong in spell::check_block(block, index, lexicon) {
            if !report.push(Diagnostic::new(
                &MISSPELT,
                wrong.address(),
                format!(
                    "{:?} is not in the {} dictionary",
                    wrong.word,
                    lexicon.language()
                ),
            )) {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{BlockId, BlockKind};

    fn empty() -> Document {
        // Empty: a new document's own paragraph would shift every index.
        Document::empty()
    }

    fn push(doc: &mut Document, kind: BlockKind, runs: Vec<Run>) -> usize {
        let id = doc.next_id();
        let mut block = Block::new(id, kind);
        block.runs = runs;
        doc.blocks.push(block);
        doc.blocks.len() - 1
    }

    fn ids(report: &Report) -> Vec<&str> {
        report.diagnostics.iter().map(|d| d.rule).collect()
    }

    fn loud() -> Options {
        Options {
            hints: true,
            off: Vec::new(),
        }
    }

    #[test]
    fn a_skipped_heading_level_is_reported_and_a_first_heading_is_not() {
        let mut doc = empty();
        push(&mut doc, BlockKind::Heading { level: 2 }, vec![]);
        push(&mut doc, BlockKind::Heading { level: 3 }, vec![]);
        push(&mut doc, BlockKind::Heading { level: 5 }, vec![]);
        let report = lint(&doc, &Options::default());
        assert_eq!(ids(&report), ["heading-skip"], "only the 3 → 5 jump");
        assert_eq!(report.diagnostics[0].at, "p3");
    }

    #[test]
    fn a_link_to_a_bookmark_that_exists_is_quiet_and_one_to_a_ghost_is_not() {
        let mut doc = empty();
        push(
            &mut doc,
            BlockKind::Paragraph,
            vec![Run::Bookmark {
                name: "intro".to_owned(),
            }],
        );
        let link = |target: &str| Run::Text {
            text: "see".to_owned(),
            style: None,
            props: CharStyle::default(),
            href: Some(target.to_owned()),
        };
        push(&mut doc, BlockKind::Paragraph, vec![link("#intro")]);
        // LibreOffice's own spelling of the same link, and an external one nobody here owns.
        push(&mut doc, BlockKind::Paragraph, vec![link("#intro|outline")]);
        push(&mut doc, BlockKind::Paragraph, vec![link("https://x.test")]);
        push(&mut doc, BlockKind::Paragraph, vec![link("#missing")]);
        doc.reindex_bookmarks();

        let report = lint(&doc, &Options::default());
        assert_eq!(ids(&report), ["unknown-bookmark"]);
        assert_eq!(report.diagnostics[0].at, "p5");
        assert!(report.diagnostics[0].message.contains("#missing"));
    }

    #[test]
    fn a_style_name_is_undeclared_until_the_document_declares_it() {
        let mut doc = empty();
        let index = push(
            &mut doc,
            BlockKind::Paragraph,
            vec![Run::Text {
                text: "x".to_owned(),
                style: Some("Emphasis".to_owned()),
                props: CharStyle::default(),
                href: None,
            }],
        );
        doc.blocks[index].style = Some("List Paragraph".to_owned());

        let report = lint(&doc, &Options::default());
        assert_eq!(
            ids(&report),
            ["undeclared-style", "undeclared-style"],
            "a document that declares nothing declares neither of them"
        );

        doc.styles.insert("Emphasis".to_owned());
        doc.styles.insert("List Paragraph".to_owned());
        assert!(
            lint(&doc, &Options::default()).is_empty(),
            "a name with a space in it is one name, not two"
        );

        // A composed run style is declared when each part is.
        doc.blocks[index].runs = vec![Run::Text {
            text: "x".to_owned(),
            style: Some("Emphasis Strong".to_owned()),
            props: CharStyle::default(),
            href: None,
        }];
        assert_eq!(ids(&lint(&doc, &Options::default())), ["undeclared-style"]);
        doc.styles.insert("Strong".to_owned());
        assert!(lint(&doc, &Options::default()).is_empty());
    }

    #[test]
    fn an_off_palette_colour_is_a_hint_and_therefore_silent_by_default() {
        let mut doc = empty();
        push(
            &mut doc,
            BlockKind::Paragraph,
            vec![Run::Text {
                text: "x".to_owned(),
                style: None,
                props: CharStyle {
                    color: Some("#123456".to_owned()),
                    background: Some("transparent".to_owned()),
                    ..CharStyle::default()
                },
                href: None,
            }],
        );
        assert!(lint(&doc, &Options::default()).is_empty());
        let report = lint(&doc, &loud());
        assert_eq!(ids(&report), ["off-palette"], "transparent is not a colour");

        // A palette colour is quiet however it is cased.
        doc.blocks[0].runs = vec![Run::Text {
            text: "x".to_owned(),
            style: None,
            props: CharStyle {
                color: Some("#FF4136".to_owned()),
                ..CharStyle::default()
            },
            href: None,
        }];
        assert!(lint(&doc, &loud()).is_empty());
    }

    #[test]
    fn an_image_is_named_as_what_the_projection_cannot_spell() {
        let mut doc = empty();
        push(
            &mut doc,
            BlockKind::Paragraph,
            vec![Run::Image {
                mime: "image/png".to_owned(),
                data: vec![0; 12],
                width: None,
                height: None,
                anchor: None,
                title: None,
                description: None,
            }],
        );
        let report = lint(&doc, &Options::default());
        assert_eq!(ids(&report), ["unspellable"]);
        assert!(report.diagnostics[0].message.contains("image/png"));
    }

    #[test]
    fn a_misspelling_is_reported_only_when_there_is_a_dictionary_to_ask() {
        let mut doc = empty();
        push(
            &mut doc,
            BlockKind::Paragraph,
            vec![Run::Text {
                text: "I recieve it".to_owned(),
                style: None,
                props: CharStyle::default(),
                href: None,
            }],
        );
        assert!(lint(&doc, &Options::default()).is_empty(), "no dictionary");
        let list = crate::spell::tests::List(&["i", "it", "receive"]);
        let report = lint_with(&doc, &Options::default(), Some(&list));
        assert_eq!(ids(&report), ["misspelt"]);
        assert_eq!(report.diagnostics[0].at, "p1+2");
        let off = Options {
            hints: false,
            off: vec!["misspelt".to_owned()],
        };
        assert!(lint_with(&doc, &off, Some(&list)).is_empty());
    }

    #[test]
    fn any_rule_can_be_silenced_by_id() {
        let mut doc = empty();
        push(&mut doc, BlockKind::Heading { level: 1 }, vec![]);
        push(&mut doc, BlockKind::Heading { level: 3 }, vec![]);
        let options = Options {
            hints: false,
            off: vec!["heading-skip".to_owned()],
        };
        assert!(lint(&doc, &options).is_empty());
    }

    #[test]
    fn every_rule_has_a_unique_id_and_a_description() {
        for (i, rule) in RULES.iter().enumerate() {
            assert!(!rule.what.is_empty(), "{} says what it checks", rule.id);
            assert!(
                RULES[..i].iter().all(|other| other.id != rule.id),
                "{} appears twice",
                rule.id
            );
        }
    }

    #[test]
    fn an_empty_document_is_clean() {
        assert!(lint(&empty(), &loud()).is_empty());
        let _ = BlockId(0);
    }
}
