// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The generator — `doc/dsl.md` layer 1, D7. **\[GENERIC\]** in neither direction: this crate
//! knows both applications, and nothing that reads a document knows this crate.
//!
//! A script is **compiled into a document**. `grind build model.rhai -o model.fods` runs it,
//! takes the value it returns and writes that; the script never opens a document, never
//! touches a file, and is not recoverable from what it produced (§1 — the arrow points one
//! way, as it does for Typst and for Jsonnet). That is the whole layer boundary, and it is
//! what keeps the language choice reversible: the artefact is a document, never the script.
//!
//! **This is not a macro, and §2 is the argument rather than this comment.** The distinction
//! is who runs it, when, and with whose authority: a macro runs when a *reader* opens a
//! document, this runs when an *author* types `grind build`. Two things enforce it.
//!
//! * **R11 — no evaluator on any read path.** Nothing in `grind-core`, `grind-sheet`,
//!   `grind-text` or any shell depends on this crate, so no code path that opens a document
//!   can evaluate anything. `tests/manifest.rs` reads every manifest in the workspace and
//!   fails the build the day one of them names `grind-build`, which is how R8 is checked one
//!   crate over.
//! * **No I/O, no clock, no randomness, and bounded** ([`engine`]). Two of those are facts
//!   about the *build* rather than about this code remembering to unregister something: see
//!   `Cargo.toml`, where Rhai's default features are turned off to take the clock out of the
//!   language and the runtime seed out of its hasher.
//!
//! ## What a script says
//!
//! It builds a value tree and returns it — never a string of projection text, because a
//! template that interpolates a value containing a quote produces a broken file, which is the
//! injection problem every stringly-typed generator has (§4.1). The host serialises, so the
//! question cannot arise.
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
//! [`sheet`] is the spreadsheet's vocabulary and [`text`] the word processor's; each module's
//! own comment is the reference for what it registers. The script's **return value** is the
//! document, and the three things it may be are the three [`Artifact`] arms below.
//!
//! ## What it returns, and what the host does with it
//!
//! [`build`] hands back an `App` — the same one a shell drives — rather than a `Document`,
//! because a generated document is nearly always asked one more question before it is
//! written: recalculate it, lint it, or (D8) assert something about a total. Writing it is
//! then the ordinary `save_file`, and `grind build` is a dozen lines in `cli/src/main.rs`.

pub mod data;
pub mod engine;
pub mod hint;
pub mod sheet;
pub mod text;

use std::fmt;
use std::rc::Rc;

use rhai::Dynamic;

pub use data::{Data, Directory, NoData};

/// What a script produced: one document, of one of the two kinds this suite has.
///
/// An `App` rather than a `Document` — see the module comment. The kind is decided by what the
/// script *returned*, never by the output's name, which is the same rule
/// `grind_core::kind` follows on the way in.
pub enum Artifact {
    Spreadsheet(grind_sheet::App),
    Text(grind_text::App),
}

impl Artifact {
    /// What kind of document this is, for a message or a report.
    pub fn kind(&self) -> grind_core::kind::DocumentKind {
        match self {
            Artifact::Spreadsheet(_) => grind_core::kind::DocumentKind::Spreadsheet,
            Artifact::Text(_) => grind_core::kind::DocumentKind::Text,
        }
    }
}

/// A script that would not run, or would not produce a document.
///
/// The position is the script's own, because a generator whose errors have no line number is
/// a generator nobody debugs — §2's "a build error with a line number, not a hang" is about
/// the bounded ones, and it would be a strange promise to keep only for those.
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
    fn at(script: &str, message: impl Into<String>) -> Self {
        Error {
            script: script.to_owned(),
            line: None,
            column: None,
            message: message.into(),
        }
    }
}

