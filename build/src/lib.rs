// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The generator: a Rhai script that **returns** a document. `grind build model.rhai -o
//! model.fods` runs it and writes the value it returns; the script never opens a document or
//! touches a file, and is not recoverable from what it produced.
//!
//! Not a macro: a macro runs when a *reader* opens a document, this when an *author* types `grind
//! build`. Two things hold that line:
//!
//! * **R11 — no evaluator on any read path.** Nothing that opens a document depends on this crate;
//!   `tests/manifest.rs` reads every manifest in the workspace to enforce it.
//! * **No I/O, no clock, no randomness, and bounded** ([`engine`]), the clock and the hasher's
//!   seed removed by feature flags in `Cargo.toml`.
//!
//! ```rhai
//! let s = sheet("Sales");
//! s.push(row(["Region", "Q1", "Q2"]).bold());
//! s.push(row(["North", 400, 450]).format(currency("EUR")));
//! s.push(row(["South", 380, 410]).format(currency("EUR")));
//! s.push(row(["Total", sum_above(), sum_above()]).bold());
//! s
//! ```
//!
//! A script builds a value tree, never projection text, so a value with a quote in it cannot break
//! the file. [`sheet`] is the spreadsheet's vocabulary and [`text`] the word processor's.
//! [`build`] hands back an `App` rather than a `Document`, since a generated document is nearly
//! always recalculated, linted or tested before it is written; [`test`](mod@test) is `grind test`.

pub mod chart;
pub mod data;
pub mod engine;
pub mod hint;
pub mod numbers;
pub mod sheet;
pub mod test;
pub mod text;

use std::fmt;
use std::rc::Rc;

use rhai::Dynamic;

pub use data::{Data, Directory, NoData};

/// What a script produced. The kind is decided by what the script returned, never by the output's
/// name.
pub enum Artifact {
    // Both boxed: each `App` is a few hundred bytes and they differ by hundreds, which is a
    // lopsided enum for a value made once per build.
    Spreadsheet(Box<grind_sheet::App>),
    Text(Box<grind_text::App>),
    /// **Many documents**: a script ending with a map of name to document is a series, and `grind
    /// build -o DIR` writes `DIR/<name>.<ext>` for each. All of one kind, never nested; `Rc`
    /// because a test asks for one by name.
    Series(Vec<(String, Rc<Artifact>)>),
}

impl Artifact {
    /// What kind of document this is, for a message or a report.
    pub fn kind(&self) -> grind_core::kind::DocumentKind {
        match self {
            Artifact::Spreadsheet(_) => grind_core::kind::DocumentKind::Spreadsheet,
            Artifact::Text(_) => grind_core::kind::DocumentKind::Text,
            // A series is of one kind, which `materialise` checks; it has at least one member.
            Artifact::Series(all) => all[0].1.kind(),
        }
    }
}

/// A script that would not run, or would not produce a document, with the script's own position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    /// The script's name, as the caller spelled it — a path, or something like `<script>`.
    pub script: String,
    /// 1-based, when Rhai knew one. A failure while *materialising* the returned tree has no
    /// line: by then the script has finished and the fault is in what it asked for.
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub message: String,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.line, self.column) {
            (Some(line), Some(column)) => write!(f, "{}:{line}:{column}: ", self.script)?,
            (Some(line), None) => write!(f, "{}:{line}: ", self.script)?,
            _ => write!(f, "{}: ", self.script)?,
        }
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

impl Error {
    pub(crate) fn at(script: &str, message: impl Into<String>) -> Self {
        Error {
            script: script.to_owned(),
            line: None,
            column: None,
            message: message.into(),
        }
    }
}

/// Every function a script may call, with its parameter names, types and documentation, as a Rhai
/// definition file (`.d.rhai`) for an editor. Rhai's standard library is left out: a script's own
/// vocabulary is what somebody is learning, and an editor already knows the rest.
pub fn definitions() -> String {
    // `grind test`'s vocabulary too: a test is written in the same file as the model, so the
    // editor open on it should complete `assert_eq` as readily as `row`.
    let mut engine = engine::engine(Rc::new(NoData));
    test::register(&mut engine);
    engine
        .definitions()
        .include_standard_packages(false)
        .single_file()
}

