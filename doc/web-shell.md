<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: CC-BY-4.0
-->

# The browser shell

What `grind-web` does, how it decides *how* to do it, and what it deliberately does not.
Normative for `ui_web/` the way `doc/sheet-shell.md` is for the spreadsheet's GTK window and
`doc/text-shell.md` is for the word processor's.

The two older gap lists said the same thing about this shell for a while — "the same list,
minimally" — and that stopped being true once it grew a palette, formatting, a clipboard and
charts. This file is where its own answers live now; the other two link here.

## The one design decision

**A browser tab has no menu bar, and inventing one is how a web application ends up a worse
copy of a desktop application.** There is no window manager to borrow chrome from, no platform
menu conventions to honour, and no reason to pretend otherwise. So:

- **One bar of verbs** across the top: open, save, undo, redo, recalculate, the document's
  name. The things somebody reaches for without thinking.
- **One tool row** for whichever document type is open — the toggles and swatches a *pointer*
  wants, and nothing that is only a word.
- **Ctrl+K opens everything else.** `ui_web/src/command.rs` is a table of every verb either
  pane has, as data; the palette filters it, the pane matches on the id. A command is reachable
  from the palette, from a key, and — where a pointer would want it — from a button, and all
  three run the same id, which is what keeps them from drifting apart.
- **The palette is also the go-to box.** A pane contributes its own rows for whatever the query
  looks like: an address (`B12`), a range, a defined name or another sheet in the spreadsheet;
  a heading, a bookmark or a `p12`/`#intro`/`§2.1.3` address in the document. That is why there
  is no separate go-to dialog and no outline dialog — `doc/text-shell.md` named both as this
  shell's next candidates, and one box answered both.

**A page with no document open shows the choice, not a spreadsheet.** This shell used to load
straight into an empty grid, on the reasoning that a page has to show *something* and the grid is
what it was first. One bundle holds two applications, so that is a guess made before anybody has
been asked, and it is wrong half the time. `Mode::Welcome` is the third mode — new spreadsheet,
new text document, or open one that exists, as three cards — and it is a *mode* rather than a
fourth surface like the code view, because it is the absence of a document rather than another
way of looking at one: no tool row applies, Save and the history buttons leave the bar with
nothing to act on, and Ctrl+K offers the same three verbs and nothing else (`command::WELCOME`).
The cards are ordinary `<button>`s naming command ids, so each is one row of the palette and one
implementation; `doc.new-sheet` and `doc.new-text` are in both document tables too, which is how a
page that has opened a spreadsheet can start a text document without opening a file that already
is one — something this shell could not do at all before. `doc.welcome` goes back, and
`index.html` now *starts* in this state rather than in the grid's, so nothing flashes a grid
while the wasm module loads.

Everything else follows from being a page rather than a window: a file **dropped** on it opens,
the **clipboard** is the browser's own (the `copy`/`cut`/`paste` events, which need no
permission — the palette's own copy and paste commands use `navigator.clipboard` and say which
key to press when a browser refuses), colours come from `Canvas`/`AccentColor` so light and
dark follow the reader's setting, and the icons are inline `<svg>` drawn from straight lines
because a page has no icon theme to ask.

## What is built

