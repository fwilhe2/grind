<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Keeping documents in the browser

**Status: open.** Two options for letting `grind-web` keep a document between visits, and what
each one costs. Nothing here is built and nothing is decided; this is not a plan.
`doc/web-shell.md` stays normative for `ui_web/`, and nothing here changes it until an option is
chosen.

## The problem

A document reaches `grind-web` as bytes — from the file picker, a drop or `?doc=<url>` — and leaves
as a download (`Shell::open` and `Shell::save` in `ui_web/src/lib.rs`). Nothing survives the tab.
Close it and every edit since the last download is gone; the `beforeunload` guard warns first,
whenever there is anything to undo. Working on one document over several days therefore means
downloading it at the end of each sitting and uploading it again at the start of the next, while
the Downloads folder fills with `book (1).fods`, `book (2).fods`.

What is wanted: open the page and carry on where you left off, with no download in between.
Downloading stays the way a document leaves the browser.

Three things are given, and both options respect them:

- **It is hosted on a home network**, as `ui_web/Dockerfile`'s static image. That image holds no
  state (`doc/web-shell.md`, *Deployment*) and that stays true: whatever is kept, the browser keeps.
- **Remote storage is a later possibility, and always opt-in** — a self-hosted S3-compatible store
  or similar. With none configured, the page must do everything it does today.
- **This shell may stand apart from the others.** They work on files; it is a different technology
  and does not have to reach the same files they do.

## Option A — the browser is where documents live

The page keeps a **library**. Uploading a file *imports* it and downloading *exports* a copy. Every
edit is stored as it is made, so nothing is ever unsaved, and earlier versions can be restored. It
is the Google Docs model with the server replaced by the browser.

Kept per document: a stable id, a name, the current bytes, and a series of snapshots, each with a
time and the snapshot it followed.

**For**

- The closest to what was asked for. In the normal course of work there is no file at all, and a
  new document never needs one.
- *Unsaved* stops being a state, and the leave-page prompt goes with it.
- Restoring an earlier version answers "undo my changes" across days, not only within one sitting.

**Against**

- **The worst case is every document at once.** Clearing browsing data, a profile reset, a new
  machine, the browser evicting the site's storage, or a change of address (see *Secure context and
  origin*) takes the whole library, back to each document's last export. Until remote storage
  exists, downloading — the thing this is meant to stop — is the only backup.
- **In practice it needs HTTPS on a stable hostname**, because only a secure context may ask the
  browser not to evict its storage. Over plain HTTP on a LAN address the library lives in storage
  the browser is free to delete.
- **It is the most new surface.** A list of documents with new, rename, delete and export-everything;
  a history view; a retention rule for snapshots; an answer for running out of quota.
  `doc/web-shell.md`'s one decision — one verb bar, one tool row, Ctrl+K — has no place for a
  library yet. Palette rows are a start; a start page is the likely end.
- **Rule 4.** Listing and restoring versions is a capability, so it needs a CLI verb or a named
  reason it has none. The honest reason may be `doc/not-doing.md`'s own *Change tracking* row —
  version control over `.fods` already covers history for files — with this history existing only
  because here there is no file.
- **A second copy with a life of its own.** An imported document and the file it came from diverge
  silently: a later edit to the file with another client never reaches the library, and nothing
  notices.

## Option B — the file is the document, the browser keeps a draft

Documents are opened as they are today. Once one is edited, the page keeps a **draft** of it
automatically. Close the tab and the next visit offers it back — "`book.fods`, edited yesterday —
continue?" — and every draft is also a row in the Ctrl+K palette. Downloading is still saving, and a
draft that has been downloaded is clean. Discarding a draft returns to the file as it was opened.
This is what LibreOffice's autorecovery and VS Code's hot exit do.

Kept per draft: a stable id, the name, the bytes as opened, the current bytes, when it was last
edited, and whether it has been downloaded since.

**For**

