// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `doc/page-model.md` PM3 item 6: after every edit of a scripted session, the pages set from the
//! pages before (`typeset_after`, which copies those it finds unchanged) equal the pages set from
//! nothing. Over the vendored documents and a long one made here, and over every `.odt` of a
//! LibreOffice checkout when `GRIND_LO_CORPUS` names one.

use grind_print::text::{Options, typeset, typeset_after};
use grind_print::{Fonts, Typesetter};
use grind_text::{App, BlockKind, Caret};

/// Set `app` again after each edit, both ways, and say where they differ.
/// Answers how many pages were copied rather than drawn.
fn equivalent(name: &str, app: &App, setter: &Typesetter) -> usize {
    let options = Options::default();
    let mut before = typeset(app, setter, &options);
    let mut copied = 0;
    for (number, edit) in steps().iter().enumerate() {
        if !edit(app) {
            continue;
        }
        let incremental = typeset_after(app, setter, &options, Some(&before));
        let fresh = typeset(app, setter, &options);
        assert_eq!(
            incremental.pages.len(),
            fresh.pages.len(),
            "{name}: page count after edit {number}"
        );
        for (page, (a, b)) in incremental.pages.iter().zip(&fresh.pages).enumerate() {
            assert!(a == b, "{name}: page {} after edit {number}", page + 1);
        }
        assert_eq!(incremental.outline, fresh.outline, "{name}: outline");
        assert_eq!(incremental.keys, fresh.keys, "{name}: keys");
        copied += incremental
            .keys
            .iter()
            .filter(|key| before.keys.contains(key))
            .count();
        before = incremental;
    }
    copied
}

/// The edits a person makes, at places spread through the document, each answering whether it
/// applied.
/// One edit, answering whether it applied.
type Step = Box<dyn Fn(&App) -> bool>;

fn steps() -> Vec<Step> {
    let at = |block: usize, offset: usize| Caret { block, offset };
    let len = |app: &App, index: usize| grind_text::caret::block_len(app, index);
    vec![
        Box::new(move |app: &App| {
            let middle = app.block_count() / 2;
            app.insert_text(at(middle, len(app, middle) / 2), "word ")
                .is_ok()
        }),
        Box::new(move |app: &App| {
            app.insert_text(at(0, 0), "Opening words, enough to wrap a line further. ")
                .is_ok()
        }),
        Box::new(move |app: &App| {
            let middle = app.block_count() / 2;
            app.split_block(at(middle, len(app, middle) / 3)).is_ok()
        }),
        Box::new(move |app: &App| app.break_page(at(app.block_count() / 3, 0)).is_ok()),
        Box::new(move |app: &App| app.join_block(app.block_count() / 2).is_ok()),
        Box::new(move |app: &App| {
            let last = app.block_count().saturating_sub(2);
            app.set_kind(last, BlockKind::Heading { level: 2 }).is_ok()
        }),
        Box::new(move |app: &App| {
            let end = app.block_count() - 1;
            let to = len(app, end).min(5);
            app.erase(at(end, 0), at(end, to)).is_ok()
        }),
        Box::new(move |app: &App| {
            let middle = app.block_count() / 2;
            app.insert_text(at(middle, 0), "x").is_ok()
        }),
    ]
}

fn setter() -> Typesetter {
    Typesetter::new(Fonts::bundled())
}

#[test]
fn copied_pages_equal_drawn_ones_in_a_long_document() {
    let app = App::new();
    let paragraph =
        "All work and no play makes a dull document, and a dull document makes no play. ";
    let mut at = 0;
    for section in 0..6 {
        app.insert(
            at,
            BlockKind::Heading { level: 1 },
            &format!("Part {section}"),
        )
        .unwrap();
        at += 1;
        for item in 0..4 {
            app.insert(
                at,
                BlockKind::ListItem { depth: 1 },
                &format!("Item {item}"),
            )
            .unwrap();
            at += 1;
        }
        for _ in 0..5 {
            app.insert(at, BlockKind::Paragraph, &paragraph.repeat(6))
                .unwrap();
            at += 1;
        }
    }
    app.insert_table(at, 3, 2, None).unwrap();
    let copied = equivalent("long", &app, &setter());
    assert!(
        copied > 10,
        "only {copied} pages copied: the test proves nothing"
    );
}

#[test]
fn copied_pages_equal_drawn_ones_in_the_vendored_documents() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../text/tests/data");
    let setter = setter();
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        let app = App::new();
        if app.open_file(&path).is_err() {
            continue;
        }
        equivalent(&path.display().to_string(), &app, &setter);
    }
}

#[test]
fn copied_pages_equal_drawn_ones_in_the_libreoffice_corpus() {
    let Some(root) = std::env::var_os("GRIND_LO_CORPUS") else {
        eprintln!("GRIND_LO_CORPUS not set: skipping the corpus");
        return;
    };
    let setter = setter();
    let (mut seen, mut copied) = (0, 0);
    let mut stack = vec![std::path::PathBuf::from(root)];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|ext| ext == "odt") {
                let app = App::new();
                if app.open_file(&path).is_ok() && app.block_count() > 0 {
                    copied += equivalent(&path.display().to_string(), &app, &setter);
                    seen += 1;
                }
            }
        }
    }
    eprintln!("{seen} documents, {copied} pages copied");
}
