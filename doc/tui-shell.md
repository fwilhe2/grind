<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# The terminal shell

What `grind-tui` does, how it decides *how* to do it, and what it deliberately does not.
Normative for `ui_tui/` the way `doc/web-shell.md` is for the browser and `doc/sheet-shell.md`
is for the spreadsheet's GTK window.

One binary, both document types, chosen by `grind_core::kind` from the file's bytes (R10).

## The three design decisions

**1. Vi, not a menu.** A terminal has a keyboard and no pointer, so the shell is modal:
**Normal** navigates, **Visual** selects, **Insert** types, and `:` opens a command line.
Every verb a toolbar would carry is a `:` command or a Visual-mode key, and the two shells
share a vocabulary wherever the underlying capability is shared — `v`, `y`, `p`, `d`, `*`, `/`,
`-`, `n`, `N` mean the same thing in a spreadsheet and in a document, because one suite should
not have two words for emphasis or two ways to say *the next one*.

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

**3. Two rows of chrome, and what buys them.** A window may have twenty-four lines, so a bar is
never free: every row of frame is a row of document nobody can read. There are two — a **title
bar** at the top and a **status bar** at the bottom, both `ui_tui/src/chrome.rs`, written once
because both halves frame the same four facts (which document, whether it has unsaved changes,
which mode the keyboard is in, where the cursor is). They earn their place by carrying what
nothing else on screen could:

* the **sheet strip** is this shell's only answer to *which sheets are there* — `:sheet <name>`
  needs the name before it can go to it, and every other shell in the suite has a tab bar;
* the **heading path** (`§2.1 Costs`) is the only thing that says where in a long document the
  caret is;
* the **mode chip** is a coloured word rather than `-- INSERT --` in the middle of a sentence,
  which is what a modal editor owes a reader who looked away for a moment;
* the status bar's right-hand end is fixed, so the **selection's arithmetic** and the caret's
  address stay in one place instead of moving with the length of whatever message is beside them.

A bar paints a ground of its own, which is not a contradiction of the rule that this shell never
picks colours: the ground is a *named* colour, so it is the reader's own blue, and the document
area between the two bars is left exactly as the terminal had it. The **completion band** is a
third row and comes and goes with the list it holds, so a document being read is never a row
shorter than one being edited.

## What is built

| | The grid | The document |
|---|---|---|
| Move | `hjkl`/arrows, `0`/`$`, `g`/`G`, `Ctrl+f`/`Ctrl+b`, counted in the tracks that are **drawn** | the same, and every vertical motion is a **wrapped line** answered by the core |
| Select | `v` — a rectangle | `v` — a run of text, across blocks |
| Edit | `i`/`a`/`c` into the formula line, Enter commits, Esc cancels | `i`/`a`/`o`, typing goes straight into the document, `x`/`X`/`J` |
| Formula help | offers while typing, the signature of the call the caret is in — a band under the formula line; Tab accepts, Up/Down pick, Esc dismisses | — |
| Clipboard | `y`/`p` — a register of tab-separated text, the shape every other spreadsheet reads | `y`/`p` — plain text, a newline splits a block |
| Format | `*`/`/`/`-` over a selection; `:bold :italic :wrap :border :align :color :fill :plain` | `*`/`/`/`_`/`~`/`-` over a selection; markdown while typing; `:color :highlight :plain` |
| Number formats | `:format` over eight presets, `:general` | — |
| Structure | `:sheet`, `:sheet-new`, `:sheet-rename`, `:sheet-delete`; `:width`, `:fit`, `:height`, `:hide`, `:show`; `:name`, `:name!`; `:format-table [--no-header] [--totals [FUNC]] [--name NAME]` (FUNC is a `TotalsFunction` id: sum, average, count, count-numbers, min, max, stdev, var) | `:h <level>`, `:li [depth]`, `:style [name]`, `:table [rows cols]`, `:move <address>`, `:mark`, `:mark!`, `:words` |
| Fill | `:down`, `:right` — the selection's leading line replicated, references shifted | — |
| Interchange | `:csv-in <file>` — at the cursor, delimiter sniffed from the file's own content (`csv::Import::sniffed`, the options every window imports with); `:csv-out <file>` — the selection or the whole used sheet, and `.tsv` writes tabs (`csv::Dialect::for_name`) | — |
| Evaluate | `:eval <formula>` — what it would come to, storing nothing | — |
| Find | `:find <text>`, then `n`/`N`; every match marked in the grid; `:s/old/new/` | `:find <text>`, then `n`/`N`; every match marked in the line; `:s/old/new/` |
| Outline | — | `:outline` — a pane, one row per heading, indented; `Enter` goes to one |
| Paper | — | `:pdf <file>` — the document typeset and written as a PDF, the export's report on the status bar; `:pages` — where each page of that PDF begins, in the outline's pane, `Enter` goes to the top of one (`doc/pdf-export.md`). A terminal cannot *show* a page, so there is no preview: a named gap |
| View modes | `:roles` — what each cell is, coloured and marked with one glyph; `:names` — a named cell underlined, the name and the formula read through its names on the formula line | `:names` — where each bookmark anchors, after the line it falls on |
| Problems | `:lint`, `:lint hints` — what the document says about itself; `j`/`k` moves, `Enter` goes to the finding | the same pane, the same keys |
| Help | `:help` — the key list, over the document, scrollable | the same, with its own section |
| Drawn from the document | **the column widths**, bold, italic, colour, background, alignment; filtered, hidden and folded tracks are gone on both axes | bold, italic, underline, strikethrough, colour, background; monospace runs and preformatted blocks dimmed; headings and `Title`/`Subtitle` emphasised; **list items indented, with a bullet**; **a table drawn as a grid**; the block's kind in the gutter |
| Addressing | `:<address>` — a cell, a range or a **defined name** | `:<address>` — `p12`, `p12+40`, `#bookmark`, `§2.1.3` |
| Assertable output | `TestBackend`, no terminal needed | the same |