| | The grid | The document |
|---|---|---|
| Start | the welcome pane, with no document open: **New spreadsheet**, **New text document**, **Open a document…**, and the same three in Ctrl+K. Either card is reachable from an open document too (`doc.new-sheet` / `doc.new-text`), and `doc.welcome` goes back | the same pane, the same three |
| Open, save | File API in, download out — no path anywhere (rule 5); also drag-and-drop, and `?doc=<url>` | the same |
| Draw | one element per visible cell, from `App::get_viewport` | one `<div>` per **laid-out line**, from `App::layout_block` |
| Select | click, Shift+click, **drag**, Shift+arrow, Ctrl+A | caret, Shift+arrow, Shift+click, **drag**, Ctrl+A |
| Edit | the formula bar, in A1 syntax; Enter/Tab commit; autocomplete and a signature hint in a band under it | type, Enter, Backspace, Delete; typing over a selection replaces it |
| Format | bold, italic, alignment, wrap, borders, text and fill colour, eight number-format presets, more/fewer decimals | bold, italic, underline, strikethrough, colour, highlight; body/Title/Subtitle/H1–H4/list; Tab and Shift+Tab renest a list item |
| Clipboard | copy/cut/paste as TSV, so a range moves between this and any other spreadsheet | copy/cut/paste as plain text; a newline pasted splits a block |
| Spelling | a wavy red line under a misspelt word in a text cell; the palette's *Spelling:* rows choose a language or turn it off — remembered | the same line, and the palette offers a correction, *Ignore*, *Add to your dictionary* and *Go to the next misspelling* for the word at the caret. The dictionaries are **fetched** from `dictionaries/` the first time a document opens with spelling on (`doc/spelling.md`, `ui_web/src/spelling.rs`) |
| The document's own layout | column widths, row heights, hidden and filtered rows, hidden columns — and now Ctrl+K → *Hide*/*Unhide row(s)/column(s)*, over `App::set_row_hidden`/`set_col_hidden` on the selection's own span, the same call the CLI's `sheet hide`/`--unhide` makes | the six heading faces, `Title` and `Subtitle`, list indents, **runs drawn as the document formatted them** — bold, italic, underline, strike, colour, highlight, links |
| The autofilter (§9.4) | Ctrl+Shift+L / Ctrl+K → *Filter rows* sets the range over the selection or clears it, a dropdown button drawn on each field of the range's own heading row, and its popover — `ui_web/src/sheet/filter_ui.rs`'s `field_values`, over the same viewport the grid draws from — lists the column's distinct values with a checkbox each, so unticking one and pressing Apply writes `App::set_filter`'s `keep` the way `ui_sheet_gtk`'s own dropdown does | — |
| Format as table | Ctrl+K → *Format as table* — the filter above plus alternating row shading, in one `App::format_table` undo step, over the selection with no dialog: its first row is the heading and the name auto-generates, the same zero-prompt shape `sheet.filter` already has. Ctrl+K → *Format as table with totals…* is the same composite having asked which aggregate the totals row carries — a `window.prompt` over `TotalsFunction::ids()`, and a second command rather than a prompt on the first so the plain one stays zero-prompt | — |
| CSV | Ctrl+K → *Import CSV…* raises the same file input the open verb does, with a different `accept` (`Pick` is which of the two a `change` event is about); the fields land at the **cursor**, with the delimiter read out of the file's own content. Out is two rows rather than one — *Export as CSV* and *Export as TSV* — because a download names itself, so the choice a save dialog carries as two filters has to be made before the file exists (`csv::Dialect::for_name` still reads it back off the name) | — |
| Pictures | — | decoded and drawn, as a `data:` URL, the picture's title its `alt`. **Alt text**: every picture wears a chip — *+ Alt text* in the accent until it has some, a quiet *ALT* on hover after — and the chip, or Ctrl+K → *Alt text for the picture…* beside one, opens `index.html`'s `#alt` `<dialog>`: the picture, the short field with `grind_text::picture::advice` and a count under it, a collapsed long description, Cancel and Save |
| Charts | **drawn as SVG** — bar, line and pie, scaled against `grind_sheet::axis_ticks` so the axis is the one the GTK shell draws | — |
| Structure | add, rename (double-click a tab) and delete sheets | the outline, in the palette |
| View modes | Ctrl+K → *Show what each cell is* / *Show where names live*: `doc/view-modes.md`'s two overlays, as one attribute per cell and no extra elements | Ctrl+K → *Show where bookmarks are* (§3.6) |
| Code view | Ctrl+K → *Show the source*: `doc/dsl.md` §6's read-only projection, one `<div>` per line, a class per `TokenKind`, the selection's own line drawn as current and clicking a line selecting what it projects | the same |
| Find and replace | **the palette is the find box**: two or more characters list the cells holding them *after* the verbs (so `bold` still puts *Bold* first), picking one goes there, and **F3** / **Shift+F3** step on through the rest across every sheet, wrapping. **Ctrl+F** opens the same palette, since the browser's own find sees only the cells on screen. **Ctrl+H** is *Replace in every cell…* — two prompts, then `App::replace` in one undo step, with the count and any formula it left alone in the message line | **the same palette** (2026-10-02): paragraphs holding the word, case-blind (`grind_text::find`), picking one selects it, **F3** / **Shift+F3** step on; **Ctrl+H** is *Replace in the document…* — two prompts, then `App::replace` in one undo step |
| Problems | Ctrl+K → *Check the document*: `grind lint`'s findings, one row each, clicking one goes where it points | the same pane, the same rows |
| Paper | — | Ctrl+K → *Export as PDF* (Ctrl+Shift+E) downloads `grind_print::export`'s PDF; *Print preview* is a sixth pane of every page drawn onto a canvas from the same display list; *Print (opens the PDF)* (Ctrl+P) opens the PDF in a tab of its own, where the browser's viewer prints it. All three run in **a second wasm module** (`ui_web_print`, `grind-web-print`), imported the first time anything is put on paper, with the bundled fonts fetched from `fonts/` beside the page at the same moment — so the font stack and the PDF writer cost a visit nothing until they are used (`doc/pdf-export.md` §7) |
| Assertable output | `ui_web/smoke.js` — the real wasm module against the real page, in jsdom, no browser | the same |

