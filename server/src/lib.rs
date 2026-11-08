// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `grind serve`: the browser shell and a directory of documents behind it, on the loopback
//! address and nowhere else (`doc/server.md`, `doc/local-only.md`). The page probes
//! `api/v1/hello` and is the static page it always was when nothing answers.

mod git;
pub mod http;
mod store;

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use http::{Request, Response, percent_decode};
use store::{Refusal, Store};

pub use store::etag;

/// What the page and the server agree on; bumped by a change either side would misread.
pub const PROTOCOL: u32 = 1;

/// The largest document a request may carry.
const LARGEST: u64 = 64 * 1024 * 1024;

pub struct Options {
    /// The store.
    pub dir: PathBuf,
    /// 0 is any free port.
    pub port: u16,
    /// The built browser shell (`ui_web/dist`); without one only the API answers.
    pub page: Option<PathBuf>,
    /// Names besides `localhost` and `127.0.0.1` a request may be addressed to — a reverse
    /// proxy's.
    pub hosts: Vec<String>,
}

pub struct Server {
    store: Mutex<Store>,
    page: Option<PathBuf>,
    hosts: Vec<String>,
}

impl Server {
    pub fn new(dir: &Path, page: Option<PathBuf>, hosts: &[String]) -> Result<Server, String> {
        let mut allowed = vec!["localhost".to_owned(), "127.0.0.1".to_owned()];
        allowed.extend(hosts.iter().map(|h| h.to_ascii_lowercase()));
        Ok(Server {
            store: Mutex::new(Store::open(dir)?),
            page,
            hosts: allowed,
        })
    }

    pub fn handle(&self, request: &Request) -> Response {
        if let Some(refused) = self.refuse_foreign(request) {
            return refused;
        }
        let response = match request.path.strip_prefix("/api/v1/") {
            Some(rest) => self.api(request, rest).with("Cache-Control", "no-store"),
            None => self.page(request).with("Cache-Control", "no-cache"),
        };
        response
            .with("X-Content-Type-Options", "nosniff")
            .with("X-Frame-Options", "DENY")
    }

    /// The loopback address is still reachable from every page the person's browser opens. A name
    /// that is not this server's is DNS rebinding, and a write without the page's own header, or
    /// from another origin, is a forged one.
    fn refuse_foreign(&self, request: &Request) -> Option<Response> {
        let known = |authority: &str| {
            let host = match authority.rsplit_once(':') {
                Some((host, port)) if port.bytes().all(|b| b.is_ascii_digit()) => host,
                _ => authority,
            };
            self.hosts.iter().any(|h| h.eq_ignore_ascii_case(host))
        };
        if !request.header("Host").is_some_and(known) {
            return Some(Response::error(
                421,
                "this server does not answer to that name",
            ));
        }
        if matches!(request.method.as_str(), "GET" | "HEAD") {
            return None;
        }
        let origin_ok = request.header("Origin").is_none_or(|origin| {
            origin
                .split_once("://")
                .is_some_and(|(_, authority)| known(authority))
        });
        match request.header("X-Grind-Request").is_some() && origin_ok {
            true => None,
            false => Some(Response::error(
                403,
                "not a request from this server's page",
            )),
        }
    }

    fn api(&self, request: &Request, rest: &str) -> Response {
        let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
        let segments: Vec<&str> = rest.split('/').collect();
        let method = request.method.as_str();
        let if_match = request.header("If-Match");
        let result = match (method, segments.as_slice()) {
            ("GET", ["hello"]) => return hello(&store),
            ("GET", ["docs"]) => store.list().map(|docs| docs_json(&docs)),
            ("POST", ["docs"]) => match request.param("name") {
                Some(name) => store
                    .create(&name, &request.body)
                    .map(|entry| Response::json(201, &entry).with("ETag", &entry.etag)),
                None => Err(Refusal::BadName("a new document needs ?name=".to_owned())),
            },
            ("GET", ["docs", id]) => store.get(id).map(|(entry, bytes)| {
                let kind = match entry.kind {
                    "spreadsheet" => grind_core::DocumentKind::Spreadsheet,
                    _ => grind_core::DocumentKind::Text,
                };
                Response::bytes(200, kind.media_type(), bytes).with("ETag", &entry.etag)
            }),
            ("PUT", ["docs", id]) => match if_match {
                Some(tag) => store.put(id, tag, &request.body).map(saved),
                None => return Response::error(428, "a save says which version it replaces"),
            },
            ("PATCH", ["docs", id]) => match json_field(&request.body, "name") {
                Some(name) => store.rename(id, if_match, &name).map(saved),
                None => Err(Refusal::BadName("a rename needs {\"name\": …}".to_owned())),
            },
            ("DELETE", ["docs", id]) => store.delete(id, if_match).map(|()| Response::empty(204)),
            ("GET", ["docs", id, "history"]) => store
                .history(id)
                .map(|versions| Response::json(200, &serde_json::json!({ "versions": versions }))),
            ("GET", ["docs", id, "at", rev]) => store
                .at(id, rev)
                .map(|bytes| Response::bytes(200, "application/octet-stream", bytes)),
            ("POST", ["docs", id, "restore"]) => {
                match (if_match, json_field(&request.body, "rev")) {
                    (Some(tag), Some(rev)) => store.restore(id, tag, &rev).map(saved),
                    (None, _) => {
                        return Response::error(428, "a restore says which version it replaces");
                    }
                    (_, None) => Err(Refusal::BadName("a restore needs {\"rev\": …}".to_owned())),
                }
            }
            (_, ["hello" | "docs"] | ["docs", _] | ["docs", _, "history" | "restore"]) => {
                return Response::error(405, format!("{method} is not a verb here"));
            }
            _ => Err(Refusal::NotFound),
        };
        result.unwrap_or_else(refused)
    }