/// The same vocabulary as **VS Code snippets**, which need no language server — the published Rhai
/// extension has none. Parsed out of [`definitions`]'s output rather than this crate's source, so
/// both are the engine's own answer and neither can describe a function that is not there.
///
/// A method is told from a free function by its first parameter's type: a host type is capitalised
/// (`Sheet`, `Row`), a built-in is not, and no constructor takes a host type — so `s.push(…)` gets
/// a snippet without the receiver.
pub fn snippets() -> String {
    let mut out = serde_json::Map::new();
    let mut doc: Vec<String> = Vec::new();
    for line in definitions().lines() {
        let line = line.trim();
        if let Some(text) = line.strip_prefix("///") {
            doc.push(text.trim().to_owned());
            continue;
        }
        let Some(signature) = line.strip_prefix("fn ").and_then(|s| s.strip_suffix(';')) else {
            // A blank line between registrations, or `module static;` — either way whatever
            // documentation was accumulating belongs to nothing and is dropped.
            if !line.is_empty() {
                doc.clear();
            }
            continue;
        };
        if let Some(entry) = snippet(signature, &doc) {
            out.insert(signature.to_owned(), entry);
        }
        doc.clear();
    }
    let mut text = serde_json::to_string_pretty(&serde_json::Value::Object(out))
        .expect("a map of strings serialises");
    text.push('\n');
    text
}

/// One entry of [`snippets`], from one signature line and the doc comment above it.
fn snippet(signature: &str, doc: &[String]) -> Option<serde_json::Value> {
    // `get rows(_: Sheet) -> int` is a property: it is typed without a call, so its body is the
    // bare name and its parameters are not offered.
    let property = signature.starts_with("get ") || signature.starts_with("set ");
    let signature = signature
        .trim_start_matches("get ")
        .trim_start_matches("set ");
    let (name, rest) = signature.split_once('(')?;
    let (params, _) = rest.split_once(')')?;

    let mut params: Vec<&str> = params
        .split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    // The receiver, if there is one — see this function's caller for the rule.
    if params
        .first()
        .and_then(|p| p.split(':').nth(1))
        .and_then(|kind| kind.trim().chars().next())
        .is_some_and(char::is_uppercase)
    {
        params.remove(0);
    }

    let body = if property {
        name.to_owned()
    } else {
        let taken: Vec<String> = params
            .iter()
            .enumerate()
            .map(|(at, param)| {
                let named = param.split(':').next().unwrap_or(param).trim();
                format!("${{{}:{}}}", at + 1, named)
            })
            .collect();
        format!("{name}({})", taken.join(", "))
    };

    Some(serde_json::json!({
        "scope": "rhai",
        "prefix": name,
        "body": [body],
        "description": doc.join("\n"),
    }))
}

/// Run a script and return the document it built, with no data to read. `script` names the source
/// in an error — a path, usually; nothing here reads a file.
pub fn build(source: &str, script: &str) -> Result<Artifact, Error> {
    build_with(source, script, Rc::new(NoData))
}

/// [`build`], with a [`Data`] for `json(…)` to read from.
pub fn build_with(source: &str, script: &str, data: Rc<dyn Data>) -> Result<Artifact, Error> {
    let engine = engine::engine(data);
    let value = engine
        .eval::<Dynamic>(source)
        .map_err(|e| self::position(script, &e))?;
    materialise(value, script)
}

