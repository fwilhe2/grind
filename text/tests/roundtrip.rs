// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Write it, read it back, get the same document — **loop C for text**, in two halves.
//!
//! The first half needs nothing installed: write with our writer, read with our reader, and
//! assert the meaning survived. It catches the whole class of bug where the writer emits
//! something the reader cannot get back — a lost space run, a list whose nesting did not
//! reconstruct, an escape that escaped twice.
//!
//! The second half is loop C proper, the same shape `sheet/tests/roundtrip.rs` has for
//! spreadsheets: put LibreOffice in the middle, in both directions.
//!
//! * **out** — build a document, write it, have LO convert it, read LO's output back, and
//!   assert we get the same document. Catches everything we emit that LO drops or reinterprets.
//! * **back** — take a Writer-authored file out of `sw/qa`, read it, write it, convert *that*,
//!   read it back, and assert it still matches what we first read. Catches the writer losing
//!   something the reader understood.
//!
//! The first half proves the writer is self-consistent; only the second proves it is *right*,
//! which is the difference that matters. It needs `soffice` on `PATH` and skips with a notice
//! without one; the "back" direction also wants `GRIND_LO_CORPUS`.
//!
//! Both halves are *semantic* round trips, not byte ones. The writer regenerates when it cannot
//! splice, so the bytes of a document read and written back are this writer's rather than the
//! original's — but what they *mean* must survive exactly.

use std::path::{Path, PathBuf};
use std::process::Command;

use grind_text::model::{Block, BlockKind, Document, Run};
use grind_text::{Form, odf};

fn text(s: &str) -> Run {
    Run::Text {
        text: s.to_owned(),
        style: None,
        href: None,
    }
}

/// Build a document from `(kind, runs)` pairs.
fn build(spec: Vec<(BlockKind, Vec<Run>)>) -> Document {
    let mut doc = Document::new();
    for (kind, runs) in spec {
        let id = doc.next_id();
        let mut block = Block::new(id, kind);
        block.runs = runs;
        doc.blocks.push(block);
    }
    doc.reindex_bookmarks();
    doc
}

/// Write, read back, and compare what the two documents *mean*.
///
/// Ids are minted fresh by the reader, so they are excluded deliberately: an id is this
/// build's handle on a block, not something the file carries.
fn roundtrip(doc: &Document, form: Form) -> Document {
    let bytes = odf::write(doc, form).expect("writes");
    let back = odf::read(&bytes).expect("reads back what it just wrote");
    assert_eq!(
        back.blocks.len(),
        doc.blocks.len(),
        "block count survived\n--- wrote ---\n{}",
        String::from_utf8_lossy(&bytes)
    );
    for (a, b) in doc.blocks.iter().zip(&back.blocks) {
        assert_eq!(
            (&a.kind, a.text(), &a.style),
            (&b.kind, b.text(), &b.style),
            "block survived\n--- wrote ---\n{}",
            String::from_utf8_lossy(&bytes)
        );
    }
    back
}

fn both_forms(doc: &Document) {
    roundtrip(doc, Form::Flat);
    roundtrip(doc, Form::Package);
}

#[test]
fn paragraphs_and_headings_survive_both_forms() {
    let doc = build(vec![
        (BlockKind::Heading { level: 1 }, vec![text("Title")]),
        (BlockKind::Paragraph, vec![text("A paragraph.")]),
        (BlockKind::Heading { level: 3 }, vec![text("Deep")]),
        (BlockKind::Paragraph, vec![]),
    ]);
    both_forms(&doc);
}

/// The one a naive writer gets wrong. XML collapses whitespace, so spaces written literally
/// come back missing — `doc/odt-format.md` §3.3.
#[test]
fn runs_of_spaces_survive_because_they_are_re_encoded() {
    for spelling in [
        "a    b",
        "  leading",
        "trailing  ",
        "  ",
        "a b c",
        "one  two   three",
        " ",
    ] {
        let doc = build(vec![(BlockKind::Paragraph, vec![text(spelling)])]);
        let back = roundtrip(&doc, Form::Flat);
        assert_eq!(
            back.blocks[0].text(),
            spelling,
            "{spelling:?} did not survive"
        );
    }
}

