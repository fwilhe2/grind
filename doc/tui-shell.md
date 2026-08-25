<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# The terminal shell

What `grind-tui` does, how it decides *how* to do it, and what it deliberately does not.
Normative for `ui_tui/` the way `doc/web-shell.md` is for the browser and `doc/sheet-shell.md`
is for the spreadsheet's GTK window.

One binary, both document types, chosen by `grind_core::kind` from the file's bytes (R10).

## The two design decisions

**1. Vi, not a menu.** A terminal has a keyboard and no pointer, so the shell is modal:
**Normal** navigates, **Visual** selects, **Insert** types, and `:` opens a command line.
Every verb a toolbar would carry is a `:` command or a Visual-mode key, and the two shells
share a vocabulary wherever the underlying capability is shared — `v`, `y`, `p`, `d`, `*`, `/`,
`-` mean the same thing in a spreadsheet and in a document, because one suite should not have
two words for emphasis.

**2. Markdown is for *typing*, never for *showing*.**

The word processor recognises `**bold**`, `*italic*`, `__underline__` and `~~struck~~` as the
closing marker lands, and `# `/`- ` at the start of a block: the markers are erased and the
document's own formatting is set (`ui_tui/src/text/markdown.rs`). It is the terminal's answer
to a formatting toolbar — the notation everybody already types, on the keyboard that is all
there is.

What it is **not** is a display convention. A formatted run is drawn with the terminal's own
bold, italic, underline and strikethrough; the markers never appear on screen. That is not
taste, it is a constraint: the core breaks lines in *this shell's* units
(`doc/text-layout.md`), and a marker drawn but never measured would put every caret after it
in the wrong column. The same reasoning rules out a markdown *source* view.

The rules that keep prose out of it are in `markdown.rs` and tested there: the content may not
be empty or padded with spaces, may hold no marker of its own (`2*3*4` is arithmetic), and the
opening marker must start a word (`snake_case` is not emphasis). `_x_` alone means nothing —
`__x__` is underline, the one deliberate divergence from markdown, because ODF has underline
and this build already spells bold `**x**`.

## What is built

| | The grid | The document |
|---|---|---|
| Move | `hjkl`/arrows, `0`/`$`, `g`/`G`, `Ctrl+f`/`Ctrl+b` | the same, and every vertical motion is a **wrapped line** answered by the core |
| Select | `v` — a rectangle | `v` — a run of text, across blocks |
| Edit | `i`/`a`/`c` into the formula line, Enter commits, Esc cancels | `i`/`a`/`o`, typing goes straight into the document, `x`/`X`/`J` |
| Clipboard | `y`/`p` — a register of tab-separated text, the shape every other spreadsheet reads | `y`/`p` — plain text, a newline splits a block |
| Format | `*`/`/`/`-` over a selection; `:bold :italic :wrap :border :align :color :fill :plain` | `*`/`/`/`_`/`~`/`-` over a selection; markdown while typing; `:color :highlight :plain` |
| Number formats | `:format` over eight presets, `:general` | — |
| Structure | `:sheet`, `:sheet-new`, `:sheet-rename`, `:sheet-delete` | `:h <level>`, `:li [depth]`, `:style [name]`, `:outline`, `:words` |
| Find | — | `:find <text>`, `:s/old/new/` |
| Drawn from the document | bold, italic, colour, background, alignment; filtered and hidden rows are folded away | bold, italic, underline, strikethrough, colour, background; headings and `Title`/`Subtitle` emphasised; the block's kind in the gutter |
| Addressing | `:<address>` — a cell or a range | `:<address>` — `p12`, `p12+40`, `#bookmark`, `§2.1.3` |
| Assertable output | `TestBackend`, no terminal needed | the same |

Both are renderers that own nothing: every paint reads `App::get_viewport` (and
`App::layout_block`) and throws the result away.

## What it does not do

Deferred by decision, not omission. Everything here is reachable from the CLI (R9).

**The medium's own limits, which are not gaps.** One font at one size, so a font family and a
size are stored and not drawn. Sixteen colours, so a document's `#ff4136` is drawn as the
nearest of them — `nearest_color` in `ui_tui/src/text/app.rs`, by squared distance in RGB.
No pictures, no charts: a chart in a file is kept and written back untouched, and nothing here
draws one.

**The grid.** Every column is ten cells wide: the document's own `col_widths` are *stored* and
not drawn, unlike every other shell (`ponytail` in `padded_as`, which is also where the column
padding counts `char`s rather than terminal cells — a column of CJK text sits a little wide).
No point mode, autocomplete or signature hints while typing a formula. No filter UI: a filter
in the file folds its rows away, and nothing creates one. No row heights, no wrapping — a row
is one line. No conditional formatting UI, no find/replace over cells.

**The document.** No pages, no print, no zoom, no RTL (`doc/text-layout.md`). No tables,
footnotes or fields, because the core has none. `:outline` prints to the status line rather
than opening a pane. An image in a file is kept and not drawn, and a named *character* style
is kept and not interpreted (`doc/text-core.md`).

**Both.** The register is this shell's own, not the system clipboard — a terminal cannot reach
one without a protocol the host may not speak, and vi's register is the convention a reader of
this shell already has. Markdown-while-typing costs two undo steps rather than one (an erase
and a style), which is the honest price of not inventing a compound action for a shell's own
convenience.

## How to see it

```sh
cargo run -p grind-tui -- book.fods       # the spreadsheet
cargo run -p grind-tui -- report.fodt     # the word processor
cargo run -p grind-tui -- --text          # a new document, empty
cargo run -p grind-tui -- --help          # every key and command
cargo test -p grind-tui                   # both keymaps, the notation, and rendering
```

`cargo test -p grind-tui` needs no terminal: the keymaps and `markdown.rs` are pure functions,
and everything about the picture goes through ratatui's `TestBackend` — which is how "a bold
run is drawn bold" and "a number sits to the right of its column" are checked rather than
described.
