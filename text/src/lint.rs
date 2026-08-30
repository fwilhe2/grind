// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The word processor's lint rules — `doc/dsl.md` §4.3, D6.
//!
//! *No third-party linter knows what a heading is.* That sentence is the whole argument for
//! this module: the interesting rules are about **documents**, and a document's vocabulary is
//! this crate's. [`grind_core::lint`] holds what a diagnostic *is* and nothing else (R8).
//!
//! Every rule is in [`RULES`] with an id, and [`RULES`] is checked against §4.3's table by
//! `cli/tests/lint.rs` — a rule the document does not name fails the build, which is D6's
//! exit criterion ("every rule named in a table and covered by a test") made mechanical, the
//! way `doc/small-group.md` is for the function list.
//!
//! Addresses are `loc.rs`'s, so every one of them is a string a user can type straight back at
//! `grind text get`.

/// The vocabulary a diagnostic is made of, re-exported so a shell reaches it through the
/// application crate it already depends on — the same ergonomics `grind_sheet::odf` offers for
/// the generic reader modules. The types are `grind_core`'s and there is no second definition.
pub use grind_core::lint::{Diagnostic, Options, Report, Rule, Severity};

use grind_core::style::PALETTE;

use crate::loc;
use crate::model::{Block, Document, Run};
use crate::style::CharStyle;

/// A heading whose level is more than one past the heading before it — 1 → 3.
///
/// Only *after* a first heading, which sets the baseline. A document whose first heading is a
/// level 2 is a fragment of a larger one as often as it is a mistake, and a rule that fires on
/// every chapter file of a book is a rule people turn off.
pub const HEADING_SKIP: Rule = Rule {
    id: "heading-skip",
    severity: Severity::Warning,
    what: "a heading level skipped — 1 followed by 3",
};

/// A link to `#name` where no `text:bookmark` of that name exists.
///
/// An error rather than a warning: the reader sees a link, clicks it, and nothing happens.
pub const UNKNOWN_BOOKMARK: Rule = Rule {
    id: "unknown-bookmark",
    severity: Severity::Error,
    what: "a bookmark referenced and never declared",
};

/// A `text:style-name` naming a style the document does not declare.
///
/// `doc/text-core.md`'s known loss, made visible: this build carries style *names* and not
/// style *definitions*, so a document it has written declares none at all and every name in it
/// is reported. That is the honest reading — the file really does refer to something it does
/// not define — and it is the diagnostic that says so out loud rather than in a doc comment.
pub const UNDECLARED_STYLE: Rule = Rule {
    id: "undeclared-style",
    severity: Severity::Warning,
    what: "a style name used and never declared",
};

/// A colour that is not one of `grind_core::style::PALETTE`'s.
///
/// A hint, and off unless asked for, because [`PALETTE`] is a default a shell offers and never
/// a limit — §4.3 says exactly that, and this rule is only useful to somebody who has decided
/// their document should keep to it.
pub const OFF_PALETTE: Rule = Rule {
    id: "off-palette",
    severity: Severity::Hint,
    what: "a colour outside the default palette",
};

/// Something in the document that the projection cannot spell.
///
/// The bijectivity guard as a diagnostic, and the row that earns the feature: opening a
/// document and being *told*, by name, what a `.grind` of it would not carry. For this
/// application that is images and nothing else (`doc/projection-text.md`), so the rule has one
/// producer today and the day a second gap appears it gains a second.
pub const UNSPELLABLE: Rule = Rule {
    id: "unspellable",
    severity: Severity::Warning,
    what: "a construct the projection cannot spell",
};

/// Every rule this application has. The order is the order `grind text lint --rules` prints.
pub const RULES: [Rule; 5] = [
    HEADING_SKIP,
    UNKNOWN_BOOKMARK,
    UNDECLARED_STYLE,
    OFF_PALETTE,
    UNSPELLABLE,
];

/// Check a document against every rule `options` wants.
pub fn lint(doc: &Document, options: &Options) -> Report {
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
    report.sort();
    report
}

/// `p12` — the address of a block, which is what every rule here reports against.
///
/// `loc::format` is the one 0↔1 conversion, so this is a call rather than an arithmetic
/// expression however tempting `index + 1` looks.
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

/// The bookmark an `xlink:href` points at, when it points inside this document.
///
/// `#intro` is the plain form; LibreOffice writes `#intro|outline` and `#intro|region` to say
/// what kind of target it expects, and the part after the bar is not part of the name. An
/// `href` naming another document — `other.odt#intro`, `https://…` — is somebody else's
/// business and answers `None`.
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

/// Every style name a block mentions — its own, and each of its runs'. Deduplicated in
/// document order, because a paragraph in one style with forty runs in another is two
/// diagnostics rather than forty-one.
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

/// Whether the document declares this style name.
///
/// A run's name may be a *composition* — `text:span` nests and the model is flat, so reading
/// joins the open names with a space (`doc/text-core.md`). So the whole string is tried first,
/// and only then its whitespace-separated pieces: a declared `"List Paragraph"` is one name
/// with a space in it, and splitting first would call it two undeclared ones.
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

/// The colours a character style sets that the palette does not have.
///
/// `transparent` is ODF's own word for *no* background rather than a colour, so it is not one
/// this rule has an opinion about.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{BlockId, BlockKind};

    fn empty() -> Document {
        Document::new()
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

        // A composed run style — `text:span` inside `text:span` — is every open name joined,
        // and is declared when each part is.
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
            }],
        );
        let report = lint(&doc, &Options::default());
        assert_eq!(ids(&report), ["unspellable"]);
        assert!(report.diagnostics[0].message.contains("image/png"));
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
