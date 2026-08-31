// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `json(…)` — the mapping, and the four walls (`build/src/data.rs`).
//!
//! The walls get more tests than the feature, which is the right proportion: reading a file is
//! two lines and refusing to read the wrong one is the whole design. Every refusal below is a
//! path somebody *will* write by accident — `../` because the data sits one directory up, an
//! absolute path because it worked in the shell — so each error has to say which wall it hit
//! rather than "no such file".

use std::path::{Path, PathBuf};
use std::rc::Rc;

use grind_build::{Artifact, Directory, build, build_with};
use grind_sheet::{App, CellValue};

/// A directory that removes itself, with the data files a test needs in it.
struct Sandbox(PathBuf);

impl Sandbox {
    fn new(name: &str) -> Sandbox {
        let dir = std::env::temp_dir().join(format!(
            "grind-data-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a sandbox");
        Sandbox(dir)
    }

    fn write(&self, name: &str, text: &str) -> PathBuf {
        let path = self.0.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("a directory");
        }
        std::fs::write(&path, text).expect("a file");
        path
    }

    /// Run a script with this directory as its data.
    fn build(&self, source: &str) -> Result<App, String> {
        let data = Directory::new(&self.0).expect("a data directory");
        match build_with(source, "model.rhai", Rc::new(data)) {
            Ok(Artifact::Spreadsheet(app)) => Ok(app),
            Ok(Artifact::Text(_)) => panic!("that script built a text document"),
            Err(e) => Err(e.to_string()),
        }
    }