    /// The page's own files, `index.html` at the root.
    fn page(&self, request: &Request) -> Response {
        if !matches!(request.method.as_str(), "GET" | "HEAD") {
            return Response::error(405, "the page is read only");
        }
        let Some(root) = &self.page else {
            return Response::bytes(
                404,
                "text/plain; charset=utf-8",
                b"grind serve has no page to serve: pass --page with a built ui_web/dist.\n"
                    .to_vec(),
            );
        };
        let Some(relative) = page_path(&request.path) else {
            return Response::error(404, "not found");
        };
        match std::fs::read(root.join(&relative)) {
            Ok(bytes) => Response::bytes(200, content_type(&relative), bytes),
            Err(_) => Response::error(404, "not found"),
        }
    }
}

fn hello(store: &Store) -> Response {
    let mut offers = vec!["store"];
    if store.has_history() {
        offers.push("history");
    }
    Response::json(
        200,
        &serde_json::json!({
            "protocol": PROTOCOL,
            "commit": grind_core::build_info::COMMIT,
            "build": grind_core::build_info::describe("grind serve", env!("CARGO_PKG_VERSION")),
            "offers": offers,
            // AGPL §13: whoever uses this over a network is offered its source.
            "source": env!("CARGO_PKG_REPOSITORY"),
        }),
    )
}

fn docs_json(docs: &[store::Entry]) -> Response {
    Response::json(200, &serde_json::json!({ "docs": docs }))
}

fn saved(entry: store::Entry) -> Response {
    Response::json(200, &entry).with("ETag", &entry.etag)
}

fn refused(refusal: Refusal) -> Response {
    match refusal {
        Refusal::NotFound => Response::error(404, "no such document"),
        Refusal::Stale(now) => {
            Response::error(412, "somebody saved this document first").with("ETag", &now)
        }
        Refusal::Exists(name) => Response::error(409, format!("{name} already exists")),
        Refusal::BadName(why) => Response::error(400, why),
        Refusal::Unreadable(why) => Response::error(422, why),
        Refusal::NoHistory => Response::error(404, "this store keeps no history"),
        Refusal::Failed(why) => Response::error(500, why),
    }
}

fn json_field(body: &[u8], field: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    value.get(field)?.as_str().map(str::to_owned)
}

/// A request path as a path below the page's directory: decoded, and with no segment that could
/// climb out of it or reach a hidden file.
fn page_path(path: &str) -> Option<PathBuf> {
    let decoded = percent_decode(path)?;
    let mut out = PathBuf::new();
    for segment in decoded.split('/').filter(|s| !s.is_empty()) {
        if segment.starts_with('.') || segment.contains(['\\', '\0']) {
            return None;
        }
        out.push(segment);
    }
    if out.as_os_str().is_empty() {
        out.push("index.html");
    }
    Some(out)
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "wasm" => "application/wasm",
        "json" => "application/json",
        "ttf" => "font/ttf",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "aff" | "dic" | "txt" => "text/plain; charset=utf-8",
        "fods" => grind_core::DocumentKind::Spreadsheet.media_type(),
        "fodt" => grind_core::DocumentKind::Text.media_type(),
        _ => "application/octet-stream",
    }
}

/// Listen on `127.0.0.1` — there is no option for any other address — and answer until killed.
/// `ready` is told the port, which is the one that was free when `options.port` is 0.
pub fn serve(options: Options, ready: impl FnOnce(u16)) -> Result<(), String> {
    let server = Arc::new(Server::new(&options.dir, options.page, &options.hosts)?);
    let listener = tiny_http::Server::http(("127.0.0.1", options.port))
        .map_err(|e| format!("127.0.0.1:{}: {e}", options.port))?;
    let port = listener
        .server_addr()
        .to_ip()
        .map_or(options.port, |address| address.port());
    ready(port);
    for incoming in listener.incoming_requests() {
        let server = server.clone();
        std::thread::spawn(move || answer(&server, incoming));
    }
    Ok(())
}

fn answer(server: &Server, mut incoming: tiny_http::Request) {
    let mut request = Request::new(incoming.method().as_str(), incoming.url());
    request.headers = incoming
        .headers()
        .iter()
        .map(|h| (h.field.as_str().to_string(), h.value.as_str().to_owned()))
        .collect();
    let too_large = incoming.body_length().is_some_and(|n| n as u64 > LARGEST);
    let mut body = Vec::new();
    let read = incoming
        .as_reader()
        .take(LARGEST + 1)
        .read_to_end(&mut body);
    let response = match read {
        _ if too_large || body.len() as u64 > LARGEST => {
            Response::error(413, "a document that large is not taken")
        }
        Err(error) => Response::error(400, error.to_string()),
        Ok(_) => server.handle(&request.with_body(body)),
    };
    let mut reply = tiny_http::Response::from_data(response.body).with_status_code(response.status);
    for (name, value) in &response.headers {
        if let Ok(header) = tiny_http::Header::from_bytes(name.as_bytes(), value.as_bytes()) {
            reply.add_header(header);
        }
    }
    if let Err(error) = incoming.respond(reply) {
        eprintln!("grind serve: {error}");
    }
}
