// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! R8's guard: **no document type's vocabulary appears in the shared crate.**
//!
//! A shared crate with no mechanical guard becomes a dumping ground within two milestones —
//! that is every shared-core project's failure mode, and `doc/suite.md` names it as the risk
//! this test exists to answer. So the rule is checked the way `doc/cli-parity.md` and
//! `doc/small-group.md` are checked: a test that reads the source and fails the build.
//!
//! Two halves, because the rule has two ways to break:
//!
//! 1. **By dependency** — `grind-core` naming a document-type crate in its manifest. That would
//!    invert the whole layering and is the loud failure.
//! 2. **By vocabulary** — a `table:table-cell` or a `text:p` creeping into these sources. That
//!    is the quiet one, and it is how the dumping ground actually starts.

/// Read at *compile* time, so this cannot pass by looking in the wrong place at runtime.
const MANIFEST: &str = include_str!("../Cargo.toml");

/// Every source file of this crate that the guard covers.
///
/// Listed rather than walked, for the same reason the parity ratchet lists its inputs: a walk
/// that finds nothing passes vacuously, and a vacuous ratchet has quietly stopped ratcheting.
/// A new module here is one line, and the count assertion below is what notices a missing one.
const SOURCES: [(&str, &str); 11] = [
    ("lib.rs", include_str!("../src/lib.rs")),
    ("build_info.rs", include_str!("../src/build_info.rs")),
    ("kind.rs", include_str!("../src/kind.rs")),
    ("locale.rs", include_str!("../src/locale.rs")),
    ("observer.rs", include_str!("../src/observer.rs")),
    ("style.rs", include_str!("../src/style.rs")),
    ("odf/mod.rs", include_str!("../src/odf/mod.rs")),
    ("odf/context.rs", include_str!("../src/odf/context.rs")),
    ("odf/names.rs", include_str!("../src/odf/names.rs")),
    ("odf/package.rs", include_str!("../src/odf/package.rs")),
    ("odf/xml.rs", include_str!("../src/odf/xml.rs")),
];

/// Source with its line comments cut away.
///
/// The guard checks **code, not prose**. `odf/context.rs` explains the element-context stack
/// by pointing at `<table:table-cell/>`, and that comment is worth more than the rule it would
/// otherwise trip: R8 is about what this crate knows how to *do*, not about which examples its
/// documentation reaches for.
///
/// A tripwire rather than a parser — it cuts at the first `//` on a line and does not know
/// about string literals containing one. Nothing in this crate has such a literal outside a
/// comment, and the cost of being wrong is a false pass on one line, not a false failure.
fn code(source: &str) -> String {
    source
        .lines()
        .map(|line| line.split_once("//").map_or(line, |(before, _)| before))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_shared_crate_depends_on_no_document_type() {
    for crate_name in ["grind-sheet", "grind-text", "grind-slides"] {
        assert!(
            !MANIFEST.contains(crate_name),
            "grind-core's Cargo.toml names `{crate_name}`. The layering is one way: a \
             document type builds on the shared crate, never the reverse (R8)."
        );
    }
}

#[test]
fn no_document_body_element_is_spelled_in_the_shared_crate() {
    // Element names that only ever appear inside one document type's body. A reader or writer
    // doing a document type's work has to spell one of these somewhere.
    const BODY: [&str; 9] = [
        "table:table-cell",
        "table:table-row",
        "table:table-column",
        "table:named-expressions",
        "text:list-item",
        "text:span",
        "text:h",
        "text:p",
        "draw:frame",
    ];

    for (file, source) in SOURCES {
        for element in BODY {
            assert!(
                !code(source).contains(element),
                "core/src/{file} spells `{element}`, which belongs to one document type. \
                 R8: the shared crate knows about packaging, namespaces, the context stack and \
                 style primitives, and nothing about what is in a document body."
            );
        }
    }
}

#[test]
fn no_body_namespace_is_dispatched_on_in_the_shared_crate() {
    // The check that would actually catch a reader growing here. Element dispatch is on a
    // resolved `(namespace, local-name)` pair (§8.1), so a context handling spreadsheet or
    // text content has to name `Ns::Table` or `Ns::Text` — which is why *these* identifiers
    // are the tripwire and the string `table:table-cell` never would have been. `Ns::Office`
    // is deliberately absent: the office namespace carries the document envelope, which is
    // this crate's own business.
    for (file, source) in SOURCES {
        // `odf/names.rs` defines the enum, so it necessarily names every variant. That is the
        // one exemption, it is this narrow, and it is checked below rather than assumed.
        if file == "odf/names.rs" {
            continue;
        }
        for ns in ["Ns::Table", "Ns::Text", "Ns::Draw"] {
            assert!(
                !code(source).contains(ns),
                "core/src/{file} dispatches on `{ns}`. R8: reading a document *body* is the \
                 job of grind-sheet or grind-text, not of the crate they share."
            );
        }
    }
}

#[test]
fn the_one_exemption_is_still_earning_it() {
    // An exemption nobody rechecks is the same hole as no guard: the file that needed it gets
    // refactored, the entry stays, and whatever takes that name is excused for free.
    let (_, names) = SOURCES
        .iter()
        .find(|(name, _)| *name == "odf/names.rs")
        .expect("odf/names.rs is one of this crate's sources");
    assert!(
        code(names).contains("Ns::Table"),
        "odf/names.rs no longer defines the namespace variants it is excused for naming — \
         drop the exemption in the test above rather than leaving a standing excuse."
    );
}

#[test]
fn the_guard_covers_every_source_file() {
    // The vacuity check. If a module is added to `src/` and not to `SOURCES`, the two tests
    // above keep passing while covering less — so count the files on disk against the list.
    fn count(dir: &std::path::Path) -> usize {
        std::fs::read_dir(dir)
            .expect("the crate's own source directory is readable")
            .flatten()
            .map(|entry| {
                let path = entry.path();
                if path.is_dir() {
                    count(&path)
                } else {
                    usize::from(path.extension().is_some_and(|e| e == "rs"))
                }
            })
            .sum()
    }

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    assert_eq!(
        count(&root),
        SOURCES.len(),
        "core/src holds a different number of .rs files than tests/generic.rs guards — \
         add the new module to SOURCES so R8 keeps covering the whole crate."
    );
}
