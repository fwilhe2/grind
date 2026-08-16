// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The CLI, exercised by running it. Nothing here links the core — the point is to test the
//! binary a user or an agent actually invokes, including its exit codes and which stream
//! each kind of output lands on.
//!
//! No test-harness crates: a sandbox directory and `Command` are enough, and a dev
//! dependency that only saves a few lines is one more thing to keep current.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A temporary directory that removes itself, so a failing test leaves no litter.
struct Sandbox(PathBuf);

impl Sandbox {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "sheet-cli-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("sandbox");
        Sandbox(dir)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn sheet(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_sheet"))
        .args(args)
        .output()
        .expect("the binary runs")
}

/// Run, require success, return stdout.
fn ok(args: &[&str]) -> String {
    let output = sheet(args);
    assert!(
        output.status.success(),
        "{args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("utf-8")
}

fn s(path: &Path) -> String {
    path.display().to_string()
}

/// One field of a JSON object, without a JSON parser — enough to assert on the two fields
/// that matter and nothing more.
fn field<'a>(json: &'a str, name: &str) -> &'a str {
    let key = format!("\"{name}\":");
    let rest = json.split(&key).nth(1).unwrap_or_else(|| {
        panic!("no field {name} in {json}");
    });
    let end = rest.find([',', '}']).expect("a field ends somewhere");
    rest[..end].trim_matches('"')
}

#[test]
fn a_new_document_has_one_empty_sheet() {
    let dir = Sandbox::new("new");
    let file = dir.path("book.ods");
    ok(&["new", &s(&file)]);
    assert!(file.exists());

    let json = ok(&["--format", "json", "info", &s(&file)]);
    assert_eq!(field(&json, "name"), "Sheet1");
    assert_eq!(field(&json, "rows"), "0");
    assert_eq!(field(&json, "cols"), "0");
}

#[test]
fn new_refuses_to_overwrite_without_force() {
    let dir = Sandbox::new("force");
    let file = dir.path("book.ods");
    ok(&["new", &s(&file)]);
    ok(&["set", &s(&file), "A1", "7"]);

    let output = sheet(&["new", &s(&file)]);
    assert!(!output.status.success());
    assert_eq!(ok(&["get", &s(&file), "A1"]).trim(), "7", "left alone");

    ok(&["new", &s(&file), "--force"]);
    assert_eq!(ok(&["get", &s(&file), "A1"]).trim(), "");
}

#[test]
fn values_and_a_formula_survive_being_written_and_read_back() {
    let dir = Sandbox::new("roundtrip");
    let file = dir.path("book.ods");
    ok(&["new", &s(&file)]);
    ok(&["set", &s(&file), "A1", "1"]);
    ok(&["set", &s(&file), "A2", "2"]);
    ok(&["set", &s(&file), "A3", "=SUM([.A1:.A2])"]);

    assert_eq!(ok(&["get", &s(&file), "A3"]).trim(), "3");
    assert_eq!(
        ok(&["get", &s(&file), "A3", "--formula"]).trim(),
        "=SUM([.A1:.A2])",
        "formula text is stored verbatim, not rewritten"
    );
}

#[test]
fn a_value_is_typed_by_what_it_looks_like_unless_text_is_forced() {
    let dir = Sandbox::new("types");
    let file = dir.path("book.ods");
    ok(&["new", &s(&file)]);
    ok(&["set", &s(&file), "A1", "-1.5"]);
    ok(&["set", &s(&file), "A2", "TRUE"]);
    ok(&["set", &s(&file), "A3", "hello"]);
    ok(&["set", &s(&file), "A4", "123", "--text"]);
    ok(&["set", &s(&file), "A5", "=SUM([.A1:.A1])", "--text"]);

    let json = ok(&["--format", "json", "view", &s(&file), "A1:A5"]);
    let types: Vec<&str> = json
        .split("\"type\":\"")
        .skip(1)
        .map(|t| &t[..t.find('"').unwrap()])
        .collect();
    assert_eq!(
        types,
        vec!["float", "boolean", "string", "string", "string"]
    );
    // --text on something starting with '=' stores the text, never a formula.
    assert_eq!(ok(&["get", &s(&file), "A5", "--formula"]).trim(), "");
}

