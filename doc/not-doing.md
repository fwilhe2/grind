<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# What this does not do

**Scope note.** This document was written for the spreadsheet in phase 7 and every unmarked
row below is still about it. Phase 10 added a second application, and the word processor's own
line lives in `doc/text-core.md` — element by element, with a test holding
`grind_text::implemented()` to it. What is repeated here are the rows a *user* would notice,
under "The word processor" in each section.

Phase 7 opens by stopping. This is the list the project exists for: not a backlog, not a
roadmap with the dates removed. A backlog is a promise; this is a boundary. Everything here
is either **never**, or **not yet with a named gate**, and the difference is stated for each
item rather than left to be inferred.

Three sections, in order of how binding they are:

1. **Never** — decided, and reopening one is a product decision, not a ticket.
2. **Not yet** — inside the feature line, unbuilt, with the phase that owns it.
3. **Built, with a limit** — a capability that exists and stops somewhere specific.

The rule for moving anything out of §1 is unchanged from `plan.md`: one item at a time, by
explicit decision, and it must survive loop C. Nothing moves because it was easy.

---

## 1. Never

### Not a spreadsheet feature at all

| Not doing | Because |
|---|---|
| **Macros and Basic** | A scripting host is a second product with a second security model. A document that computes is the goal; a document that *executes* is not. |
| **Extensions** | An extension API freezes the core's internals into a contract. The core is 18 months old and the shape is still moving. |
| **OLE embedding** | Embedding another application's document means hosting another application. |
| **Change tracking** | Per-cell revision history is a data model of its own, layered under every mutation. Version control over `.fods` covers the real need — see `doc/cli-recipes.md`. |
| **Database ranges and data sources** | `table:database-range` is read past and dropped. A spreadsheet that is a database client is a different program. |
| **Scenarios, goal seek, solver** | Three separate optimisation UIs over the same evaluator. The evaluator is the asset; the UIs are not. |
| **Sparklines** | A chart in a cell, with its own layout model, for a chart nobody reads. |

### Formats

| Not doing | Because |
|---|---|
| **Writing `.xlsx`** | Writing Excel means owning the Excel semantics this project was built to not have — the 1900 leap-year bug, a different error set, a different text→number rule. Reading it is a cheap escape hatch and stays *possible*; writing it is not. |
| **`.xls`** | Same, plus a binary format. |
| **Anything that is not ODF, for writing** | CSV is the one exception, and it is in §2 because it carries no semantics to get wrong. |

### OpenFormula beyond the line

| Not doing | Because |
|---|---|
| **Large Group** | 26 complex-number functions and specialised conversions. This is the named example of the bloat the project exists to avoid. |
| **Inline arrays (§5.13)** | Excluded from Small Group by §2.3.2. Parsed as a *syntactic* exclusion in loop B, never evaluated. |
| **Array/matrix formulas** | Read as ordinary formulas. A whole second evaluation mode — result shapes, implicit iteration, `table:number-matrix-*-spanned` — for a construct §2.3.2 does not ask for. |
| **Reference union `~`** | §2.3.2 excludes it from the operator set by name. Lexed and parsed so a document containing one still loads; evaluating it is `#VALUE!`. |
| **Quoted labels and automatic intersection (§5.10)** | Excluded by §2.3.2, and the second is the feature that makes a spreadsheet's meaning depend on where a formula sits. |
| **Regular expressions in criteria** | Wildcards are implemented (`wildcard.rs`); regexes are LibreOffice's own either/or, and picking the other branch doubles the surface of every criteria function. |
| **Medium Group, wholesale** | Not never — but never *as a block*. It moves category by category on evidence that a real week of use needed one, which is the same gate as anything else here. `ROW` and `COLUMN` (§6.13.29, §6.13.4) are the first and so far only two to have moved, on 2026-08-16: R7's `fizzbuzz.fods` is eighteen copies of `IF(MOD(ROW();15)=0;…)` and recalculated to eighteen `#NAME?`. The evidence is written down in `doc/small-group.md`'s second half, which is what the gate asks for. |

### Presentation

