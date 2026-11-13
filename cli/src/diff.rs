// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `grind diff`: two versions of one document, compared by the cores (`grind_sheet::diff`,
//! `grind_text::diff`) and printed here — as lines a person reads, or as JSON.

use std::path::{Path, PathBuf};

use grind_sheet::diff::{CellChange, Content, SheetChange};
use grind_sheet::{App, DocumentKind};
use grind_text::diff::{Change, Piece};
use serde::Serialize;

/// Where git passes `/dev/null` for the side of a diff that does not exist.
const NOTHING: &str = "/dev/null";

#[derive(Debug, Serialize)]
pub struct DiffReport {
    /// The path git names, under `--git`; the later file otherwise.
    pub path: String,
    pub kind: &'static str,
    pub same: bool,
    /// `added` or `removed` when one side is git's `/dev/null`, and nothing is compared.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub whole: Option<&'static str>,
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    pub sheet: Option<grind_sheet::diff::Diff>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub blocks: Vec<BlockReport>,
    #[serde(skip)]
    exit_code: bool,
    #[serde(skip)]
    git: bool,
}

#[derive(Debug, Serialize)]
pub struct BlockReport {
    pub change: &'static str,
    #[serde(skip)]
    mark: char,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<SideReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<SideReport>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub words: Vec<PieceReport>,
}

#[derive(Debug, Serialize)]
pub struct SideReport {
    pub at: String,
    pub kind: String,
    pub text: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PieceReport {
    Same(String),
    Removed(String),
    Added(String),
}

pub fn run(files: &[PathBuf], git: bool, exit_code: bool) -> Result<DiffReport, String> {
    let (path, before, after) = match (git, files) {
        (false, [before, after]) => (show(after), before, after),
        (true, [path, before, _, _, after, _, _]) => (show(path), before, after),
        (false, _) => return Err("diff takes two files: the earlier version and the later".into()),
        (true, _) => {
            return Err(format!(
                "--git takes the seven arguments git passes to a diff command, not {}",
                files.len()
            ));
        }
    };
    let gone = |file: &Path| file == Path::new(NOTHING);
    let report = |kind: DocumentKind| DiffReport {
        path: path.clone(),
        kind: kind.label(),
        same: false,
        whole: None,
        sheet: None,
        blocks: Vec::new(),
        exit_code,
        git,
    };
    if gone(before) || gone(after) {
        let present = if gone(before) { after } else { before };
        return Ok(DiffReport {
            whole: Some(if gone(before) { "added" } else { "removed" }),
            ..report(super::document_kind(present)?)
        });
    }
    let kind = super::document_kind(before)?;
    if super::document_kind(after)? != kind {
        return Err(format!(
            "{} and {} are different kinds of document",
            before.display(),
            after.display()
        ));
    }
    let mut out = report(kind);
    match kind {
        DocumentKind::Spreadsheet => {
            let diff = open_sheet(after)?.diff(&open_sheet(before)?);
            out.same = diff.is_empty();
            out.sheet = Some(diff);
        }
        DocumentKind::Text => {
            let diff = super::open_text(after)?.diff(&super::open_text(before)?);
            out.same = diff.is_empty();
            out.blocks = diff.blocks.into_iter().map(block).collect();
        }
        kind => return Err(super::unsupported(before, Some(kind))),
    }
    Ok(out)
}

/// A spreadsheet as it is on disk — no session's history, which belongs to one file only.
fn open_sheet(file: &Path) -> Result<App, String> {
    let app = App::new();
    app.open_file(file)
        .map_err(|e| format!("{}: {e}", file.display()))?;
    Ok(app)
}

fn show(path: &Path) -> String {
    path.display().to_string()
}

fn block(change: grind_text::diff::BlockChange) -> BlockReport {
    let side = |side: grind_text::diff::Side| SideReport {
        at: side.at,
        kind: side.kind,
        text: side.text,
    };
    BlockReport {
        change: change.change.label(),
        mark: match change.change {
            Change::Added => '+',
            Change::Removed => '-',
            Change::Changed => '~',
            Change::Moved => '>',
            Change::Reformatted => '*',
        },
        before: change.before.map(side),
        after: change.after.map(side),
        words: change
            .words
            .into_iter()
            .map(|piece| match piece {
                Piece::Same(s) => PieceReport::Same(s),
                Piece::Removed(s) => PieceReport::Removed(s),
                Piece::Added(s) => PieceReport::Added(s),
            })
            .collect(),
    }
}

impl DiffReport {
    pub fn failed(&self) -> bool {
        self.exit_code && !self.same
    }

