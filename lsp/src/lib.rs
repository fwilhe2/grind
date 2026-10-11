// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! **`grind lsp`** — a language server for the two languages, `doc/init.md` I5.
//!
//! One server, speaking the Language Server Protocol over stdio, so every editor that speaks it
//! — VS Code through a client, Helix, Zed, Neovim, Emacs — gets the same answers rather than one
//! editor's configuration each (`doc/init.md` §3.3, §3.9 b).
//!
//! | | `.grind` (`grind.rs`) | `.rhai` (`script.rs`) |
//! |---|---|---|
//! | diagnostics | a read error where it says, every lint finding on its cell's line | the syntax error, compiled and **never run** |
//! | completion | the projection's nodes; inside a formula, the functions and defined names | the host vocabulary — methods after a `.`, free functions elsewhere |
//! | hover | a cell's formula and what it shows; a name's definition; a block's addresses | every registration of the name, with its documentation |
//! | inlay hints | what each formula computes, **shown and never written** | — |
//! | go to definition | a defined name's or a sheet's node | — |
//!
//! **Everything comes from the core.** The server holds no knowledge of its own: a `.grind` is
//! opened by the same `open_bytes` the CLI uses, linted by the same rules, located through the
//! span map its reader recorded (`App::source_anchors`). A server that answered differently
//! from `grind lint` would be a bug in the server.
//!
//! **No evaluator** (R11, `build/tests/manifest.rs`). This crate opens documents, so it may not
//! link `grind-build`; the vocabulary arrives as the text `grind definitions` prints, and the
//! syntax check as a function, both through [`Host`], which `grind lsp` implements. Which also
//! makes the rule visible: a script runs because a person typed a command, never because an
//! editor opened a file.
//!
//! Full-document sync, one document at a time, everything recomputed per change: the files this
//! is for are written by hand, and a person types slower than a projection reads.

mod grind;
mod lines;
mod script;

use std::collections::HashMap;

use lsp_server::{Connection, ErrorCode, Message, Notification, Request, Response};
use lsp_types::notification::{
    DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, Notification as _,
    PublishDiagnostics,
};
use lsp_types::request::{
    Completion, GotoDefinition, HoverRequest, InlayHintRequest, Request as _,
};
use lsp_types::{
    CompletionOptions, CompletionParams, CompletionResponse, DidChangeTextDocumentParams,
    DidCloseTextDocumentParams, DidOpenTextDocumentParams, GotoDefinitionParams,
    GotoDefinitionResponse, HoverParams, HoverProviderCapability, InlayHintParams, Location, OneOf,
    PublishDiagnosticsParams, ServerCapabilities, ServerInfo, TextDocumentSyncCapability,
    TextDocumentSyncKind, Uri,
};

use lines::Lines;

/// What the server needs and may not link itself — the generator's vocabulary and its syntax
/// check, and the dictionaries — handed in by the binary that does link them.
pub trait Host {
    /// The host vocabulary as a Rhai definition file: `grind definitions`' output.
    fn definitions(&self) -> String;
    /// Whether a script parses. Must compile and **never evaluate** (`grind_build::check`).
    fn check_script(&self, source: &str) -> Result<(), ScriptError>;
    /// Check spelling in an opened spreadsheet, as `grind lint` does. Nothing, by default.
    fn attach_spelling_sheet(&self, _app: &grind_sheet::App) {}
    /// The same for a text document.
    fn attach_spelling_text(&self, _app: &grind_text::App) {}
}

/// A script that does not parse, and where — 1-based, when the compiler knew.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptError {
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub message: String,
}

/// What this server can do, as the protocol asks a server to say it.
pub fn capabilities() -> ServerCapabilities {
    ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        completion_provider: Some(CompletionOptions {
            trigger_characters: Some(vec![".".to_owned()]),
            ..CompletionOptions::default()
        }),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        definition_provider: Some(OneOf::Left(true)),
        inlay_hint_provider: Some(OneOf::Left(true)),
        ..ServerCapabilities::default()
    }
}

