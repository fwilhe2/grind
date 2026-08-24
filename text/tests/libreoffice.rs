// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Documents LibreOffice Writer actually wrote — vendored, and **this test never skips**.
//!
//! `sheet/tests/kb.rs` is the spreadsheet's version of this and says why it exists: loop A's
//! corpus is LibreOffice's own, lives outside the repository and skips when it is not there,
//! and *a requirement that skips is a preference*. Text had no such fixture at all — every
//! check on real Writer output needed `GRIND_LO_CORPUS` — so `data/` holds a small one that
//! is always present.
//!
//! **What is in it**, and why it is a useful sample rather than a big one: a document with the
//! light formatting a real document has, saved by Writer in **both forms**. A `Title` and a
//! `Subtitle` (which are paragraphs, not headings — a trap for anything that assumes a big
//! bold line is a `text:h`), two heading levels that *are* headings, character spans inside
//! paragraphs, an **edited default paragraph style** (which is what puts every body paragraph
//! on an automatic `P1` instead of `Standard`, and what the file is named after), an empty
//! paragraph, and a `text:s`. The `.odt` is a whole LibreOffice package: `styles.xml`,
//! `settings.xml`, `meta.xml`, `manifest.rdf` and a PNG thumbnail, none of which this build
//! models and all of which have to come back.
//!
//! **Adding one is dropping a file in.** Everything below walks `data/`, so a new document
//! from Writer is covered the moment it lands: it must load, it must survive an untouched save
//! byte for byte, and if it is one of a `.odt`/`.fodt` pair the two forms must agree. The
//! assertions that name *this* document are last and are the only part that knows what is in
//! it.
//!
//! Licensing: the fixtures are **CC0-1.0**, declared in `REUSE.toml` rather than annotated —
//! a header inside the XML would change the document under test, which is the whole thing
//! being measured. See that file for why data is not AGPL here.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use grind_text::{App, BlockKind, Form};

/// The document this file's named assertions are about.
const EDITED_DEFAULT: &str = "edited-default-paragraph-style";

fn data() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data")
}

/// Every vendored document, in a stable order.
///
/// Globbed rather than listed, unlike R7's corpus: that one names its files because the
/// requirement *is* those files, and this one is a harness whose whole point is that adding a
/// document to it costs nothing.
fn documents() -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(data())
        .expect("text/tests/data exists")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("odt") | Some("fodt")
            )
        })
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "the fixtures went missing");
    paths
}