#[test]
fn a_nested_list_reconstructs_from_the_depths_the_model_flattened() {
    let doc = build(vec![
        (BlockKind::Paragraph, vec![text("before")]),
        (BlockKind::ListItem { depth: 1 }, vec![text("one")]),
        (BlockKind::ListItem { depth: 1 }, vec![text("two")]),
        (BlockKind::ListItem { depth: 2 }, vec![text("two a")]),
        (BlockKind::ListItem { depth: 2 }, vec![text("two b")]),
        (BlockKind::ListItem { depth: 1 }, vec![text("three")]),
        (BlockKind::Paragraph, vec![text("after")]),
    ]);
    both_forms(&doc);
}

/// A jump of two levels at once has to open two elements, not one malformed one.
#[test]
fn a_list_that_jumps_two_levels_still_nests_properly() {
    let doc = build(vec![
        (BlockKind::ListItem { depth: 1 }, vec![text("one")]),
        (BlockKind::ListItem { depth: 3 }, vec![text("deep")]),
        (BlockKind::ListItem { depth: 1 }, vec![text("back")]),
    ]);
    both_forms(&doc);
}

/// A document ending inside a list must still close every element it opened.
#[test]
fn a_document_ending_in_a_list_is_still_well_formed() {
    let doc = build(vec![
        (BlockKind::Paragraph, vec![text("intro")]),
        (BlockKind::ListItem { depth: 2 }, vec![text("last")]),
    ]);
    both_forms(&doc);
}

/// A tab or a newline *inside a run's text* is significant whitespace and has to become the
/// element ODF has for it, exactly as a run of spaces does.
///
/// Loop C is what found this: the model has had [`Run::Tab`] and [`Run::Break`] since S4, but a
/// paragraph whose text merely *contained* `\t` — which is what `grind text set` produces —
/// wrote the character literally, and LibreOffice handed it back as a space.
#[test]
fn tabs_and_newlines_inside_a_run_survive_as_elements() {
    for (spelling, want) in [
        ("a\tb", "a\tb"),
        ("a\nb", "a\nb"),
        ("\ttab first", "\ttab first"),
        ("trailing tab\t", "trailing tab\t"),
        ("a\t  spaced after a tab", "a\t  spaced after a tab"),
        ("two\n\nbreaks", "two\n\nbreaks"),
        // XML normalises both line endings to `\n` before a reader ever sees them
        // (XML 1.0 §2.11), so this is what a reader gets whatever we write.
        ("crlf\r\nhere", "crlf\nhere"),
        ("cr\rhere", "cr\nhere"),
    ] {
        let doc = build(vec![(BlockKind::Paragraph, vec![text(spelling)])]);
        let bytes = odf::write(&doc, Form::Flat).expect("writes");
        let back = odf::read(&bytes).expect("reads back");
        assert_eq!(
            back.blocks[0].text(),
            want,
            "{spelling:?} did not survive\n--- wrote ---\n{}",
            String::from_utf8_lossy(&bytes)
        );
    }
}

