<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
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

Everything else follows from being a page rather than a window: a file **dropped** on it opens,
the **clipboard** is the browser's own (the `copy`/`cut`/`paste` events, which need no
permission — the palette's own copy and paste commands use `navigator.clipboard` and say which
key to press when a browser refuses), colours come from `Canvas`/`AccentColor` so light and
dark follow the reader's setting, and the icons are inline `<svg>` drawn from straight lines
because a page has no icon theme to ask.

## What is built

| | The grid | The document |
|---|---|---|
| Open, save | File API in, download out — no path anywhere (rule 5); also drag-and-drop, and `?doc=<url>` | the same |
| Draw | one element per visible cell, from `App::get_viewport` | one `<div>` per **laid-out line**, from `App::layout_block` |
| Select | click, Shift+click, **drag**, Shift+arrow, Ctrl+A | caret, Shift+arrow, Shift+click, **drag**, Ctrl+A |
| Edit | the formula bar, in A1 syntax; Enter/Tab commit | type, Enter, Backspace, Delete; typing over a selection replaces it |
| Format | bold, italic, alignment, wrap, borders, text and fill colour, eight number-format presets, more/fewer decimals | bold, italic, underline, strikethrough, colour, highlight; body/Title/Subtitle/H1–H4/list; Tab and Shift+Tab renest a list item |
| Clipboard | copy/cut/paste as TSV, so a range moves between this and any other spreadsheet | copy/cut/paste as plain text; a newline pasted splits a block |
| The document's own layout | column widths, row heights, hidden and filtered rows, hidden columns | the six heading faces, `Title` and `Subtitle`, list indents, **runs drawn as the document formatted them** — bold, italic, underline, strike, colour, highlight, links |
| Pictures | — | decoded and drawn, as a `data:` URL |
| Charts | **drawn as SVG** — bar, line and pie, scaled against `grind_sheet::axis_ticks` so the axis is the one the GTK shell draws | — |
| Structure | add, rename (double-click a tab) and delete sheets | the outline, in the palette |
| View modes | Ctrl+K → *Show what each cell is* / *Show where names live*: `doc/view-modes.md`'s two overlays, as one attribute per cell and no extra elements | Ctrl+K → *Show where bookmarks are* (§3.6) |
| Assertable output | `ui_web/smoke.js` — the real wasm module against the real page, in jsdom, no browser | the same |

## What it does not do

Deferred by decision, not omission. Everything here is reachable from the CLI (R9).

**The grid.** No point mode, autocomplete or signature hints while typing a formula — the
three things `doc/sheet-shell.md`'s M7 gave the GTK window, and the largest remaining gap.
No dragging a column edge to resize one (the document's own widths are *drawn*, and
`grind sheet width` sets them). No fill handle — Ctrl+D and Ctrl+R are the whole of filling
here. No filter UI: a filter in the file hides the rows it says to, and nothing creates one.
No conditional formatting UI. No find/replace. No freeze panes, no zoom. A chart is a picture:
it cannot be created, edited, moved or recoloured from this shell, which the GTK window can do
and `grind sheet chart-*` can do everywhere.

**The document.** No input method: a character arrives as a `KeyboardEvent.key`, so CJK
candidate windows do not work — and **dead keys do not either**, which is worse than it was
written: a dead key reports `key == "Dead"`, four characters where the keymap wants one, so it
is dropped and nothing is composed from it. On a German or French layout `` ` `` is a dead key,
which means `` `code` `` cannot be typed into this pane at all. See the `TODO:` at the top of
`text/src/markdown.rs`. No tables, footnotes or fields, because the core has
none. No pages, no print, no zoom. No RTL — excluded by decision in `doc/text-layout.md`. An
image sitting *mid-sentence* (`text:anchor-type="char"`) still draws as the placeholder
character, and an image is fit to the column rather than to its own `svg:width` — both are
`grind-text-gtk`'s gaps too. A named *character* style is kept and not interpreted
(`doc/text-core.md`), so there is no style picker for one.

**The view modes** (`doc/view-modes.md` V7). Drawn entirely in the stylesheet: `data-role`,
`data-mark` and `data-name` on a cell, and `content: attr(…)` for the marker and the hint, so a
mode costs one attribute per cell rather than a second element per cell. The hues are
`style::PALETTE`'s, mixed towards `CanvasText` — the CSS form of `ui_sheet_gtk`'s
`theme::readable`, so they separate from whichever ground the browser is painting. What is
missing beside the GTK window: a range anchor is not outlined, the hint is not *measured* against
the value (CSS ellipsis rather than §3.2's drop, so a narrow column shows a truncated name where
the window would show none), and the word processor's marks sit at the end of the line they fall
on rather than at their offset in it.

**Both.** The whole document is in the DOM — no windowing, because a document has as many
blocks as somebody typed and a scroll-position-to-block map only pays for itself on documents
nobody has written yet. Character advances are measured once each and cached, so kerning
between two characters is lost (`ponytail` in `ui_web/src/text/mod.rs`). The colour grid offers
`grind_core::style::PALETTE` and not an arbitrary hex — a palette is a default a shell offers
and never a limit, so a colour a *file* already had is drawn as it is and only a *new* colour
is restricted to the table.

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