**Check the document** — `grind lint`'s findings as a fourth surface (`doc/dsl.md` §4.3, D6),
from the palette like every other verb. One row per finding, carrying the address it is about;
clicking a row goes there and puts the document back. A pane rather than a dialog, which is this
shell's one design decision applied again: a browser tab has no window manager to keep a second
window in front of the first. `problems.rs` is shared by both panes, like `code.rs`.

**An Excel workbook opens** (`doc/xlsx-import.md`, X6) from the picker, which lists `.xlsx` and `.xlsm`, or from `?doc=`: the page's message line says the report, and the download is `budget.fods`. A page cannot overwrite the file it was given anyway, so here the rename is the whole of the rule — sniffed from the bytes,
imported through `grind_xlsx::open`, and opened as a **new, unsaved ODF document with no path**
under `grind_xlsx::suggested_name` (`budget.xlsx` → `budget.fods`), with the report's one
sentence (`Report::summary`) shown. Having no path is the point: one way in and never out
(`doc/not-doing.md` §1), so nothing can write ODF over the workbook it came from. `grind-open`
decides it, the same in every shell, and the crate's `xlsx` feature compiles it out.

**A Word document opens** the same way into the text pane (`doc/docx-import.md`, DX6): the
picker lists `.docx`, `.docm` and `.dotx`, the message line says the report, and the download is
`letter.fodt` — `grind-open` again, behind the crate's `docx` feature.

## What it does not do

Deferred by decision, not omission. Everything here is reachable from the CLI (R9).

**The code view is read-only, and shown instead of the document rather than beside it.** §6.2 is
right that a split is what a person eventually wants; it is also a second viewport to keep in step
and a resize handle to build, and what pays for itself first is the correspondence. `show()` is
the one function that decides which of the three panes is on screen, from the mode and from
whether a projection is open — it was two, and closing the command palette closed the code view.
Editing it is gated in `doc/dsl.md` §6.4.

**The grid.** **Autocomplete and signature hints are built** (`ui_web/src/sheet/assist.rs`,
over `grind_sheet::formula::assist` — the same answers `ui_sheet_gtk`'s M7 and `ui_win32`'s W9
read), drawn as a one-line band under the formula bar rather than a popover, for the reason
`ui_win32`'s own band exists: the keyboard must never leave the `<input>` while the list
narrows, and a popover is a second focusable element. Tab accepts, the arrows step, Escape
dismisses for that word only — Enter is left to commit the cell, as everywhere else. **Point
mode** — arrow keys building a reference into a half-typed formula — is still not built and is
now the largest remaining gap: this shell edits in exactly one place, the formula bar, and a
caret that leaves it to build a reference would be a third editing mode.
No dragging a column edge to resize one (the document's own widths are *drawn*, and
`grind sheet width` sets them). No fill handle — Ctrl+D and Ctrl+R are the whole of filling
here. **The autofilter is built** — a dropdown per field of the range's own heading row and a
popover over its distinct values, the same shape `ui_sheet_gtk`'s own dropdown has — one of
the two gaps this line used to name.
*Format as table* is built too, and is static styling applied once rather than a live rule, so
it is not a rule. Conditional formatting is three palette rows — a rule over the selection
asked for in two prompts (the condition, then the look by its word), one removed by number from
a numbered list, and every rule touching the selection removed. **CSV is built both ways** and carries the same named gap every other window does: no
dialog for an import's seven options, so a file that needs `--text`, a locale or a delimiter
the sniffer got wrong is `grind sheet import-csv`'s (R9). Find and replace carry the options
the CLI names but no control for them — match case, whole cell, one sheet, a range — so those
are `grind sheet find`/`replace`'s, and nothing marks every hit in the grid at once. No freeze panes, no
zoom. A chart is a picture:
it cannot be created, edited, moved or recoloured from this shell, which the GTK window can do
and `grind sheet chart-*` can do everywhere.