#[test]
fn tabs_breaks_bookmarks_and_links_survive() {
    let doc = build(vec![(
        BlockKind::Paragraph,
        vec![
            Run::Bookmark {
                name: "start".to_owned(),
            },
            text("see "),
            Run::Text {
                text: "the docs".to_owned(),
                style: None,
                href: Some("https://example.invalid/a?b=1&c=2".to_owned()),
            },
            Run::Tab,
            Run::Break,
            Run::Text {
                text: "emphasised".to_owned(),
                style: Some("Emph".to_owned()),
                href: None,
            },
        ],
    )]);
    let back = roundtrip(&doc, Form::Flat);
    assert_eq!(back.bookmarks.len(), 1, "the anchor came back");
    let hrefs: Vec<_> = back.blocks[0]
        .runs
        .iter()
        .filter_map(|r| match r {
            Run::Text { href: Some(h), .. } => Some(h.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        hrefs,
        vec!["https://example.invalid/a?b=1&c=2"],
        "an ampersand in a URL is escaped once, not twice"
    );
}

#[test]
fn markup_characters_in_text_are_escaped_rather_than_emitted() {
    let doc = build(vec![(
        BlockKind::Paragraph,
        vec![text("a < b & c > d \"quoted\" 'single'")],
    )]);
    let back = roundtrip(&doc, Form::Flat);
    assert_eq!(back.blocks[0].text(), "a < b & c > d \"quoted\" 'single'");
}

/// R3: nothing written that the document does not use.
#[test]
fn the_output_carries_only_the_boilerplate_it_needs() {
    let plain = build(vec![(BlockKind::Paragraph, vec![text("hi")])]);
    let bytes = odf::write(&plain, Form::Flat).expect("writes");
    let xml = String::from_utf8(bytes).expect("utf-8");
    assert!(
        !xml.contains("xmlns:xlink"),
        "a document with no link declares no xlink namespace:\n{xml}"
    );
    assert!(
        xml.lines().count() < 12,
        "a one-paragraph document should be a handful of lines, not a template:\n{xml}"
    );

    let linked = build(vec![(
        BlockKind::Paragraph,
        vec![Run::Text {
            text: "x".to_owned(),
            style: None,
            href: Some("https://x/".to_owned()),
        }],
    )]);
    let xml = String::from_utf8(odf::write(&linked, Form::Flat).expect("writes")).expect("utf-8");
    assert!(
        xml.contains("xmlns:xlink"),
        "and a document with one does:\n{xml}"
    );
}

/// The package form has to be a package: `mimetype` first, stored, byte-exact (§1.1).
#[test]
fn the_package_form_is_sniffable_as_a_text_document() {
    let doc = build(vec![(BlockKind::Paragraph, vec![text("hi")])]);
    let bytes = odf::write(&doc, Form::Package).expect("writes");
    assert_eq!(
        grind_text::kind(&bytes),
        Some(grind_text::DocumentKind::Text),
        "a reader must be able to tell what this is before parsing it"
    );
}

/// Writing does not change the document — the property every "save" depends on.
#[test]
fn writing_twice_produces_the_same_bytes() {
    let doc = build(vec![
        (BlockKind::Heading { level: 2 }, vec![text("H")]),
        (BlockKind::ListItem { depth: 1 }, vec![text("a  b")]),
    ]);
    let once = odf::write(&doc, Form::Flat).expect("writes");
    let twice = odf::write(&doc, Form::Flat).expect("writes");
    assert_eq!(once, twice);

    // And reading-then-writing is stable: the second generation equals the first.
    let back = odf::read(&once).expect("reads");
    let again = odf::write(&back, Form::Flat).expect("writes");
    assert_eq!(
        String::from_utf8_lossy(&once),
        String::from_utf8_lossy(&again),
        "a document that has been through the reader writes identically"
    );
}

// --- Loop C proper: ours -> LibreOffice -> ours -------------------------------------------

const DEFAULT_CHECKOUT: &str = "/home/florian/code/github.com/LibreOffice/core";

/// Writer's test data, under the checkout root. `GRIND_LO_CORPUS` names the **root**, because
/// one clone serves both applications — Calc's corpus is at `sc/qa/unit/data`.
const CORPUS: &str = "sw/qa";

/// How many corpus documents the "back" direction takes. Each `soffice` conversion costs
/// seconds; loop A already reads all 1763, and what is under test here is the writer, which
/// does not vary per file.
const SAMPLE: usize = 20;

/// Skip corpus documents longer than this. A block-by-block comparison over a thousand-page
/// document is not a better test, only a slower one.
const MAX_COMPARED_BLOCKS: usize = 2_000;

/// A scratch directory that cleans itself up.
struct Lab {
    dir: PathBuf,
}

impl Lab {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("text-loop-c-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("in")).unwrap();
        std::fs::create_dir_all(dir.join("out")).unwrap();
        Self { dir }
    }

    fn input(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.dir.join("in").join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    /// Convert every staged input to flat XML in **one** soffice invocation.
    ///
    /// One invocation because startup dominates: a couple of seconds each time, against
    /// milliseconds per document once it is up. The private `UserInstallation` profile is not
    /// optional — without it this fights the developer's own running LibreOffice for the
    /// profile lock and either blocks or silently does nothing.
    fn convert(&self, inputs: &[PathBuf]) -> PathBuf {
        let out = self.dir.join("out");
        let status = self.try_convert(inputs);
        assert!(status.success(), "soffice exited with {status}");
        out
    }

    fn try_convert(&self, inputs: &[PathBuf]) -> std::process::ExitStatus {
        Command::new("soffice")
            .arg("--headless")
            .arg(format!(
                "-env:UserInstallation=file://{}",
                self.dir.join("profile").display()
            ))
            .args(["--convert-to", "fodt", "--outdir"])
            .arg(self.dir.join("out"))
            .args(inputs)
            .status()
            .expect("soffice failed to start")
    }
}

impl Drop for Lab {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn have_soffice() -> bool {
    Command::new("soffice")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Is there a `soffice` on `PATH`, and can it handle *text* documents?
///
/// The second half is not paranoia. **The oracle this project pins is Calc-only.**
/// `ci/libreoffice-image`'s `share/registry/` holds `calc.xcd` and no `writer.xcd`, so that
/// build imports a `.fodt` as a *spreadsheet* and has no `fodt` export filter to convert one
/// back with — every test below would fail against it for a reason that has nothing to do with
/// this code. A full LibreOffice of the same version (26.2.5.2) converts all of them.
///
/// So: probe once, and skip with a notice rather than going red. The moment the pinned image is
/// rebuilt with Writer in it, these tests start running in CI with no change here — which is
/// the point of detecting the capability instead of hard-coding the skip.
///
/// Probed by *doing* the thing rather than by inspecting the install, because "which filters
/// are registered" is a question about a LibreOffice build's internals and "did it convert my
/// document" is the question the tests actually depend on.
fn oracle_ready(what: &str) -> bool {
    static PROBE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let ready = *PROBE.get_or_init(|| {
        if !have_soffice() {
            return false;
        }
        let lab = Lab::new("probe");
        let doc = build(vec![(BlockKind::Paragraph, vec![text("probe")])]);
        let path = lab.input("probe.fodt", &odf::write(&doc, Form::Flat).expect("writes"));
        lab.try_convert(std::slice::from_ref(&path));
        lab.dir.join("out/probe.fodt").exists()
    });
    if !ready {
        eprintln!(
            "skipping loop C (text, {what}): no soffice on PATH, or one that cannot convert a \
             text document — the pinned image (ci/libreoffice-image) is Calc-only"
        );
    }
    ready
}

/// Read back what LO produced for one staged input, by name.
fn converted(out: &Path, input: &Path) -> Document {
    let name = input.file_stem().unwrap().to_str().unwrap();
    let path = out.join(format!("{name}.fodt"));
    assert!(
        path.exists(),
        "LibreOffice produced no output for {}: it could not open what we wrote",
        input.display()
    );
    grind_text::read_file(&path).unwrap_or_else(|e| panic!("re-reading {}: {e}", path.display()))
}

/// What came back, with LibreOffice's one structural addition allowed for.
///
/// **A Writer document cannot be empty.** Its model has no body without a paragraph in it, so
/// the degenerate document — `grind text new` and nothing else — comes back holding one empty
/// paragraph. Measured, not assumed: `a_document_with_no_blocks_comes_back_holding_one` below
/// is the test that pins it, so this allowance goes red if LibreOffice ever stops doing it.
///
/// Deliberately narrow. It fires only for a document that had *no* blocks at all, so a real
/// document that loses or gains a trailing paragraph still fails.
fn allowing_libreoffices_paragraph<'a>(want: &Document, got: &'a Document) -> &'a [Block] {
    let gained_one = want.blocks.is_empty()
        && got.blocks.len() == 1
        && got.blocks[0].kind == BlockKind::Paragraph
        && got.blocks[0].is_empty();
    if gained_one { &[] } else { &got.blocks }
}

/// Every way two documents differ, as sentences. Not `assert_eq!` on the whole document: the
/// interesting output is *which block* moved, and a `Debug` dump of two hundred paragraphs is
/// not something anyone reads.
///
/// Structure and text, compared exactly. **Style names are not compared**, and that is this
/// loop's one documented loosening — see
/// [`a_style_name_the_document_does_not_declare_does_not_survive`] for the measurement behind
/// it and the test that keeps it honest.
fn differences(label: &str, want: &Document, got: &Document) -> Vec<String> {
    let mut out = Vec::new();
    let got_blocks = allowing_libreoffices_paragraph(want, got);

    if want.blocks.len() != got_blocks.len() {
        out.push(format!(
            "{label}: {} blocks in, {} out",
            want.blocks.len(),
            got_blocks.len()
        ));
    }
    for (i, (w, g)) in want.blocks.iter().zip(got_blocks).enumerate() {
        if w.kind != g.kind {
            out.push(format!(
                "{label}: block {i} was {:?}, back as {:?}",
                w.kind, g.kind
            ));
        }
        if w.text() != g.text() {
            out.push(format!(
                "{label}: block {i} said {:?}, back as {:?}",
                w.text(),
                g.text()
            ));
        }
        if out.len() > 20 {
            out.push(format!("{label}: ... and more"));
            return out;
        }
    }
    // A bookmark is `loc.rs`'s stable address, so losing one silently breaks every script that
    // used `#name`. Compared as the set of names: which block holds one is already covered by
    // the block comparison above, and the ids are minted fresh by each read.
    let (w, g): (Vec<_>, Vec<_>) = (
        want.bookmarks.keys().collect(),
        got.bookmarks.keys().collect(),
    );
    if w != g {
        out.push(format!("{label}: bookmarks {w:?}, back as {g:?}"));
    }
    out
}

/// The documents pushed through LibreOffice, one per thing the writer has to get right.
///
/// Shared with [`every_case_survives_our_own_round_trip`], so a case added here is checked both
/// ways round without being written twice.
fn cases() -> Vec<(String, Document)> {
    let mut all = vec![
        // The degenerate document. Legal, and the shape most likely to be written as something
        // LibreOffice rejects outright.
        ("empty".to_owned(), Document::new()),
        (
            "headings".to_owned(),
            build(vec![
                (BlockKind::Heading { level: 1 }, vec![text("One")]),
                (BlockKind::Paragraph, vec![text("under one")]),
                (BlockKind::Heading { level: 2 }, vec![text("One point one")]),
                (
                    BlockKind::Heading { level: 6 },
                    vec![text("As deep as we author")],
                ),
                // Read at any level (rng:6867). Authoring stops at 6, but a document that
                // arrived with a level-9 heading has to leave with one.
                (
                    BlockKind::Heading { level: 9 },
                    vec![text("Deeper than that")],
                ),
                (BlockKind::Heading { level: 1 }, vec![text("Two")]),
            ]),
        ),
        // The whole reason `characters()` exists: every one of these comes back as a single
        // space if the writer emits the whitespace literally.
        (
            "whitespace".to_owned(),
            build(vec![
                (BlockKind::Paragraph, vec![text("  leading")]),
                (BlockKind::Paragraph, vec![text("trailing  ")]),
                (BlockKind::Paragraph, vec![text("inner    spaces")]),
                (BlockKind::Paragraph, vec![text("one  two   three")]),
                (BlockKind::Paragraph, vec![text(" ")]),
                (BlockKind::Paragraph, vec![text("a\tb")]),
                (BlockKind::Paragraph, vec![text("a\nb")]),
                (BlockKind::Paragraph, vec![text("a\t  after a tab")]),
                // An empty paragraph is a real thing — it is how a document spaces itself.
                (BlockKind::Paragraph, vec![]),
                (BlockKind::Paragraph, vec![text("after the empty one")]),
            ]),
        ),
        (
            "text".to_owned(),
            build(vec![
                (BlockKind::Paragraph, vec![text("plain")]),
                (
                    BlockKind::Paragraph,
                    vec![text("<tag> & \"quoted\" 'single'")],
                ),
                (BlockKind::Paragraph, vec![text("Grüße — ünïcodé ✓ 𝄞")]),
                // Text that looks like markup but is not, and the one entity a naive writer
                // escapes twice.
                (
                    BlockKind::Paragraph,
                    vec![text("&amp; is not an ampersand")],
                ),
            ]),
        ),
        (
            "lists".to_owned(),
            build(vec![
                (BlockKind::Paragraph, vec![text("before")]),
                (BlockKind::ListItem { depth: 1 }, vec![text("one")]),
                (BlockKind::ListItem { depth: 1 }, vec![text("two")]),
                (BlockKind::ListItem { depth: 2 }, vec![text("two a")]),
                (BlockKind::ListItem { depth: 2 }, vec![text("two b")]),
                (BlockKind::ListItem { depth: 1 }, vec![text("three")]),
                (BlockKind::Paragraph, vec![text("after")]),
            ]),
        ),
        // A jump of two levels has to open two elements, and a document ending inside a list
        // has to close everything it opened.
        (
            "lists-uneven".to_owned(),
            build(vec![
                (BlockKind::ListItem { depth: 1 }, vec![text("one")]),
                (BlockKind::ListItem { depth: 3 }, vec![text("deep")]),
                (BlockKind::ListItem { depth: 1 }, vec![text("back")]),
                (
                    BlockKind::ListItem { depth: 2 },
                    vec![text("last, and unclosed")],
                ),
            ]),
        ),
    ];

    all.push((
        "inline".to_owned(),
        build(vec![
            (
                BlockKind::Paragraph,
                vec![
                    Run::Bookmark {
                        name: "intro".to_owned(),
                    },
                    text("see "),
                    Run::Text {
                        text: "the docs".to_owned(),
                        style: None,
                        // An ampersand in a URL is escaped once, not twice, and LibreOffice is
                        // the only judge of that which counts.
                        href: Some("https://example.invalid/a?b=1&c=2".to_owned()),
                    },
                    text(" then"),
                    Run::Tab,
                    text("tabbed"),
                    Run::Break,
                    text("and broken"),
                ],
            ),
            (
                BlockKind::Heading { level: 2 },
                vec![
                    Run::Bookmark {
                        name: "second anchor".to_owned(),
                    },
                    text("a bookmark on a heading"),
                ],
            ),
        ]),
    ));

    all
}

/// The local half, over the shared cases: our writer's output, read by our reader.
///
/// Green here and red in [`documents_we_write_survive_libreoffice`] is the interesting
/// combination — it means the writer is self-consistent and wrong.
#[test]
fn every_case_survives_our_own_round_trip() {
    for (name, doc) in cases() {
        for form in [Form::Flat, Form::Package] {
            let bytes = odf::write(&doc, form).unwrap_or_else(|e| panic!("{name}: {e}"));
            let back = odf::read(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
            let failures = differences(&name, &doc, &back);
            assert!(
                failures.is_empty(),
                "{name} ({form:?}): {failures:#?}\n--- wrote ---\n{}",
                String::from_utf8_lossy(&bytes)
            );
        }
    }
}

#[test]
fn documents_we_write_survive_libreoffice() {
    if !oracle_ready("out") {
        return;
    }

    let lab = Lab::new("out");
    let cases = cases();
    // Both physical forms, every case. They share a content writer but not a container, and the
    // container is where §1.1's byte-level rules live.
    let staged: Vec<_> = cases
        .iter()
        .flat_map(|(name, doc)| {
            [(Form::Flat, "fodt"), (Form::Package, "odt")].map(|(form, ext)| {
                let bytes = grind_text::write_bytes(doc, form).unwrap();
                // Distinct stems: LO names its output after the input's stem, so `x.odt` and
                // `x.fodt` would both convert onto `x.fodt`.
                let path = lab.input(&format!("{name}-{ext}.{ext}"), &bytes);
                (doc, path)
            })
        })
        .collect();

    let out = lab.convert(&staged.iter().map(|(_, p)| p.clone()).collect::<Vec<_>>());

    let mut failures = Vec::new();
    for (doc, path) in &staged {
        let label = path.file_name().unwrap().to_str().unwrap();
        failures.extend(differences(label, doc, &converted(&out, path)));
    }

    for f in &failures {
        eprintln!("  {f}");
    }
    eprintln!("loop C (text, out): {} documents", staged.len());
    assert!(
        failures.is_empty(),
        "loop C (text, out): {} differences",
        failures.len()
    );
}

// --- The measured facts the comparison above is allowed to lean on ------------------------

/// **A Writer document cannot be empty.** The degenerate document comes back holding one empty
/// paragraph, which is why [`allowing_libreoffices_paragraph`] exists.
///
/// Its own test rather than a constant in a comment: the allowance is a hole in loop C, and a
/// hole that nothing checks is indistinguishable from a bug. If LibreOffice ever starts
/// preserving an empty body, this goes red and the allowance comes out.
#[test]
fn a_document_with_no_blocks_comes_back_holding_one() {
    if !oracle_ready("empty") {
        return;
    }
    let lab = Lab::new("empty");
    let bytes = grind_text::write_bytes(&Document::new(), Form::Flat).expect("writes");
    let path = lab.input("empty.fodt", &bytes);
    let back = converted(&lab.convert(std::slice::from_ref(&path)), &path);

    assert_eq!(back.blocks.len(), 1, "one paragraph, not none and not two");
    assert_eq!(back.blocks[0].kind, BlockKind::Paragraph);
    assert!(back.blocks[0].is_empty(), "and nothing in it");
}

/// **A `text:style-name` this build wrote does not survive, because this build declares no
/// styles.** That is loop C's one documented loosening for text, and the reason [`differences`]
/// compares structure and text but not style names.
///
/// Six cases, measured together because only the contrast makes the rule legible:
///
/// | What the document says | What comes back | |
/// |---|---|---|
/// | A name LibreOffice itself defines (`Quotations`) | `Quotations` | kept |
/// | `office:styles`, with a property | `NamedWith` | kept |
/// | `office:styles`, with no properties at all | `NamedBare` | kept |
/// | `office:automatic-styles`, with a property | **`P1`** | formatting kept, *name* renumbered |
/// | `office:automatic-styles`, with no properties | `Standard` | dropped |
/// | Declared nowhere | `Standard` | dropped |
///
/// So the rule is not "LibreOffice mangles style names". It is ODF's own distinction, applied
/// exactly: a **named** style is an identity and keeps its name; an **automatic** style is
/// anonymous direct formatting by definition, so its name is not identity and LibreOffice
/// renumbers it into its own sequence; a name that resolves to nothing is not formatting at all
/// and goes.
///
/// What that costs *us* is the last row. This writer is minimal by intent (R3) and emits no
/// `office:styles`, and the model carries a style's *name* but never its properties
/// (`doc/text-core.md`) — so there is nothing it could declare. A **regenerated** document
/// therefore refers to styles that are not there, and `grind text style p1 --style Mine` on a
/// document this build authored means nothing to LibreOffice.
///
/// R6 is what keeps that from mattering in the common case: a document *read from a file*
/// splices, so its own `office:styles` is still in the bytes and its names still resolve. The
/// gap is a document this build authored from nothing, and it is written down in
/// `doc/text-core.md` rather than papered over.
///
/// The day the writer learns to declare styles, the last row goes red and [`differences`] gains
/// a style comparison.
#[test]
fn a_style_name_this_build_wrote_does_not_survive() {
    if !oracle_ready("styles") {
        return;
    }
    // Hand-written rather than built through the writer, because the point is a document that
    // *does* declare its styles — which this writer cannot produce.
    const DECLARED: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:version="1.4" office:mimetype="application/vnd.oasis.opendocument.text">
 <office:styles>
  <style:style style:name="NamedWith" style:family="paragraph">
   <style:text-properties fo:font-weight="bold"/>
  </style:style>
  <style:style style:name="NamedBare" style:family="paragraph"/>
 </office:styles>
 <office:automatic-styles>
  <style:style style:name="AutoWith" style:family="paragraph">
   <style:text-properties fo:font-style="italic"/>
  </style:style>
  <style:style style:name="AutoBare" style:family="paragraph"/>
 </office:automatic-styles>
 <office:body>
  <office:text>
   <text:p text:style-name="Quotations">built in</text:p>
   <text:p text:style-name="NamedWith">named, with a property</text:p>
   <text:p text:style-name="NamedBare">named, with nothing in it</text:p>
   <text:p text:style-name="AutoWith">automatic, with a property</text:p>
   <text:p text:style-name="AutoBare">automatic, with nothing in it</text:p>
   <text:p text:style-name="NeverDeclared">not declared at all</text:p>
  </office:text>
 </office:body>
</office:document>
"#;

    let lab = Lab::new("styles");
    let path = lab.input("styles.fodt", DECLARED.as_bytes());
    let back = converted(&lab.convert(std::slice::from_ref(&path)), &path);

    let styles: Vec<_> = back.blocks.iter().map(|b| b.style.as_deref()).collect();
    assert_eq!(
        styles,
        vec![
            Some("Quotations"),
            Some("NamedWith"),
            Some("NamedBare"),
            // Renumbered, not dropped: the italic is still there, under LibreOffice's own name
            // for it. An automatic style is anonymous, so the name was never the identity.
            Some("P1"),
            // Nothing to keep, so nothing kept: `Standard` is LibreOffice's default paragraph
            // style, which is what a paragraph with no style of its own gets.
            Some("Standard"),
            Some("Standard"),
        ],
        "a named style keeps its name; an automatic one keeps only its formatting"
    );
    // And the text is untouched in every row — a dropped style loses formatting, never content.
    assert_eq!(
        back.blocks.iter().map(Block::text).collect::<Vec<_>>(),
        vec![
            "built in",
            "named, with a property",
            "named, with nothing in it",
            "automatic, with a property",
            "automatic, with nothing in it",
            "not declared at all",
        ]
    );
}

// --- direction "back": LibreOffice -> ours -> LibreOffice -> ours -------------------------

/// Corpus documents to push back out through the writer.
///
/// Longest first. Taking the alphabetically first twenty instead gives a sample of regression
/// fixtures — one-paragraph documents that reproduce one import bug each — and a few dozen
/// blocks to check the whole writer against.
fn sample(root: &Path) -> Vec<(PathBuf, Document)> {
    fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect(&path, out);
            } else if matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("odt" | "fodt")
            ) {
                out.push(path);
            }
        }
    }

    let mut files = Vec::new();
    collect(root, &mut files);
    files.sort();

    let mut eligible: Vec<_> = files
        .iter()
        // Unreadable or encrypted: loop A owns that verdict, not this one.
        .filter_map(|path| Some((path.clone(), grind_text::read_file(path).ok()?)))
        .filter(|(_, doc)| !doc.blocks.is_empty() && doc.blocks.len() <= MAX_COMPARED_BLOCKS)
        .collect();
    eligible.sort_by(|a, b| b.1.blocks.len().cmp(&a.1.blocks.len()).then(a.0.cmp(&b.0)));
    eligible.truncate(SAMPLE);
    eligible.sort_by(|a, b| a.0.cmp(&b.0));
    eligible
}

