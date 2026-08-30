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
            "grind-cli-{name}-{}-{:?}",
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

/// Run `grind` with exactly these arguments — for the suite-level verbs, which sit at the top
/// level and take no application name.
fn grind(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_grind"))
        .args(args)
        // A developer's own `~/.config/grind/locale` must not leak into a test that
        // asserts specific separators — every test spawns through here, so this is the
        // one place that needs to say so.
        .env_remove("GRIND_LOCALE")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("HOME")
        .output()
        .expect("the binary runs")
}

/// Run `grind sheet …` — the spreadsheet application. Most of this file wants this one, so it
/// prepends the app name rather than making every call site spell it.
fn sheet(args: &[&str]) -> Output {
    let mut argv = vec!["sheet"];
    argv.extend_from_slice(args);
    grind(&argv)
}

/// Run `grind sheet …`, require success, return stdout.
fn ok(args: &[&str]) -> String {
    succeeds(sheet(args), args)
}

/// The same for a suite-level verb — `grind info`, `grind convert` — which takes no
/// application name because it works out the kind from the file.
fn ok_top(args: &[&str]) -> String {
    succeeds(grind(args), args)
}

fn succeeds(output: Output, args: &[&str]) -> String {
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

    let json = ok_top(&["--format", "json", "info", &s(&file)]);
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
    ok(&[
        "format",
        &s(&file),
        "A1",
        "currency",
        "--symbol",
        "$",
        "--grouping",
    ]);
    ok(&["format", &s(&file), "A2", "percent", "--decimals", "1"]);

    assert_eq!(ok(&["view", &s(&file), "A1:A2"]), "1,234.50\u{a0}$\n7.5%\n");
    assert_eq!(
        ok(&["view", &s(&file), "A1:A2", "--raw"]),
        "1234.5\n0.075\n"
    );
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

/// `--show` is the read half a toolbar needs: `set_style` replaces, so "bold as well" is a
/// read, a merge and a write — and the CLI can do it because a GUI can.
#[test]
fn show_prints_the_styling_a_toolbar_would_merge_into() {
    let dir = Sandbox::new("show");
    let file = dir.path("book.ods");
    ok(&["new", &s(&file)]);
    ok(&["set", &s(&file), "A1", "1234.5"]);
    assert_eq!(
        ok(&["style", &s(&file), "A1", "--show"]),
        "",
        "a plain cell says nothing"
    );
    assert_eq!(ok(&["format", &s(&file), "A1", "--show"]), "");

    ok(&[
        "style",
        &s(&file),
        "A1",
        "--bold",
        "--background",
        "#ffff00",
    ]);
    let shown = ok(&["style", &s(&file), "A1", "--show"]);
    assert!(shown.contains("fo:font-weight\tbold"), "{shown}");
    assert!(shown.contains("fo:background-color\t#ffff00"), "{shown}");

    // A palette name is the same attribute a GUI's swatch writes — the table is the core's
    // (`style::PALETTE`), so neither shell has its own idea of navy. Its colour reaches a
    // border too, because a name in the file is an attribute LibreOffice drops silently.
    ok(&[
        "style",
        &s(&file),
        "A1",
        "--color",
        "navy",
        "--background",
        "Silver",
        "--border",
        "0.5pt solid red",
    ]);
    let shown = ok(&["style", &s(&file), "A1", "--show"]);
    assert!(shown.contains("fo:color\t#001f3f"), "{shown}");
    assert!(
        shown.contains("fo:background-color\t#dddddd"),
        "case is irrelevant: {shown}"
    );
    assert!(shown.contains("fo:border\t0.5pt solid #ff4136"), "{shown}");
    // A name that is not one says so, and names the palette rather than only the hex form.
    let refused = sheet(&["style", &s(&file), "A1", "--color", "nvy"]);
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("navy"));

    // Merging is the caller's job, and this is what makes it possible: read, add italic,
    // write, and the bold is still there.
    ok(&[
        "style",
        &s(&file),
        "A1",
        "--bold",
        "--italic",
        "--background",
        "#ffff00",
    ]);
    let shown = ok(&["style", &s(&file), "A1", "--show"]);
    assert!(shown.contains("fo:font-style\titalic") && shown.contains("fo:font-weight\tbold"));

    // A format prints the flags that would recreate it, and says that they would.
    ok(&[
        "format",
        &s(&file),
        "A1",
        "currency",
        "--decimals",
        "2",
        "--grouping",
        "--symbol",
        "€",
        "--locale",
        "de-DE",
    ]);
    let shown = ok(&["format", &s(&file), "A1", "--show"]);
    for line in [
        "kind\tcurrency",
        "decimals\t2",
        "grouping\ttrue",
        "symbol\t€",
        "locale\tde-DE",
        "preset\ttrue",
    ] {
        assert!(shown.contains(line), "{line} missing from {shown}");
    }
    // JSON carries the structure rather than the prose, which is what a picker restores from.
    let json = ok(&["--format", "json", "style", &s(&file), "A1", "--show"]);
    assert_eq!(field(&json, "ref"), "A1");
    assert_eq!(field(&json, "font_weight"), "bold");

    // Showing and setting are different requests, and asking for both at once is a mistake
    // clap catches rather than a silent precedence rule.
    assert!(
        !sheet(&["style", &s(&file), "A1", "--show", "--bold"])
            .status
            .success()
    );
    assert!(
        !sheet(&["format", &s(&file), "A1", "date", "--show"])
            .status
            .success()
    );
    assert!(!sheet(&["format", &s(&file), "A1"]).status.success());
    assert!(
        !sheet(&["style", &s(&file), "A1:B2", "--show"])
            .status
            .success()
    );
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

/// `examples/sample-sheet.sh` is the inventory of what this build can do, and an inventory that
/// is not run is a wish list. Running it here means a feature that stops working, or a
/// command that changes its flags, fails the build rather than the next reader.
#[test]
fn the_sample_script_still_builds_its_document() {
    let dir = Sandbox::new("sample");
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the workspace root")
        .join("examples/sample-sheet.sh");

    let output = Command::new("bash")
        .arg(&script)
        .arg(s(&dir.path("out")))
        .env("GRIND", env!("CARGO_BIN_EXE_grind"))
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
    assert_eq!(ok(&["get", &book, "B17"]).trim(), "2026-08-16");
    assert_eq!(ok(&["get", &book, "B17", "--raw"]).trim(), "46250");
    assert_eq!(
        ok(&["get", &book, "B8", "--formula"]).trim(),
        "=SUM([.B2:.B7])"
    );
    // A styled and formatted header, and a currency cell whose value is untouched.
    assert_eq!(ok(&["get", &book, "B2", "--raw"]).trim(), "1800");
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
            "examples/sample-sheet.sh writes invalid ODF:\n{}",
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
///
/// The fixture is written by hand because nothing here can produce it: a cached value that
/// disagrees with its formula is what a *document from another program* looks like, and
/// `sheet set` cannot make one — `App::enter` replaces a formula rather than leaving a value
/// beside it.
#[test]
fn recalc_warns_on_stderr_when_it_turns_a_value_into_an_error() {
    let dir = Sandbox::new("spoil");
    let file = dir.path("book.fods");
    std::fs::write(
        &file,
        r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" office:version="1.4" office:mimetype="application/vnd.oasis.opendocument.spreadsheet">
 <office:body>
  <office:spreadsheet>
   <table:table table:name="Sheet1">
    <table:table-column/>
    <table:table-row><table:table-cell table:formula="of:=SUBTOTAL(9;[.B1:.B2])" office:value-type="float" office:value="169.625"><text:p>169.625</text:p></table:table-cell></table:table-row>
   </table:table>
  </office:spreadsheet>
 </office:body>
</office:document>"#,
    )
    .unwrap();

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
    assert!(
        output.status.success(),
        "staleness is a warning, not an error"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("1 formula cell(s)"), "got: {stderr}");
    assert!(
        stderr.contains("sheet recalc"),
        "it must say what to do: {stderr}"
    );
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

/// The point of a name is a formula that reads, so the assertions are formulas.
#[test]
fn a_named_range_can_be_defined_and_used_from_a_formula() {
    let dir = Sandbox::new("names");
    let file = dir.path("book.fods");
    ok(&["new", &s(&file)]);
    for (row, value) in [(1, "10"), (2, "20"), (3, "30"), (4, "40")] {
        ok(&["set", &s(&file), &format!("A{row}"), value]);
    }

    // An address becomes a *named range*: sheet-qualified and absolute on every axis, or it
    // would mean a different range read from another sheet or two rows down.
    ok(&["name", &s(&file), "sales", "A1:A4"]);
    assert_eq!(
        ok(&["name", &s(&file), "sales"]).trim(),
        "[$Sheet1.$A$1:.$A$4]"
    );

    ok(&["set", &s(&file), "C1", "=SUM(sales)"]);
    assert_eq!(ok(&["get", &s(&file), "C1"]).trim(), "100");

    // A leading `=` makes it a named *expression* instead, and one name may use another.
    ok(&["name", &s(&file), "average", "=AVERAGE(sales)"]);
    ok(&["set", &s(&file), "C2", "=average"]);
    assert_eq!(ok(&["get", &s(&file), "C2"]).trim(), "25");

    // Both survive the round trip through the file, which is the only thing that makes a
    // name worth having.
    assert!(ok_top(&["info", &s(&file)]).contains("[$Sheet1.$A$1:.$A$4]"));

    // Redefining is not a second name — §5.11 names are case-consistent.
    ok(&["name", &s(&file), "SALES", "A1:A2"]);
    ok(&["recalc", &s(&file)]);
    assert_eq!(ok(&["get", &s(&file), "C1"]).trim(), "30");
    assert_eq!(
        // Lines, not occurrences: `average` mentions `sales` in its own expression.
        ok_top(&["info", &s(&file)])
            .lines()
            .filter(|l| l.starts_with("sales\t"))
            .count(),
        1,
        "redefining made a second name"
    );

    // Deleting one a formula still mentions is allowed, and says what it cost.
    let output = sheet(&["name", &s(&file), "sales", "--delete"]);
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("no longer defined"));
    ok(&["recalc", &s(&file)]);
    assert_eq!(ok(&["get", &s(&file), "C1"]).trim(), "#NAME?");

    assert!(
        !sheet(&["name", &s(&file), "sales", "--delete"])
            .status
            .success()
    );
}

/// A name that cannot be lexed, or that a reference would win against, is refused where the
/// user is — storing one means every formula mentioning it says `#NAME?` for good.
#[test]
fn a_name_that_no_formula_could_mention_is_refused() {
    let dir = Sandbox::new("badnames");
    let file = dir.path("book.fods");
    ok(&["new", &s(&file)]);

    for bad in ["A1", "$B$7", "my range", "1st", ""] {
        let output = sheet(&["name", &s(&file), bad, "B1:B2"]);
        assert!(!output.status.success(), "{bad:?} should be refused");
    }
    // And the expression is parsed before it is stored, for the same reason.
    assert!(!sheet(&["name", &s(&file), "ok", "=SUM("]).status.success());
    assert!(
        !sheet(&["name", &s(&file), "ok", "not an address"])
            .status
            .success()
    );
    assert!(
        ok_top(&["info", &s(&file)])
            .lines()
            .all(|l| !l.starts_with("ok"))
    );
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

    ok_top(&["convert", &s(&ods), &s(&fods)]);
    assert!(
        std::fs::read(&fods).unwrap().starts_with(b"<?xml"),
        "flat form is XML"
    );
    assert_eq!(ok(&["view", &s(&fods), "A1:B1"]), "1.5\ttext\n");

    ok_top(&["convert", &s(&fods), &s(&back)]);
    assert_eq!(
        std::fs::read(&back).unwrap()[..2],
        *b"PK",
        "package form is a zip"
    );
    assert_eq!(ok(&["view", &s(&back), "A1:B1"]), "1.5\ttext\n");
}

/// `doc/dsl.md` D4: the projection is a *form*, so it is reached by the verb that moves a
/// document between forms rather than by an export verb of its own.
///
/// Out and back, because that is the claim being made. A conversion to a lossy format is a
/// one-way trip and this is not one: the projection is bijective with the model, so the
/// document that comes back from `.grind` is the document that went in — a formula still a
/// formula, and a style still a style.
#[test]
fn convert_reaches_the_projection_and_comes_back() {
    let dir = Sandbox::new("convert-projection");
    let fods = dir.path("book.fods");
    let grind_file = dir.path("book.grind");
    let back = dir.path("back.fods");
    ok(&["new", &s(&fods)]);
    ok(&["set", &s(&fods), "A1", "1.5"]);
    ok(&["set", &s(&fods), "A2", "=[.A1]*2"]);
    ok(&["style", &s(&fods), "A1", "--bold"]);

    ok_top(&["convert", &s(&fods), &s(&grind_file)]);
    let text = std::fs::read_to_string(&grind_file).unwrap();
    assert!(
        text.starts_with("grind spreadsheet\n"),
        "the projection names its kind in its first line: {text}"
    );

    // And every command already reads one, because `read_bytes` sniffs the form (D4's other
    // half, landed with D1).
    assert_eq!(ok(&["view", &s(&grind_file), "A1:A2"]), "1.5\n3\n");

    ok_top(&["convert", &s(&grind_file), &s(&back)]);
    assert_eq!(
        ok(&["view", &s(&back), "A2", "--formulas"]).trim(),
        "=[.A1]*2",
        "the formula is still a formula, in ODF syntax verbatim"
    );
    assert_eq!(
        ok(&["style", &s(&back), "A1", "--show"]).trim(),
        "fo:font-weight\tbold",
        "and the style is still a style"
    );
}

/// The same verb, the other application. `grind convert` is suite level — it reads the kind out
/// of the file — so a text document reaches the projection through exactly the same command,
/// which is what makes the projection a *form* rather than a spreadsheet feature.
#[test]
fn convert_reaches_the_projection_for_a_text_document_too() {
    let dir = Sandbox::new("text-projection");
    let fodt = dir.path("report.fodt");
    let out = dir.path("report.grind");
    let back = dir.path("back.fodt");
    succeeds(grind(&["text", "new", &s(&fodt)]), &["text", "new"]);
    succeeds(
        grind(&[
            "text",
            "insert",
            &s(&fodt),
            "--heading",
            "1",
            "--text",
            "Field Notes",
        ]),
        &["text", "insert"],
    );

    ok_top(&["convert", &s(&fodt), &s(&out)]);
    let text = std::fs::read_to_string(&out).unwrap();
    assert!(text.starts_with("grind text\n"), "{text}");
    assert!(text.contains("h 1 \"Field Notes\""), "{text}");

    ok_top(&["convert", &s(&out), &s(&back)]);
    let view = succeeds(grind(&["text", "view", &s(&back)]), &["text", "view"]);
    assert!(view.contains("Field Notes"), "{view}");
}

/// **D5 through the CLI, both applications**: editing a `.grind` in place rewrites one line and
/// leaves everything the model has no room for exactly as it was.
///
/// It is a CLI test rather than a core one because the core tests can only prove that
/// `write_bytes` splices; this proves that the *product* does — that `grind sheet set book.grind`
/// picks the projection form from the name, finds the retained text on the document it just
/// read, and puts the file back. R6 has never been a library property here.
#[test]
fn editing_a_projection_in_place_rewrites_one_line() {
    let dir = Sandbox::new("projection-r6");

    let book = dir.path("book.grind");
    std::fs::write(
        &book,
        "grind spreadsheet\n\n// Q3 forecast, by hand.\nsheet Sales {\n    at A1 {\n        \
         row North   4200    4800\n    }\n}\n",
    )
    .unwrap();
    let before = std::fs::read_to_string(&book).unwrap();
    ok(&["set", &s(&book), "B1", "4300"]);
    let after = std::fs::read_to_string(&book).unwrap();
    assert_eq!(
        changed_lines(&before, &after),
        [(
            "        row North   4200    4800",
            "        row North   4300    4800"
        )],
        "one value, and the alignment either side of it is the file's own"
    );
    assert!(after.contains("// Q3 forecast, by hand."), "{after}");

    let report = dir.path("report.grind");
    std::fs::write(
        &report,
        "grind text\n\n// The chapter this file is about.\nh 1 \"Addresses\"\n\np \"A paragraph.\"\n",
    )
    .unwrap();
    let before = std::fs::read_to_string(&report).unwrap();
    succeeds(
        grind(&["text", "type", &s(&report), "p2", "Another "]),
        &["text", "type"],
    );
    let after = std::fs::read_to_string(&report).unwrap();
    assert_eq!(
        changed_lines(&before, &after),
        [("p \"A paragraph.\"", "p \"Another A paragraph.\"")]
    );
    assert!(
        after.contains("// The chapter this file is about."),
        "{after}"
    );
}

/// **A spreadsheet written without doing the arithmetic** (`doc/projection-sheet.md`).
///
/// The cached value is optional: `cell B5 "=SUM([.B2:.B4])"` is a whole cell, and `recalc` is
/// what fills the answers in. The reason this is a CLI test is that the interesting half is what
/// happens *between* the forms — a formula with no cached value used to be outside the used
/// extent, which is the rectangle the ODF writer emits, so converting a hand-written model to
/// `.fods` gave back a file with no formulas in it at all.
#[test]
fn a_model_written_without_its_answers_keeps_them_through_every_form() {
    let dir = Sandbox::new("no-math");
    let model = dir.path("model.grind");
    std::fs::write(
        &model,
        "grind spreadsheet

// Written by hand. No arithmetic was done by the author.
         sheet Budget {
    at A1 {
        row Rent      1800
        row Food      520
    }
         
    cell B3 \"=SUM([.B1:.B2])\"
    cell B4 \"=[.B3]*12\"
}
",
    )
    .unwrap();

    // It reads, and the formulas are there with nothing cached for them.
    assert_eq!(
        ok(&["get", &s(&model), "B3", "--formula"]).trim(),
        "of:=SUM([.B1:.B2])"
    );
    assert_eq!(ok(&["get", &s(&model), "B3"]).trim(), "", "no answer yet");

    // Through ODF — both forms, since the package regenerates and the flat one splices.
    for (name, form) in [("model.fods", "flat"), ("model.ods", "package")] {
        let out = dir.path(name);
        ok_top(&["convert", &s(&model), &s(&out)]);
        assert_eq!(
            ok(&["get", &s(&out), "B4", "--formula"]).trim(),
            "of:=[.B3]*12",
            "{form}: the formula did not survive the conversion"
        );
    }

    // And `recalc` is the step that gives it answers — in the projection itself, one line of
    // diff per cell, with everything the model has no room for still there (D5).
    let before = std::fs::read_to_string(&model).unwrap();
    ok(&["recalc", &s(&model)]);
    let after = std::fs::read_to_string(&model).unwrap();
    assert_eq!(ok(&["get", &s(&model), "B3"]).trim(), "2320");
    assert_eq!(ok(&["get", &s(&model), "B4"]).trim(), "27840");
    assert_eq!(
        changed_lines(&before, &after).len(),
        2,
        "one line per formula"
    );
    assert!(after.contains("// Written by hand."), "{after}");
}

/// Which lines two texts differ on, as `(before, after)`.
fn changed_lines<'a>(before: &'a str, after: &'a str) -> Vec<(&'a str, &'a str)> {
    let (a, b): (Vec<_>, Vec<_>) = (before.lines().collect(), after.lines().collect());
    assert_eq!(a.len(), b.len(), "a splice does not change the line count");
    a.into_iter().zip(b).filter(|(x, y)| x != y).collect()
}

/// `doc/flat-first.md`: naming `.ods` or `.odt` asks for a package and gets one; naming
/// anything else — a bare stem most of all — gets flat XML, because the whole point of R6's
/// diffable writer is lost the moment the file it applies to is a zip.
///
/// Both applications, and the extensionless case in both, because this is a *product* default
/// rather than a spreadsheet one and a suite that applied it to half its document types would
/// be worse than one that did not apply it at all.
#[test]
fn a_name_that_does_not_ask_for_a_package_gets_flat_xml() {
    let dir = Sandbox::new("flat-first");
    let zip = |path: &Path| std::fs::read(path).unwrap()[..2] == *b"PK";

    for (app, stem, package) in [
        ("sheet", "book", "book.ods"),
        ("text", "report", "report.odt"),
    ] {
        let bare = dir.path(stem);
        let named = dir.path(package);
        succeeds(grind(&[app, "new", &s(&bare)]), &["new", stem]);
        succeeds(grind(&[app, "new", &s(&named)]), &["new", package]);

        assert!(!zip(&bare), "{stem}: a name asking for nothing is flat");
        assert!(zip(&named), "{package}: a name asking for a package is one");
        // And both are still documents of the right kind, read back through the sniffer that
        // never looks at the name.
        for path in [&bare, &named] {
            let report = succeeds(grind(&["--format", "json", "info", &s(path)]), &["info"]);
            assert!(
                report.contains(match app {
                    "sheet" => "spreadsheet",
                    _ => "text document",
                }),
                "{}: {report}",
                s(path)
            );
        }
    }
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
    assert!(out.contains("of the Small Group's 110"), "{out}");
    // The functions moved in by explicit decision are counted apart from the claim, so the
    // summary is never the nonsense "112 of 110".
    assert!(out.contains("beyond it: COLUMN, ROW"), "{out}");
    assert!(!out.contains("SUBTOTAL"), "not in the Small Group");
}

#[test]
fn functions_long_adds_the_alias_and_category_beside_the_spec_signature() {
    let out = ok(&["functions", "--long", "--filter", "pv"]);
    assert!(out.contains("Present Value"), "{out}");
    assert!(out.contains("Financial"), "{out}");
    assert!(out.contains("§6.12.41"), "{out}");
}

#[test]
fn fmt_friendly_explains_a_formula_without_touching_a_document() {
    let out = ok(&["fmt", "--friendly", "=ROUND([.A1];2)"]);
    assert_eq!(out.trim(), "Round(Value: A1, Digits: 2)");

    // Read-only: never a fourth spelling of the formula, so it cannot be combined with the
    // two that do round-trip.
    let output = sheet(&["fmt", "--friendly", "--display", "=SUM([.A1])"]);
    assert!(!output.status.success());
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

/// The whole sheet lifecycle through the CLI, on disk: a sheet is only real once it comes
/// back out of the file it was written to.
#[test]
fn a_sheet_can_be_added_renamed_and_deleted_from_the_cli() {
    let dir = Sandbox::new("sheets");
    let file = dir.path("book.ods");
    ok(&["new", &s(&file)]);
    ok(&["add", &s(&file), "Data"]);
    ok(&["set", &s(&file), "Data.A1", "7"]);
    assert_eq!(ok(&["get", &s(&file), "Data.A1"]).trim(), "7");

    // Case-insensitively, the way a reference resolves a sheet name (§5.8).
    ok(&["rename", &s(&file), "data", "Q3 Actuals"]);
    assert_eq!(ok(&["get", &s(&file), "'Q3 Actuals'.A1"]).trim(), "7");

    let output = sheet(&["add", &s(&file), "q3 actuals"]);
    assert!(!output.status.success(), "a duplicate name is refused");

    ok(&["remove", &s(&file), "Q3 Actuals"]);
    let json = ok_top(&["--format", "json", "info", &s(&file)]);
    assert!(!json.contains("Q3 Actuals"), "gone from the file: {json}");

    let output = sheet(&["remove", &s(&file), "Sheet1"]);
    assert!(!output.status.success(), "the last sheet stays");
}

/// Deleting a sheet is undoable across invocations, which is the whole reason the inverse
/// carries the sheet rather than its name.
#[test]
fn undoing_a_deleted_sheet_from_a_session_restores_its_cells() {
    let dir = Sandbox::new("sheet-undo");
    let file = dir.path("book.ods");
    let session = dir.path("session.json");
    ok(&["new", &s(&file)]);
    ok(&["add", &s(&file), "Data"]);
    ok(&["set", &s(&file), "Data.B2", "kept"]);

    ok(&["--session", &s(&session), "remove", &s(&file), "Data"]);
    ok(&["--session", &s(&session), "undo", &s(&file)]);
    assert_eq!(ok(&["get", &s(&file), "Data.B2"]).trim(), "kept");
}

/// The text sample's twin of the test above, and the same rule: a feature without a line in
/// `examples/sample-text.sh` is a feature nobody can see.
#[test]
fn the_text_sample_script_still_builds_its_document() {
    let dir = Sandbox::new("sample-text");
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the workspace root");

    let output = Command::new("bash")
        .arg(root.join("examples/sample-text.sh"))
        .arg(s(&dir.path("out")))
        .env("GRIND", env!("CARGO_BIN_EXE_grind"))
        .output()
        .expect("bash runs");
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    // Both forms exist, and the document reads back as the one the script described.
    assert!(dir.path("out/sample.fodt").exists());
    assert!(dir.path("out/sample.odt").exists());
    let doc = s(&dir.path("out/sample.fodt"));

    // The move happened: the appendix is now the first section.
    let outline = ok_top(&["text", "outline", &doc]);
    let first = outline.lines().next().expect("a heading");
    assert!(first.ends_with("Appendix"), "{outline}");

    // The bookmark still names the block it was put on, after a section moved above it.
    assert_eq!(
        ok_top(&["text", "get", &doc, "#addresses"]).trim(),
        "Addresses"
    );

    // Spaces XML would have collapsed came back.
    assert!(
        ok_top(&["text", "view", &doc]).contains("columns:    one    two    three"),
        "a run of spaces survived the round trip"
    );

    // And it is valid ODF, held against the same schema `sample.fods` is.
    let schema = root.join("doc/OpenDocument-v1.4-schema.rng");
    match Command::new("jing")
        .arg("-i")
        .arg(&schema)
        .arg(&doc)
        .output()
    {
        Ok(out) => assert!(
            out.status.success(),
            "sample.fodt is not valid ODF:\n{}",
            String::from_utf8_lossy(&out.stdout)
        ),
        Err(_) => eprintln!("skipping: no `jing` on PATH; schema validity unchecked"),
    }
}
