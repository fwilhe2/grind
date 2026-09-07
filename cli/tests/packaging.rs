// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! **Every binary the suite has is packaged**, checked the way R11 is checked in
//! `build/tests/manifest.rs`: read the manifests and the workflow, and fail the build.
//!
//! The bug this exists to prevent already happened once. `grind-text-gtk` was built by
//! `gtk.yml`, tested, linted and documented, and the packaging workflow produced packages for
//! the other three binaries — so the word processor was a shell somebody could run from a
//! checkout and not one anybody could install, and nothing said so. A shell with no `.deb` is
//! invisible in exactly the way `doc/plan.md` rule 4 says a capability with no CLI verb is, and
//! for the same reason: the omission is silent.
//!
//! The workflow read here is `artifacts.yml`, which absorbed the former `packaging.yml` when
//! the release builds were consolidated: one `cargo build --release` per platform, with the
//! packages, the sizes and the breakdowns as later steps of the same job. What that changed
//! for this file is that `cargo deb` is now invoked with `--no-build` — it packages the
//! binary that job already built rather than building a second one — so the assertion below
//! looks for `cargo deb -p <package>` as a *prefix*, and the flag order in the workflow is
//! `-p` first for exactly that reason.
//!
//! **Why this lives in `cli/tests/`.** The rule is suite-level rather than any one app's, and
//! the two GTK crates are deliberately outside `cargo build --workspace`'s path (they need
//! `libgtk-4-dev`), so a guard living in `ui_text_gtk/tests/` would not run in the job that
//! gates every push. `cli/tests/parity.rs` is the other suite-level ratchet and reads other
//! crates' sources for the same reason.
//!
//! Everything is `include_str!`, so it is read at *compile* time and cannot pass by looking in
//! the wrong place at runtime.

const WORKSPACE: &str = include_str!("../../Cargo.toml");
const WORKFLOW: &str = include_str!("../../.github/workflows/artifacts.yml");

/// A crate that ships a binary: its directory, its package name, the binary that lands in
/// `/usr/bin`, and its manifest.
struct Packaged {
    dir: &'static str,
    package: &'static str,
    binary: &'static str,
    manifest: &'static str,
}

const PACKAGED: [Packaged; 4] = [
    Packaged {
        dir: "cli",
        package: "grind-cli",
        binary: "grind",
        manifest: include_str!("../Cargo.toml"),
    },
    Packaged {
        dir: "ui_sheet_gtk",
        package: "grind-sheet-gtk",
        binary: "grind-sheet-gtk",
        manifest: include_str!("../../ui_sheet_gtk/Cargo.toml"),
    },
    Packaged {
        dir: "ui_text_gtk",
        package: "grind-text-gtk",
        binary: "grind-text-gtk",
        manifest: include_str!("../../ui_text_gtk/Cargo.toml"),
    },
    Packaged {
        dir: "ui_tui",
        package: "grind-tui",
        binary: "grind-tui",
        manifest: include_str!("../../ui_tui/Cargo.toml"),
    },
];

/// The members that ship no binary, each with the reason. A crate here is *not* a package that
/// was forgotten.
const UNPACKAGED: [(&str, &str); 6] = [
    ("core", "a library"),
    ("sheet", "a library"),
    ("text", "a library"),
    ("build", "a library"),
    (
        "ui_web",
        "a wasm bundle served as files, not installed from a repository",
    ),
    // Not an oversight and not deferred work: a `.deb` or an `.rpm` of a `.exe` would install
    // something no Linux machine can run. `artifacts.yml`'s `windows` job builds it on
    // `windows-latest` and uploads it, which is that platform's equivalent of the packaging
    // steps here. What Windows *packaging* means — a portable executable, an installer, file
    // associations through ProgIDs — is W8's question in `doc/windows-shell.md`, and the day it
    // is answered the answer belongs in a guard of its own rather than in this one.
    (
        "ui_win32",
        "a Windows executable; artifacts.yml builds and uploads it, and a .deb would be unrunnable",
    ),
];

