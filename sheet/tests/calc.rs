// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The tape (`doc/calc.md`): extracting a constant from a sheet cell, and promoted tapes held to
//! the schema (R2) and to LibreOffice's recalculation. `jing` and `soffice` on `PATH` are what can
//! skip.

use std::path::{Path, PathBuf};
use std::process::Command;

use grind_sheet::locale::Locale;
use grind_sheet::{App, CellValue, Form, Pos, Tape};

fn number(app: &App, row: u32, col: u32) -> CellValue {
    let v = app.get_viewport(0, row..row + 1, col..col + 1).unwrap();
    v.get(row, col).cloned().unwrap_or_default()
}

fn book() -> App {
    let app = App::new();
    app.set_cell(0, Pos::new(0, 0), 1200.0).unwrap();
    app.set_formula(0, Pos::new(0, 1), "of:=[.A1]*0.19+0.19*0")
        .unwrap();
    app
}

#[test]
fn a_constant_becomes_a_named_input_cell_in_one_undo_step() {
    let app = book();
    let before = app.save_bytes(Form::Flat).unwrap();
    assert!(
        app.extract_constant(0, Pos::new(0, 1), "0.19", None, Pos::new(0, 3), "vat")
            .is_err()
    );
    app.extract_constant(0, Pos::new(0, 1), "0.19", Some(1), Pos::new(0, 3), "vat")
        .unwrap();
    assert_eq!(
        app.formula(0, Pos::new(0, 1)).unwrap().as_deref(),
        Some("of:=[.A1]*vat+0.19*0")
    );
    assert_eq!(number(&app, 0, 3), CellValue::Number(0.19));
    assert!(
        app.names()
            .iter()
            .any(|(n, e)| n == "vat" && e == "[$Sheet1.$D$1]")
    );
    assert!(app.undo());
    assert_eq!(app.save_bytes(Form::Flat).unwrap(), before);
}

#[test]
fn extraction_never_overwrites_and_never_reuses_a_name() {
    let app = book();
    let at = Pos::new(0, 1);
    assert!(
        app.extract_constant(0, at, "0.19", Some(1), Pos::new(0, 0), "vat")
            .is_err()
    );
    app.set_name("vat", "1").unwrap();
    assert!(
        app.extract_constant(0, at, "0.19", Some(1), Pos::new(0, 3), "vat")
            .is_err()
    );
    assert!(
        app.extract_constant(0, at, "0.19", Some(1), Pos::new(0, 3), "C1")
            .is_err()
    );
    assert!(
        app.extract_constant(0, at, "7", None, Pos::new(0, 3), "seven")
            .is_err()
    );
}

#[test]
fn a_percent_goes_into_the_cell_as_its_value() {
    let app = App::new();
    app.set_formula(0, Pos::new(0, 0), "of:=200*5%").unwrap();
    app.extract_constant(0, Pos::new(0, 0), "5%", None, Pos::new(1, 0), "rate")
        .unwrap();
    assert_eq!(number(&app, 1, 0), CellValue::Number(0.05));
    assert_eq!(
        app.formula(0, Pos::new(0, 0)).unwrap().as_deref(),
        Some("of:=200*rate")
    );
}

#[test]
fn an_extraction_that_would_change_the_answer_is_refused() {
    let app = App::new();
    app.set_formula(0, Pos::new(0, 0), "of:=ISREF(3)").unwrap();
    let refused = app
        .extract_constant(0, Pos::new(0, 0), "3", None, Pos::new(1, 0), "three")
        .unwrap_err()
        .to_string();
    assert!(refused.contains("would change"), "{refused}");
    assert_eq!(
        app.formula(0, Pos::new(0, 0)).unwrap().as_deref(),
        Some("of:=ISREF(3)")
    );
    assert_eq!(app.names().len(), 0);
}

// --- promoted tapes ----------------------------------------------------------------------

/// Tapes that between them use inputs, names, handles in both directions, `%`, a range over
/// lines, text, logicals, an error value and a locale.
fn tapes() -> Vec<(&'static str, Tape)> {
    let mut out = Vec::new();
    let mut push = |label, locale: Option<&str>, lines: &[&str]| {
        let mut tape = Tape::new(locale.and_then(Locale::parse));
        for line in lines {
            tape.push(line).unwrap();
        }
        out.push((label, tape));
    };
    push(
        "invoice",
        None,
        &[
            "vat = 19%",
            "net = 1200",
            "discount = -50",
            "net + discount",
            "#4 * (1 + vat)",
            "ROUND(#5 / 12; 2)",
            "SUM(#4:#6)",
        ],
    );
    push(
        "functions",
        None,
        &[
            "r = 0.05",
            "PMT(r/12; 360; -250000)",
            "IF(#2 > 1000; \"high\"; \"low\")",
            "LEN(#3) & \" chars\"",
            "=r=0.05",
            "1/0",
            "IFERROR(#6; 0) + later",
            "later = SQRT(16)",
        ],
    );
    push(
        "de-DE",
        Some("de-DE"),
        &["preis = 1.234,5", "menge = 3", "preis * menge", "#3 * 0,9"],
    );
    out
}