    pub fn print_text(&self) {
        if self.git {
            println!("{} ({})", self.path, self.kind);
        }
        if let Some(whole) = self.whole {
            println!("{whole}: the whole document");
            return;
        }
        if self.same {
            // Nothing is the diff tool's answer for no difference; outside git, say it.
            if !self.git {
                println!("no differences");
            }
            return;
        }
        if let Some(diff) = &self.sheet {
            print_sheet(diff);
        }
        for change in &self.blocks {
            print_block(change);
        }
    }
}

fn print_sheet(diff: &grind_sheet::diff::Diff) {
    for sheet in &diff.sheets {
        match (sheet.change, &sheet.renamed_from) {
            (SheetChange::Added, _) => println!("+ {} ({} cells)", sheet.name, sheet.cells),
            (SheetChange::Removed, _) => println!("- {} ({} cells)", sheet.name, sheet.cells),
            (SheetChange::Changed, Some(was)) => println!("~ {was} → {}", sheet.name),
            (SheetChange::Changed, None) => println!("~ {}", sheet.name),
        }
        for cell in &sheet.edited {
            println!("  {}", edit(cell));
        }
        if !sheet.recomputed.is_empty() {
            let at = grind_sheet::diff::ranges(sheet.recomputed.iter().map(|c| c.pos).collect());
            println!(
                "  {} computed {} as a result: {}",
                sheet.recomputed.len(),
                match sheet.recomputed.len() {
                    1 => "cell changed",
                    _ => "cells changed",
                },
                at.join(", ")
            );
        }
        if !sheet.restyled.is_empty() {
            println!("  look changed: {}", sheet.restyled.join(", "));
        }
        if !sheet.also.is_empty() {
            println!("  also changed: {}", sheet.also.join(", "));
        }
    }
    for name in &diff.names {
        match (&name.before, &name.after) {
            (None, Some(now)) => println!("+ name {} = {now}", name.name),
            (Some(was), None) => println!("- name {} = {was}", name.name),
            (Some(was), Some(now)) => println!("~ name {}: {was} → {now}", name.name),
            (None, None) => {}
        }
    }
}

/// `~ D4: =[.B4]-[.C4] (100) → 250`, `+ B9: "Note"`, `- B10: 5`.
fn edit(cell: &CellChange) -> String {
    match (&cell.before, &cell.after) {
        (None, Some(now)) => format!("+ {}: {}", cell.at, content(now)),
        (Some(was), None) => format!("- {}: {}", cell.at, content(was)),
        (Some(was), Some(now)) => format!("~ {}: {} → {}", cell.at, content(was), content(now)),
        (None, None) => format!("~ {}", cell.at),
    }
}

fn content(content: &Content) -> String {
    match (&content.formula, content.value.is_empty()) {
        (Some(formula), false) => format!("{formula} ({})", content.value),
        (Some(formula), true) => formula.clone(),
        (None, _) => content.value.clone(),
    }
}

fn print_block(change: &BlockReport) {
    let Some(side) = change.after.as_ref().or(change.before.as_ref()) else {
        return;
    };
    let kind = match (&change.before, &change.after) {
        (Some(was), Some(now)) if was.kind != now.kind => format!("{} → {}", was.kind, now.kind),
        _ => side.kind.clone(),
    };
    let note = match (change.change, &change.before) {
        ("moved", Some(was)) => format!(", moved from {}", was.at),
        ("reformatted", _) => ", reformatted".to_owned(),
        _ => String::new(),
    };
    let text = if change.words.is_empty() {
        side.text.clone()
    } else {
        change
            .words
            .iter()
            .map(|piece| match piece {
                PieceReport::Same(s) => s.clone(),
                PieceReport::Removed(s) => format!("[-{s}-]"),
                PieceReport::Added(s) => format!("{{+{s}+}}"),
            })
            .collect()
    };
    println!("{} {} {kind}{note}: {text}", change.mark, side.at);
}
