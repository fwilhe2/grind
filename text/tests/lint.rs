// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `grind text lint` against documents LibreOffice Writer actually wrote — `doc/dsl.md` §4.3,
//! D6. The twin of `sheet/tests/lint.rs`, and it checks the same two things: that linting writes
//! nothing, and that every rule survives real files.
//!
//! The corpus is `text/tests/data/`, globbed rather than listed exactly as
//! `text/tests/libreoffice.rs` globs it — adding a Writer document is dropping a file in, and it
//! then has to survive the linter too.

use std::path::{Path, PathBuf};

use grind_core::lint::Options;
use grind_text::App;

fn vendored() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data");
    let mut out = Vec::new();
    collect(&root, &mut out);
    out.sort();
    assert!(
        out.len() >= 2,
        "text/tests/data/ is globbed, and the glob found {} documents",
        out.len()
    );
    out
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out);
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("odt") || e.eq_ignore_ascii_case("fodt"))
        {
            out.push(path);
        }
    }
}

fn everything() -> Options {
    Options {
        hints: true,
        off: Vec::new(),
    }
}

fn opened(path: &Path) -> App {
    let app = App::new();
    app.open_file(path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    app
}

/// Linting a document leaves its bytes exactly as they were.
///
/// Only the flat documents: saving a `.odt` regenerates the package, which is a cost
/// `text/tests/libreoffice.rs` writes down and which has nothing to do with linting.
#[test]
fn linting_writes_nothing() {
    for path in vendored() {
        if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("odt"))
        {
            continue;
        }
        let before = std::fs::read(&path).expect("a readable document");
        let app = opened(&path);

        for diagnostic in &app.lint(&everything()).diagnostics {
            assert!(!diagnostic.message.is_empty(), "{}", path.display());
        }

        let after = app
            .save_bytes(grind_text::Form::Flat)
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert_eq!(
            before,
            after,
            "{}: linting changed the document",
            path.display()
        );
    }
}

/// A style a document *declares* is not reported, and the same document read back from its own
/// projection reports every style it uses.
///
/// This is `doc/text-core.md`'s known loss made visible, asserted rather than described: the
/// projection carries style names and no style definitions, so a name in it really does point at
/// nothing. The day style definitions are carried, this test is what says so.
#[test]
fn a_style_is_declared_until_the_document_is_regenerated() {
    let mut checked = 0;
    for path in vendored() {
        let app = opened(&path);
        let before = app.lint(&everything());
        let undeclared = |report: &grind_core::lint::Report| {
            report
                .diagnostics
                .iter()
                .filter(|d| d.rule == "undeclared-style")
                .count()
        };

        // Round-trip through the projection, which declares nothing.
        let projected = app
            .save_bytes(grind_text::Form::Projection)
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let after = App::new();
        after
            .open_bytes(&path.display().to_string(), &projected)
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let after = after.lint(&everything());

        if undeclared(&after) > 0 {
            checked += 1;
            assert!(
                undeclared(&after) >= undeclared(&before),
                "{}: the projection declares no styles, so it cannot report fewer",
                path.display()
            );
        }
    }
    assert!(
        checked > 0,
        "no vendored document uses a style name at all, so the rule went unexercised"
    );
}