    /// The message a script failed with. `Result::expect_err` wants `Debug` on the other arm,
    /// and neither an `App` nor a document has one — nor should they.
    fn fails(&self, source: &str) -> String {
        match self.build(source) {
            Err(message) => message,
            Ok(_) => panic!("that script was supposed to fail"),
        }
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn at(app: &App, address: &str) -> CellValue {
    let reference = grind_sheet::a1::parse(address).expect("an address");
    let (sheet, pos, _) = grind_sheet::a1::resolve(app, &reference).expect("a place");
    app.get(sheet, pos).expect("a cell")
}

/// The feature, in the shape it was asked for: the numbers in a file, the shape in the script.
#[test]
fn a_script_builds_a_document_out_of_a_data_file() {
    let dir = Sandbox::new("rows");
    dir.write(
        "prices.json",
        r#"[{"item": "Coffee", "price": 4.5}, {"item": "Tea", "price": 3}]"#,
    );
    let app = dir
        .build(
            r#"
            let s = sheet("Prices");
            s.push(row(["Item", "Price"]).bold());
            for line in json("prices.json") { s.push([line.item, line.price]); }
            s.push(row(["Total", sum_above()]).bold());
            s
            "#,
        )
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(at(&app, "A2"), CellValue::Text("Coffee".into()));
    assert_eq!(at(&app, "B2"), CellValue::Number(4.5));
    assert_eq!(at(&app, "B4"), CellValue::Number(7.5));
}

/// The mapping, all of it, in one script — `doc/generator-spec.md` §3.5's paragraph, executed.
#[test]
fn json_arrives_as_the_values_a_script_already_has() {
    let dir = Sandbox::new("types");
    dir.write(
        "data.json",
        r#"{"n": 7, "big": 2.5, "s": "text", "yes": true, "nothing": null, "list": [1, 2]}"#,
    );
    let app = dir
        .build(
            r#"
            let d = json("data.json");
            let s = sheet("S");
            s.push([d.n, d.big, d.s, d.yes, d.nothing, d.list.len]);
            // An integer stays an integer, which is what makes this work at all.
            s.set("A2", type_of(d.n));
            s.set("B2", type_of(d.big));
            s
            "#,
        )
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(at(&app, "A1"), CellValue::Number(7.0));
    assert_eq!(at(&app, "B1"), CellValue::Number(2.5));
    assert_eq!(at(&app, "C1"), CellValue::Text("text".into()));
    assert_eq!(at(&app, "D1"), CellValue::Bool(true));
    assert_eq!(at(&app, "E1"), CellValue::Empty, "null is the empty cell");
    assert_eq!(at(&app, "F1"), CellValue::Number(2.0));
    assert_eq!(at(&app, "A2"), CellValue::Text("i64".into()));
    assert_eq!(at(&app, "B2"), CellValue::Text("f64".into()));
}

/// Wall 2, the one somebody hits by accident: the data is one directory up, so they write `..`.
/// The message has to say that rather than "no such file", or they will try harder.
#[test]
fn a_path_that_leaves_the_directory_is_refused_by_name() {
    let dir = Sandbox::new("escape");
    dir.write("inside.json", "[]");
    for name in ["../outside.json", "../../etc/hosts", "sub/../../out.json"] {
        let message = dir.fails(&format!(r#"json("{name}"); sheet("S")"#));
        assert!(message.contains("`..` leaves it"), "{name}: {message}");
    }
}

#[test]
fn an_absolute_path_is_refused() {
    let dir = Sandbox::new("absolute");
    let message = dir.fails(r#"json("/etc/hosts"); sheet("S")"#);
    assert!(message.contains("never absolutely"), "{message}");
}

/// The second half of wall 2, and the reason the first half is not enough: a name with nothing
/// suspicious in it, pointing out of the directory. Only the filesystem knows.
#[cfg(unix)]
#[test]
fn a_symlink_pointing_out_of_the_directory_is_refused() {
    let dir = Sandbox::new("symlink");
    let outside = Sandbox::new("symlink-target");
    outside.write("secret.json", r#"{"secret": true}"#);
    std::os::unix::fs::symlink(outside.0.join("secret.json"), dir.0.join("link.json"))
        .expect("a symlink");

    let message = dir.fails(r#"json("link.json"); sheet("S")"#);
    assert!(message.contains("outside the data directory"), "{message}");
}

/// A file inside a subdirectory is ordinary — the wall is the *root*, not the shape of the name.
#[test]
fn a_file_in_a_subdirectory_is_fine() {
    let dir = Sandbox::new("subdir");
    dir.write("data/regions.json", r#"["North", "South"]"#);
    let app = dir
        .build(
            r#"
            let s = sheet("S");
            for r in json("data/regions.json") { s.push([r]); }
            s
            "#,
        )
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(at(&app, "A2"), CellValue::Text("South".into()));
}

/// The default is no data at all, and the error names the flag rather than the file — a script
/// asking for data it cannot have should not read as a missing file.
#[test]
fn without_a_data_directory_the_error_says_so() {
    let message = match build(r#"json("prices.json"); sheet("S")"#, "model.rhai") {
        Err(error) => error.to_string(),
        Ok(_) => panic!("there is nowhere to read from"),
    };
    assert!(message.contains("no data directory"), "{message}");
    assert!(message.contains("--data"), "{message}");
    // And it has the position of the line that asked, like every other error.
    assert!(message.starts_with("model.rhai:1:"), "{message}");
}

#[test]
fn a_file_that_is_not_json_fails_where_it_was_read() {
    let dir = Sandbox::new("broken");
    dir.write("bad.json", "{oops");
    let message = dir.fails(r#"json("bad.json"); sheet("S")"#);
    assert!(message.contains("bad.json"), "{message}");
    assert!(message.starts_with("model.rhai:1:"), "{message}");
}

#[test]
fn a_missing_file_is_an_error_naming_it() {
    let dir = Sandbox::new("missing");
    let message = dir.fails(r#"json("nope.json"); sheet("S")"#);
    assert!(message.contains("nope.json"), "{message}");
}

/// The cache, from the outside: reading in a loop is one read. Asserted by *deleting* the file
/// after the first read, which nothing but a cache survives.
#[test]
fn reading_the_same_file_twice_does_not_read_it_twice() {
    let dir = Sandbox::new("cache");
    let path = dir.write("once.json", "[1, 2, 3]");
    let data = Directory::new(&dir.0).expect("a data directory");
    let data = Rc::new(data);

    let source = r#"
        let first = json("once.json").len;
        // Whatever happens to the file now, the second read is the first read's answer.
        let second = json("once.json").len;
        let s = sheet("S");
        s.push([first, second]);
        s
    "#;
    // The script reads it once before the delete and once after; both must answer.
    std::fs::remove_file(&path).expect("gone");
    let failed = build_with(source, "model.rhai", data.clone());
    assert!(failed.is_err(), "the first read of a missing file fails");

    dir.write("once.json", "[1, 2, 3]");
    let Ok(Artifact::Spreadsheet(app)) = build_with(source, "model.rhai", data) else {
        panic!("it builds");
    };
    assert_eq!(at(&app, "A1"), CellValue::Number(3.0));
    assert_eq!(at(&app, "B1"), CellValue::Number(3.0));
}

/// What a build read, in order — the inputs, which is what somebody wants when the output
/// changed and the script did not.
#[test]
fn a_directory_remembers_what_was_read() {
    let dir = Sandbox::new("inputs");
    dir.write("a.json", "[1]");
    dir.write("b.json", "[2]");
    let data = Rc::new(Directory::new(&dir.0).expect("a data directory"));
    let source = r#"
        json("b.json");
        json("a.json");
        json("b.json");
        sheet("S")
    "#;
    build_with(source, "model.rhai", data.clone()).expect("it builds");

    let read: Vec<String> = data
        .inputs()
        .iter()
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect();
    assert_eq!(read, vec!["b.json", "a.json"], "in order, without repeats");
}

/// A data directory that is not one is an error before any script runs.
#[test]
fn a_data_directory_has_to_be_a_directory() {
    let dir = Sandbox::new("notadir");
    let file = dir.write("data.json", "[]");
    assert!(Directory::new(&file).is_err());
    assert!(Directory::new(Path::new("/no/such/place")).is_err());
}