/// Serve over stdin and stdout until the client says to stop.
pub fn serve(host: &dyn Host) -> Result<(), String> {
    let (connection, threads) = Connection::stdio();
    run(&connection, host)?;
    drop(connection);
    threads.join().map_err(|e| e.to_string())
}

/// Serve on a connection: the handshake, then every message until `shutdown` and `exit`.
pub fn run(connection: &Connection, host: &dyn Host) -> Result<(), String> {
    let (id, _params) = connection.initialize_start().map_err(|e| e.to_string())?;
    let reply = serde_json::json!({
        "capabilities": capabilities(),
        "serverInfo": ServerInfo {
            name: "grind".to_owned(),
            version: Some(env!("CARGO_PKG_VERSION").to_owned()),
        },
    });
    connection
        .initialize_finish(id, reply)
        .map_err(|e| e.to_string())?;

    let mut server = Server {
        vocabulary: script::vocabulary(&host.definitions()),
        documents: HashMap::new(),
        host,
    };
    for message in &connection.receiver {
        match message {
            Message::Request(request) => {
                if connection
                    .handle_shutdown(&request)
                    .map_err(|e| e.to_string())?
                {
                    return Ok(());
                }
                let response = server.request(request);
                connection
                    .sender
                    .send(response.into())
                    .map_err(|e| e.to_string())?;
            }
            Message::Notification(notification) => {
                if let Some(published) = server.notification(notification) {
                    let note = Notification::new(PublishDiagnostics::METHOD.to_owned(), published);
                    connection
                        .sender
                        .send(note.into())
                        .map_err(|e| e.to_string())?;
                }
            }
            Message::Response(_) => {}
        }
    }
    Ok(())
}

enum Language {
    Grind,
    Rhai,
}

struct Document {
    text: String,
    language: Language,
    /// The projection as read, for a `.grind`; rebuilt on every change.
    grind: Option<grind::Grind>,
}

struct Server<'h> {
    vocabulary: Vec<script::Function>,
    documents: HashMap<String, Document>,
    host: &'h dyn Host,
}

/// Which language a file is, from its name first and the editor's word for it second — a
/// `.grind` arrives as `kdl` when the editor was told it is KDL, and an unsaved one has no name
/// to go by, only grind's client calling it `grind`.
fn language(uri: &Uri, id: &str) -> Option<Language> {
    let path = uri.as_str();
    if path.ends_with(".grind") || id == "grind" {
        Some(Language::Grind)
    } else if path.ends_with(".rhai") || id == "rhai" {
        Some(Language::Rhai)
    } else {
        None
    }
}

