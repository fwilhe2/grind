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

The word processor recognises `**bold**`, `*italic*`, `__underline__`, `~~struck~~` and
`` `code` `` as the closing marker lands, and `# `/`- `/```` ``` ```` at the start of a block:
the markers are erased and the document's own formatting is set. It is the terminal's answer to
a formatting toolbar — the notation everybody already types, on the keyboard that is all there
is.

**The reading lives in the core**, not here: `grind_text::markdown` and
`App::type_markdown`. It started in this shell and moved the day it worked, for the reason
`doc/text-layout.md` gives about line breaking — three shells recognising `**` three ways
would be three editors. All four now type through the same call, and one press of undo takes
back the whole of `**bold**` because it is one action.

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

A character typed after a closing marker is not emphasised — `App::type_markdown`'s `resume`,
which every shell carries and none of them reads.

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
| View modes | `:roles` — what each cell is, coloured and marked with one glyph; `:names` — a named cell underlined, the name and the formula read through its names on the formula line | `:names` — where each bookmark anchors, after the line it falls on |
| Problems | `:lint`, `:lint hints` — what the document says about itself; `j`/`k` moves, `Enter` goes to the finding | the same pane, the same keys |
| Help | `:help` — the key list, over the document, scrollable | the same, with its own section |
| Drawn from the document | bold, italic, colour, background, alignment; filtered and hidden rows are folded away | bold, italic, underline, strikethrough, colour, background; monospace runs and preformatted blocks dimmed; headings and `Title`/`Subtitle` emphasised; the block's kind in the gutter |
| Addressing | `:<address>` — a cell or a range | `:<address>` — `p12`, `p12+40`, `#bookmark`, `§2.1.3` |
| Assertable output | `TestBackend`, no terminal needed | the same |

Both are renderers that own nothing: every paint reads `App::get_viewport` (and
`App::layout_block`) and throws the result away.

**`:lint`** — `grind lint`'s findings in a pane (`doc/dsl.md` §4.3, D6), `:lint hints` for the
house-style ones. `j`/`k` moves, `Enter` goes to the finding, any other key closes — the pane
vocabulary this shell already has. `problems.rs` is **shared by both halves**, like `code.rs`
and for a stronger reason: a diagnostic is document-type-neutral by construction, so the two
halves differ only in what an address means, and each resolves one the way it already resolves
any other.

## What it does not do

Deferred by decision, not omission. Everything here is reachable from the CLI (R9).

**The medium's own limits, which are not gaps.** One font at one size, so a font *size* is
stored and not drawn, and a monospace family cannot be drawn as a different font — everything
in a terminal already is one. A `` `code` `` run and a preformatted block are **dimmed**
instead, so they are at least visible as their own kind of text; the document carries the
family either way, and the browser draws both in an actual monospace face.

**SGR 2 is optional, and keeping it is now a decision rather than an omission.** Many terminals
and themes draw dim identically to normal, so a `` `code` `` *run* may be showing nothing at
all — but every alternative is worse. A colour or a background would be the **document's**: a
run's own `fo:color` already becomes exactly that, so a shell-chosen one could not be told apart
from a document-chosen one and would overwrite a run that had both. Bold, italic, underline and
strikethrough are each already a property of a run. Reverse video is the selection and the
caret. And a marker in the line (`` ` ``) is a character the core never measured, which puts
every caret after it in the wrong column — decision 2 above, and the reason markdown here is for
*typing* and never for showing. So a run keeps DIM and this says what that costs. **The block
half does not depend on it**: a fenced block says `pre` in the gutter beside its address, which
is plain text every terminal draws.

Sixteen colours, so a
document's `#ff4136` is drawn as the nearest of them — `nearest_color` in
`ui_tui/src/text/app.rs`, by squared distance in RGB.
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

**The view modes** (`doc/view-modes.md` V7). The role glyph takes a column of the ten a cell
has, which is the mode's price and is paid only while it is on. A name is *not* drawn inside its
cell — ten characters cannot hold a value and a hint, and §3.2 does not let the value yield — so
an anchored cell is underlined and the name itself is spelled out on the formula line for the
cell under the cursor. Sixteen colours again: the roles are drawn in named terminal colours
rather than the palette's own hexes, because an RGB escape is not what every terminal reads and a
mode invisible over `ssh` is a mode half this shell's readers cannot use. A range anchor is not
outlined, and the marks in the word processor sit after the line rather than at their offset in
it — an offset inside the line is an offset the caret counts.

**Both.** The **code view** (`:source`, `doc/dsl.md` §6) is a pane over the document like `:help`
and not a split, and it is **read-only**: `j`/`k` move a line cursor and put the selection on
whatever that line projects, and any other key closes it. A split is what a person eventually
wants and it is two viewports to keep in step; what pays for itself first is the correspondence.
Editing it is gated in `doc/dsl.md` §6.4. It is a `:` command rather than a key for the reason
`:roles` and `:names` are — a mode, and this shell's keys are vi's motions — and it does not
reopen decision 2 above: markdown is still never *drawn* as markers, and the projection is a
separate pane showing a different notation, which is exactly how a source view avoids the problem
that rules the inline one out.

`:help` is a pane over the document rather than a window beside it, and it takes the
whole screen while it is open — a key list is what the reader asked to look at, and half of one
is worse than none. The register is this shell's own, not the system clipboard — a terminal cannot reach
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