#[test]
fn view_prints_a_tab_separated_grid() {
    let dir = Sandbox::new("view");
    let file = dir.path("book.ods");
    ok(&["new", &s(&file)]);
    ok(&["set", &s(&file), "A1", "1"]);
    ok(&["set", &s(&file), "B1", "2"]);
    ok(&["set", &s(&file), "A2", "3"]);

    assert_eq!(ok(&["view", &s(&file), "A1:B2"]), "1\t2\n3\t\n");
    // No range at all means the used extent.
    assert_eq!(ok(&["view", &s(&file)]), "1\t2\n3\t\n");
    assert_eq!(
        ok(&["view", &s(&file), "A1:B2", "--max-rows", "1"]),
        "1\t2\n"
    );
}

/// A number format is display only: the stored value must come back untouched, and a
/// formatted cell must still be the number a later formula reads.
#[test]
fn format_changes_what_a_cell_shows_and_not_what_it_holds() {
    let dir = Sandbox::new("format");
    let file = dir.path("book.ods");
    ok(&["new", &s(&file)]);
    ok(&["set", &s(&file), "A1", "1234.5"]);
    ok(&["set", &s(&file), "A2", "0.075"]);
    ok(&["format", &s(&file), "A1", "currency", "--symbol", "$", "--grouping"]);
    ok(&["format", &s(&file), "A2", "percent", "--decimals", "1"]);

    assert_eq!(ok(&["view", &s(&file), "A1:A2"]), "1,234.50\u{a0}$\n7.5%\n");
    assert_eq!(ok(&["view", &s(&file), "A1:A2", "--raw"]), "1234.5\n0.075\n");
    // The value a formula sees is the value, not the display.
    ok(&["set", &s(&file), "B1", "=[.A1]*2"]);
    assert_eq!(ok(&["get", &s(&file), "B1", "--raw"]).trim(), "2469");

    // JSON carries both spellings, always, so a consumer picks without re-running.
    let json = ok(&["--format", "json", "get", &s(&file), "A2"]);
    assert_eq!(field(&json, "value"), "0.075");
    assert_eq!(field(&json, "text"), "7.5%");

    // `general` is the absence of a format, and the whole range is one undo step.
    ok(&["format", &s(&file), "A1:A2", "general"]);
    assert_eq!(ok(&["view", &s(&file), "A1:A2"]), "1234.5\n0.075\n");
}

/// A date is a number, and without a format it shows as one. Formatting it is what makes a
/// spreadsheet a spreadsheet.
#[test]
fn a_date_shows_as_a_date_once_the_cell_says_so() {
    let dir = Sandbox::new("format-date");
    let file = dir.path("book.ods");
    ok(&["new", &s(&file)]);
    ok(&["set", &s(&file), "A1", "=DATE(2026;8;16)"]);
    assert_eq!(ok(&["get", &s(&file), "A1"]).trim(), "46250");

    ok(&["format", &s(&file), "A1", "date"]);
    assert_eq!(ok(&["get", &s(&file), "A1"]).trim(), "2026-08-16");
    assert_eq!(ok(&["get", &s(&file), "A1", "--raw"]).trim(), "46250");
}

/// The rectangle is bounded, because a format costs an entry per cell.
#[test]
fn formatting_an_absurd_rectangle_fails_instead_of_trying() {
    let dir = Sandbox::new("format-huge");
    let file = dir.path("book.ods");
    ok(&["new", &s(&file)]);
    let output = sheet(&["format", &s(&file), "A1:ZZ100000", "date"]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("more than"),
        "{output:?}"
    );
}

