// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The word processor's pages from the outside (`doc/pdf-export.md`): the page a document
//! states, how it breaks into pages, and the PDF and preview it exports to.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn grind(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_grind"))
        .args(args)
        .env_remove("GRIND_LOCALE")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("HOME")
        .output()
        .expect("the binary runs")
}

fn ok(args: &[&str]) -> String {
    let output = grind(args);
    assert!(
        output.status.success(),
        "{args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("utf-8")
}

fn writer(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../text/tests/data")
        .join(name)
        .to_string_lossy()
        .into_owned()
}

struct Sandbox(PathBuf);

impl Sandbox {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("grind-pdf-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("sandbox");
        Sandbox(dir)
    }

    fn path(&self, name: &str) -> String {
        self.0.join(name).to_string_lossy().into_owned()
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn info_names_the_page_a_text_document_states() {
    // Its first paragraph's style names the `HTML` master page, with no next one, so every page
    // is set on that one's 1 cm margins — what LibreOffice prints (`doc/odt-format.md` §5c, 17).
    let info = ok(&["info", &writer("numbered-list.fodt")]);
    assert!(
        info.contains("page\tA4\t210.01 × 297 mm\tmargins 10 10 20 10 mm"),
        "{info}"
    );

    let json = ok(&["--format", "json", "info", &writer("picture.fodt")]);
    assert!(json.contains("\"page\":{"), "{json}");
    assert!(json.contains("\"stated\":true"), "{json}");
    assert!(json.contains("\"width_mm\":215.9"), "{json}");
}

#[test]
fn a_document_stating_no_page_reports_the_a4_it_will_print_on() {
    let dir = Sandbox::new("no-page");
    let file = dir.path("new.fodt");
    ok(&["text", "new", &file]);
    let info = ok(&["info", &file]);
    assert!(
        info.contains("page\tA4\t210 × 297 mm\tmargins 20 20 20 20 mm\t(default)"),
        "{info}"
    );
}

#[test]
#[cfg(feature = "pdf")]
fn export_pdf_writes_a_pdf_and_says_what_it_did() {
    let dir = Sandbox::new("export");
    let out = dir.path("list.pdf");
    let said = ok(&["text", "export-pdf", &writer("numbered-list.fodt"), &out]);
    assert!(said.contains("1 page, A4."), "{said}");
    let bytes = std::fs::read(&out).expect("the PDF was written");
    assert!(bytes.starts_with(b"%PDF-"));
    // Twice is the same bytes, which is what lets a PDF live in a test.
    let again = dir.path("again.pdf");
    ok(&["text", "export-pdf", &writer("numbered-list.fodt"), &again]);
    assert_eq!(bytes, std::fs::read(&again).unwrap());
}

#[test]
#[cfg(feature = "pdf")]
fn export_pdf_prints_on_the_paper_asked_for_and_only_iso_paper() {
    let dir = Sandbox::new("paper");
    let out = dir.path("a5.pdf");
    let said = ok(&[
        "text",
        "export-pdf",
        &writer("picture.fodt"),
        &out,
        "--paper",
        "a5",
    ]);
    assert!(said.contains("A5."), "{said}");
    let refused = grind(&[
        "text",
        "export-pdf",
        &writer("picture.fodt"),
        &out,
        "--paper",
        "letter",
    ]);
    assert!(!refused.status.success());
    let err = String::from_utf8_lossy(&refused.stderr);
    assert!(err.contains("a4"), "names what it does take: {err}");
}

#[test]
#[cfg(feature = "pdf")]
fn export_pdf_in_json_is_the_report() {
    let dir = Sandbox::new("json");
    let out = dir.path("x.pdf");
    let json = ok(&[
        "--format",
        "json",
        "text",
        "export-pdf",
        &writer("numbered-list.fodt"),
        &out,
    ]);
    assert!(json.contains("\"pages\":1"), "{json}");
    assert!(json.contains("\"missing_glyphs\":0"), "{json}");
    assert!(json.contains("\"summary\":\"1 page, A4.\""), "{json}");
}

/// Each page as `n<TAB>first<TAB>end`, addresses a person can hand straight back to
/// `grind text get` — and each page ends where the next begins.
#[test]
#[cfg(feature = "pdf")]
fn pages_lists_where_each_page_begins_and_ends() {
    let dir = Sandbox::new("pages");
    let file = dir.path("long.fodt");
    ok(&["text", "new", &file]);
    let paragraph = "All work and no play makes a dull document. ".repeat(80);
    for _ in 0..4 {
        ok(&["text", "insert", &file, "--text", &paragraph]);
    }
    let pages = ok(&["text", "pages", &file]);
    let rows: Vec<Vec<&str>> = pages.lines().map(|l| l.split('\t').collect()).collect();
    assert!(rows.len() >= 2, "{pages}");
    assert_eq!(rows[0][0], "1");
    assert_eq!(rows[0][1], "p1+0");
    for pair in rows.windows(2) {
        assert_eq!(pair[0][2], pair[1][1], "{pages}");
    }
    // A smaller page holds less, so the same document takes more of them.
    let a6 = ok(&["text", "pages", &file, "--paper", "a6"]);
    assert!(a6.lines().count() > rows.len(), "{a6}");
}

/// A page as a PNG at a resolution — the preview, from the command line (rule 4).
#[test]
#[cfg(feature = "pdf")]
fn preview_draws_one_page_as_a_png() {
    let dir = Sandbox::new("preview");
    let out = dir.path("p1.png");
    ok(&[
        "text",
        "preview",
        &writer("numbered-list.fodt"),
        &out,
        "--dpi",
        "36",
    ]);
    let png = std::fs::read(&out).expect("the PNG was written");
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
    // The IHDR's width and height: half of A4's points at 36 dpi.
    let be = |at: usize| u32::from_be_bytes(png[at..at + 4].try_into().unwrap());
    assert_eq!((be(16), be(20)), (297, 420));

    let past = grind(&[
        "text",
        "preview",
        &writer("numbered-list.fodt"),
        &out,
        "--page",
        "2",
    ]);
    assert!(!past.status.success());
    assert!(
        String::from_utf8_lossy(&past.stderr).contains("1 page"),
        "says how many there are"
    );
}

/// `--bundled-fonts` sets everything in the faces compiled in, so the same document gives the
/// same PDF on any machine — and a family only the machine might have is reported as substituted.
#[test]
#[cfg(feature = "pdf")]
fn bundled_fonts_ignore_the_machine() {
    let dir = Sandbox::new("bundled");
    let file = dir.path("g.fodt");
    ok(&["text", "new", &file]);
    ok(&["text", "insert", &file, "--text", "Set in a face"]);
    ok(&[
        "text",
        "format",
        &file,
        "p2",
        "--font",
        "Grind Test Face That Nobody Has",
    ]);
    let out = dir.path("g.pdf");
    let said = ok(&["text", "export-pdf", &file, &out, "--bundled-fonts"]);
    assert!(
        said.contains("\u{201c}Grind Test Face That Nobody Has\u{201d} set in Liberation Serif."),
        "{said}"
    );
}
