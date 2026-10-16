// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! **`grind lsp`, the binary, over real stdio** — `doc/init.md` I5's exit criterion: the server
//! driven the way an editor drives it, on `examples/quote.grind` and on a script, with the
//! vocabulary and the syntax check the binary hands it (`grind_lsp::Host`).
//!
//! `lsp/tests/session.rs` covers every request in-process with a stand-in host; this is the
//! part only the real binary can show — the framing on stdout, the real vocabulary after `s.`,
//! the real compiler's errors, and a script that would never finish being opened in an
//! instant, because nothing in a language server runs one.
#![cfg(feature = "lsp")]

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

struct Server {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    next: i64,
}

impl Server {
    fn start() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_grind"))
            .arg("lsp")
            // Spelling follows the person's remembered setting; a test follows nobody's.
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("HOME")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("grind lsp starts");
        let input = child.stdin.take().expect("stdin");
        let output = BufReader::new(child.stdout.take().expect("stdout"));
        let mut server = Server {
            child,
            input,
            output,
            next: 1,
        };
        let init = server.request("initialize", json!({ "capabilities": {} }));
        assert_eq!(init["serverInfo"]["name"], "grind");
        server.send(json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }));
        server
    }

    fn send(&mut self, message: Value) {
        let body = message.to_string();
        write!(self.input, "Content-Length: {}\r\n\r\n{body}", body.len()).expect("written");
        self.input.flush().expect("flushed");
    }

    /// One framed message off stdout — which must hold nothing else.
    fn receive(&mut self) -> Value {
        let mut length = None;
        loop {
            let mut header = String::new();
            self.output.read_line(&mut header).expect("a header");
            let header = header.trim_end();
            if header.is_empty() {
                break;
            }
            let (name, value) = header.split_once(": ").expect("a header line");
            if name.eq_ignore_ascii_case("content-length") {
                length = Some(value.parse::<usize>().expect("a length"));
            }
        }
        let mut body = vec![0; length.expect("Content-Length")];
        self.output.read_exact(&mut body).expect("the body");
        serde_json::from_slice(&body).expect("JSON")
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next;
        self.next += 1;
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        loop {
            let message = self.receive();
            if message["id"] == id {
                assert!(message.get("error").is_none(), "{method}: {message}");
                return message["result"].clone();
            }
        }
    }

    fn open(&mut self, uri: &str, language: &str, text: &str) -> Vec<Value> {
        self.send(
            json!({ "jsonrpc": "2.0", "method": "textDocument/didOpen", "params": {
            "textDocument": { "uri": uri, "languageId": language, "version": 1, "text": text } } }),
        );
        loop {
            let message = self.receive();
            if message["method"] == "textDocument/publishDiagnostics" {
                return message["params"]["diagnostics"]
                    .as_array()
                    .expect("array")
                    .clone();
            }
        }
    }

    fn at(&mut self, method: &str, uri: &str, line: usize, character: usize) -> Value {
        self.request(
            method,
            json!({ "textDocument": { "uri": uri }, "position": { "line": line, "character": character } }),
        )
    }

    fn stop(mut self) {
        self.request("shutdown", Value::Null);
        self.send(json!({ "jsonrpc": "2.0", "method": "exit" }));
        let status = self.child.wait().expect("it exits");
        assert!(status.success(), "{status}");
    }
}

#[test]
fn the_quote_over_stdio() {
    let text = include_str!("../../examples/quote.grind");
    let uri = "file:///work/quote.grind";
    let mut server = Server::start();
    assert_eq!(
        server.open(uri, "kdl", text),
        Vec::<Value>::new(),
        "it lints clean, spelling included"
    );

    let (line, column) = text
        .lines()
        .enumerate()
        .find_map(|(i, l)| l.find("cell G14").map(|c| (i, c)))
        .expect("G14");
    let hover = server.at("textDocument/hover", uri, line, column + 6);
    assert!(
        hover["contents"]["value"]
            .as_str()
            .expect("markdown")
            .contains("Quote.G14")
    );

    let hints = server.request(
        "textDocument/inlayHint",
        json!({ "textDocument": { "uri": uri },
                "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 999, "character": 0 } } }),
    );
    assert!(hints.as_array().expect("hints").len() >= 20);
    server.stop();
}

#[test]
fn a_script_with_the_real_vocabulary() {
    let uri = "file:///work/model.rhai";
    let mut server = Server::start();
    // Half-typed, `s.` does not parse yet — the compiler says so, as an editor shows while
    // somebody types — and completion is offered all the same.
    let found = server.open(uri, "rhai", "let s = sheet(\"A\");\ns.\n");
    assert_eq!(found.len(), 1, "{found:?}");

    let items = server.at("textDocument/completion", uri, 1, 2);
    let labels: Vec<&str> = items
        .as_array()
        .expect("items")
        .iter()
        .map(|i| i["label"].as_str().expect("label"))
        .collect();
    for method in ["push", "format", "style", "width", "name"] {
        assert!(labels.contains(&method), "{method} after `s.`: {labels:?}");
    }
    assert!(
        !labels.contains(&"sheet"),
        "a free function is not offered after a dot"
    );
    let push = items
        .as_array()
        .expect("items")
        .iter()
        .find(|i| i["label"] == "push")
        .expect("push");
    assert!(
        !push["documentation"]["value"]
            .as_str()
            .unwrap_or_default()
            .is_empty(),
        "with its documentation: {push}"
    );

    let hover = server.at("textDocument/hover", uri, 0, 10);
    let hover = hover["contents"]["value"].as_str().expect("markdown");
    assert!(
        hover.contains("sheet(name"),
        "the free function's signature: {hover}"
    );
    server.stop();
}

#[test]
fn a_script_is_compiled_and_never_run() {
    let mut server = Server::start();
    // It would never finish, and it would fail if it did: neither happens to an open file.
    let started = Instant::now();
    let found = server.open(
        "file:///work/forever.rhai",
        "rhai",
        "loop { }\nthrow \"ran\";\n",
    );
    assert_eq!(found, Vec::<Value>::new());
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "opened, not run"
    );

    let found = server.open(
        "file:///work/broken.rhai",
        "rhai",
        "let s = sheet(\"A\");\ns.push(row(1, 2)\n",
    );
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(
        found[0]["range"]["start"]["line"], 2,
        "the compiler's line, from 0"
    );
    server.stop();
}
