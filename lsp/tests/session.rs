// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A whole session, over an in-memory connection: the handshake, documents opened and changed,
//! and every request the server answers — on `examples/quote.grind`, the projection guide's own
//! document, and on a script. The host is a stand-in, because this crate may not link the
//! evaluator (R11); `cli/tests/lsp.rs` runs the real binary with the real vocabulary.

use std::thread::JoinHandle;

use grind_lsp::{Host, ScriptError};
use lsp_server::{Connection, Message, Notification, Request, RequestId};
use serde_json::{Value, json};

const QUOTE: &str = include_str!("../../examples/quote.grind");

/// A host with a two-function vocabulary and a syntax check that only knows one mistake.
struct Stand;

impl Host for Stand {
    fn definitions(&self) -> String {
        "module static;\n\n/// A sheet with a name.\nfn sheet(name: string) -> Sheet;\n\n\
         /// Append a row, and answer the index it landed on.\nfn push(sheet: Sheet, row: Row) -> int;\n"
            .to_owned()
    }
    fn check_script(&self, source: &str) -> Result<(), ScriptError> {
        match source.find("((") {
            None => Ok(()),
            Some(_) => Err(ScriptError {
                line: Some(2),
                column: Some(3),
                message: "Expecting ')'".to_owned(),
            }),
        }
    }
}

struct Client {
    connection: Connection,
    server: Option<JoinHandle<Result<(), String>>>,
    next: i32,
}

impl Client {
    fn start() -> Self {
        let (server, client) = Connection::memory();
        let thread = std::thread::spawn(move || grind_lsp::run(&server, &Stand));
        let mut client = Client {
            connection: client,
            server: Some(thread),
            next: 1,
        };
        let capabilities = client.request("initialize", json!({ "capabilities": {} }));
        assert_eq!(capabilities["capabilities"]["hoverProvider"], true);
        client.notify("initialized", json!({}));
        client
    }

    fn notify(&self, method: &str, params: Value) {
        self.connection
            .sender
            .send(Notification::new(method.to_owned(), params).into())
            .expect("sent");
    }

    /// A request's result; notifications that arrive first are dropped.
    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = RequestId::from(self.next);
        self.next += 1;
        self.connection
            .sender
            .send(Request::new(id.clone(), method.to_owned(), params).into())
            .expect("sent");
        loop {
            match self.connection.receiver.recv().expect("an answer") {
                Message::Response(r) if r.id == id => match r.response_result {
                    Ok(result) => return result,
                    Err(e) => panic!("{method}: {e:?}"),
                },
                _ => continue,
            }
        }
    }

    /// The diagnostics published after opening or changing a document.
    fn diagnostics(&self) -> Vec<Value> {
        loop {
            match self.connection.receiver.recv().expect("diagnostics") {
                Message::Notification(n) if n.method == "textDocument/publishDiagnostics" => {
                    return n.params["diagnostics"].as_array().expect("array").clone();
                }
                _ => continue,
            }
        }
    }

    fn open(&self, uri: &str, text: &str) -> Vec<Value> {
        self.notify(
            "textDocument/didOpen",
            json!({ "textDocument": { "uri": uri, "languageId": "kdl", "version": 1, "text": text } }),
        );
        self.diagnostics()
    }

    fn change(&self, uri: &str, version: i32, text: &str) -> Vec<Value> {
        self.notify(
            "textDocument/didChange",
            json!({
                "textDocument": { "uri": uri, "version": version },
                "contentChanges": [{ "text": text }],
            }),
        );
        self.diagnostics()
    }

    fn at(&mut self, method: &str, uri: &str, line: usize, character: usize) -> Value {
        self.request(
            method,
            json!({ "textDocument": { "uri": uri }, "position": { "line": line, "character": character } }),
        )
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.request("shutdown", Value::Null);
        self.notify("exit", Value::Null);
        if let Some(server) = self.server.take() {
            assert_eq!(server.join().expect("the server thread"), Ok(()));
        }
    }
}

/// The 0-based line of the first line of `text` containing `needle`, and the column it starts at.
fn find(text: &str, needle: &str) -> (usize, usize) {
    text.lines()
        .enumerate()
        .find_map(|(i, line)| line.find(needle).map(|c| (i, c)))
        .unwrap_or_else(|| panic!("{needle:?} is in the text"))
}

const QUOTE_URI: &str = "file:///work/quote.grind";

