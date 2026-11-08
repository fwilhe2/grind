// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The server in-process: requests in, responses out, no socket.

use std::path::{Path, PathBuf};
use std::process::Command;

use grind_server::Server;
use grind_server::http::{Request, Response};
use serde_json::Value;

struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Dir {
        let dir = std::env::temp_dir().join(format!("grind-server-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Dir(dir)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn data(path: &str) -> Vec<u8> {
    std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join(path)).unwrap()
}

fn get(path: &str) -> Request {
    Request::new("GET", path).with_header("Host", "127.0.0.1:8080")
}

/// A request as the page sends one.
fn send(method: &str, path: &str) -> Request {
    Request::new(method, path)
        .with_header("Host", "127.0.0.1:8080")
        .with_header("X-Grind-Request", "1")
}

fn json(response: &Response) -> Value {
    serde_json::from_slice(&response.body).unwrap_or(Value::Null)
}

fn docs(server: &Server) -> Vec<Value> {
    json(&server.handle(&get("/api/v1/docs")))["docs"]
        .as_array()
        .unwrap()
        .clone()
}

fn id_of(server: &Server, name: &str) -> String {
    docs(server)
        .iter()
        .find(|d| d["name"] == name)
        .unwrap_or_else(|| panic!("{name} is not listed"))["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// What the page does: open the bytes, type into A1, save in the same form.
fn edit_a1(bytes: &[u8], input: &str) -> Vec<u8> {
    edit(bytes, 0, input)
}

fn edit(bytes: &[u8], col: u32, input: &str) -> Vec<u8> {
    let app = grind_sheet::App::new();
    app.open_bytes("budget.fods", bytes).unwrap();
    app.enter(
        0,
        grind_sheet::Pos::new(0, col),
        input,
        grind_sheet::RecalcMode::Document,
    )
    .unwrap();
    app.save_bytes(grind_sheet::Form::Flat).unwrap()
}

#[test]
fn hello_says_what_it_offers() {
    let dir = Dir::new("hello");
    let server = Server::new(&dir.0, None, &[]).unwrap();
    let hello = server.handle(&get("/api/v1/hello"));
    assert_eq!(hello.status, 200);
    let body = json(&hello);
    assert_eq!(body["protocol"], grind_server::PROTOCOL);
    assert_eq!(body["offers"], serde_json::json!(["store"]));
    assert_eq!(body["commit"], grind_core::build_info::COMMIT);
    assert!(body["source"].as_str().unwrap().starts_with("https://"));
}

#[test]
fn the_library_is_the_directory() {
    let dir = Dir::new("library");
    std::fs::write(
        dir.0.join("budget.fods"),
        data("sheet/tests/data/kb/formula.fods"),
    )
    .unwrap();
    std::fs::create_dir(dir.0.join("letters")).unwrap();
    std::fs::write(
        dir.0.join("letters/one.fodt"),
        data("text/tests/data/numbered-list.fodt"),
    )
    .unwrap();
    std::fs::write(dir.0.join("notes.txt"), "not a document").unwrap();
    std::fs::write(dir.0.join(".hidden.fods"), "hidden").unwrap();
    let server = Server::new(&dir.0, None, &[]).unwrap();
    let names: Vec<_> = docs(&server).iter().map(|d| d["name"].clone()).collect();
    assert_eq!(names, ["budget.fods", "letters/one.fodt"]);
    let kinds: Vec<_> = docs(&server).iter().map(|d| d["kind"].clone()).collect();
    assert_eq!(kinds, ["spreadsheet", "text"]);

    // Ids are kept: listing again, or after a restart, names the same documents the same way.
    let id = id_of(&server, "budget.fods");
    let again = Server::new(&dir.0, None, &[]).unwrap();
    assert_eq!(id_of(&again, "budget.fods"), id);

    let opened = server.handle(&get(&format!("/api/v1/docs/{id}")));
    assert_eq!(opened.status, 200);
    assert_eq!(opened.body, data("sheet/tests/data/kb/formula.fods"));
    assert_eq!(
        opened.header("ETag"),
        Some(grind_server::etag(&opened.body).as_str())
    );
    assert_eq!(server.handle(&get("/api/v1/docs/nope")).status, 404);
    let head = Request::new("HEAD", &format!("/api/v1/docs/{id}")).with_header("Host", "localhost");
    assert_eq!(server.handle(&head).header("ETag"), opened.header("ETag"));
}

#[test]
fn a_save_from_the_page_is_the_file_the_cli_would_write() {
    let dir = Dir::new("save");
    let original = data("sheet/tests/data/kb/formula.fods");
    std::fs::write(dir.0.join("budget.fods"), &original).unwrap();
    let server = Server::new(&dir.0, None, &[]).unwrap();
    let id = id_of(&server, "budget.fods");
    let opened = server.handle(&get(&format!("/api/v1/docs/{id}")));
    let tag = opened.header("ETag").unwrap().to_owned();

    let edited = edit_a1(&opened.body, "7");
    let put = send("PUT", &format!("/api/v1/docs/{id}"))
        .with_header("If-Match", &tag)
        .with_body(edited.clone());
    let saved = server.handle(&put);
    assert_eq!(
        saved.status,
        200,
        "{}",
        String::from_utf8_lossy(&saved.body)
    );
    let on_disk = std::fs::read(dir.0.join("budget.fods")).unwrap();
    // `grind sheet set budget.fods A1 7` is the same core over the same file.
    assert_eq!(on_disk, edit_a1(&original, "7"));
    assert_eq!(on_disk, edited);
    assert_ne!(saved.header("ETag"), Some(tag.as_str()));
}

#[test]
fn the_second_save_from_an_old_version_is_refused() {
    let dir = Dir::new("conflict");
    std::fs::write(
        dir.0.join("budget.fods"),
        data("sheet/tests/data/kb/formula.fods"),
    )
    .unwrap();
    let server = Server::new(&dir.0, None, &[]).unwrap();
    let id = id_of(&server, "budget.fods");
    let opened = server.handle(&get(&format!("/api/v1/docs/{id}")));
    let tag = opened.header("ETag").unwrap().to_owned();
    let path = format!("/api/v1/docs/{id}");

    let first = send("PUT", &path)
        .with_header("If-Match", &tag)
        .with_body(edit_a1(&opened.body, "1"));
    assert_eq!(server.handle(&first).status, 200);
    let second = send("PUT", &path)
        .with_header("If-Match", &tag)
        .with_body(edit_a1(&opened.body, "2"));
    let refused = server.handle(&second);
    assert_eq!(refused.status, 412);
    let now = grind_server::etag(&std::fs::read(dir.0.join("budget.fods")).unwrap());
    assert_eq!(refused.header("ETag"), Some(now.as_str()));
    // Overwriting is a choice the page offers, and says so with `*`.
    let forced = send("PUT", &path)
        .with_header("If-Match", "*")
        .with_body(edit_a1(&opened.body, "2"));
    assert_eq!(server.handle(&forced).status, 200);
    // A save that names no version at all is not guessed at.
    let blind = send("PUT", &path).with_body(edit_a1(&opened.body, "3"));
    assert_eq!(server.handle(&blind).status, 428);

    // Another program writing the file is a new version too.
    let before = server
        .handle(&get(&path))
        .header("ETag")
        .unwrap()
        .to_owned();
    std::fs::write(dir.0.join("budget.fods"), edit_a1(&opened.body, "9")).unwrap();
    let stale = send("PUT", &path)
        .with_header("If-Match", &before)
        .with_body(edit_a1(&opened.body, "4"));
    assert_eq!(server.handle(&stale).status, 412);
}

#[test]
fn what_is_written_must_be_the_document_the_name_says() {
    let dir = Dir::new("check");
    let server = Server::new(&dir.0, None, &[]).unwrap();
    let sheet = data("sheet/tests/data/kb/formula.fods");
    let text = data("text/tests/data/numbered-list.fodt");
    let create = |name: &str, bytes: &[u8]| {
        server.handle(&send("POST", &format!("/api/v1/docs?name={name}")).with_body(bytes.to_vec()))
    };
    assert_eq!(
        create("a.fods", &text).status,
        422,
        "a text document as a spreadsheet"
    );
    assert_eq!(
        create("a.ods", &sheet).status,
        422,
        "a flat file as a package"
    );
    assert_eq!(create("a.fods", b"<nonsense").status, 422);
    assert_eq!(create("../a.fods", &sheet).status, 400);
    assert_eq!(create(".a.fods", &sheet).status, 400);
    assert_eq!(create("a.txt", &sheet).status, 400);
    assert!(
        std::fs::read_dir(&dir.0).unwrap().all(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with('.'))
    );

    let made = create("Haushalt%202026.fods", &sheet);
    assert_eq!(made.status, 201);
    assert_eq!(json(&made)["name"], "Haushalt 2026.fods");
    assert_eq!(
        std::fs::read(dir.0.join("Haushalt 2026.fods")).unwrap(),
        sheet
    );
    assert_eq!(create("Haushalt%202026.fods", &sheet).status, 409);
}

#[test]
fn rename_and_delete_keep_the_id_and_the_bytes() {
    let dir = Dir::new("rename");
    let sheet = data("sheet/tests/data/kb/formula.fods");
    std::fs::write(dir.0.join("old.fods"), &sheet).unwrap();
    let server = Server::new(&dir.0, None, &[]).unwrap();
    let id = id_of(&server, "old.fods");
    let path = format!("/api/v1/docs/{id}");

    let renamed = server.handle(&send("PATCH", &path).with_body(r#"{"name":"new.fods"}"#));
    assert_eq!(renamed.status, 200);
    assert_eq!(id_of(&server, "new.fods"), id);
    assert_eq!(docs(&server).len(), 1);
    let kind_change = server.handle(&send("PATCH", &path).with_body(r#"{"name":"new.ods"}"#));
    assert_eq!(kind_change.status, 400);

    assert_eq!(server.handle(&send("DELETE", &path)).status, 204);
    assert!(docs(&server).is_empty());
    let trash = dir.0.join(".grind/trash").join(format!("{id}-new.fods"));
    assert_eq!(std::fs::read(trash).unwrap(), sheet);
}

#[test]
fn a_request_from_elsewhere_is_refused() {
    let dir = Dir::new("foreign");
    std::fs::write(
        dir.0.join("a.fods"),
        data("sheet/tests/data/kb/formula.fods"),
    )
    .unwrap();
    let server = Server::new(&dir.0, None, &["grind.home.arpa".to_owned()]).unwrap();
    let id = id_of(&server, "a.fods");

    // DNS rebinding: a name this server is not.
    let rebound = Request::new("GET", "/api/v1/docs").with_header("Host", "evil.example:8080");
    assert_eq!(server.handle(&rebound).status, 421);
    let unnamed = Request::new("GET", "/api/v1/docs");
    assert_eq!(server.handle(&unnamed).status, 421);
    let proxied = Request::new("GET", "/api/v1/docs").with_header("Host", "grind.home.arpa");
    assert_eq!(server.handle(&proxied).status, 200);

    // A form on another site can post here; it cannot add the page's header, and its origin
    // gives it away when it tries.
    let forged =
        Request::new("DELETE", &format!("/api/v1/docs/{id}")).with_header("Host", "localhost:8080");
    assert_eq!(server.handle(&forged).status, 403);
    let cross =
        send("DELETE", &format!("/api/v1/docs/{id}")).with_header("Origin", "https://evil.example");
    assert_eq!(server.handle(&cross).status, 403);
    assert_eq!(docs(&server).len(), 1);
    let same = send("DELETE", &format!("/api/v1/docs/{id}"))
        .with_header("Origin", "http://127.0.0.1:8080");
    assert_eq!(server.handle(&same).status, 204);
}

#[test]
fn the_page_is_served_and_nothing_beside_it() {
    let dir = Dir::new("page-store");
    let page = Dir::new("page");
    std::fs::write(page.0.join("index.html"), "<title>grind</title>").unwrap();
    std::fs::write(page.0.join("grind_web_bg.wasm"), b"\0asm").unwrap();
    std::fs::write(page.0.join(".secret"), "no").unwrap();
    std::fs::write(dir.0.join("outside.txt"), "no").unwrap();
    let server = Server::new(&dir.0, Some(page.0.clone()), &[]).unwrap();

    let index = server.handle(&get("/"));
    assert_eq!(
        (index.status, index.body.as_slice()),
        (200, &b"<title>grind</title>"[..])
    );
    let wasm = server.handle(&get("/grind_web_bg.wasm"));
    assert_eq!(wasm.header("Content-Type"), Some("application/wasm"));
    for path in [
        "/.secret",
        "/../outside.txt",
        "/%2e%2e/outside.txt",
        "/nope.js",
    ] {
        assert_eq!(server.handle(&get(path)).status, 404, "{path}");
    }
    assert_eq!(server.handle(&send("PUT", "/index.html")).status, 405);

    // A `.gz` beside a file is that file, for a browser that takes gzip.
    std::fs::write(page.0.join("grind_web_bg.wasm.gz"), b"\x1f\x8b").unwrap();
    let compressed =
        server.handle(&get("/grind_web_bg.wasm").with_header("Accept-Encoding", "gzip, br"));
    assert_eq!(compressed.body, b"\x1f\x8b");
    assert_eq!(compressed.header("Content-Encoding"), Some("gzip"));
    assert_eq!(compressed.header("Content-Type"), Some("application/wasm"));
    assert_eq!(compressed.header("Vary"), Some("Accept-Encoding"));
    for refused in ["", "br", "gzip;q=0", "identity"] {
        let plain =
            server.handle(&get("/grind_web_bg.wasm").with_header("Accept-Encoding", refused));
        assert_eq!(plain.body, b"\0asm", "{refused:?}");
        assert_eq!(plain.header("Content-Encoding"), None, "{refused:?}");
    }

    let bare = Server::new(&dir.0, None, &[]).unwrap();
    assert_eq!(bare.handle(&get("/")).status, 404);
    assert_eq!(bare.handle(&get("/api/v1/hello")).status, 200);
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn in_a_git_work_tree_every_save_is_a_commit() {
    if Command::new("git").arg("--version").output().is_err() {
        eprintln!("no git: skipped");
        return;
    }
    let dir = Dir::new("history");
    git(&dir.0, &["init", "-q"]);
    git(&dir.0, &["config", "user.name", "Test"]);
    git(&dir.0, &["config", "user.email", "test@example.invalid"]);
    git(&dir.0, &["config", "commit.gpgsign", "false"]);
    let server = Server::new(&dir.0, None, &[]).unwrap();
    assert_eq!(
        json(&server.handle(&get("/api/v1/hello")))["offers"],
        serde_json::json!(["store", "history"])
    );
    // No formulas, so B1, one line in the file, is all that changes.
    let sheet = data("sheet/tests/data/kb/minimal-with-styles.fods");
    let made =
        server.handle(&send("POST", "/api/v1/docs?name=budget.fods").with_body(sheet.clone()));
    assert_eq!(made.status, 201);
    let id = json(&made)["id"].as_str().unwrap().to_owned();
    let path = format!("/api/v1/docs/{id}");

    let mut bytes = sheet.clone();
    for n in 1..=20 {
        let tag = grind_server::etag(&bytes);
        bytes = edit(&bytes, 1, &n.to_string());
        let put = send("PUT", &path)
            .with_header("If-Match", &tag)
            .with_body(bytes.clone());
        assert_eq!(server.handle(&put).status, 200);
        // One cell is one line of diff, both ways.
        let stat = git(&dir.0, &["show", "--numstat", "--format=", "HEAD"]);
        assert_eq!(stat.trim(), "1\t1\tbudget.fods", "save {n}");
    }
    let renamed = server.handle(&send("PATCH", &path).with_body(r#"{"name":"plan.fods"}"#));
    assert_eq!(renamed.status, 200);

    let history = json(&server.handle(&get(&format!("{path}/history"))));
    let versions = history["versions"].as_array().unwrap();
    // Added, twenty saves, renamed — and the rename followed.
    assert_eq!(versions.len(), 22);
    assert_eq!(versions[0]["subject"], "Rename budget.fods to plan.fods");
    assert_eq!(versions[21]["subject"], "Add budget.fods");
    assert_eq!(versions[1]["author"], "Test");

    let first = versions[21]["rev"].as_str().unwrap();
    let at = server.handle(&get(&format!("{path}/at/{first}")));
    assert_eq!(at.body, sheet);
    assert_eq!(server.handle(&get(&format!("{path}/at/HEAD"))).status, 404);

    let restore = send("POST", &format!("{path}/restore"))
        .with_header("If-Match", &grind_server::etag(&bytes))
        .with_body(format!(r#"{{"rev":"{first}"}}"#));
    let restored = server.handle(&restore);
    assert_eq!(
        restored.status,
        200,
        "{}",
        String::from_utf8_lossy(&restored.body)
    );
    assert_eq!(std::fs::read(dir.0.join("plan.fods")).unwrap(), sheet);
    assert!(git(&dir.0, &["log", "-1", "--format=%s"]).starts_with("Restore plan.fods to "));
    // The store's own directory is never part of the history.
    assert!(!git(&dir.0, &["ls-files"]).contains(".grind"));

    assert_eq!(server.handle(&send("DELETE", &path)).status, 204);
    assert_eq!(
        git(&dir.0, &["log", "-1", "--format=%s"]).trim(),
        "Delete plan.fods"
    );
}