Both are renderers that own nothing: every paint reads `App::get_viewport` (and
`App::layout_block`) and throws the result away.

**A motion counts in tracks the document shows.** `ui_tui/src/sheet/keymap.rs` is pure and has
never seen a document, so it is handed the two folded sets (`Folded`) and steps over them —
`doc/sheet-shell.md` names exactly that as the upgrade for the GNOME window's own version of this
gap. It is not a nicety: a filter that folds five rows away used to mean five presses of `j` to
move one row on screen, with the cursor invisible for four of them, which does not read as a
cursor on a hidden row — it reads as a terminal dropping keystrokes. The set the motions step over
is the same one the grid folds away, so the cursor cannot land where nothing is drawn. Note that
**`App::hidden_rows` is the filter's answer alone**; the manual half is `App::manually_hidden_rows`
and `App::folded_rows` is where the two are unioned, once, for both callers.

**The grid's arithmetic is `ui_tui/src/sheet/geom.rs`**, which has never heard of a terminal —
the `geom.rs` every other shell in the suite has, for the reason they all give: the layout
decisions are the part most worth testing and the part hardest to test through a display. It is
where a column's ODF length becomes a number of cells (one cell is a tenth of an inch, *derived*:
the suite's default column is an inch and this shell draws an unsized one ten cells wide), where
the visible columns are accumulated from the scroll position, and where padding is measured in
terminal cells rather than in `char`s so a column of CJK text lines up with the one beside it.

**A table is a grid, and `Measures` is why.** A document is a flat sequence of blocks and a
terminal is a stack of lines, so everything here is one block per line — except a table, whose
cells are placed by *coordinate*. `grind_text::Faces` is handed a block's kind, which is enough
to indent a list item, and not enough to know a block is in a cell; so the cells are a map, built
once per frame from the blocks in and around the view, and `Faces::of` reads it back. That is the
same shape `ui_text_gtk/src/view.rs`'s `Column` has, and it is required rather than chosen:
`Faces::of` is called while `App` holds its read lock and must not ask the document anything.
Every cell is then laid out at its **own** measure, which is what makes `j` inside a cell land
where the ink is.

**`:lint`** — `grind lint`'s findings in a pane (`doc/dsl.md` §4.3, D6), `:lint hints` for the
house-style ones. `j`/`k` moves, `Enter` goes to the finding, any other key closes — the pane
vocabulary this shell already has. `problems.rs` is **shared by both halves**, like `code.rs`
and for a stronger reason: a diagnostic is document-type-neutral by construction, so the two
halves differ only in what an address means, and each resolves one the way it already resolves
any other.

