// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A `.rhai` script: completion and hover from the host vocabulary, and syntax errors.
//!
//! **Nothing here analyses Rhai.** `doc/dsl.md` §9 declines to write a Rhai language server,
//! and this is not one: it does not know what `s` is, what a variable holds or what a function
//! of the script's own returns. It knows the vocabulary `grind build` registers — read out of
//! `grind definitions`' own text, the file `snippets()` is made from too — and offers it, with
//! the documentation every registration is required to carry. After a `.` it offers the
//! methods and properties, anywhere else the free functions; that one distinction is the
//! difference between `s.pu` offering `push` and offering everything that starts with `pu`.
//!
//! **Nothing here runs a script either.** The syntax check is [`crate::Host::check_script`],
//! which compiles and never evaluates (`grind_build::check`): a script runs because a person
//! typed `grind build` or `grind test`, never because an editor opened the file.

use lsp_types::{
    CompletionItem, CompletionItemKind, CompletionItemLabelDetails, Diagnostic, DiagnosticSeverity,
    Documentation, Hover, HoverContents, InsertTextFormat, MarkupContent, MarkupKind, Position,
    Range,
};

use crate::lines::Lines;

/// One registration, as the definition file spells it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Function {
    pub name: String,
    /// Every parameter, the receiver included, as `(name, type)`.
    pub params: Vec<(String, String)>,
    pub returns: Option<String>,
    pub doc: String,
    /// Called as `x.name(…)`: the first parameter is a host type, which Rhai spells with a
    /// capital — `grind_build::snippets`' rule, for the same reason.
    pub method: bool,
    /// Read as `x.name`, with no call.
    pub property: bool,
}

impl Function {
    /// The parameters a caller writes: the receiver dropped for a method.
    fn written(&self) -> &[(String, String)] {
        match self.method {
            true => &self.params[1..],
            false => &self.params,
        }
    }

    fn signature(&self) -> String {
        let params: Vec<String> = self
            .written()
            .iter()
            .map(|(name, ty)| format!("{name}: {ty}"))
            .collect();
        let receiver = match self.method {
            true => format!("{}.", self.params[0].1),
            false => String::new(),
        };
        let returns = self
            .returns
            .as_deref()
            .filter(|r| *r != "()")
            .map(|r| format!(" -> {r}"))
            .unwrap_or_default();
        match self.property {
            true => format!("{receiver}{}{returns}", self.name),
            false => format!("{receiver}{}({}){returns}", self.name, params.join(", ")),
        }
    }

    fn markdown(&self) -> String {
        format!("```rhai\n{}\n```\n\n{}", self.signature(), self.doc)
    }

    fn completion(&self) -> CompletionItem {
        let insert = match self.property {
            true => self.name.clone(),
            false => {
                let stops: Vec<String> = self
                    .written()
                    .iter()
                    .enumerate()
                    .map(|(i, (name, _))| format!("${{{}:{name}}}", i + 1))
                    .collect();
                format!("{}({})", self.name, stops.join(", "))
            }
        };
        let params: Vec<&str> = self.written().iter().map(|(n, _)| n.as_str()).collect();
        CompletionItem {
            label: self.name.clone(),
            label_details: Some(CompletionItemLabelDetails {
                detail: (!self.property).then(|| format!("({})", params.join(", "))),
                description: self.returns.clone().filter(|r| r != "()"),
            }),
            kind: Some(match (self.property, self.method) {
                (true, _) => CompletionItemKind::PROPERTY,
                (false, true) => CompletionItemKind::METHOD,
                (false, false) => CompletionItemKind::FUNCTION,
            }),
            detail: Some(self.signature()),
            documentation: Some(Documentation::MarkupContent(MarkupContent {
                kind: MarkupKind::Markdown,
                value: self.doc.clone(),
            })),
            insert_text: Some(insert),
            insert_text_format: Some(InsertTextFormat::SNIPPET),
            filter_text: Some(self.name.clone()),
            ..CompletionItem::default()
        }
    }
}

/// Every function `grind definitions` printed, in its order.
pub fn vocabulary(definitions: &str) -> Vec<Function> {
    let mut functions = Vec::new();
    let mut doc: Vec<String> = Vec::new();
    for line in definitions.lines() {
        let line = line.trim();
        if let Some(text) = line.strip_prefix("///") {
            doc.push(text.strip_prefix(' ').unwrap_or(text).to_owned());
            continue;
        }
        let Some(signature) = line.strip_prefix("fn ").and_then(|s| s.strip_suffix(';')) else {
            if !line.is_empty() {
                doc.clear();
            }
            continue;
        };
        if let Some(function) = parse(signature, doc.join("\n")) {
            functions.push(function);
        }
        doc.clear();
    }
    functions
}