/// `examples/sample.sh` is the inventory of what this build can do, and an inventory that
/// is not run is a wish list. Running it here means a feature that stops working, or a
/// command that changes its flags, fails the build rather than the next reader.
#[test]
fn the_sample_script_still_builds_its_document() {
    let dir = Sandbox::new("sample");
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the workspace root")
        .join("examples/sample.sh");

    let output = Command::new("bash")
        .arg(&script)
        .arg(s(&dir.path("out")))
        .env("SHEET", env!("CARGO_BIN_EXE_sheet"))
        .output()
        .expect("bash runs");
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    // Both forms exist, and the document reads back as the one the script described.
    assert!(dir.path("out/sample.ods").exists());
    assert!(dir.path("out/sample.fods").exists());
    let book = s(&dir.path("out/sample.ods"));
    assert_eq!(ok(&["get", &book, "B12"]).trim(), "2026-08-16");
    assert_eq!(ok(&["get", &book, "B12", "--raw"]).trim(), "46250");
    assert_eq!(ok(&["get", &book, "B5", "--formula"]).trim(), "=SUM([.B2:.B4])");
    // A styled and formatted header, and a currency cell whose value is untouched.
    assert_eq!(ok(&["get", &book, "B2", "--raw"]).trim(), "1234.5");
    assert!(ok(&["get", &book, "B2"]).contains('\u{20ac}'));

    // And it is valid ODF. The sample is the one document that uses every feature there
    // is — formats, styles, dates, names — so it is the strongest thing to hold the
    // RELAX NG schema against. `core/tests/kb.rs` explains the `jing -i`.
    let schema = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the workspace root")
        .join("doc/OpenDocument-v1.4-schema.rng");
    match Command::new("jing")
        .arg("-i")
        .arg(&schema)
        .arg(s(&dir.path("out/sample.fods")))
        .output()
    {
        Err(_) => eprintln!("skipping: no `jing` on PATH; schema validity unchecked"),
        Ok(out) => assert!(
            out.status.success(),
            "examples/sample.sh writes invalid ODF:\n{}",
            String::from_utf8_lossy(&out.stdout)
        ),
    }
}

#[test]
fn recalc_updates_a_stale_value_and_reports_it() {
    let dir = Sandbox::new("recalc");
    let file = dir.path("book.ods");
    ok(&["new", &s(&file)]);
    ok(&["set", &s(&file), "A1", "1"]);
    ok(&["set", &s(&file), "A2", "=[.A1]*10"]);
    assert_eq!(ok(&["get", &s(&file), "A2"]).trim(), "10");

    ok(&["set", &s(&file), "A1", "5"]);
    assert_eq!(
        ok(&["get", &s(&file), "A2"]).trim(),
        "10",
        "stale until recalc"
    );

    let json = ok(&["--format", "json", "recalc", &s(&file)]);
    assert_eq!(field(&json, "changed"), "true");
    assert_eq!(ok(&["get", &s(&file), "A2"]).trim(), "50");

    // Nothing left to do, and a no-op is a success.
    let json = ok(&["--format", "json", "recalc", &s(&file)]);
    assert_eq!(field(&json, "changed"), "false");
}

/// This build implements part of OpenFormula, so recalculating a document that uses the rest
/// destroys a good cached value. The warning is the difference between that and silent data
/// loss, and it must not reach stdout.
#[test]
fn recalc_warns_on_stderr_when_it_turns_a_value_into_an_error() {
    let dir = Sandbox::new("spoil");
    let file = dir.path("book.ods");
    ok(&["new", &s(&file)]);
    ok(&["set", &s(&file), "A1", "=SUBTOTAL(9;[.B1:.B2])"]);
    ok(&["set", &s(&file), "A1", "169.625"]);

    let output = sheet(&["recalc", &s(&file)]);
    assert!(
        output.status.success(),
        "a spoiled value is a warning, not a failure"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("became errors"), "got: {stderr}");
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains("became errors"),
        "diagnostics belong on stderr"
    );
}

/// Editing a cell a formula reads leaves the document disagreeing with itself, and saying so
/// is the whole point: ODF has no dirty bit, so the file on disk claims a total that is no
/// longer the sum of its parts, and LibreOffice will show that stale total.
///
/// A warning rather than an automatic recalculation, for the same reason `recalc` reports
/// `spoiled`: this build implements the Small Group, and recalculating a document that uses
/// anything else turns good cached values into `#NAME?`. That choice stays the user's.
#[test]
fn editing_a_cell_a_formula_reads_warns_that_the_document_is_stale() {
    let dir = Sandbox::new("stale");
    let file = dir.path("book.fods");
    ok(&["new", &s(&file)]);
    ok(&["set", &s(&file), "A1", "1"]);
    ok(&["set", &s(&file), "A2", "=[.A1]*10"]);
    ok(&["recalc", &s(&file)]);
    assert_eq!(ok(&["get", &s(&file), "A2"]).trim(), "10");

    let output = sheet(&["set", &s(&file), "A1", "5"]);
    assert!(output.status.success(), "staleness is a warning, not an error");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("1 formula cell(s)"), "got: {stderr}");
    assert!(stderr.contains("sheet recalc"), "it must say what to do: {stderr}");
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains("recalc"),
        "diagnostics belong on stderr"
    );
    // The stale value really is still on disk — that is the thing being warned about.
    assert_eq!(ok(&["get", &s(&file), "A2"]).trim(), "10");

    // Machine-readable too, so a shell or a script does not have to scrape stderr.
    let json = ok(&["--format", "json", "set", &s(&file), "A1", "6"]);
    assert!(json.contains("\"stale\":1"), "got: {json}");

    // And it goes away once the document agrees with itself again. The field is omitted
    // rather than written as zero, so the common case carries no noise.
    ok(&["recalc", &s(&file)]);
    let json = ok(&["--format", "json", "set", &s(&file), "B9", "0"]);
    assert!(!json.contains("\"stale\""), "got: {json}");
}

