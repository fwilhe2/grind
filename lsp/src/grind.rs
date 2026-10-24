// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A `.grind`: the projection, opened the way every command opens it and asked what an editor
//! wants to know. Every answer is the core's — read by `open_bytes`, linted by the same rules,
//! recalculated by the same engine, located through the span map its reader recorded
//! (`App::source_anchors`) — so nothing here can disagree with `grind lint`. Nothing is written:
//! inline values are **shown**, and `grind sheet recalc` stays the verb that writes them.

use std::ops::Range;

use grind_core::kind::DocumentKind;
use grind_core::lint::{Options, Severity};
use grind_core::projection::{Anchor, kdl, node_span};
use grind_sheet::a1;
use lsp_types::{
    CompletionItem, CompletionItemKind, Diagnostic, DiagnosticSeverity, Documentation, Hover,
    HoverContents, InlayHint, InlayHintKind, InlayHintLabel, InsertTextFormat, MarkupContent,
    MarkupKind, NumberOrString, Position, Range as LspRange,
};

use crate::Host;
use crate::lines::Lines;

/// The projection's nodes, as snippets — the file `grind definitions --projection` prints and
/// `grind init` writes, so completion here and there offer the same words.
const NODES: &str = include_str!("../../.vscode/grind-projection.code-snippets");

/// An opened document of either kind.
pub enum Opened {
    Sheet(Box<grind_sheet::App>),
    Text(Box<grind_text::App>),
}