/// `push(sheet: Sheet, row: Row) -> int`, or `get rows(sheet: Sheet) -> int`.
fn parse(signature: &str, doc: String) -> Option<Function> {
    let property = signature.starts_with("get ") || signature.starts_with("set ");
    let signature = signature
        .trim_start_matches("get ")
        .trim_start_matches("set ");
    let (name, rest) = signature.split_once('(')?;
    let (params, tail) = rest.split_once(')')?;
    let params: Vec<(String, String)> = params
        .split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| match p.split_once(':') {
            Some((name, ty)) => (name.trim().to_owned(), ty.trim().to_owned()),
            None => (p.to_owned(), "?".to_owned()),
        })
        .collect();
    let method = params
        .first()
        .is_some_and(|(_, ty)| ty.starts_with(|c: char| c.is_ascii_uppercase()));
    let returns = tail.trim().strip_prefix("->").map(|r| r.trim().to_owned());
    Some(Function {
        name: name.trim().to_owned(),
        params,
        returns,
        doc,
        method,
        property: property && method,
    })
}

/// Whether the word at `start` follows a `.` — `s.pu`, or a bare `s.` just typed.
fn after_dot(text: &str, start: usize) -> bool {
    text[..start].ends_with('.')
}

/// Completion at a position: methods after a dot, free functions anywhere else, one item per
/// registration so every overload's parameters are on offer.
pub fn complete(vocabulary: &[Function], lines: &Lines, at: Position) -> Vec<CompletionItem> {
    let offset = lines.offset(at);
    let (word, _) = lines.word_at(offset);
    let dotted = after_dot(lines.text(), word.start);
    vocabulary
        .iter()
        .filter(|f| f.method == dotted)
        .map(Function::completion)
        .collect()
}

/// Hover over a name: every registration of it, with its documentation.
pub fn hover(vocabulary: &[Function], lines: &Lines, at: Position) -> Option<Hover> {
    let offset = lines.offset(at);
    let (span, word) = lines.word_at(offset);
    if word.is_empty() {
        return None;
    }
    let dotted = after_dot(lines.text(), span.start);
    let mut matching: Vec<&Function> = vocabulary.iter().filter(|f| f.name == word).collect();
    // After a dot it is a method; anywhere else prefer the free function, and fall back to
    // every registration of the name rather than to nothing.
    if matching.iter().any(|f| f.method == dotted) {
        matching.retain(|f| f.method == dotted);
    }
    if matching.is_empty() {
        return None;
    }
    let value = matching
        .iter()
        .map(|f| f.markdown())
        .collect::<Vec<_>>()
        .join("\n\n---\n\n");
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value,
        }),
        range: Some(lines.range(span)),
    })
}

/// A syntax error, where the compiler put it.
pub fn diagnostics(lines: &Lines, error: Option<crate::ScriptError>) -> Vec<Diagnostic> {
    let Some(error) = error else {
        return Vec::new();
    };
    let start = match (error.line, error.column) {
        (Some(line), Some(column)) => lines.at_line_column(line as usize, column as usize),
        (Some(line), None) => lines.at_line_column(line as usize, 1),
        _ => Position::new(0, 0),
    };
    let end = Position::new(start.line, start.character + 1);
    vec![Diagnostic {
        range: Range { start, end },
        severity: Some(DiagnosticSeverity::ERROR),
        source: Some("grind".to_owned()),
        message: error.message,
        ..Diagnostic::default()
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEFINITIONS: &str = "module static;

/// A sheet with a name.
fn sheet(name: string) -> Sheet;

/// Append a row.
///
/// Second paragraph.
fn push(sheet: Sheet, row: Row) -> int;

/// How many rows.
fn get rows(sheet: Sheet) -> int;
";

    #[test]
    fn the_definition_file_is_read_as_functions_methods_and_properties() {
        let v = vocabulary(DEFINITIONS);
        assert_eq!(v.len(), 3);
        assert!(!v[0].method && !v[0].property);
        assert!(v[1].method && !v[1].property);
        assert_eq!(v[1].doc, "Append a row.\n\nSecond paragraph.");
        assert_eq!(v[1].signature(), "Sheet.push(row: Row) -> int");
        assert!(v[2].method && v[2].property);
        assert_eq!(v[2].signature(), "Sheet.rows -> int");
    }

    #[test]
    fn after_a_dot_only_methods_are_offered() {
        let v = vocabulary(DEFINITIONS);
        let text = "let s = sheet(\"A\");\ns.\n";
        let lines = Lines::new(text);
        let names = |at| -> Vec<String> {
            complete(&v, &lines, at)
                .into_iter()
                .map(|c| c.label)
                .collect()
        };
        assert_eq!(names(Position::new(1, 2)), ["push", "rows"]);
        assert_eq!(
            names(Position::new(0, 9)),
            ["sheet"],
            "no dot, a free function"
        );
        let push = complete(&v, &lines, Position::new(1, 2)).remove(0);
        assert_eq!(push.insert_text.as_deref(), Some("push(${1:row})"));
    }

    #[test]
    fn hover_names_the_method_after_a_dot() {
        let v = vocabulary(DEFINITIONS);
        let lines = Lines::new("s.push(r);");
        let hover = hover(&v, &lines, Position::new(0, 3)).expect("a method");
        let HoverContents::Markup(text) = hover.contents else {
            panic!("markdown")
        };
        assert!(
            text.value.contains("Sheet.push(row: Row) -> int"),
            "{}",
            text.value
        );
        assert!(text.value.contains("Append a row."));
        assert!(
            super::hover(&v, &lines, Position::new(0, 8)).is_none(),
            "`r` is the script's own"
        );
    }
}