#[test]
fn libreoffice_documents_survive_our_writer() {
    if !oracle_ready("back") {
        return;
    }
    let root = PathBuf::from(
        std::env::var("GRIND_LO_CORPUS").unwrap_or_else(|_| DEFAULT_CHECKOUT.to_owned()),
    )
    .join(CORPUS);
    if !root.is_dir() {
        eprintln!(
            "skipping loop C (text, back): no LibreOffice corpus at {}",
            root.display()
        );
        return;
    }

    let files = sample(&root);
    assert!(
        !files.is_empty(),
        "no documents found in {}",
        root.display()
    );

    let lab = Lab::new("back");
    let staged: Vec<_> = files
        .iter()
        .enumerate()
        .map(|(i, (path, doc))| {
            let bytes = grind_text::write_bytes(doc, Form::Package).unwrap();
            // Numbered: corpus stems are not unique across `sw/qa`'s many directories.
            let staged = lab.input(&format!("{i:03}.odt"), &bytes);
            (path.clone(), doc, staged)
        })
        .collect();

    // A comparison finds nothing in a document that holds nothing, so the sample has to be
    // shown to have substance — otherwise a drifting filter turns this into twenty empty
    // documents agreeing with twenty empty documents, which passes forever.
    let blocks: usize = staged.iter().map(|(_, doc, _)| doc.blocks.len()).sum();
    eprintln!(
        "loop C (text, back): {} documents, {blocks} blocks",
        files.len()
    );
    assert!(
        blocks > 500,
        "sample holds only {blocks} blocks; it is not testing the writer"
    );

    let out = lab.convert(&staged.iter().map(|(_, _, p)| p.clone()).collect::<Vec<_>>());

    let mut failures = Vec::new();
    for (original, doc, path) in &staged {
        let label = original.file_name().unwrap().to_str().unwrap();
        failures.extend(differences(label, doc, &converted(&out, path)));
    }

    for f in failures.iter().take(30) {
        eprintln!("  {f}");
    }
    assert!(
        failures.is_empty(),
        "loop C (text, back): {} differences",
        failures.len()
    );
}