#[test]
fn the_quote_is_clean_and_every_formula_shows_its_value() {
    let mut client = Client::start();
    assert_eq!(
        client.open(QUOTE_URI, QUOTE),
        Vec::<Value>::new(),
        "it lints clean"
    );

    let hints = client.request(
        "textDocument/inlayHint",
        json!({ "textDocument": { "uri": QUOTE_URI },
                "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 999, "character": 0 } } }),
    );
    let hints = hints.as_array().expect("hints");
    let formulas = QUOTE.lines().filter(|l| l.contains("\"=")).count();
    assert_eq!(hints.len(), formulas, "one per formula cell");
    // The bench rate is read through a name from the other sheet.
    let (line, _) = find(QUOTE, "cell F5  \"=benchRate\"");
    let f5 = hints
        .iter()
        .find(|h| h["position"]["line"] == line)
        .expect("F5's value");
    assert_eq!(f5["label"], "→ £62.00");
}

#[test]
fn hover_reads_a_cell_and_a_name() {
    let mut client = Client::start();
    client.open(QUOTE_URI, QUOTE);
    let (line, column) = find(QUOTE, "cell G14");
    let hover = client.at("textDocument/hover", QUOTE_URI, line, column + 6);
    let text = hover["contents"]["value"].as_str().expect("markdown");
    assert!(text.contains("**Quote.G14**"), "{text}");
    assert!(
        text.contains("SUMIF(kind;\"Labor\";amount)"),
        "the formula, names and all: {text}"
    );
    assert!(text.contains("shows **"), "{text}");

    let (line, column) = find(QUOTE, "\"=benchRate\"");
    let hover = client.at("textDocument/hover", QUOTE_URI, line, column + 3);
    let text = hover["contents"]["value"].as_str().expect("markdown");
    assert!(text.contains("**benchRate** — a defined name"), "{text}");
}

#[test]
fn a_name_goes_to_its_definition() {
    let mut client = Client::start();
    client.open(QUOTE_URI, QUOTE);
    let (line, column) = find(QUOTE, "\"=benchRate\"");
    let found = client.at("textDocument/definition", QUOTE_URI, line, column + 3);
    let (declared, _) = find(QUOTE, "name benchRate");
    assert_eq!(found["range"]["start"]["line"], declared, "{found}");
}

#[test]
fn completion_knows_whether_it_is_inside_a_formula() {
    let mut client = Client::start();
    let text =
        "grind spreadsheet\n\nname rate \"[$A.$B$1]\"\n\nsheet A {\n    cell B2 \"=SU\"\n    \n}\n";
    client.open("file:///work/c.grind", text);
    let labels = |v: &Value| -> Vec<String> {
        v.as_array()
            .expect("items")
            .iter()
            .map(|i| i["label"].as_str().expect("label").to_owned())
            .collect()
    };
    let (line, column) = find(text, "=SU");
    let inside = labels(&client.at(
        "textDocument/completion",
        "file:///work/c.grind",
        line,
        column + 3,
    ));
    assert!(inside.contains(&"SUM".to_owned()), "a function");
    assert!(inside.contains(&"rate".to_owned()), "a defined name");
    assert!(!inside.contains(&"cell".to_owned()), "and no node");

    let outside = labels(&client.at("textDocument/completion", "file:///work/c.grind", 6, 4));
    assert!(outside.contains(&"cell".to_owned()), "a node: {outside:?}");
    assert!(!outside.contains(&"SUM".to_owned()));
}

#[test]
fn a_finding_and_a_read_error_land_on_their_lines() {
    let mut client = Client::start();
    let uri = "file:///work/b.grind";
    let text = "grind spreadsheet\n\n// a comment the writer would not produce\nsheet A {\n    cell B1 \"=[Gone.A1]\"\n}\n";
    let found = client.open(uri, text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0]["range"]["start"]["line"], 4);
    assert_eq!(found[0]["range"]["start"]["character"], 4);
    assert_eq!(found[0]["code"], "missing-sheet");
    assert_eq!(found[0]["severity"], 1, "an error");

    // A node the reader does not know, and then a string the parser cannot close.
    let found = client.change(uri, 2, "grind spreadsheet\n\nsheet A {\n    cel B1 1\n}\n");
    assert_eq!(
        (
            found[0]["range"]["start"]["line"].clone(),
            found[0]["range"]["start"]["character"].clone()
        ),
        (json!(3), json!(4))
    );
    let found = client.change(
        uri,
        3,
        "grind spreadsheet\n\nsheet A {\n    cell B1 \"=SUM(\n}\n",
    );
    assert_eq!(found[0]["range"]["start"]["line"], 3, "{found:?}");

    // And fixed again, the diagnostics go away; the server is still answering.
    assert_eq!(
        client.change(uri, 4, "grind spreadsheet\n\nsheet A {\n    cell B1 1\n}\n"),
        Vec::<Value>::new()
    );
    assert!(client.at("textDocument/hover", uri, 3, 10).is_object());
}

#[test]
fn a_script_offers_methods_after_a_dot_and_reports_a_syntax_error() {
    let mut client = Client::start();
    let uri = "file:///work/model.rhai";
    let text = "let s = sheet(\"A\");\ns.\n";
    assert_eq!(client.open(uri, text), Vec::<Value>::new());
    let items = client.at("textDocument/completion", uri, 1, 2);
    let items = items.as_array().expect("items");
    assert_eq!(items.len(), 1, "only the method: {items:?}");
    assert_eq!(items[0]["label"], "push");
    assert!(
        items[0]["documentation"]["value"]
            .as_str()
            .expect("docs")
            .contains("Append a row")
    );

    let hover = client.at("textDocument/hover", uri, 0, 10);
    assert!(
        hover["contents"]["value"]
            .as_str()
            .expect("markdown")
            .contains("A sheet with a name")
    );

    let found = client.change(uri, 2, "let s = sheet(\"A\");\ns.push((\n");
    assert_eq!(found.len(), 1);
    assert_eq!(
        found[0]["range"]["start"],
        json!({ "line": 1, "character": 2 })
    );
}