**An Excel workbook opens** (`doc/xlsx-import.md`, X6) from the command line: `grind-tui budget.xlsx` is a spreadsheet, the title bar says `budget.fods`, the status line carries the report and names the `:w` that would save it, and `:q` refuses to lose it — sniffed from the bytes,
imported through `grind_xlsx::open`, and opened as a **new, unsaved ODF document with no path**
under `grind_xlsx::suggested_name` (`budget.xlsx` → `budget.fods`), with the report's one
sentence (`Report::summary`) shown. Having no path is the point: one way in and never out
(`doc/not-doing.md` §1), so nothing can write ODF over the workbook it came from. `import.rs` is
this shell's whole half of it, and is compiled out with the crate's `xlsx` feature.

**A Word document opens** the same way into the word processor (`doc/docx-import.md`, DX6):
`grind-tui letter.docx` is a text document titled `letter.fodt`, unsaved and with no path, the
report's sentence on the status line — `import::open_word`, behind the crate's `docx` feature.

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
document's `#ff4136` is drawn as one of them — `ui_tui/src/ink.rs`, by the rule "The UX pass"
below gives: a text colour as its hue family's member that reads on a light *and* a dark
terminal, a fill as its nearest, and ODF's automatic ink on a fill.
No pictures, no charts: a chart in a file is kept and written back untouched, and nothing here
draws one.

**A row is one line**, so a **row height** is stored and not drawn — `:height` writes one into
the document in ODF's own unit and says on the status line that this shell will not show it.
That is the medium and not a gap in the same way a font size is: every other shell draws it, the
CLI reads it back, and a document that arrives with row heights keeps them. Wrapped cell text is
the same limit seen from the other side, and it is why `:wrap` is stored and not drawn either.

**The grid.** No point mode while typing a formula — the offers and the signature band are built
(`ui_tui/src/sheet/assist.rs`), and arrow keys building a reference into a half-typed formula are
not, because that is a third editing mode rather than a read-out. No *standalone* filter UI: a
filter in the file folds its rows away, and nothing creates one on its own — `:format-table`
does create one, as one facet of the composite it applies (`App::format_table`,
`sheet/src/table_format.rs`), the same way the GTK shell's dialog does. No conditional
formatting UI: the banding `:format-table` paints is static cell styling, applied once, not a
live rule. No charts, no autofit — a column's ideal width is
`ui_sheet_gtk`'s measurement of the text in it, and this shell would need the same pass.

**The document.** No pages, no print, no zoom, no RTL (`doc/text-layout.md`). No footnotes or
fields, because the core has none. An image in a file is kept and not drawn, and a named
*character* style is kept and not interpreted (`doc/text-core.md`). A table is drawn as a grid
now, but **every column of it is the same width**: a table carries no style of its own in this
build, so there are no column widths to honour and no merge to create — `doc/text-core.md`'s
gap, showing through. A merged cell in a file is read, kept and written back, and drawn as an
ordinary one.

**What the table grid costs, written down.** The map of which block is in which cell covers the
view and a page either side of it, not the whole document (the `ponytail` on `Measures`). A caret
motion that leaps out of the view into a table it has never drawn measures that table's first
block at the full measure for one frame; the frame after it is right, because `draw` rebuilds the
map either side of following the caret. The upgrade is a flow over the whole document, which is
what the GNOME window builds — and what a terminal showing thirty lines of a thousand-block
report should not.

**The view modes** (`doc/view-modes.md` V7). The role glyph takes a column of the cell's own
width, which is the mode's price and is paid only while it is on — and a column clipped at the
window's right-hand edge has none to give, so it keeps its text instead. A name is *not* drawn
inside its cell — ten characters cannot hold a value and a hint, and §3.2 does not let the value
yield — so an anchored cell is underlined and the name itself is spelled out on the formula line
for the cell under the cursor. Sixteen colours again: the roles are drawn in named terminal colours
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

**Four panes and one widget's worth of them.** `:help`, `:source`, `:lint` and `:outline` are all
panes *over* the document rather than windows beside it, and each takes the whole screen while it
is open — what the reader asked to look at is what they are reading, and half of a key list is
worse than none. Three of them know what they are holding; the fourth, `ui_tui/src/pick.rs`, does
not: a row is a string to show and a string to go to, which is every list a document can offer.
The outline is its first caller and a list of defined names or of bookmarks would be the same
pane with a different `Vec` — a shell that grew a fifth list widget would have five answers to
"which key closes this". `ui_win32/src/dialog.rs`'s `choose` is the same idea in another toolkit.