- **The worst case is today.** A draft that is deleted loses the edits since the last download,
  which is exactly what closing the tab loses now. The browser never holds the only copy of
  anything that was not already at risk.
- It solves the stated problem — no re-upload between sittings — with the least new surface: an
  offer on arrival, palette rows, and a discard command.
- HTTPS is recommended rather than required. A draft evicted over plain HTTP costs what the same
  edits cost without this feature.
- The file stays the document, so the other clients, git and `doc/flat-first.md` keep their
  meaning. The browser is one more place a file is edited, not a second home for it.
- Nothing new for rule 4. A draft is shell plumbing, like the clipboard, and the CLI's equivalent
  already exists: every command writes the file.
- Storage stays small — a handful of drafts, not a history of every document.

**Against**

- **Downloading is still saving.** Downloads still fills with `book (1).fods`, once per finished
  piece of work: less often, not never.
- **A draft can go stale.** A draft of `book.fods` from Tuesday and a `book.fods` changed with the
  TUI on Wednesday are two versions of one document. Keeping a hash of the bytes as opened lets the
  page notice that a file being opened is not the one a draft was made from; noticing is cheap, and
  what to offer then is a design question.
- No earlier versions beyond *as opened* — and adding snapshots is where B starts turning into A.
- The leave-page prompt keeps its job but changes meaning, from "you will lose this" to "this has
  not been downloaded".

### A variant: B with the File System Access API

In Chromium a page may ask for a handle on a real file and write to it (`showOpenFilePicker`,
`FileSystemFileHandle.createWritable`). A handle can be stored in IndexedDB, so a draft could save
*into the file itself* and Downloads would stop filling; permission is usually asked for again on a
later visit. Firefox and Safari do not implement it and it needs a secure context, so at most it is
a better path on one engine, with plain B everywhere else.

## Side by side

| | A — a library | B — a draft |
|---|---|---|
| Re-upload between sittings | No | No |
| Downloading | Exports a copy | Saves; Downloads still fills |
| If browser storage is lost | Every document, back to its last export | Edits since the last download — today's behaviour |
| HTTPS on a stable hostname | Needed in practice | Recommended |
| New interface | Library, history, retention, quota, export-everything | Offer on arrival, palette rows, discard |
| *Unsaved* | Gone | Means "not downloaded" |
| Earlier versions | Snapshots | *As opened* only |
| Rule 4 | Version history needs a verb or a reason | Nothing new |
| The other clients | Separate; import and export are the bridge | Unchanged; the file is still the document |
| Remote storage, later | The library syncs with it | Files are opened from it and saved back to it |
| Size of the change | Larger | Smaller |

## What holds under either option

**Store ODF bytes, never the model.** A stored `Document` or `Action` would be stranded by the next
deployment of the wasm bundle; ODF is already a versioned format, and `App::open_bytes` and
`App::save_bytes` already read and write it. Store the **flat** form (`doc/flat-first.md`) whatever
form a document arrived in, and choose the form only on download, which `form_of` already does from
the name. Splicing works within one form, so a document that arrived as a package regenerates on
every write until it has been read back from its first stored bytes.

**IndexedDB is a reasonable store.** It holds bytes and small records, every browser has it, and —
unlike the rest of the storage APIs — it works outside a secure context. Which storage API to use is
the least consequential decision here. One hazard is worth knowing before writing Rust against it:
a transaction commits itself as soon as nothing is pending on it, so awaiting anything other than
one of its own requests inside a transaction ends that transaction.

**Undo within a sitting is unchanged, and neither option carries Ctrl+Z across a reload.** A stored
document comes back through `App::open_bytes`, which drops history by design in both apps. Carrying
it across is possible for the spreadsheet — `grind_sheet::Session` is already serialisable, and
`grind sheet --session` uses it — and is a model decision for text, because `grind_text::Action`
addresses a block by position and carries `BlockId`s that a fresh read reassigns
(`doc/not-doing.md`, *Undo and redo across invocations*). Snapshots answer "undo my changes" across
days for both document types without either.