fn open(path: &Path) -> App {
    let app = App::new();
    app.open_file(path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    app
}

/// Every block as `(kind, style, text)` — everything a reader gets, in one comparable shape.
fn blocks(app: &App) -> Vec<(BlockKind, Option<String>, String)> {
    app.get_viewport(0..app.block_count())
        .iter()
        .map(|block| (block.kind.clone(), block.style.clone(), block.text.clone()))
        .collect()
}

/// R5, with no corpus checkout: what Writer writes, this reads.
#[test]
fn every_vendored_document_loads() {
    for path in documents() {
        let app = open(&path);
        assert!(
            app.block_count() > 0,
            "{} loaded as an empty document",
            path.display()
        );
    }
}

/// R6, against a file none of us wrote — which is the version that matters, because the
/// reason to care is somebody's own document opening and not showing up as a commit.
///
/// `text/tests/diffable.rs` proves this property against documents written for the purpose.
/// **Flat form only**: see below for what a package does instead, and why.
#[test]
fn an_untouched_flat_save_returns_the_bytes_exactly() {
    let mut checked = 0;
    for path in documents() {
        if Form::from_path(&path) != Form::Flat {
            continue;
        }
        let app = open(&path);
        let before = std::fs::read(&path).expect("reads");
        let after = app.save_bytes(Form::Flat).expect("writes");
        assert!(
            before == after,
            "{} came back changed ({} bytes in, {} out)",
            path.display(),
            before.len(),
            after.len()
        );
        checked += 1;
    }
    assert!(checked > 0, "no flat document is vendored");
}

/// **What saving a package costs today**, asserted rather than discovered.
///
/// `text/src/odf/source.rs` states the boundary — *only the flat form; a `.odt` is a zip, and
/// a zip has no diff to preserve* — and this is that sentence with a number against it. A
/// document Writer saved as 9 entries comes back as 3: `content.xml`, the manifest and the
/// mimetype. `styles.xml`, `settings.xml`, `meta.xml`, `manifest.rdf` and the thumbnail are
/// **gone**, on a plain open-and-save with no edit at all.
///
/// That is a real cost and it is bigger than the diff it was justified by: the flat form loses
/// nothing here, so the same document is lossless in one container and lossy in the other. The
/// fix is not a bigger model — it is keeping the original archive and replacing one entry in
/// it, which is the same retain-and-splice trick one level up. Until then this test is where
/// the price is written down, and the day the writer learns it, this goes red and gets
/// replaced by the byte-exact assertion above.
#[test]
fn saving_a_package_regenerates_it_and_keeps_only_the_content() {
    let path = data().join(format!("{EDITED_DEFAULT}.odt"));
    let app = open(&path);
    let before = std::fs::read(&path).expect("reads");
    let after = app.save_bytes(Form::Package).expect("writes");
    assert!(
        after.len() < before.len() / 4,
        "the package no longer regenerates — {} bytes in, {} out, and this test owes an update",
        before.len(),
        after.len()
    );

    // What survives is what the model carries: every block, its kind and its style *name*.
    // What does not is everything the model never had — which is why this is worth a test
    // rather than a comment.
    let again = App::new();
    again.open_bytes("again.odt", &after).expect("reads back");
    assert_eq!(blocks(&app), blocks(&again), "the content itself survives");
}

/// The package reader and the flat reader must agree about the same document.
///
/// Writer saved one document twice, so the only difference between the two files is the
/// container — and a difference in what comes out of them is a bug in one of the two readers
/// rather than a fact about the document.
#[test]
fn both_forms_of_a_document_read_the_same() {
    let mut pairs: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for path in documents() {
        let stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .expect("a name")
            .to_owned();
        pairs.entry(stem).or_default().push(path);
    }
    let mut compared = 0;
    for (stem, forms) in pairs {
        let [flat, package] = &forms[..] else {
            continue;
        };
        assert_eq!(
            blocks(&open(flat)),
            blocks(&open(package)),
            "{stem}: the two forms disagree"
        );
        compared += 1;
    }
    assert!(compared > 0, "no document is vendored in both forms");
}

/// What is actually in the fixture — the assertions that would notice a reader losing
/// something, rather than a reader failing outright.
#[test]
fn the_writer_document_reads_as_it_was_written() {
    let app = open(&data().join(format!("{EDITED_DEFAULT}.fodt")));
    let blocks = blocks(&app);
    assert_eq!(blocks.len(), 10);

    // A `Title` and a `Subtitle` are **paragraphs** in ODF, however large Writer draws them.
    // Only `text:h` is a heading, and only its `text:outline-level` decides the level
    // (`doc/odt-format.md` §2) — so the outline here is two entries, not four.
    assert_eq!(blocks[0].0, BlockKind::Paragraph);
    assert_eq!(blocks[0].1.as_deref(), Some("Title"));
    assert_eq!(blocks[0].2, "Lorem Ipsum");
    assert_eq!(blocks[1].1.as_deref(), Some("Subtitle"));

    let outline = app.outline();
    let seen: Vec<(String, u32, &str)> = outline
        .iter()
        .map(|heading| (heading.address(), heading.level, heading.text.as_str()))
        .collect();
    assert_eq!(
        seen,
        vec![
            ("\u{a7}1".to_owned(), 1, "Lorem Ipsum"),
            ("\u{a7}1.1".to_owned(), 2, "Dolor Sit Amet"),
        ],
        "two headings, and the outline path is computed rather than stored"
    );

    // The style name the *file* uses, kept verbatim — `Heading_20_1` is ODF's encoding of
    // "Heading 1", and a reader that helpfully decoded it would write back something
    // LibreOffice does not resolve.
    assert_eq!(blocks[4].0, BlockKind::Heading { level: 1 });
    assert_eq!(blocks[4].1.as_deref(), Some("Heading_20_1"));

    // Every body paragraph is on the automatic `P1` rather than `Standard`, which is what
    // editing the default paragraph style in Writer does and what this file is named after.
    assert_eq!(blocks[2].1.as_deref(), Some("P1"));

    // `<text:s/>` is ODF's run-length encoding of spaces and is expanded on read
    // (`doc/odt-format.md` §3.3) — the paragraph ends with two spaces, one of them encoded.
    assert!(
        blocks[7].2.ends_with("facilisi.  "),
        "the encoded space was lost: {:?}",
        &blocks[7].2[blocks[7].2.len() - 20..]
    );

    // An empty paragraph is a block, not nothing: it is what somebody pressed Enter for.
    assert_eq!(blocks[8].2, "");

    // Direct character formatting — the `text:span`s Writer wrote inside two of these
    // paragraphs — is what `grind text formatting` exists to point at.
    let styled: Vec<usize> = app.formatting().iter().map(|block| block.index).collect();
    assert!(styled.contains(&1) && styled.contains(&5), "{styled:?}");

    assert_eq!(app.counts().words, 407);
    assert_eq!(app.counts().headings, 2);
}

/// Editing one paragraph of a real Writer document changes one paragraph of its XML.
///
/// The differentiator, measured where it counts. Everything Writer put in the file and this
/// build has no model for — `text:sequence-decls`, the page layout, the RSID attributes on
/// every automatic style — is still there afterwards, because the writer never regenerated it.
#[test]
fn editing_one_paragraph_of_a_writer_document_changes_one_line() {
    let path = data().join(format!("{EDITED_DEFAULT}.fodt"));
    let app = open(&path);
    app.set_text(9, "Nam liber tempor, edited.").expect("edits");
    let before = std::fs::read_to_string(&path).expect("reads");
    let after = String::from_utf8(app.save_bytes(Form::Flat).expect("writes")).expect("utf-8");

    let removed: Vec<&str> = before
        .lines()
        .filter(|line| !after.lines().any(|other| other == *line))
        .collect();
    let added: Vec<&str> = after
        .lines()
        .filter(|line| !before.lines().any(|other| other == *line))
        .collect();
    assert_eq!(removed.len(), 1, "{removed:#?}");
    assert_eq!(added.len(), 1, "{added:#?}");
    assert!(added[0].contains("Nam liber tempor, edited."), "{added:#?}");
    // The style name survives the edit: `set_text` replaces a block's *content*.
    assert!(added[0].contains("text:style-name=\"P1\""), "{added:#?}");
    // And the parts of the file nobody touched are untouched, including the ones this build
    // has no model for at all.
    assert!(after.contains("<text:sequence-decls>"));
    assert!(after.contains("officeooo:rsid"));
}

/// The layout engine over a document that was not written for it — including the empty
/// paragraph, which is the case that was wrong until the GTK shell found it
/// (`doc/text-shell.md`).
#[test]
fn every_block_lays_out_including_the_empty_one() {
    let app = open(&data().join(format!("{EDITED_DEFAULT}.odt")));
    for index in 0..app.block_count() {
        let layout = app
            .layout_block(index, 72.0, &grind_text::Fixed)
            .expect("lays out");
        assert!(
            !layout.lines().is_empty(),
            "block {index} laid out as no lines at all"
        );
        assert!(layout.height() >= 1.0, "block {index} has no height");
    }
}