/// A GUI shell also ships the three files a desktop needs to know it exists, under its own
/// reverse-DNS app ID (`doc/suite.md`, "One binary per document type — for GTK only").
struct Desktop {
    package: &'static str,
    binary: &'static str,
    id: &'static str,
    entry: &'static str,
    /// The media types this application claims, both forms of its one document type.
    mime: [&'static str; 2],
}

const DESKTOPS: [Desktop; 2] = [
    Desktop {
        package: "grind-sheet-gtk",
        binary: "grind-sheet-gtk",
        id: "io.github.fwilhe2.Sheet",
        entry: include_str!("../../ui_sheet_gtk/data/io.github.fwilhe2.Sheet.desktop"),
        mime: [
            "application/vnd.oasis.opendocument.spreadsheet",
            "application/vnd.oasis.opendocument.spreadsheet-flat-xml",
        ],
    },
    Desktop {
        package: "grind-text-gtk",
        binary: "grind-text-gtk",
        id: "io.github.fwilhe2.Text",
        entry: include_str!("../../ui_text_gtk/data/io.github.fwilhe2.Text.desktop"),
        mime: [
            "application/vnd.oasis.opendocument.text",
            "application/vnd.oasis.opendocument.text-flat-xml",
        ],
    },
];

/// The three files a GUI shell installs, pulled in at compile time so a deleted one is a build
/// failure rather than a package that ships an `Icon=` pointing at nothing. Only their
/// existence is the assertion; `desktop-file-validate` and `appstreamcli validate` are what
/// check their contents, and neither is a thing to make `cargo test` depend on.
const INSTALLED: [&str; 4] = [
    include_str!("../../ui_sheet_gtk/data/io.github.fwilhe2.Sheet.metainfo.xml"),
    include_str!("../../ui_sheet_gtk/data/icons/hicolor/scalable/apps/io.github.fwilhe2.Sheet.svg"),
    include_str!("../../ui_text_gtk/data/io.github.fwilhe2.Text.metainfo.xml"),
    include_str!("../../ui_text_gtk/data/icons/hicolor/scalable/apps/io.github.fwilhe2.Text.svg"),
];

/// The Linux job's release build, which is the one line naming every packaged binary at once.
///
/// `artifacts.yml` has more than one `cargo build --release` — the `windows` job has its own,
/// for a binary that is deliberately not packaged ([`UNPACKAGED`]) — so this cannot simply take
/// the first one and must say which it means. Matching on `-p grind-cli` is that: the CLI is in
/// every packaged set and in no other job's build line.
fn release_line() -> &'static str {
    let mut lines = WORKFLOW
        .lines()
        .filter(|line| line.contains("cargo build --release") && line.contains("-p grind-cli"));
    let line = lines
        .next()
        .expect("artifacts.yml's linux job builds the binaries before packaging them");
    assert!(
        lines.next().is_none(),
        "more than one release build in artifacts.yml names grind-cli, so this test cannot \
         tell which one the packaging steps read"
    );
    line
}

