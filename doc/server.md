<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: CC-BY-4.0
-->

# A server behind the browser shell

**Status: plan, unbuilt, nothing decided until the open questions at the end are answered.**
`doc/web-shell.md` stays normative for `ui_web/`; `doc/web-storage.md` is the record of what the
page can keep *without* a server, and this document is the other half of its *Remote storage,
later* and *What neither option does* sections. `doc/plan.md`'s R1–R7 and the architecture rules
in `CLAUDE.md` outrank every word here.

## 1. The one rule: the server is found, never required

`ui_web/dist` is a static page today and it stays one. Served from GitHub Pages, from
`ui_web/Dockerfile`'s static image or from `python -m http.server`, it must do exactly what it does
now — open bytes, edit, download — with no request to anything it did not fetch already.

So the page **probes** and never assumes:

- On start it asks `GET api/v1/hello` **relative to its own address**. A 404, a network error or
  a body it does not understand means *no server*, and the page is the page it is today. Nothing
  is shown about a server that is not there.
- A server answers with its protocol version, its `grind_core::build_info` and what it offers
  (`store`, `live`, …). The page turns on exactly those features.
- **Same origin only, at first.** A server is the thing that serves the page, so there is no CORS,
  no mixed content, no configuration and no second URL anybody types. A Pages-hosted page talking
  to somebody's home server (`?server=https://…`) is a later, separate decision — it needs CORS,
  credentials in the browser and an answer to version skew (§5.4) — and is listed in §10.