**The document.** **Dead keys compose, and that was a bug worth the fix.** A dead key — `` ` ``,
`´`, `^`, `~`, which is most of a German, French or Spanish layout's punctuation — reports
`KeyboardEvent.key == "Dead"`, four characters where a keymap taking exactly one wants one, so
it used to be dropped and nothing was ever composed from it: `` `code` `` could not be typed
into this pane at all on those keyboards. Both halves of a composition are now refused by the
keymap (the `"Dead"` that opens one, and every keystroke reporting `isComposing`) and the text
is taken from `compositionend` instead, which is the one event that carries what was actually
produced. **What is still missing is the rest of an input method**: the same event ought to
deliver an IME's committed text, but a candidate window has no editable host to position itself
against — this shell has no `contenteditable` by decision — so CJK input is *unverified* rather
than working, and is written here as a gap until somebody types into it. **A table is drawn as a
grid** since the UX pass below, and cannot be *inserted* here — `grind text table` and the GTK
window's Insert Table… can. No footnotes or fields, because the core has
none. No pages, no print, no zoom. No RTL — excluded by decision in `doc/text-layout.md`. An
image sitting *mid-sentence* (`text:anchor-type="char"`) still draws as the placeholder
character, and an image is fit to the column rather than to its own `svg:width` — both are
`grind-text-gtk`'s gaps too. A named *character* style is kept and not interpreted
(`doc/text-core.md`), so there is no style picker for one.

**The view modes** (`doc/view-modes.md` V7). Drawn entirely in the stylesheet: `data-role`,
`data-mark`, `data-name` and `data-anchor` on a cell, and `content: attr(…)` for the marker and
the hint, so a mode costs one attribute per cell rather than a second element per cell. The hues
are `style::PALETTE`'s, mixed towards `CanvasText` — the CSS form of `ui_sheet_gtk`'s
`theme::readable`, so they separate from whichever ground the browser is painting. **A range
anchor is now outlined too**: `data-anchor` carries the boundary edges a cell sits on
(`top`/`bottom`/`left`/`right`, space-separated) and a `border-*` per edge draws the same muted
rectangle `ui_sheet_gtk`'s `draw_hints` does, one attribute rather than a second element. The
word processor's bookmark marks now carry a tick at their own offset too (`.mark-tick`, the
`x_at` boundary `ui_text_gtk` draws), beside the name still written at the end of the line they
fall on. What is still missing beside the GTK window: the hint is not *measured* against the
value (CSS ellipsis rather than §3.2's drop, so a narrow column shows a truncated name where the
window would show none).

**Both.** The whole document is in the DOM — no windowing, because a document has as many
blocks as somebody typed and a scroll-position-to-block map only pays for itself on documents
nobody has written yet. Character advances are measured once each and cached, so kerning
between two characters is lost (`ponytail` in `ui_web/src/text/mod.rs`). The colour grid offers
`grind_core::style::PALETTE` and not an arbitrary hex — a palette is a default a shell offers
and never a limit, so a colour a *file* already had is drawn as it is and only a *new* colour
is restricted to the table.

**The welcome pane's recent documents are copies, not paths.** A page is handed a `File` by a
picker and never a path, so there is nothing to re-open the usual way — the File System Access
API's handles would change that, and they are neither universal nor free (a stored handle needs
permission again on the next visit). So `ui_web/src/recent.rs` keeps the bytes: the four documents
last opened or saved, each up to 768 KiB, base64 in `localStorage`, listed on the welcome pane and
in the palette (*Open …*, and *Forget the recent documents*). Opening one opens the copy as it was
then, not the file on disk. `?doc=<url>` remains the link form, and therefore the bookmark.
**Starting a new document replaces the one that pane is holding**, in the same tab, and asks
first when there is anything to lose — `window.confirm`, over the same `can_undo` approximation
`beforeunload` already uses, and asked of the pane being *replaced* rather than the one on
screen. The three native shells offer Save / Don't Save / Cancel there; a page gets two buttons,
because `confirm` is the only modal a page has without building one. A runtime that will not ask
at all — a sandboxed frame without `allow-modals`, and every headless test harness — goes ahead:
the `confirm` helper tells "answered no" from "did not ask" through `Reflect` rather than through
the typed binding, which coerces the second into the first.

## The UX pass — the same review the GNOME windows had

Screenshots of the page in a real (headless) Chromium, light and dark, rather than jsdom's
zero-sized rectangles — which is what found most of this, and what found that **every build in
the development VM had been bundling a stale module**: `build.sh` and `smoke.sh` read the wasm
from `target/` whatever `CARGO_TARGET_DIR` said, so the page loaded, every check passed and none
of the change was in it. Both read `CARGO_TARGET_DIR` now, as `scripts/run.sh` already did.

* **A document's colours read on a dark page** (`ink.rs`) — `ui_sheet_gtk`'s `theme::ink` rule
  over the same `grind_core::color`: a navy word was invisible on the dark ground in both panes,
  and a heading row the document filled silver kept the dark theme's white text across it. The
  page repaints when the reader switches schemes, since these colours are computed rather than
  left to the stylesheet.
* **The page follows the caret.** `follow_caret` measured the caret's `offsetTop`, which is from
  its block (`position: relative` for the bullet) rather than from the pane, so PageDown walked
  the caret off the bottom and the page never moved.
* **A hidden column is hidden.** A zero-width `<col>` does not size a `max-content` table's
  column — its cells' text does — so the column the document hid was drawn at full width. It is
  left out now, and the heading after the fold carries an accent edge and a tooltip.
* **A filtered heading ends where its button begins** (`td.has-filter`), rather than running
  under it — the GNOME grid's fix, and the same screenshot.
* **The filter list orders numbers by value** — `2,250.00 €` came before `220.00 €` in both this
  pane and the GNOME window. `grind_sheet::filter::offered` is the list for every shell now; text
  keeps the model's code-point order, since anything else is the collation `doc/not-doing.md`
  gates.
* **The status line says Sum, Count and Average** for a range and nothing for one cell, as the
  other windows' do — it said `Budget · 20×10 used · 3×2 selected`. The text pane's is in words
  (`Heading 1 · 90 words`), the address moved to its tooltip.
* **Formatting at a bare caret**: Bold, then type, is bold — it answered *Select some text
  first*. The pending style is the `resume` `type_markdown` already carried; moving forgets it.
* **Double-click selects a word, triple-click the paragraph** — `grind_text::word::around`, the
  GNOME window's answer.
* **Tables are grids** (above), and the welcome page lost a focus ring drawn round the whole page.

**The grid honours the document's column widths, and a number never overflows.** The table was
`table-layout: fixed` and `width: max-content`, and the second won: a column the document made
0.6in wide grew to fit whatever was in it, so this was the one client where a long label or a
long number widened its column rather than being cut at it. The table is given the width its
`<col>`s add up to now, text cut at a column ends in `…`, and a number that no longer fits is
`###` — `hash_overflowing`, measured once the rows are in the page, over
`grind_sheet::numfmt::overflow` (`doc/feature-matrix.md` §5).