| Not doing | Because |
|---|---|
| **Fonts** | LibreOffice rewrites `fo:font-family` into an `office:font-face-decls` reference, and nothing here can draw a glyph anyway (`doc/ods-format.md` §5.4). It comes back when a renderer exists, not before. |
| **Conditional formatting beyond one rule type** | The general form is a rule engine with its own evaluation order. One rule type is the whole of the demonstrated need. |
| **Draw objects beyond images** | Shapes, connectors, and their layout model are a drawing application. |
| **Pivot tables** | Revisit *once*, later, as `plan.md` says — and only against a real use, because the honest version is an aggregation engine plus a UI plus a serialisation format. |
| **More than one chart type** | One that round-trips proves the mechanism. The second is taste. |

### The word processor

The full list is `doc/text-core.md`; these are the ones worth stating twice.

| Not doing | Because |
|---|---|
| **Sections and multi-column layout** | A section exists to give a run of content its own page geometry. It is layout, and there is no page model — `doc/suite.md`'s fork was settled on Path A, with pagination itself moved to §2 under a named gate |
| **Text frames, a drawing canvas** | Positioned boxes with flow-around are a page-layout program |
| **Generated indices** — six of `text-content`'s sixteen alternatives | Each is a derivation with its own numbering and update rules, over content nobody edits by hand. Read and preserved; never authored |
| **Mail merge, forms, bibliography, master documents** | Each is a second product with its own data model |
| **Authoring change tracking** | Already never for spreadsheets, and the reason is stronger here. **But it survives a splice** — R6 means an opened document keeps it, and refusing to *author* change tracking is not the same as destroying it |
| **`.docx` writing** | The `doc/xlsx-import.md` asymmetry, for the same reason: one way in, never out |

---

## 2. Not yet

Inside the feature line, unbuilt, and each one is a gate rather than a wish. Nothing here is
half-built and hidden: **no shell can do any of it either**, which is why none of them are
CLI parity gaps.

| Not yet | Owner | Gate |
|---|---|---|
| **Reordering sheets** | When a shell has somewhere to drag one to | A new sheet is appended and that is the whole vocabulary; `Action::InsertSheet` already carries an index, so a move is two actions in a batch when something can ask for one. |
| **CSV import/export** | Phase 9 shells | Format-neutral, and `doc/cli-recipes.md` already drives the import half from the shell. |
| **Sort** | Phase 9 shells | Needs a collation decision first — `eval.rs:503` is code-point order after case folding, not locale collation. Filtering is built (§9.4, `sheet/src/filter.rs`): a set of values compares for equality, which is the half of "sort and filter" that needs no collation. |
| **Find/replace** | Phase 9 shells | Trivial over the column store; there is nothing to type into yet. |
| **Freeze panes** | Phase 9 shells | Purely a view concern, and there is no view. |
| **One chart type** | Phase 9 shells | Must round-trip through LibreOffice like everything else. |
| **Print to PDF** | Phase 9 shells | Via the platform, so it needs a platform. |
| **Preserving what the model does not carry, on a *regenerating* save** | No gate | R6 is met by not regenerating: an opened document is edited in place, so `office:meta`, `office:settings`, unreferenced styles and other vendors' extensions are never touched. They are still lost when the writer *does* regenerate — a new row, a changed format, a conversion between forms — and closing that would mean modelling all of ODF, which is the trade this project exists not to make. |
| **Splicing a `.ods`** | No gate | A zip has no diff to preserve, so the package form always regenerates. |
| **Splicing a format or style change** | When a shell makes it the common edit | A new `style:style` has to be merged into the file's own `office:automatic-styles` — a second splice site and a pool. `sheet format` and `sheet style` regenerate. |
| **`calcext:` on the way out** | No gate; R4 allows it, R2 outranks it | `calcext:value-type` is not valid against the ODF schema, so an item earns its place only with a measured LibreOffice behaviour that cannot be had any other way. Read and ignored today. |
| **Incremental recalculation** | When a UI makes whole-document recalc feel slow | `eval.rs:16`. `graph.rs` is in the plan and unbuilt on purpose; recursion-with-memoisation is the topological order today. |
| **Reading `.xlsx`** | **Phase 11** — `doc/xlsx-import.md` | Scheduled, by explicit decision: Excel is where other people's documents come from, and converting them today means driving a 400 MB office suite headless. One way in — writing `.xlsx` stays in §1. Read by its own filter rather than `calamine`, whose trade is written down in that plan. |

### The word processor