Inside `ui_web`, everything a server changes goes through one seam — a `Backend` with two
implementations, `Local` (today's file picker and download, unchanged) and `Remote`. A command in
`command.rs` that needs a server is simply absent from the palette when there is none, by the same
omission rule that already hides the sheet's verbs over a text document.

## 2. What it is, physically

**`grind serve DIR`** — a suite-level verb of the `grind` binary, behind a `server` cargo feature,
the way `grind lsp` is behind `lsp`. The code is its own crate, **`grind-server`** (`server/`), and
`grind-cli` is a thin arm over it. One binary, no second thing to package; `--no-default-features`
still builds a `grind` with none of it.

- **Stack**: `axum` on `tokio`, its built-in WebSocket for live sessions. Plumbing, so it can be
  lazy (`CLAUDE.md`: format-neutral plumbing can be). It is the largest dependency this workspace
  would add to the CLI — **measure it** (`artifacts.yml` already reports sizes) and, if it moves
  the 3.6 MB figure too far, ship the feature off by default and enable it in the container image.
- **It serves the page.** `grind serve` serves a `dist/` (`--dist`, defaulting to one embedded in
  or installed beside the binary) as well as `/api/v1/`. That is what makes same-origin free, and
  it means **the page and the server are always the same build** (§5.4).
- **R11 holds**: `grind-server` opens documents, so it does not depend on `grind-build`. A
  generator stays something a person runs.
- **AGPL §13**: a server is exactly where it applies. The About dialog already carries the build
  stamp; the server adds a source link to `hello` and the page shows it.
- **Deployment**: a second image, `grind-server`, beside the unchanged static `grind-web` —
  `ui_web/deploy/` grows a `compose.yaml` variant with a volume, the first deployment of this
  project that has state to back up.

## 3. What it can do

In the order it should be built, each tier useful without the next.

### Tier 1 — a place documents live (the store)

- **A directory of flat ODF files** is the store — `DIR`, nothing else. Not a database and not the
  model: `doc/web-storage.md`'s *store ODF bytes, never the model* and `doc/flat-first.md` both
  say so, and it buys the thing no database would: **every other client opens the same files**.
  `grind sheet set DIR/budget.fods A1 1`, the GNOME window, `git diff` — they already work on it.
- Stable ids in a sidecar (`DIR/.grind/index.json`: id → path), because a name is not a key
  (`doc/web-storage.md`, *cheap now, expensive later*).
- **List, open, save, create, rename, delete** over plain HTTP: `GET /api/v1/docs`,
  `GET /api/v1/docs/{id}` (bytes + `ETag`), `PUT` with `If-Match` (412 when somebody else saved
  first — whole-document conflict detection, never a merge, at this tier), `POST`, `DELETE`.
- **Save is the core's save.** The server never writes bytes the page computed blindly: a `PUT`
  is read with `App::open_bytes` and written by the server's own `save_bytes` into the file it
  came from, so R6 splicing and rule 7 (*never worse*, `Error::WouldLose`) hold on the server
  exactly as on a desktop. (The page sends bytes it saved itself; the server checks them by
  reading them, and refuses what does not read.)
- **Other writers.** A file changed on disk while nobody has it open is simply the new document. A
  file changed on disk while a session has it open is a conflict notice (`notify` crate for
  inotify/FSEvents, or a cheap mtime check on every request at first).
- In the page: a **library** pane on the welcome screen (Open from server / New on server), Save
  writing back instead of downloading, Download still there. This is `doc/web-storage.md`'s
  option B with remote storage, and it answers that document's open question 1: a second device
  sees the documents.

### Tier 2 — history

- **Versions are git commits** when `DIR` is a git repository: each server save is a commit with
  the editor's name, so *history*, *diff* and *restore* are `git log`, R6's one-line diffs and a
  checkout — the property the flat form and splicing were built for. Run the `git` executable;
  no `gix` dependency for this. Without git, the server keeps nothing but the current file and the
  history verbs are absent (§1's omission rule).
- `GET /api/v1/docs/{id}/history`, `…/at/{rev}` (bytes, opened read-only in the page),
  `POST …/restore`.
- Optionally the projection beside it (`grind project` is already git's `textconv`), so a diff in
  the history pane reads as cells and blocks rather than XML.

### Tier 3 — things a static page cannot do

Most of what a server usually adds the page already does itself — `.xlsx`/`.docx` import,
spelling, PDF export and preview all run in wasm. What is genuinely new:

- **Search across documents**: every cell and every block of every file, indexed on save. This is
  `doc/sqlite.md` Part III's workspace index, and the server is the first place it has a reason
  to exist.
- **Lint the library**: `grind lint` over every file, findings in the library pane.
- **Fetch a URL** for `?doc=` that does not send CORS headers.
- **Sharing a link**: `…/d/{id}` opens that document from the server — read-only or editable, per
  §5.

### Tier 4 — live editing

Several browsers on one document, each seeing the others' edits and carets as they happen. §4 is
how, and it is most of the work.

## 4. Concurrent editing — the design decision

### 4.1 What the core gives us today

Measured in the code, not assumed:

- **The sheet** (`sheet/src/action.rs`): every cell edit is addressed by `(sheet index, Pos)`, and
  **there is no row or column insert or delete at all**. Two people editing different cells
  commute already; the same cell is a last-writer-wins register; the only *structural* actions
  are `InsertSheet`/`RemoveSheet`/`RenameSheet` and charts — and a sheet *index* is unstable
  under insert and remove.
- **The text** (`text/src/action.rs`): three actions, `SetBlock`/`InsertBlock`/`RemoveBlock`,
  addressed by block **index**, and `SetBlock` **replaces the whole block**. Two people typing in
  one paragraph would each overwrite the other's keystrokes. `BlockId`s are stable within an
  `App` but reassigned by a fresh read (`doc/not-doing.md`, *Undo across invocations*).
- **Undo is the inverse of an `Action`** — so rolling local edits back and forth is already a
  core mechanism, not something a collaboration layer has to build.
- **The core is deterministic and the same code runs in wasm and natively.** That is the property
  the whole design below rests on.

### 4.2 Two ways to do it

**A — a CRDT library (Yrs, Automerge, Loro) holding the document.** The document is mirrored into
a CRDT; peers exchange CRDT updates; the CRDT merges; the grind `Document` is rebuilt from it.

- For: offline-first and peer-to-peer for free; mature libraries.
- Against, and these decide it:
  1. **A second model of every field of `Document`** — cells, formats, styles, merges, filters,
     rules, names, charts, checkboxes, blocks, runs, tables — kept in step in both directions
     forever. `doc/projection-sheet.md` exists because one extra spelling of the model already
     needed a mechanical guard; this would be a fourth, and the one with the most complicated
     semantics.
  2. **A CRDT merges syntax, and ODF semantics are the product.** Renaming a sheet rewrites every
     formula that names it (`formula::rename`, an AST substitution). A CRDT that merges a
     concurrent formula edit with that rename produces a formula naming a sheet that no longer
     exists. Only the core can merge it correctly, because only the core knows what a reference
     is — the same argument `CLAUDE.md` makes against an Excel library with a translator.
  3. **It contradicts *store ODF bytes, never the model*.** CRDT history is the model, serialised,
     and stranded by the next change to it.
  4. Its main strength — no authority — is exactly what having a server makes unnecessary.

**B — the server sequences, every client rebases by replay** (ProseMirror's collab model, Figma's
in spirit). **Recommended.**

- The server holds **one `App` per live document**, in one task (an actor: no locks across
  documents, one order per document). Clients send **operations**; the server applies them in the
  order they arrive, numbers each one, and broadcasts it.
- A client applies its own operations at once (optimistically) and keeps them as *pending*. When
  the server's next operation arrives it **undoes its pending operations** (the core's inverses),
  applies the server's, **maps** its pending operations through it, and re-applies them. When its
  own operation comes back numbered, it is no longer pending.
- Because both sides run the same core, *replaying the same operations in the same order gives the
  same document* — on the server, on every client, and in a test. Convergence is a property of
  determinism, not of a merge algorithm.
- A same-cell conflict is resolved by order and **shown**: the loser's cell changes under them, and
  a notice says who changed it — rather than a CRDT's silent interleaving.
- Offline works for as long as the server keeps the operations since the client's last version
  (the op log, §4.4): reconnecting is one long rebase. A document diverged further than that is a
  whole-document conflict, Tier 1's 412.

### 4.3 What B needs from the core

The work is not the server; it is making operations **addressed by identity and mappable**. A new
layer above `Action`, in each app crate (R8: the *mechanism* — version numbers, pending queue,
rebase loop — is generic and lives in `grind_core::collab`; the operations are the apps'):

| Operation | Addressed by | Mapped through a concurrent… |
|---|---|---|
| sheet: enter / clear / format / style a cell or range | `SheetId` + `Pos` | nothing (cells commute; same cell = order wins) |
| sheet: insert / remove / rename sheet | `SheetId` | a rename re-runs `formula::rename` over the other side's formulas — **intent, not text** |
| sheet: tracks, merges, filters, rules, names, charts | `SheetId` + their own key | mostly nothing; merge-over-merge is refused like it is today |
| text: insert text, erase | `BlockId` + offset | an insert or erase earlier in the same block shifts the offset |
| text: split, join | `BlockId` + offset | the offset moves to the block it now lives in |
| text: set kind, character style over a span | `BlockId` + span | the span shifts like an offset pair |
| text: insert / remove / move block | `BlockId` | a removed target makes the op a no-op, reported |

Three changes follow, each worth having anyway:

1. **`SheetId`** — a stable id for a sheet, the way `BlockId` is for a block.
2. **Text operations finer than `SetBlock`.** `App::insert_text`, `erase`, `split_block`,
   `join_block` already exist as *calls*; they become *operations* that can be sent, mapped and
   replayed. `SetBlock` stays the undo entry underneath.
3. **Ids that survive the wire.** The server's `App` is the one place ids are assigned; a joining
   client receives them with the snapshot (§4.4) instead of re-reading the bytes and inventing its
   own.

The position mapping is the one genuinely subtle piece, and it is small: an offset in one block
through an insert or erase in the same block. That is the case a sequence CRDT exists for, and
with a sequencer it is a dozen lines and a property test.

### 4.4 State on the server

- **While a document is live, the server's `App` is the truth**; the file is not rewritten per
  keystroke. It is **flushed** with `save_bytes` — R6, rule 7, a git commit if Tier 2 is on — on
  idle, on an interval, and when the last client leaves.
- Between flushes an **append-only op log** per document survives a crash. It is model-shaped, so
  it is ephemeral by rule: replayed on start by the same build, flushed and deleted; a server
  started by a different build than the log's refuses to replay it and keeps it beside the file.
- **A flush the core refuses** (`Error::WouldLose`) leaves the file alone, keeps the session and
  the log, and tells every client — the same sentence the desktop shells show.
- **A joining client** gets the flushed bytes plus the operations since, or — simpler and the
  first version — the server's current `save_bytes` plus the ids (§4.3, item 3) and the version number.

### 4.5 What else live editing needs

- **Recalculation.** Values are derived; operations carry inputs. Each side recalculates. Risk:
  wasm and native may disagree in the last bit of a transcendental function (`libm` vs. the
  platform's), which could show as a different 15th digit. Test it with
  `sheet/tests/data/functions/` evaluated both ways before trusting it; the fallback is the
  server broadcasting cached values for formula cells.
- **Presence**: every client's caret and selection, as the suite's own addresses (`B7`,
  `C2:D9`, `#intro+5`) — ephemeral, never stored, drawn by the page as a coloured caret or cell
  outline with a name.
- **Undo means *my* edits.** Ctrl+Z inverts the user's own last operation, mapped through
  everything since; if it no longer applies (the cell was changed since by someone else, the
  block was deleted) it is reported, never forced. `App`'s history is per process and stays the
  answer for one person; this is a separate stack in `grind_core::collab`.
- **Version skew** is impossible by construction while the server serves the page (§2); the
  handshake still compares `build_info` and makes the page read-only, with a reload offered, when
  they differ.

## 5. Who may do what

A home server first. In order:

1. **No authentication**, bound to `127.0.0.1` by default; `--listen 0.0.0.0:8080` is an explicit
   choice. A name for presence is asked once and kept in `localStorage`.
2. **A trusted header** from a reverse proxy (`--user-header Remote-User` — Authelia,
   oauth2-proxy, Tailscale serve all set one). grind stores no passwords and builds no login page.
3. Per-document read-only links (a random token in `…/d/{id}?k=…`) if sharing outside the
   household is wanted.

Accounts, roles and permissions per document are a product of their own and stay out (§9).

## 6. CLI parity (rule 4)

The server is a capability, so `grind` reaches it: `grind serve` is the server, and every store
verb is plain HTTP that `curl` reaches. Two more are cheap and worth it:

- `grind remote ls|get|put|history URL` — the store from a terminal.
- **A CLI edit to a live document** must go through the server, or the next flush overwrites it.
  Either the server takes a lock file in `DIR/.grind/` that every `grind` writer honours (refuse,
  name the server), or — later — `grind sheet set http://host/d/{id} A1 1` sends an operation.
  The lock is the first answer.

`doc/feature-matrix.md` gains a *server* column; the desktop shells could join a live session later
(they are Rust and link the same core), but nothing here promises it.

## 7. Milestones

| | What | Exit criterion |
|---|---|---|
| **SV0** | `grind-server` crate, `grind serve DIR` serving `dist/` and `api/v1/hello`; the page's probe and the `Backend` seam | The page on `python -m http.server` and on `grind serve` both pass `ui_web/smoke.sh`; the first shows no server UI, the second shows the probe's answer |
| **SV1** | Tier 1: list, open, save (`If-Match`), create, rename, delete; library pane; the server's own `save_bytes` | Two browsers, one saves first, the other gets 412 and a choice; a document saved from the page is byte-identical to the same edit made by `grind sheet set` (R6 holds) |
| **SV2** | Tier 2: git history, view a version, restore | Twenty saves are twenty commits, each one line of diff for a one-cell edit |
| **SV3** | Deployment: image, `compose.yaml` with a volume, size measured in `artifacts.yml` | `docker compose up` on a clean machine, a document survives `down`/`up` |
| **SV4** | Core prep: `SheetId`; text operations finer than `SetBlock`; `grind_core::collab` (versions, pending, rebase); position mapping | **Convergence test**: N simulated clients, random operations, random delivery delays, all end with byte-identical `save_bytes` — a loop-F-shaped property over the vendored documents |
| **SV5** | Live sessions over WebSocket: the actor, broadcast, rebase in the page, op log, flush | Two pages typing in one paragraph lose no keystroke; killing the server mid-session loses nothing after restart |
| **SV6** | Presence and per-user undo | Carets of others drawn in both panes; Ctrl+Z never undoes someone else's edit |
| **SV7** | Tier 3: search and lint across the library, `?doc=` proxy, share links | — |
| **SV8** | Auth: trusted header, read-only links | — |

SV1–SV3 are useful on their own and change nothing in the core. SV4 is the real decision: it is
where the core gains a layer, and it should not start until SV1 has been lived with.

## 8. Testing

- The server in-process with `tower`'s test client — no port, no browser, `cargo test -p
  grind-server`.
- The page against a real `grind serve` in `ui_web/smoke.sh` (jsdom has `fetch` and a WebSocket
  polyfill); the no-server half of the smoke test stays exactly as it is, and is the check §1's
  rule is kept.
- SV4's convergence property is the important one and is pure Rust: no network, operations
  delivered by a seeded scheduler, the same seed reproducing the same interleaving.
- Recalculation parity wasm/native (§4.5) over the function samples, in CI's wasm job.

## 9. What this will not do

- **Be required.** Every feature of the page without a server stays.
- **Store the model as the document.** The file on disk is ODF; the op log is ephemeral.
- **A CRDT library as the document**, for §4.2's reasons. If peer-to-peer or long offline
  divergence ever becomes a requirement, a sequence CRDT *inside a text block* is the place it
  would go, and nowhere else.
- **Accounts, a user database, per-document permissions.** A reverse proxy's identity is enough.
- **Comments, suggestions, tracked changes.** The model carries none of them (`doc/not-doing.md`);
  live editing does not change that.
- **Scale.** One process, one machine, documents in memory while open — a household or a team,
  not a SaaS.

## 10. Open questions

1. **Is live editing the goal, or the store?** SV1–SV3 are a week's shape; SV4–SV6 are a phase.
   If the real need is "my documents on every device", stop after SV3.
2. **Same origin only, or a Pages-hosted page talking to a home server?** The second needs CORS,
   a token in the browser and a skew policy, and is not in the milestones.
3. **`server` on by default in `grind`**, or only in the container image? Decided by SV3's size
   measurement.
4. **Git as the history**, or the server's own snapshots? Git is recommended because it is what
   the flat form and R6 were built to make good; it makes `git` a runtime dependency of Tier 2.
5. **A CLI edit to a live document** — lock and refuse (recommended first), or route through the
   server?
6. **Who are the users** — one person on several devices, a household, a small team? It decides
   §5.