fn have(program: &str, arg: &str) -> bool {
    Command::new(program)
        .arg(arg)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

struct Lab {
    dir: PathBuf,
}

impl Lab {
    fn new(what: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("sheet-calc-{what}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("out")).unwrap();
        Self { dir }
    }
}

impl Drop for Lab {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn promoted_tapes_are_valid_odf() {
    if Command::new("jing").output().is_err() {
        eprintln!("skipping: no `jing` on PATH; schema validity unchecked");
        return;
    }
    let lab = Lab::new("jing");
    let schema = Path::new(env!("CARGO_MANIFEST_DIR")).join("../doc/OpenDocument-v1.4-schema.rng");
    for (label, tape) in tapes() {
        let path = lab.dir.join(format!("{label}.fods"));
        tape.save(&path).unwrap();
        let out = Command::new("jing")
            .arg("-i")
            .arg(&schema)
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{label}: {}",
            String::from_utf8_lossy(&out.stdout)
        );
    }
}

#[test]
fn promoted_tapes_read_back_the_same_in_every_form() {
    let lab = Lab::new("forms");
    for (label, tape) in tapes() {
        for ext in ["fods", "ods", "grind"] {
            let path = lab.dir.join(format!("{label}.{ext}"));
            tape.save(&path).unwrap();
            let app = App::new();
            app.open_file(&path).unwrap();
            let (rows, cols) = app.used_extent(0).unwrap();
            let view = app.get_viewport(0, 0..rows, 0..cols).unwrap();
            let promoted = tape.promote().unwrap();
            for (i, line) in tape.lines().iter().enumerate() {
                let row = u32::try_from(i).unwrap();
                assert_eq!(
                    view.get(row, 1).cloned().unwrap_or_default(),
                    promoted.sheets[0].get(Pos::new(row, 1)),
                    "{label}.{ext} #{}",
                    line.handle
                );
                assert_eq!(
                    app.formula(0, Pos::new(row, 1)).unwrap().as_deref(),
                    promoted.sheets[0].formula(Pos::new(row, 1)),
                    "{label}.{ext} #{}",
                    line.handle
                );
            }
            assert_eq!(
                app.names().len(),
                tape.lines().iter().filter(|l| l.name.is_some()).count()
            );
        }
    }
}

#[test]
fn libreoffice_recalculates_promoted_tapes_to_the_same_values() {
    if !have("soffice", "--version") {
        eprintln!("skipping: no soffice on PATH");
        return;
    }
    let lab = Lab::new("lo");
    for (label, tape) in tapes() {
        // No cached values: LibreOffice has to compute every formula itself.
        let mut doc = tape.promote().unwrap();
        let formulas: Vec<Pos> = doc.sheets[0].formulas().map(|(p, _)| p).collect();
        for &p in &formulas {
            doc.sheets[0].set(p, CellValue::Empty);
        }
        let app = App::new();
        app.open_document(doc).unwrap();
        let input = lab.dir.join(format!("{label}.fods"));
        app.save_file(&input).unwrap();
        let status = Command::new("soffice")
            .arg("--headless")
            .arg(format!(
                "-env:UserInstallation=file://{}",
                lab.dir.join("profile").display()
            ))
            .args(["--convert-to", "fods", "--outdir"])
            .arg(lab.dir.join("out"))
            .arg(&input)
            .status()
            .unwrap();
        assert!(status.success());
        let back = App::new();
        back.open_file(&lab.dir.join("out").join(format!("{label}.fods")))
            .unwrap();
        let ours = tape.promote().unwrap();
        for p in formulas {
            let theirs = number(&back, p.row, p.col);
            let mine = ours.sheets[0].get(p);
            let agrees = match (&theirs, &mine) {
                (CellValue::Number(x), CellValue::Number(y)) => {
                    (x - y).abs() <= 1e-12 * x.abs().max(y.abs()).max(1.0)
                }
                (CellValue::Text(t), CellValue::Text(m)) if m.starts_with('#') => {
                    t == m || t.starts_with("Err:")
                }
                _ => theirs == mine,
            };
            assert!(
                agrees,
                "{label} row {}: LibreOffice {theirs:?}, the tape {mine:?}",
                p.row + 1
            );
        }
    }
}