## A server behind it

The page is static and stays so. When `grind serve DIR` served it, a probe of `api/v1/hello`
finds the server, and the page gains a library, Save that writes back, and history — through
`src/server.rs`, the one module that sends anything but a plain GET. With no server, nothing
about one is shown (`smoke.js` checks that; `smoke-server.js` drives the page against a real
`grind serve`). `doc/server.md` is the design and the protocol.

## How to see it

```sh
ui_web/build.sh release                              # writes ui_web/dist
python3 -m http.server --directory ui_web/dist 8000  # a module needs http, not file://
ui_web/build.sh release && ui_web/smoke.sh release   # the browser boundary, in jsdom
scripts/run.sh web /tmp/grind-demo/sample.fodt       # served next to the page, opened with ?doc=
cargo test -p grind-web                              # the palette, the keymaps, the layout, the pieces
```

`cargo test -p grind-web` is everything that runs without a browser, and it is deliberately a
lot: the command table and its fuzzy match (`command.rs`), both keymaps, the viewport and track
arithmetic (`sheet/layout.rs`), the chart's own geometry and escaping (`sheet/chart.rs`), and
the line-cutting that turns formatting, a selection and a caret into `<span>`s
(`text/runs.rs`). Everything a browser is actually needed for is in `smoke.js`.

