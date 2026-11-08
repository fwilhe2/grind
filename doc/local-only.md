<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: CC-BY-4.0
-->

# Local only

**Decided 2026-11-07.** grind runs on the person's computer and talks to no other one.

> **R12 — no network.** Nothing grind builds sends or fetches anything over a network: no
> service, no update check, no telemetry, no server named in a setting. Not as a default, not
> as an option. Checked by `cli/tests/local_only.rs`: no HTTP, WebSocket or TLS client crate in
> `Cargo.lock`, no manifest turning on a browser or Windows network API (`RequestInit`,
> `Win32_Networking`, …), and no socket or spawned downloader in shipped code.

## What it rules out, and what it does not

- **Ruled out:** any feature that needs another computer to work, however useful, including one
  that does nothing until configured. A setting that can name a remote server is a way for the
  text to leave, and "local unless you change it" is a weaker promise than this one.
- **Not affected:** a link in a document, opened in the system's browser when the person clicks
  it. The person asked for that, and the browser does it, not grind.
- **The browser shell** is a page served from somewhere, and fetches only files beside it — its
  wasm module, dictionaries, fonts, a `?doc=` path. An absolute URL is refused (`doc_param` in
  `ui_web/src/lib.rs`), and `ui_web/smoke.sh` fails on any fetch that leaves the page's own
  files.
- **`grind serve`** (`doc/server.md`, decided 2026-11-08) is a listener, not a client, and it
  listens on `127.0.0.1` with no option to name another address: the person's own browser on
  the person's own computer. Another computer reaches it only through a reverse proxy the person
  runs: the Kubernetes sample puts Caddy in the pod for that, a separate program in a separate
  container, and grind itself still listens on loopback alone. It sends nothing anywhere, and refuses a request
  addressed to a name it was not given or written by another site's page. The page's writes to
  it are relative `api/v1/` paths from one module, `ui_web/src/server.rs`. The guard holds all of
  it: one bind, with the loopback address written into it, and `tiny_http` depended on by
  `grind-server` alone.
- **Tests and tools** are not what grind ships. The LibreOffice and Euro-Office oracles run in
  containers, the corpora are downloaded by scripts, CI fetches crates. A test may serve itself
  on loopback; the guard leaves `tests/` directories alone.

## What it removed

The LanguageTool integration (`doc/grammar.md` §5): the one feature that talked to a server.
Grammar is checked by the built-in checker alone. If it comes back, R12 sets the shape: a server
on the loopback address, or a program the person installs separately, and never a public
service.