impl Server<'_> {
    /// A document opened, changed or closed — and the diagnostics that follow from it.
    fn notification(&mut self, notification: Notification) -> Option<PublishDiagnosticsParams> {
        match notification.method.as_str() {
            DidOpenTextDocument::METHOD => {
                let params: DidOpenTextDocumentParams =
                    notification.extract(DidOpenTextDocument::METHOD).ok()?;
                let item = params.text_document;
                let language = language(&item.uri, &item.language_id)?;
                let key = item.uri.as_str().to_owned();
                self.documents.insert(
                    key.clone(),
                    Document {
                        text: item.text,
                        language,
                        grind: None,
                    },
                );
                Some(self.analyse(&key, item.uri, Some(item.version)))
            }
            DidChangeTextDocument::METHOD => {
                let params: DidChangeTextDocumentParams =
                    notification.extract(DidChangeTextDocument::METHOD).ok()?;
                let key = params.text_document.uri.as_str().to_owned();
                let document = self.documents.get_mut(&key)?;
                // Full sync: the last change is the whole text.
                document.text = params.content_changes.into_iter().last()?.text;
                Some(self.analyse(
                    &key,
                    params.text_document.uri,
                    Some(params.text_document.version),
                ))
            }
            DidCloseTextDocument::METHOD => {
                let params: DidCloseTextDocumentParams =
                    notification.extract(DidCloseTextDocument::METHOD).ok()?;
                self.documents.remove(params.text_document.uri.as_str());
                Some(PublishDiagnosticsParams {
                    uri: params.text_document.uri,
                    diagnostics: Vec::new(),
                    version: None,
                })
            }
            _ => None,
        }
    }

    fn analyse(&mut self, key: &str, uri: Uri, version: Option<i32>) -> PublishDiagnosticsParams {
        let host = self.host;
        let document = self.documents.get_mut(key).expect("just stored");
        let lines = Lines::new(&document.text);
        let diagnostics = match document.language {
            Language::Grind => {
                let name = uri
                    .path()
                    .as_str()
                    .rsplit('/')
                    .next()
                    .unwrap_or("document.grind");
                let analysed = grind::analyse(name, &document.text, host);
                let diagnostics = analysed.diagnostics.clone();
                document.grind = Some(analysed);
                diagnostics
            }
            Language::Rhai => script::diagnostics(&lines, host.check_script(&document.text).err()),
        };
        PublishDiagnosticsParams {
            uri,
            diagnostics,
            version,
        }
    }

    fn request(&self, request: Request) -> Response {
        let id = request.id.clone();
        let result = match request.method.as_str() {
            Completion::METHOD => {
                request
                    .extract::<CompletionParams>(Completion::METHOD)
                    .map(|(_, p)| {
                        let at = p.text_document_position;
                        self.with(&at.text_document.uri, |doc, lines| {
                            let items = match doc.language {
                                Language::Grind => {
                                    grind::complete(doc.grind.as_ref(), lines, at.position)
                                }
                                Language::Rhai => {
                                    script::complete(&self.vocabulary, lines, at.position)
                                }
                            };
                            serde_json::to_value(CompletionResponse::Array(items))
                        })
                    })
            }
            HoverRequest::METHOD => {
                request
                    .extract::<HoverParams>(HoverRequest::METHOD)
                    .map(|(_, p)| {
                        let at = p.text_document_position_params;
                        self.with(&at.text_document.uri, |doc, lines| {
                            let hover = match doc.language {
                                Language::Grind => doc
                                    .grind
                                    .as_ref()
                                    .and_then(|g| grind::hover(g, lines, at.position)),
                                Language::Rhai => {
                                    script::hover(&self.vocabulary, lines, at.position)
                                }
                            };
                            serde_json::to_value(hover)
                        })
                    })
            }
            InlayHintRequest::METHOD => request
                .extract::<InlayHintParams>(InlayHintRequest::METHOD)
                .map(|(_, p)| {
                    self.with(&p.text_document.uri, |doc, lines| {
                        let hints = doc
                            .grind
                            .as_ref()
                            .map(|g| grind::inlay_hints(g, lines, p.range))
                            .unwrap_or_default();
                        serde_json::to_value(hints)
                    })
                }),
            GotoDefinition::METHOD => request
                .extract::<GotoDefinitionParams>(GotoDefinition::METHOD)
                .map(|(_, p)| {
                    let at = p.text_document_position_params;
                    let uri = at.text_document.uri.clone();
                    self.with(&at.text_document.uri, |doc, lines| {
                        let found = doc
                            .grind
                            .as_ref()
                            .and_then(|g| grind::definition(g, lines, at.position))
                            .map(|range| {
                                GotoDefinitionResponse::Scalar(Location {
                                    uri: uri.clone(),
                                    range,
                                })
                            });
                        serde_json::to_value(found)
                    })
                }),
            _ => {
                return Response::new_err(
                    id,
                    ErrorCode::MethodNotFound as i32,
                    format!("grind lsp does not answer {}", request.method),
                );
            }
        };
        match result {
            Ok(value) => Response::new_ok(id, value),
            Err(e) => Response::new_err(id, ErrorCode::InvalidParams as i32, e.to_string()),
        }
    }

    /// Run `f` over an open document; an unknown one answers `null`, which every request takes
    /// to mean "nothing here".
    fn with(
        &self,
        uri: &Uri,
        f: impl FnOnce(&Document, &Lines) -> serde_json::Result<serde_json::Value>,
    ) -> serde_json::Value {
        match self.documents.get(uri.as_str()) {
            Some(document) => f(document, &Lines::new(&document.text)).unwrap_or_default(),
            None => serde_json::Value::Null,
        }
    }
}