The register is this shell's own, not the system clipboard — a terminal cannot reach
one without a protocol the host may not speak, and vi's register is the convention a reader of
this shell already has. Markdown-while-typing costs two undo steps rather than one (an erase
and a style), which is the honest price of not inventing a compound action for a shell's own
convenience.

## The UX pass — in a real terminal, light and dark

The review the other three shells had, and the first time this one was looked at in a terminal
rather than through `TestBackend`: `grind-tui` in an `xterm` on an Xvfb display, driven by
synthetic key presses, at 80×24 and 120×36, on a white theme and a dark one. Every item below was a
screenshot first. No capability moved.

- **A document's colours read in either terminal** (`ink.rs`, shared by both halves). The other
  shells know which ground they paint on; a terminal does not say, so this rule has to hold on
  both. Navy text had become terminal *black* — invisible on every dark theme — and a heading row
  filled silver kept a dark theme's light ink on a white fill. A text colour is now its hue
  family's member among the seven named colours that clear a contrast of 2.5 on white *and* on a
  dark ground (measured, and a test holds it); a grey, black or white of its own is the terminal's
  own ink; a fill is its nearest named colour at xterm's values, so silver is a light-grey band
  rather than a white one that erased it; and text with no colour of its own on a fill is ODF's
  automatic ink over `grind_core::color`, the rule every other shell's `ink` applies.
- **`:roles` erased every formula on a light terminal.** A computed cell was drawn `White`; it is
  the terminal's own ink now, and no role may be a colour one of the two grounds erases (a test).
  The code view's keywords and the error marks moved off blue and red for the same measurement.
- **A number is never cut.** A ten-cell column drew `2026-08-16` as `2026-08-1` — the first of
  August — and `3,710.00 €` as `3,710.00`. A number, date or time that does not fit is `###`, what
  LibreOffice and Excel draw and the one spelling nobody reads as a value; text that does not fit
  ends in `…`, the other shells' own mark (`sheet/geom.rs`, `Fit`). The hashes are
  `grind_sheet::numfmt::overflow`, the rule all four shells now draw — see "One rule for a number
  that does not fit" in `doc/feature-matrix.md` §5. The honest cost, written down:
  at this shell's ten cells to the inch a default column holds nine characters, so the sample
  document shows five `###` that the windows draw in full — `:width` widens one.
- **A tab is drawn.** It was measured as one cell and drawn as none, so `name⇥value` read
  `namevalue` and every caret after it stood a column right of its character. It is four cells —
  the browser's and the Windows pane's width — and drawn as four spaces (`text::TAB`).
- **Typing has a visible insertion point.** The grid's formula line kept a caret it never drew,
  so Left and Right moved something nobody could see, and the `:` line had none either. Both now
  place the terminal's **own** cursor, as a bar — the shape an insertion point has in every editor
  that tells one from a block — restored to the reader's own shape on the way out. The text pane
  keeps its reversed caret, which was always visible.
- **A selected row is one bar.** The outline, the problems pane and the code view each reversed
  their spans one at a time, so a coloured address or token reversed into a block of its own
  colour — a cyan block beside a black one, stopping where the text did. `pick::selected_row` is
  the whole row, edge to edge, with the row's own colours set aside on it.
- **A pane is titled with the file's name**, as the title bar is, rather than its whole path.

What the pass left, and why:

- **The mouse.** Decision 1 says a terminal has no pointer, and nearly every terminal in use sends
  mouse events: a click to move the cursor and a wheel to scroll are what a reader tries first.
  That is a change to a normative decision rather than a fix, so it is named here rather than made.
- **Ten cells to the inch** is what makes the `###` above common; twelve would match what a window's
  inch holds, at the price of one column fewer on an 80-column terminal. Also a decision.
- **The terminal cursor as the focus point everywhere** — a screen magnifier or braille display
  follows the hardware cursor, and in Normal mode this shell hides it and draws its own. Placing it
  on the active cell and the text caret is the accessibility floor this pass stopped short of.

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

## The welcome pane (2026-10-03)

`grind-tui` with no file and neither `--sheet` nor `--text` no longer guesses a spreadsheet: it
opens `ui_tui/src/welcome.rs`, three choices (`s` new spreadsheet, `t` new text document, `o` a
path to open — the arrows and Enter work too) and `q` to leave. `:welcome` returns to it from
either pane, refusing over unsaved work like `:new`; `:welcome!` discards. Recent files stay a
named gap: a list needs a store this shell does not have.
