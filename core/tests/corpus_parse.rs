// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Loop B, first half — does the parser understand what the world writes?
//!
//! Evaluating a formula is phase 4's exit criterion; *parsing* every formula in the corpus
//! is the part that can be checked today, and it is the cheap way to find out whether the
//! grammar in `formula/` matches the grammar in the wild rather than the one in my head.
//! 500+ per-function fixtures and 350+ real documents is a far broader test than any set of
//! hand-written cases.
//!
//! Point it at a LibreOffice checkout:
//!
//!     SHEET_LO_CORPUS=/path/to/libreoffice/core/sc/qa/unit/data cargo test

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use sheet_core::formula::lex::Token;
use sheet_core::formula::parse::parse;

const DEFAULT_CORPUS: &str = "/home/florian/code/github.com/LibreOffice/core/sc/qa/unit/data";

/// `functions/` is loop B's own corpus; the other two come along because real documents
/// contain formulas the per-function fixtures never exercise.
const DIRS: [&str; 3] = ["functions", "ods", "fods"];

fn corpus_root() -> Option<PathBuf> {
    let root = PathBuf::from(
        std::env::var("SHEET_LO_CORPUS").unwrap_or_else(|_| DEFAULT_CORPUS.to_owned()),
    );
    root.is_dir().then_some(root)
}

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
            Some("ods" | "fods")
        ) {
            out.push(path);
        }
    }
}

/// Why a formula that did not parse is not a parser bug.
///
/// Consulted only after a failure, and every arm is a *syntactic* property rather than a
/// file name — a corpus file must never be special-cased (CLAUDE.md). Two kinds live here:
/// constructs §2.3.2 puts outside the Small Group, and formulas the corpus contains that
/// §5's grammar does not describe at all.
fn excused(formula: &str, message: &str) -> Option<&'static str> {
    if formula.contains('{') {
        return Some("inline array (§5.13, excluded by §2.3.2)");
    }
    if formula.contains('~') {
        return Some("reference union `~` (excluded by §2.3.2 G)");
    }
    if message.contains("unexpected character '\\''") {
        return Some("quoted label (§5.10, optional and unimplemented)");
    }
    if juxtaposed(formula) {
        // `of:=NOT(0)NOT(0)`, `of:=([.A4]=[.E4])AND([.B4]=[.F4])`: two operands with no
        // operator between them. §5.2's Expression production has no such form. LO reads
        // these back — they carry cached values — but the grammar does not allow them, and
        // guessing an operator is how a wrong answer gets computed confidently.
        return Some("two operands with no operator (not §5.2 Expression)");
    }
    if message == "expected a value" && formula.contains(";*") {
        return Some("an operator where a parameter belongs (not §5.2 Expression)");
    }
    None
}

/// Does the formula put two operands next to each other with no operator between?
///
/// Answered by re-lexing rather than by pattern-matching the text, so that a `)` inside a
/// string or a sheet name cannot fake it.
fn juxtaposed(formula: &str) -> bool {
    let Ok(tokens) = sheet_core::formula::lex::lex(formula.trim_start_matches("of:=")) else {
        return false;
    };
    tokens.windows(2).any(|pair| {
        let ends_operand = matches!(
            pair[0],
            Token::Number(_)
                | Token::Text(_)
                | Token::Error(_)
                | Token::Ref(_)
                | Token::Name(_)
                | Token::RParen
        );
        let starts_operand = matches!(
            pair[1],
            Token::Number(_)
                | Token::Text(_)
                | Token::Error(_)
                | Token::Ref(_)
                | Token::Name(_)
                | Token::Func(_)
                | Token::LParen
        );
        ends_operand && starts_operand
    })
}

#[test]
fn every_corpus_formula_parses() {
    let Some(root) = corpus_root() else {
        eprintln!(
            "skipping: no LibreOffice corpus at {DEFAULT_CORPUS}; \
             set SHEET_LO_CORPUS to run loop B's parse half"
        );
        return;
    };

    let mut files = Vec::new();
    for dir in DIRS {
        collect(&root.join(dir), &mut files);
    }
    files.sort();
    assert!(!files.is_empty(), "corpus at {} is empty", root.display());

    let mut total = 0usize;
    // Grouped by reason so the output is a work list rather than a wall of near-duplicates.
    let mut excuses: BTreeMap<&str, usize> = BTreeMap::new();
    let mut failures: BTreeMap<String, (usize, Vec<String>)> = BTreeMap::new();
    for path in &files {
        let Ok(doc) = sheet_core::read_file(path) else {
            continue; // loop A owns reading; a document it cannot open is its problem.
        };
        for sheet in &doc.sheets {
            for (_, formula) in sheet.formulas() {
                total += 1;
                let Err(e) = parse(formula) else { continue };
                if let Some(reason) = excused(formula, &e.message) {
                    *excuses.entry(reason).or_default() += 1;
                    continue;
                }
                let entry = failures.entry(e.message).or_default();
                entry.0 += 1;
                if entry.1.len() < 3 && !entry.1.iter().any(|f| f == formula) {
                    entry.1.push(formula.to_owned());
                }
            }
        }
    }

    let failed: usize = failures.values().map(|(n, _)| n).sum();
    let excluded: usize = excuses.values().sum();
    eprintln!(
        "loop B (parse): {total} formulas in {} documents, {} parsed, \
         {excluded} excluded, {failed} failed",
        files.len(),
        total - failed - excluded,
    );
    for (reason, count) in &excuses {
        eprintln!("  {count:>5}  excluded: {reason}");
    }
    for (message, (count, examples)) in &failures {
        eprintln!("  {count:>5}  {message}");
        for example in examples {
            eprintln!("           {example}");
        }
    }

    assert!(failed == 0, "{failed} formulas failed to parse");
}