/// Whether a script parses — compiled by the same engine `grind build` runs, never evaluated. What
/// an editor may do with a script it has open: a script runs because a person typed `grind build`
/// or `grind test`, never because a file was opened, so the language server gets this and not
/// [`build`]. The engine is the real one because its limits are part of the language.
pub fn check(source: &str, script: &str) -> Result<(), Error> {
    let mut engine = engine::engine(Rc::new(NoData));
    test::register(&mut engine);
    engine.compile(source).map(|_| ()).map_err(|error| {
        let at = error.position();
        Error {
            script: script.to_owned(),
            line: at.line().map(|l| l as u32),
            column: at.position().map(|c| c as u32),
            message: without_position(&error.to_string()),
        }
    })
}

/// The returned tree as a document. A script building one sheet may return the bare sheet, since
/// `spreadsheet()` around a single `sheet(…)` is ceremony.
pub(crate) fn materialise(value: Dynamic, script: &str) -> Result<Artifact, Error> {
    if value.is::<rhai::Map>() {
        return series(value.cast::<rhai::Map>(), script);
    }
    single(value, script)
}

/// A map of name to document: the files a series writes. A name is a file's stem, so what a file
/// system would mangle is refused here rather than halfway through writing the series.
fn series(map: rhai::Map, script: &str) -> Result<Artifact, Error> {
    if map.is_empty() {
        return Err(Error::at(
            script,
            "a series of documents needs at least one",
        ));
    }
    let mut all: Vec<(String, Rc<Artifact>)> = Vec::new();
    for (name, value) in map {
        let valid = !name.is_empty()
            && name.len() <= 100
            && !name.starts_with('.')
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
        if !valid {
            return Err(Error::at(
                script,
                format!(
                    "{name:?} cannot be a file name: a series is named by letters, digits, `-`, `_` and `.`"
                ),
            ));
        }
        if value.is::<rhai::Map>() {
            return Err(Error::at(
                script,
                format!("{name}: a series is a map of documents, not of maps"),
            ));
        }
        let artifact = single(value, script).map_err(|mut e| {
            e.message = format!("{name}: {}", e.message);
            e
        })?;
        all.push((name.to_string(), Rc::new(artifact)));
    }
    let kind = all[0].1.kind();
    if let Some((name, _)) = all.iter().find(|(_, one)| one.kind() != kind) {
        return Err(Error::at(
            script,
            format!("{name} is not a {kind:?} like the others: a series is one kind of document"),
        ));
    }
    Ok(Artifact::Series(all))
}

fn single(value: Dynamic, script: &str) -> Result<Artifact, Error> {
    let named = value.type_name().to_owned();
    if let Some(book) = value.clone().try_cast::<sheet::Book>() {
        return sheet::materialise(&book)
            .map(|app| Artifact::Spreadsheet(Box::new(app)))
            .map_err(|e| Error::at(script, e));
    }
    if let Some(one) = value.clone().try_cast::<sheet::Sheet>() {
        return sheet::materialise(&sheet::Book::of(one))
            .map(|app| Artifact::Spreadsheet(Box::new(app)))
            .map_err(|e| Error::at(script, e));
    }
    if let Some(doc) = value.try_cast::<text::Doc>() {
        return text::materialise(&doc)
            .map(|app| Artifact::Text(Box::new(app)))
            .map_err(|e| Error::at(script, e));
    }
    Err(Error::at(
        script,
        format!(
            "a script has to end with the document it built — a `sheet(…)`, a `spreadsheet()` \
             or a `text()` — or with a map of names to documents. This one ended with {named}"
        ),
    ))
}

/// A Rhai failure as ours, with its position carried across.
pub(crate) fn position(script: &str, error: &rhai::EvalAltResult) -> Error {
    let at = error.position();
    Error {
        script: script.to_owned(),
        line: at.line().map(|l| l as u32),
        column: at.position().map(|c| c as u32),
        message: without_position(&error.to_string()),
    }
}

/// Rhai appends ` (line 3, position 5)` to its own `Display`, and [`Error`] prints the
/// position in front where a compiler puts one. Saying it twice reads like two errors.
fn without_position(message: &str) -> String {
    match message.rsplit_once(" (line ") {
        Some((head, tail)) if tail.ends_with(')') => head.to_owned(),
        _ => message.to_owned(),
    }
}