/// What one version of a `.grind` says: the document, if it read, where its addresses are, and
/// what is wrong with it.
pub struct Grind {
    pub opened: Option<Opened>,
    pub anchors: Vec<Anchor>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Read a projection and check it. Spreadsheets are recalculated *after* linting — the lint
/// is about the file as written, a stale cached value included, and the inline values are
/// about what the formulas compute.
pub fn analyse(name: &str, text: &str, host: &dyn Host) -> Grind {
    let lines = Lines::new(text);
    let failed = |message: String| Grind {
        opened: None,
        anchors: Vec::new(),
        diagnostics: vec![located(&lines, &message)],
    };
    let opened = match grind_sheet::kind(text.as_bytes()) {
        Some(DocumentKind::Spreadsheet) => {
            let app = grind_sheet::App::new();
            if let Err(e) = app.open_bytes(name, text.as_bytes()) {
                return failed(e.to_string());
            }
            host.attach_spelling_sheet(&app);
            Opened::Sheet(Box::new(app))
        }
        Some(DocumentKind::Text) => {
            let app = grind_text::App::new();
            if let Err(e) = app.open_bytes(name, text.as_bytes()) {
                return failed(e.to_string());
            }
            host.attach_spelling_text(&app);
            Opened::Text(Box::new(app))
        }
        _ => {
            return failed(
                "not a projection: the first line says which kind it is — `grind spreadsheet` \
                 or `grind text`"
                    .to_owned(),
            );
        }
    };
    let (report, anchors) = match &opened {
        Opened::Sheet(app) => (app.lint(&Options::default()), app.source_anchors()),
        Opened::Text(app) => (app.lint(&Options::default()), app.source_anchors()),
    };
    if let Opened::Sheet(app) = &opened {
        let _ = app.recalc();
    }
    let diagnostics = report
        .diagnostics
        .iter()
        .map(|d| {
            let span = anchors
                .iter()
                .find(|a| a.address == d.at)
                .map(|a| a.span.clone());
            Diagnostic {
                range: lines.range(span.clone().unwrap_or(0..0)),
                severity: Some(match d.severity {
                    Severity::Error => DiagnosticSeverity::ERROR,
                    Severity::Warning => DiagnosticSeverity::WARNING,
                    Severity::Hint => DiagnosticSeverity::HINT,
                }),
                code: Some(NumberOrString::String(d.rule.to_owned())),
                source: Some("grind".to_owned()),
                // A finding the file never spelled the address of sits at the top, so it has
                // to say where it is about.
                message: match (span, d.at.is_empty()) {
                    (None, false) => format!("{}: {}", d.at, d.message),
                    _ => d.message.clone(),
                },
                ..Diagnostic::default()
            }
        })
        .collect();
    Grind {
        opened: Some(opened),
        anchors,
        diagnostics,
    }
}

/// A read error as a diagnostic at the place it names.
///
/// The projection's errors are one line of text, and they name a place in one of two
/// spellings: the KDL parser's `line 4, column 13: …` and the readers' `` `cel` at offset 33: …``.
/// Both are turned back into a position here, against the text that was read; anything else
/// lands on the first line.
pub fn located(lines: &Lines, message: &str) -> Diagnostic {
    let message = message.strip_prefix("projection: ").unwrap_or(message);
    let number = |s: &str| -> Option<usize> {
        let digits: String = s.chars().take_while(char::is_ascii_digit).collect();
        digits.parse().ok()
    };
    let start = if let Some(rest) = message.find("line ").map(|at| &message[at + 5..])
        && let Some(line) = number(rest)
        && let Some(column) = rest
            .find(", column ")
            .and_then(|at| number(&rest[at + 9..]))
    {
        lines.at_line_column(line, column)
    } else if let Some(at) = message.find(" at offset ")
        && let Some(offset) = number(&message[at + 11..])
    {
        lines.position(offset)
    } else {
        Position::new(0, 0)
    };
    let end = lines.position(lines.offset(start) + 1);
    Diagnostic {
        range: LspRange { start, end },
        severity: Some(DiagnosticSeverity::ERROR),
        source: Some("grind".to_owned()),
        message: message.to_owned(),
        ..Diagnostic::default()
    }
}

/// The address whose span is the narrowest around a byte: a cell inside a grid row rather than
/// the row, a block rather than nothing.
fn anchor_at(anchors: &[Anchor], byte: usize) -> Option<&Anchor> {
    anchors
        .iter()
        .filter(|a| a.span.start <= byte && byte <= a.span.end)
        .min_by_key(|a| a.span.end - a.span.start)
}

/// One cell, by the address a projection spells it with.
fn cell(app: &grind_sheet::App, address: &str) -> Option<(usize, grind_sheet::Pos)> {
    let reference = a1::parse(address).ok()?;
    let (sheet, pos, _) = a1::resolve(app, &reference).ok()?;
    Some((sheet, pos))
}

/// Hover over a cell: its formula as the formula bar reads it, and what it shows. Over a block:
/// every address it answers to. Over a defined name: what it stands for.
pub fn hover(grind: &Grind, lines: &Lines, at: Position) -> Option<Hover> {
    let opened = grind.opened.as_ref()?;
    let byte = lines.offset(at);
    let markdown = |value: String, span: Range<usize>| Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value,
        }),
        range: Some(lines.range(span)),
    };
    match opened {
        Opened::Sheet(app) => {
            let (span, word) = lines.word_at(byte);
            // A name is case-insensitive in ODF, and `names()` answers in lower case.
            if !word.is_empty()
                && let Some((_, expression)) = app
                    .names()
                    .into_iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case(word))
            {
                return Some(markdown(
                    format!("**{word}** — a defined name\n\n`{expression}`"),
                    span,
                ));
            }
            let anchor = anchor_at(&grind.anchors, byte)?;
            let (sheet, pos) = cell(app, &anchor.address)?;
            let shown = app.value_text(sheet, pos).ok()?;
            let mut text = format!("**{}**", anchor.address);
            if let Ok(Some(formula)) = app.named_formula(sheet, pos) {
                text.push_str(&format!("\n\n`{formula}`"));
            }
            text.push_str(&format!(
                "\n\nshows **{}**",
                if shown.is_empty() { "nothing" } else { &shown }
            ));
            Some(markdown(text, anchor.span.clone()))
        }
        Opened::Text(_) => {
            let anchor = anchor_at(&grind.anchors, byte)?;
            let names: Vec<&str> = grind
                .anchors
                .iter()
                .filter(|a| a.span == anchor.span)
                .map(|a| a.address.as_str())
                .collect();
            Some(markdown(
                format!(
                    "**{}** — the addresses this block answers to",
                    names.join(" · ")
                ),
                anchor.span.clone(),
            ))
        }
    }
}

/// What every formula in view computes, at the end of its cell — shown, never written.
pub fn inlay_hints(grind: &Grind, lines: &Lines, within: LspRange) -> Vec<InlayHint> {
    let Some(Opened::Sheet(app)) = &grind.opened else {
        return Vec::new();
    };
    let (from, to) = (lines.offset(within.start), lines.offset(within.end));
    grind
        .anchors
        .iter()
        .filter(|a| a.span.end >= from && a.span.start <= to)
        .filter_map(|a| {
            let (sheet, pos) = cell(app, &a.address)?;
            app.formula(sheet, pos).ok()??;
            let shown = app.value_text(sheet, pos).ok()?;
            Some(InlayHint {
                position: lines.position(a.span.end),
                label: InlayHintLabel::String(format!("→ {shown}")),
                kind: Some(InlayHintKind::TYPE),
                text_edits: None,
                tooltip: None,
                padding_left: Some(true),
                padding_right: None,
                data: None,
            })
        })
        .collect()
}

/// Where a defined name or a sheet is declared — its `name` or `sheet` node.
pub fn definition(grind: &Grind, lines: &Lines, at: Position) -> Option<LspRange> {
    let Some(Opened::Sheet(_)) = &grind.opened else {
        return None;
    };
    let (_, word) = lines.word_at(lines.offset(at));
    if word.is_empty() {
        return None;
    }
    let document: kdl::KdlDocument = lines.text().parse().ok()?;
    document
        .nodes()
        .iter()
        .find(|node| {
            matches!(node.name().value(), "name" | "sheet")
                && node
                    .entries()
                    .first()
                    .and_then(|e| e.value().as_string())
                    .is_some_and(|v| v.eq_ignore_ascii_case(word))
        })
        .map(|node| lines.range(node_span(node)))
}