/// A document with no formulas is never stale, and must not pay for the check.
#[test]
fn a_document_without_formulas_is_never_reported_stale() {
    let dir = Sandbox::new("nostale");
    let file = dir.path("book.fods");
    ok(&["new", &s(&file)]);
    let output = sheet(&["set", &s(&file), "A1", "1"]);
    assert!(String::from_utf8_lossy(&output.stderr).is_empty());
}

#[test]
fn a_sheet_qualified_address_reaches_the_right_sheet() {
    let dir = Sandbox::new("sheets");
    let file = dir.path("two.fods");
    // Two sheets, written by hand: nothing in the core can add one yet.
    std::fs::write(
        &file,
        r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
 xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0"
 xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"
 office:mimetype="application/vnd.oasis.opendocument.spreadsheet">
 <office:body><office:spreadsheet>
  <table:table table:name="First"><table:table-column/>
   <table:table-row><table:table-cell office:value-type="float" office:value="1"/></table:table-row>
  </table:table>
  <table:table table:name="Data"><table:table-column/><table:table-column/>
   <table:table-row><table:table-cell/><table:table-cell/></table:table-row>
   <table:table-row><table:table-cell/><table:table-cell office:value-type="float" office:value="42"/></table:table-row>
  </table:table>
 </office:spreadsheet></office:body></office:document>"#,
    )
    .unwrap();

    assert_eq!(ok(&["get", &s(&file), "Data.B2"]).trim(), "42");
    assert_eq!(
        ok(&["get", &s(&file), "data.b2"]).trim(),
        "42",
        "names match case-insensitively"
    );
    assert_eq!(
        ok(&["get", &s(&file), "A1"]).trim(),
        "1",
        "no sheet means the first"
    );

    let output = sheet(&["get", &s(&file), "Nope.A1"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("no such sheet"));
}

#[test]
fn a_session_carries_undo_across_invocations() {
    let dir = Sandbox::new("session");
    let file = dir.path("book.ods");
    let session = s(&dir.path("book.session"));
    ok(&["new", &s(&file)]);
    ok(&["set", &s(&file), "A1", "1", "--session", &session]);
    ok(&["set", &s(&file), "A1", "2", "--session", &session]);
    assert_eq!(ok(&["get", &s(&file), "A1"]).trim(), "2");

    ok(&["undo", &s(&file), "--session", &session]);
    assert_eq!(ok(&["get", &s(&file), "A1"]).trim(), "1");
    ok(&["undo", &s(&file), "--session", &session]);
    assert_eq!(ok(&["get", &s(&file), "A1"]).trim(), "");

    ok(&["redo", &s(&file), "--session", &session]);
    assert_eq!(ok(&["get", &s(&file), "A1"]).trim(), "1");
}

#[test]
fn undoing_with_nothing_left_succeeds_and_says_it_changed_nothing() {
    let dir = Sandbox::new("noop");
    let file = dir.path("book.ods");
    let session = s(&dir.path("book.session"));
    ok(&["new", &s(&file)]);

    let json = ok(&["--format", "json", "undo", &s(&file), "--session", &session]);
    assert_eq!(field(&json, "changed"), "false");
    assert_eq!(field(&json, "written"), "false");
}

#[test]
fn undo_without_a_session_is_an_error_that_names_the_flag() {
    let dir = Sandbox::new("nosession");
    let file = dir.path("book.ods");
    ok(&["new", &s(&file)]);

    let output = sheet(&["undo", &s(&file)]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty(), "errors must not reach stdout");
    assert!(String::from_utf8_lossy(&output.stderr).contains("--session"));
}

#[test]
fn a_dry_run_changes_nothing_on_disk() {
    let dir = Sandbox::new("dryrun");
    let file = dir.path("book.ods");
    let session = dir.path("book.session");
    ok(&["new", &s(&file)]);
    let before = std::fs::read(&file).unwrap();

    let json = ok(&[
        "--format",
        "json",
        "--dry-run",
        "set",
        &s(&file),
        "A1",
        "99",
        "--session",
        &s(&session),
    ]);
    assert_eq!(
        field(&json, "changed"),
        "true",
        "the command did do something"
    );
    assert_eq!(field(&json, "written"), "false", "but not to disk");

    assert_eq!(
        std::fs::read(&file).unwrap(),
        before,
        "the document is untouched"
    );
    assert!(!session.exists(), "a dry run writes no session either");
}

#[test]
fn convert_moves_between_the_package_and_flat_forms() {
    let dir = Sandbox::new("convert");
    let ods = dir.path("book.ods");
    let fods = dir.path("book.fods");
    let back = dir.path("back.ods");
    ok(&["new", &s(&ods)]);
    ok(&["set", &s(&ods), "A1", "1.5"]);
    ok(&["set", &s(&ods), "B1", "text"]);

    ok(&["convert", &s(&ods), &s(&fods)]);
    assert!(
        std::fs::read(&fods).unwrap().starts_with(b"<?xml"),
        "flat form is XML"
    );
    assert_eq!(ok(&["view", &s(&fods), "A1:B1"]), "1.5\ttext\n");

    ok(&["convert", &s(&fods), &s(&back)]);
    assert_eq!(
        std::fs::read(&back).unwrap()[..2],
        *b"PK",
        "package form is a zip"
    );
    assert_eq!(ok(&["view", &s(&back), "A1:B1"]), "1.5\ttext\n");
}

#[test]
fn clear_empties_a_cell_and_can_keep_the_computed_value() {
    let dir = Sandbox::new("clear");
    let file = dir.path("book.ods");
    ok(&["new", &s(&file)]);
    ok(&["set", &s(&file), "A1", "4"]);
    ok(&["set", &s(&file), "A2", "=[.A1]*2"]);

    ok(&["clear", &s(&file), "A2", "--formula-only"]);
    assert_eq!(ok(&["get", &s(&file), "A2"]).trim(), "8", "the value stays");
    assert_eq!(ok(&["get", &s(&file), "A2", "--formula"]).trim(), "");

    ok(&["clear", &s(&file), "A2"]);
    assert_eq!(ok(&["get", &s(&file), "A2"]).trim(), "");
}

#[test]
fn fmt_normalises_a_formula_without_touching_a_document() {
    // §5.5 Table 1: `^` is left-associative, so `2^3^2` needs no brackets and means 64.
    assert_eq!(ok(&["fmt", "=2^3^2"]).trim(), "=2^3^2");
    assert_eq!(ok(&["fmt", "=SUM([.A1:.A2])"]).trim(), "=SUM([.A1:.A2])");

    let output = sheet(&["fmt", "=SUM("]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}

#[test]
fn functions_lists_what_this_build_implements() {
    let out = ok(&["functions"]);
    assert!(out.contains("SUM"));
    assert!(out.contains("of 110 in the Small Group"));
    assert!(!out.contains("SUBTOTAL"), "not in the Small Group");
}

#[test]
fn a_malformed_address_fails_without_writing_anything() {
    let dir = Sandbox::new("badaddr");
    let file = dir.path("book.ods");
    ok(&["new", &s(&file)]);
    let before = std::fs::read(&file).unwrap();

    for bad in ["SUM(A1)", "1+1", ""] {
        let output = sheet(&["set", &s(&file), bad, "1"]);
        assert!(!output.status.success(), "{bad} should not be an address");
        assert!(output.stdout.is_empty(), "errors must not reach stdout");
    }
    // `get` takes one cell, not a range.
    assert!(!sheet(&["get", &s(&file), "A1:B2"]).status.success());
    assert_eq!(std::fs::read(&file).unwrap(), before);
}

#[test]
fn a_missing_file_reports_the_path() {
    let output = sheet(&["get", "/nonexistent/book.ods", "A1"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("/nonexistent/book.ods"));
}