/// **What an editor needs to help somebody write a script**: every function this build
/// registers, with its parameter names, its types and its documentation, as a Rhai definition
/// file (`.d.rhai`).
///
/// This is Rhai's own mechanism for the purpose and the language server's own format, so
/// nothing here is invented: `grind definitions > grind.d.rhai` beside a script is enough for
/// completion and hover in an editor that speaks Rhai (`doc/generator-spec.md` §9).
///
/// The standard library is left out. A script's own vocabulary is what somebody is trying to
/// learn, and burying forty of those functions in six hundred of Rhai's own is how a reference
/// becomes unreadable — an editor already knows the standard ones.
pub fn definitions() -> String {
    engine::engine(Rc::new(NoData))
        .definitions()
        .include_standard_packages(false)
        .single_file()
}

/// The same vocabulary as **VS Code snippets**, for the editor most people actually have open.
///
/// [`definitions`] is the right answer and needs a **language server** to read it; measured
/// against the published Rhai extension, there is not one (`doc/editor-setup.md` says what was
/// measured and how). A snippet file needs no server at all — VS Code reads `*.code-snippets`
/// out of a workspace itself, given only the language id a syntax extension already registers —
/// and carries a name, a call with its parameters as tab stops, and the documentation `hint`
/// already required. That is completion and inline documentation for the whole host API, from
/// the same source as the definition file, which is the property worth having: **both are the
/// engine's own answer**, so neither can describe a function that is not there.
///
/// The parse is of [`definitions`]'s output rather than of this crate's source, for that
/// reason. Its shape is Rhai's: doc comment lines, then one `fn` line per registration.
///
/// **A method is told from a free function by its first parameter's type.** Rhai spells a host
/// type with a capital (`Sheet`, `Row`, `Format`) and a built-in without one (`string`, `int`,
/// `array`), and every constructor here takes a built-in or nothing — so a leading capital is
/// the receiver, and `s.push(…)` gets a snippet with the receiver dropped rather than one that
/// asks for the sheet twice.
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

/// Run a script and return the document it built, with no data to read.
///
/// `script` is what to call the source in an error — a path, usually. Nothing here reads a
/// file: rule 5 has no exception for a generator, and `grind-cli` is the one that owns paths.
pub fn build(source: &str, script: &str) -> Result<Artifact, Error> {
    build_with(source, script, Rc::new(NoData))
}

/// [`build`], with somewhere for `json(…)` to read from.
///
/// The data is a [`Data`] rather than a path for the reason `build` takes a string rather than
/// a file: this crate has no filesystem. `grind build` hands over a [`Directory`] rooted at the
/// script's own directory, and `build/src/data.rs` is where the four walls around that are.
pub fn build_with(source: &str, script: &str, data: Rc<dyn Data>) -> Result<Artifact, Error> {
    let engine = engine::engine(data);
    let value = engine
        .eval::<Dynamic>(source)
        .map_err(|e| self::position(script, &e))?;
    materialise(value, script)
}

/// The returned tree as a document.
///
/// Three accepted shapes, and the third is sugar: a script that builds one sheet may return
/// the sheet, because `spreadsheet()` around a single `sheet(…)` is ceremony. `doc/dsl.md`
/// §4.2's sketch ends with a bare `s`, and it runs.
fn materialise(value: Dynamic, script: &str) -> Result<Artifact, Error> {
    let named = value.type_name().to_owned();
    if let Some(book) = value.clone().try_cast::<sheet::Book>() {
        return sheet::materialise(&book)
            .map(Artifact::Spreadsheet)
            .map_err(|e| Error::at(script, e));
    }
    if let Some(one) = value.clone().try_cast::<sheet::Sheet>() {
        return sheet::materialise(&sheet::Book::of(one))
            .map(Artifact::Spreadsheet)
            .map_err(|e| Error::at(script, e));
    }
    if let Some(doc) = value.try_cast::<text::Doc>() {
        return text::materialise(&doc)
            .map(Artifact::Text)
            .map_err(|e| Error::at(script, e));
    }
    Err(Error::at(
        script,
        format!(
            "a script has to end with the document it built — a `sheet(…)`, a `spreadsheet()` \
             or a `text()`. This one ended with {named}"
        ),
    ))
}

/// A Rhai failure as ours, with its position carried across.
fn position(script: &str, error: &rhai::EvalAltResult) -> Error {
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