/// Inside a formula — a string that starts with `=` — the functions and the defined names;
/// anywhere else, the projection's nodes.
pub fn complete(grind: Option<&Grind>, lines: &Lines, at: Position) -> Vec<CompletionItem> {
    let byte = lines.offset(at);
    let text = lines.text();
    let line_start = text[..byte].rfind('\n').map_or(0, |i| i + 1);
    let before = &text[line_start..byte];
    match open_string(before) {
        Some(string) if string.starts_with('=') || string.starts_with("of:=") => {
            let mut items: Vec<CompletionItem> = grind_sheet::formula::funcs::catalog()
                .iter()
                .map(|f| CompletionItem {
                    label: f.name.to_owned(),
                    kind: Some(CompletionItemKind::FUNCTION),
                    detail: Some(format!("{}({})", f.name, f.signature)),
                    documentation: Some(Documentation::String(f.brief.to_owned())),
                    insert_text: Some(format!("{}($0)", f.name)),
                    insert_text_format: Some(InsertTextFormat::SNIPPET),
                    ..CompletionItem::default()
                })
                .collect();
            if let Some(Grind {
                opened: Some(Opened::Sheet(app)),
                ..
            }) = grind
            {
                items.extend(
                    app.names()
                        .into_iter()
                        .map(|(name, expression)| CompletionItem {
                            label: name,
                            kind: Some(CompletionItemKind::VARIABLE),
                            detail: Some(expression),
                            ..CompletionItem::default()
                        }),
                );
            }
            items
        }
        Some(_) => Vec::new(),
        None => nodes(),
    }
}

/// The text of a string still open at the end of `before`, or `None` outside one.
fn open_string(before: &str) -> Option<&str> {
    let mut open = None;
    let mut escaped = false;
    for (at, c) in before.char_indices() {
        match (c, escaped) {
            ('\\', false) => escaped = true,
            ('"', false) => open = if open.is_some() { None } else { Some(at + 1) },
            _ => escaped = false,
        }
        if c != '\\' {
            escaped = false;
        }
    }
    open.map(|start| &before[start..])
}

/// Every node of the projection, from the snippet file.
fn nodes() -> Vec<CompletionItem> {
    let body: String = NODES
        .lines()
        .skip_while(|l| l.starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    let Ok(serde_json::Value::Object(snippets)) = serde_json::from_str(&body) else {
        return Vec::new();
    };
    snippets
        .values()
        .filter_map(|snippet| {
            let prefix = snippet["prefix"].as_str()?;
            let lines: Vec<&str> = match &snippet["body"] {
                serde_json::Value::Array(lines) => {
                    lines.iter().filter_map(|l| l.as_str()).collect()
                }
                serde_json::Value::String(line) => vec![line.as_str()],
                _ => return None,
            };
            let description = snippet["description"].as_str().unwrap_or_default();
            Some(CompletionItem {
                label: prefix.to_owned(),
                kind: Some(CompletionItemKind::KEYWORD),
                detail: description.lines().next().map(str::to_owned),
                documentation: Some(Documentation::MarkupContent(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: description.to_owned(),
                })),
                insert_text: Some(lines.join("\n")),
                insert_text_format: Some(InsertTextFormat::SNIPPET),
                ..CompletionItem::default()
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_spellings_of_a_read_error_land_where_they_say() {
        let text = "grind spreadsheet\n\nsheet A {\n    cel B1 1\n}\n";
        let lines = Lines::new(text);
        let d = located(
            &lines,
            "projection: `cel` at offset 33: `cel` is not something a sheet holds",
        );
        assert_eq!(d.range.start, Position::new(3, 4));
        let d = located(&lines, "projection: line 4, column 5: unexpected");
        assert_eq!(d.range.start, Position::new(3, 4));
        let d = located(&lines, "something with no place");
        assert_eq!(d.range.start, Position::new(0, 0));
    }

    #[test]
    fn a_string_is_open_until_it_closes() {
        assert_eq!(open_string("    cell B5 \"=SU"), Some("=SU"));
        assert_eq!(open_string("    cell B5 \"=SUM()\" "), None);
        assert_eq!(
            open_string("    row \"a \\\" b"),
            Some("a \\\" b"),
            "an escaped quote"
        );
    }

    #[test]
    fn the_node_snippets_are_read() {
        let nodes = nodes();
        assert!(nodes.len() >= 20, "{}", nodes.len());
        assert!(nodes.iter().any(|n| n.label == "cell"));
    }
}