/// Both package formats, for every binary. `cargo deb` and `cargo generate-rpm` read different
/// blocks and are invoked differently — `-p` is a *package* name for one and a *path* for the
/// other — so each is asserted in the spelling its own tool takes.
#[test]
fn every_binary_is_built_by_the_packaging_workflow() {
    for crate_ in PACKAGED {
        let (dir, package) = (crate_.dir, crate_.package);
        assert!(
            crate_.manifest.contains("[package.metadata.deb]"),
            "{dir}/Cargo.toml has no [package.metadata.deb], so `cargo deb -p {package}` has \
             nothing to read"
        );
        assert!(
            crate_.manifest.contains("[package.metadata.generate-rpm]"),
            "{dir}/Cargo.toml has no [package.metadata.generate-rpm]"
        );
        assert!(
            crate_
                .manifest
                .contains(&format!("target/release/{}", crate_.binary)),
            "{dir}/Cargo.toml's assets do not name target/release/{}, so the package would \
             ship no binary",
            crate_.binary
        );

        assert!(
            WORKFLOW.contains(&format!("cargo deb -p {package}")),
            "artifacts.yml does not run `cargo deb -p {package}`, so {package} builds \
             everywhere except where somebody could install it"
        );
        assert!(
            WORKFLOW.contains(&format!("cargo generate-rpm -p {dir}")),
            "artifacts.yml does not run `cargo generate-rpm -p {dir}` (that tool takes the \
             directory, not the package name)"
        );
        assert!(
            release_line().contains(&format!("-p {package}")),
            "artifacts.yml's linux job does not build {package} in release, so neither \
             `cargo deb --no-build` nor `cargo generate-rpm -p {dir}` would find a binary. \
             That one line is what both packagers read, which is why it exists at all"
        );
    }
}

/// The vacuity guard, [`build/tests/manifest.rs`]'s second test in this file's terms: a new
/// shell added to the workspace and to neither list here would leave this file passing while
/// checking one crate fewer, which is how a ratchet quietly stops.
#[test]
fn every_member_of_the_workspace_is_accounted_for() {
    let members = WORKSPACE
        .split_once("members = [")
        .expect("the workspace lists its members")
        .1
        .split_once(']')
        .expect("that list ends")
        .0;
    let members: Vec<&str> = members
        .split(',')
        .map(|entry| entry.trim().trim_matches('"'))
        .filter(|entry| !entry.is_empty())
        .collect();
    assert!(
        members.len() >= 9,
        "the workspace has more than a few crates"
    );

    for member in members {
        let packaged = PACKAGED.iter().any(|crate_| crate_.dir == member);
        let exempt = UNPACKAGED.iter().any(|(dir, _)| *dir == member);
        assert!(
            packaged || exempt,
            "{member} is in the workspace and in neither list here. If it ships a binary, add \
             it to PACKAGED and to artifacts.yml; if it does not, add it to UNPACKAGED with \
             the reason."
        );
    }
}

/// What a desktop reads. `Exec=` naming something other than the installed binary is a
/// packaging bug that no build catches and no test would have caught: `Exec=sheet-gtk` sat in
/// the spreadsheet's entry from M9 until the word processor's was written beside it.
#[test]
fn each_gui_shell_declares_itself_to_the_desktop() {
    for app in DESKTOPS {
        let (id, package) = (app.id, app.package);
        assert!(
            app.entry.contains(&format!("Exec={} %f", app.binary)),
            "{id}.desktop's Exec= does not run `{} %f`, which is the binary the package puts \
             in /usr/bin",
            app.binary
        );
        assert!(
            app.entry.contains(&format!("Icon={id}")),
            "{id}.desktop's Icon= is not the app ID, so the installed icon is not found"
        );
        for media in app.mime {
            assert!(
                app.entry.contains(media),
                "{id}.desktop does not claim {media}, so double-clicking one of those files \
                 does not open {package} (doc/suite.md, \"Mime types\")"
            );
        }
        // The entry names an icon and an AppStream component; the package has to actually
        // carry them, or an installed application has no icon and does not appear in GNOME
        // Software. That the files exist at all is [`INSTALLED`], at compile time.
        let manifest = PACKAGED
            .iter()
            .find(|crate_| crate_.package == package)
            .expect("a GUI shell is a packaged crate");
        for asset in [
            format!("data/{id}.desktop"),
            format!("data/{id}.metainfo.xml"),
            format!("data/icons/hicolor/scalable/apps/{id}.svg"),
        ] {
            assert!(
                manifest.manifest.contains(&asset),
                "{}/Cargo.toml's packaging assets do not include {asset}",
                manifest.dir
            );
        }
    }

    for file in INSTALLED {
        assert!(!file.trim().is_empty(), "an installed data file is empty");
    }
}