**One writer per document.** Two tabs on one stored document each write their own version, and the
last write silently wins. Web Locks would enforce a single writer but need a secure context;
`BroadcastChannel` does not, and is enough to refuse a second tab.

**Saving waits, and a closed tab loses the wait.** A write per keystroke is not free. R6 splices an
edit to existing content, but a new block, a new cell, or a cell's format or style regenerates the
whole document (`doc/not-doing.md`, *Splicing an insertion, a deletion or a move* and *Splicing a
format or style change*), on the main thread. So a write is debounced, and the debounce interval is
what a closed tab can lose: IndexedDB is asynchronous, the page is not kept alive for a write
started while it unloads, and the last reliable moment is `visibilitychange` to hidden. How long
`save_bytes` takes on a large document in wasm is **unmeasured**; measure it before choosing the
interval.

**Secure context and origin.** These are only available over HTTPS or on `localhost`:

- `navigator.storage.persist()` — the request not to evict. Without it a site's storage is
  best-effort and may be deleted under disk pressure; browsers grant it by their own rules, some by
  asking the person.
- Web Locks, service workers, `navigator.clipboard` and the File System Access API.

WebKit additionally deletes a site's script-writable storage — IndexedDB included — after seven
days of Safari use without a visit to it, unless the page was added to the Home Screen as a web
app.

Storage belongs to an **origin**: scheme, host and port. `http://192.168.1.20:8080`,
`http://nas.local:8080` and `https://docs.example.lan` are three separate, empty stores, and so are
`ui_web/build.sh`'s `http.server` on port 8000 and the container on port 8080. A DHCP lease that
moves the server looks like deletion. Under A that is why one stable HTTPS hostname is a
requirement; under B it costs drafts.

**Testing.** `ui_web/smoke.js` runs in jsdom, which has no IndexedDB, so the storage layer needs a
stand-in there (`fake-indexeddb`) or a boundary narrow enough that `cargo test -p grind-web` covers
everything above it.

## What neither option does

- **Reach a second device.** Hosting the page on a home server does not put documents on it: a
  phone opening the same address sees nothing. Only remote storage changes that.
- **Let several people edit at once.** Live co-editing needs a server and a merge model over ODF.
  That is a different product, and neither option leads to it.

## Remote storage, later

Both options can grow into it, differently.

- **A with remote storage** replicates the library. The browser's copy becomes a replica that
  accepts edits while offline, so conflicts are real: one document changed on two devices before
  either synced.
- **B with remote storage** makes the store where files live. Opening reads one, saving writes it
  back, and the draft holds offline edits in between — no downloads at all, and no library to sync.
  A store the desktop can also mount (WebDAV, which Nextcloud and many NAS devices serve) would put
  the other clients back on the same files.

Either way the browser talks to the store directly, because the page has no server behind it: the
store needs CORS configured for the page's origin, and its credentials live in the browser. And
either way sync means **whole documents with conflict detection** — a write made conditional on the
version it was based on (an ETag, where the store honours one) — never merging edits.

Two things are cheap now and expensive to add later, under either option: **a stable id** for every
stored document instead of its name as the key, and **the version each stored copy was based on**.

## Getting from one to the other

**B to A is an extension.** The same records gain snapshots, lose their expiry and get a list;
nothing already promised is taken back.

**A to B withdraws a promise.** A library people have come to treat as where their documents live
would become a place drafts wait, and whoever relied on it has to export first.

B first keeps both paths open; A first commits to one.

## Open questions

1. **Is a second device a need?** If it is, remote storage is the core of this rather than a later
   extension, and the store matters more than the choice between A and B.
2. **How is the server reached today** — an address and port, a hostname, HTTPS?
3. **Which browsers and devices** have to work?
4. **How much may a closed tab lose** — one second, ten?
5. **A second tab on the same document** — refuse it, or make it work?
6. **Under B, a file is opened that a draft already exists for, and the bytes differ.** What is
   offered?