| Not yet | Owner | Gate |
|---|---|---|
| **Undo and redo across invocations** | A session file | `grind sheet --session` carries an undo stack between processes through `grind_sheet::Session`. Giving `grind text` the same means making `grind_text::Action` serialisable, which is a decision about the model rather than about the CLI. No shell can undo a text document either, so it is not a parity gap — `doc/cli-parity-text.md` records it |
| **Splicing an insertion, a deletion or a move** | When it is the common edit | R6 splices *content* edits today: retyping a paragraph, restyling one, changing its kind. Changing the block **sequence** regenerates, because splicing that means deciding where new bytes go and with what indentation, for a diff no longer obviously smaller. `grind_sheet` draws the line in the same place — a cell that did not exist regenerates too |
| **Tables in a text document** | S6+ | The element vocabulary is shared with the spreadsheet; the model is not — an ODT table cell holds *blocks*, not a value. And `doc/odt-format.md` §5 has an open `UNVERIFIED` question about whether `table:formula` in a Writer table is even OpenFormula |
| **Footnotes, fields, a table of contents** | Somewhere to put one | A footnote's *content* models fine; its *placement* is the page model, which is gated above. A continuous view can still show one — at the end of the flow, or inline — so this is a shell question rather than a blocked one, and it waits for a shell |
| **`style:parent-style-name`** | Before a shell shows formatting | Not followed, as for cells — but it matters far more here, because text documents are built on a named-style hierarchy and not following it loses most of a document's formatting rather than one number format |
| **Style *definitions* — writing `office:styles` back** | With `style:parent-style-name`, above | The model carries a style's *name* and never its properties, so there is nothing for the writer to declare. Loop C measured what that costs: a name this build invents does not survive LibreOffice, because an undeclared name is not a style (`doc/odt-format.md` §5b) |
| **Pagination — a page model, and everything that needs one** | **Loop D at a stated floor**, and a real week of use proving the continuous view insufficient | Page size, margins, `fo:break-before`, headers, footers and footnote placement are read, preserved and round-tripped, and never rendered as pages. Building it means building the engine *and* the differential that keeps it honest — render a corpus document our way and LibreOffice's way, compare line-break positions and page boundaries, ratchet the percentage exactly as loop B does. The pinned oracle pays off twice there, because a digest pins the *fonts* as well as the renderer and an unpinned layout differential is noise. **This row is stable under every path in `doc/text-layout.md`** — pagination is gated whichever way that decision goes. What is *not* settled is the separate question of whether **line** layout belongs in the core, which that document is open on |
| **Right-to-left layout (bidi, UAX #9)** | A real RTL document somebody wants to edit | **An explicit exclusion, not an oversight** (`doc/text-layout.md`, decision 1). Layout is left-to-right only: `unicode-bidi` is not taken, and a document with Hebrew or Arabic in it lays out as though it were LTR, which is *wrong for that document* rather than merely unstyled. Affordable only because of R6 — such a document is still read, preserved byte-for-byte and written back correctly, so the file is right and only the view is wrong. `style:writing-mode` (rng:2864) is preserved and never consulted. Reopening means taking `unicode-bidi` at the layout engine's existing seam and answering what caret movement means in mixed-direction text — logical or visual — which is a question worth answering with a document in hand rather than guessing now |
| **Any shell but the CLI** | R10, `doc/suite.md` S8–S10 | The terminal shell is next, then GTK, then the browser. A document type reachable from only one shell is the hole R10 exists to forbid |

---

## 3. Built, with a limit

Capabilities that exist and stop somewhere specific. Each limit is named where it lives in
the code, and this table is an index rather than a second source of truth.

| Capability | Stops at | Where |
|---|---|---|
| **Locales** | The decimal separator and the grouping separator. Switzerland's apostrophe and India's lakh grouping are wrong. | `locale.rs:16` |
| **Month and weekday names** | English, whatever the document's locale says. | `numfmt/` module docs |
| **Text→number conversion** | ISO 8601 only. Part 4 §6.3.6 makes it `HOST-LOCALE`-dependent, so LibreOffice reads `"0,005"` in a German document and this does not. | `date.rs` |
| **A date a formula computes** | Displays as its serial until the cell is formatted — the subtype belongs in `formula::value` (§4.3.3), not in `numfmt`. | `datetime.rs:15` |
| **Preset date formats** | The ISO spelling. The model holds `DD.MM.YYYY` fine; nothing can ask for one. | `numfmt::preset` |
| **`style:map`** | Read, rendered and round-tripped; `sheet format` cannot build one. | `numfmt/` module docs |
| **`style:parent-style-name`** | Not followed. A cell style inheriting its data style from a parent loses the format. | `read.rs:622` |
| **Border widths** | Compared numerically in loop C, because LibreOffice re-quantises them (`0.5pt` → `0.51pt`). | `style.rs` |
| **Border line styles, on screen** | Read, written and round-tripped; *drawn* solid, so `dashed` and `double` look like `solid`. The width and colour are honoured, which is what carries the meaning of a ruled table. | `grid.rs`'s `draw_borders` |
| **Wrapped text, on screen** | Wraps inside its own cell and clips at the row height, because every row is one line tall until the model carries heights. A font size larger than the row clips for the same reason. | `grid.rs`'s `draw_cells` |
| **The in-cell editor's font** | The widget's, not the cell's — a bold cell is edited in a regular weight. `gtk::Text` is a real child widget, and restyling it per cell is a second font path for the duration of one edit. | `doc/gtk-shell.md` M7 |
| **Formatting a whole column** | Formats the column's *used* part. A real column default is `table:default-cell-style-name`, which the reader honours on the way in and the model cannot yet write. | `Grid::target` |
| **The default colour palette** | Seventeen named colours (<https://clrs.cc/>, `style::PALETTE`) offered by a shell's swatches and `sheet style --color`. A *default, not a limit*: any `#rrggbb` is still accepted, a GUI's *Custom…* opens a colour dialog, and a document's own colour is kept whatever it is. What is deliberately absent is a *user-editable* palette, which needs settings a shell does not have yet. | `style.rs` |
| **The format picker's vocabulary** | Exactly `numfmt::preset`'s parameters, which is what makes GUI- and CLI-formatted documents identical. A document's format that is outside it — `DD.MM.YYYY`, a two-branch currency — is kept, rendered, and reported as one the picker cannot build (`Format::is_preset`) rather than silently replaced. | `ui_gtk/src/formatting.rs` |
| **`NOW` / `TODAY`** | UTC, not the host's local time. | `date.rs:283` |
| **String comparison** | Code-point order after case folding, not locale collation. §6.4.9 permits it. | `eval.rs:503` |
| **Corrupt-zip recovery** | Unbuilt. No corpus file needs it, and it belongs with the spec's explicit repair mode. | `CLAUDE.md` |
| **Loop C's `back` direction** | Skips formula-bearing documents. | `roundtrip.rs` |
| **Named expressions** | One flat map, so a sheet-local name is visible document-wide. | `model.rs:258` |
| **Renaming or deleting a sheet** | Formulas naming it are not rewritten, so they go stale and recalculate to an error. Visible rather than silent: every write warns, and `sheet recalc` counts it. | `App::rename_sheet` |

### The word processor

| Capability | Stops at | Where |
|---|---|---|
| **R6's diffable write** | *Content* edits to blocks the file already spells. Opening a document and saving it returns its bytes exactly; editing one paragraph changes one line; a structural change regenerates and loses what the model does not carry | `text/tests/diffable.rs` |
| **Nested `text:span`** | Flattened on read, composing the style names into one. Lossy for the *names*, lossless for the rendering — and only reachable at all for a paragraph somebody edited, since R6 never rewrites one nobody touched | `doc/text-core.md` |
| **Lists** | Flattened into the block sequence with a depth, so the `text:list` element's own style name and `text:continue-numbering` are not carried and are lost if that list is edited | `model::BlockKind` |
| **Headings** | Read at any level — the schema's `positiveInteger` has no ceiling — and authored at 1–6 | `doc/text-core.md` |
| **What LibreOffice actually does** | Unmeasured. `doc/odt-format.md` §5 is six named questions, all `UNVERIFIED`, and none of them may be implemented until one carries a citation | `doc/odt-format.md` §5 |

The `ponytail:` comments are the full ledger and outnumber this table; these are the ones a
*user* would notice.

---

## The point

Every row above is a thing that will not be in the way — of the code, of a reviewer, or of
someone deciding whether this program does what they need. A feature list that ends is only
credible if the ending is written down, and this is where it is written down.