## Deployment

The image is the page **and `grind serve` behind it** (`doc/server.md`), so a deployed page has
a library, Save writes back and nothing is a download unless asked for. `ui_web/Dockerfile`
builds in two stages:

- **Build**: `rust:1-bookworm`, the `wasm32-unknown-unknown` target, the `wasm-bindgen-cli`
  version `Cargo.lock` pins (installed in a layer of its own, so a source change does not
  recompile the tool), then `ui_web/build.sh release` and `cargo build --release -p grind-cli`
  from the same tree — the page and the server are always the same build.
- **Runtime**: `gcr.io/distroless/cc-debian12:nonroot`: `grind` at `/usr/bin/grind`, the page at
  `/usr/share/grind/web` (where `grind serve` looks without `--page`), and nothing else — no
  shell, no package manager, no root. And **no `git`**, so a container's page offers no history;
  `grind serve` run directly over a git repository is how to have it.

**`grind serve` listens on 127.0.0.1 only** (R12, `doc/local-only.md`), and that decides every
sample: a published port (`-p 8080:8080`) forwards to the container's own address, where nothing
listens.

- `ui_web/deploy/compose.yaml` and `ui_web/deploy/podman.sh run` share the **host's network**,
  so the page is at `http://127.0.0.1:8080/` on that machine and on no other. The documents are a
  host directory, written as the host's user.
- `ui_web/deploy/kubernetes.yaml` puts a **Caddy sidecar** in the pod. It shares the pod's
  loopback, listens on the pod's address (8081), asks for a **login** and hands every request to
  grind. The login is there because whoever reaches the Service could otherwise read, overwrite
  and delete every document: grind's own checks (the Host, the page's header, the Origin) stop
  another site's page, not a program. The user and bcrypt hash are a Secret made with `kubectl`,
  never in the file, so the pod does not start without one. grind is told the public host name
  with `--host`, because it refuses requests addressed to any name it was not given. The probes
  cannot reach loopback and must not need the login, so Caddy has a second port (8082) outside
  the Service, with its own `/healthz` and grind's `api/v1/hello` and nothing else; grind's
  probes sit on grind's container, so a grind that stops answering is the one restarted, and send
  `Host: localhost`. The documents are a PersistentVolumeClaim, so the Deployment is one replica,
  replaced with `Recreate` rather than rolled.
- **Compression** is grind's: `ui_web/build.sh release` writes a gzip twin beside each module,
  script, stylesheet, font and dictionary, and `grind serve` sends it to a browser that takes
  gzip, so every sample serves the module at about a quarter of its size.
- grind exits on SIGTERM by its own handler, so as a container's PID 1 it needs no `--init`.
- `httproute.yaml` (Gateway API) and `ingress.yaml` (Ingress API) reach that Service from
  outside the cluster, one of the two per cluster. `ingress-nginx` is retired (end of life
  31 March 2026, no security fixes), which is why the Gateway API file is the default and the
  Ingress one is written in no controller's dialect. Their host name is the `--host`.
- `ui_web/deploy/podman.sh kube` plays `kubernetes.yaml` under `podman kube play` with the
  documents as a local directory — the pod, the proxy and the probes, without a cluster.

`cli/tests/deploy.rs` holds the samples to each other and to the workflow: the image
`container.yml` publishes, grind on loopback with the proxy the only ports, the login, the probes'
port and Host, one replica over one claim, and both containers unprivileged and read-only under
the `restricted` Pod Security Standard.

`.github/workflows/container.yml`'s `web-image` job builds this image on every push to `main`, one
leg per architecture on a runner of its own architecture rather than under qemu, `load: true` so
it can be smoke-tested — `grind serve` on the runner's network, asked for `api/v1/hello` and
`index.html` — before it is pushed as `latest-amd64` / `latest-arm64`; `web-manifest` joins the
two into `latest`. `.github/scripts/image-size.sh` measures it, from the daemon on a pull request
and off the registry after a push. Most of it is now `grind` itself, which carries every
dictionary and font.
