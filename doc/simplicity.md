<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: CC-BY-4.0
-->

# Simple, not easy

**Status: decided 2026-11-20. Phases 0 to 3 and 5 done; phase 4 under way.**

*Simple* is one concept in one place doing one job; *easy* is near at hand. Most of the
accidental complexity here came from easy steps: a decision copied into the next shell, a feature
flag, a module made `pub` so a test can reach it, a paragraph appended to a status table. ODF,
OOXML, R6's splicing and the native toolkits are inherent and are not this document's business.

## The rules

These hold for every change, and the ones a test can hold are held by one
(`cli/tests/simplicity.rs`, beside the other workspace-wide guards), as ratchets that only
tighten.

1. **Done means deleted.** A migration ends with the old path removed. The paths that still exist
   twice are listed below and the list only shrinks.
2. **Decide once, in the core.** When two shells hold the same decision — a key, a constant, a
   piece of geometry — it moves into the document type's crate.
3. **No optionality without a user.** No cargo feature, setting or flag unless someone named
   needs the other answer.
4. **No `pub` without an outside caller.** A test that needs an internal is a unit test.
5. **Code carries no history**: no milestone tags, no "since", no "used to". A date in code is
   data or the evidence a ratchet was measured on, never a changelog.
6. **Docs describe the present.** A design document says what is true and why; its status is one
   line; how it got there is in `git log`.
7. **A new loop or oracle needs evidence** that it catches what the existing ones cannot.

## Where the complexity is

**The same decision in five or six places.** Every shell has its own key map with its own
`Dir`/`Motion`/`Action` (ten files); GTK, Win32 and the browser hold the same CUA table. Selection
anchors and carets live in each shell's state. Five `geom.rs`/`layout.rs` files answer which cell
is under a pixel. The text side already solved this — `grind_print` sets the pages and every
graphical shell blits them — and the sheet side has not.

**Two ways to do one thing**, found 2026-11-20 and both since closed: a chart's geometry, decided
by the core for Win32 and the Mac and by GTK and the browser for themselves; and a text document
in the browser, drawn as its pages or, while they loaded, as lines of the browser's own layout.
A new one goes in a table here until it is closed.

**Optionality nobody uses.** Seven cargo features on the CLI and two or three on each shell,
about 130 `cfg` sites and a `--no-default-features` job, while every text window depended on
`grind-print` anyway.

**Public surface far past its callers.** `grind-xlsx` and `grind-docx` export every module;
outside them, a handful of entry points are called. `grind-sheet` has over 600 `pub fn`s.

**Docs that became build logs.** Status cells of a thousand words, milestone journals,
per-milestone evidence sections; designs nobody decided sitting beside the normative ones. The
code is mostly clean of this — the history is in `doc/`.

## The plan

### Phase 0 — guardrails

`cli/tests/simplicity.rs` holds three counts, each equal to its ceiling so that lowering one
lowers the ceiling in the same commit: `pub mod` per library crate (rule 4), the cargo features
and the platform each answers (rule 3), and milestone tags in comments (rule 5, at zero).

### Phase 1 — cuts

| Cut | What goes | State |
|---|---|---|
| The Euro-Office oracle | its two loop D tests, `scripts/euro-office-*`, `ci/euro-office-image`, `euro-office.yml`, and the comparison's allowances for it | done |
| Undecided designs | `doc/sqlite.md`, `doc/web-storage.md`, `doc/ideas.md` (its built items are `grind diff` and the lint rules, which say what they are) | done |
| Cargo features | the import, PDF, spelling, grammar, language-server and server features of the CLI, `grind-open` and every shell, and the `--no-default-features` job. `grind-print`'s font and hyphenation bundling and `grind-spell`'s dictionaries stay: the browser needs them off | done |
| The browser's text line renderer | the page view is the only one. Its alt-text chips moved onto the pages; a reader layer gives screen readers and the smoke test the words on screen; the smoke test hands the page the print module, so it checks the view people use. A text document waits for the print module before it shows | done |

Considered and kept: the macOS shell, `--session`, checkboxes in cells, the EMF/WMF renderer, the
grammar checker, the language server.

### Phase 2 — finish the half-done migrations

Done: every graphical shell draws a chart from `chart_paint`'s marks, with GTK's look, and
hit-tests it through `chart_paint::Picture`; the browser draws a text document only as its
pages.

### Phase 3 — narrow the surfaces

Done for modules and functions: a module or function no other crate and none of the crate's
own integration tests name is private or `pub(crate)`, in every library crate, and what only
being public had kept alive is deleted. Types and fields are the same question, one size down,
and are asked as phase 4 touches them. clerk (`../clerk`) is checked before each.

### Phase 4 — the shell's decisions move into the core

One layer at a time, GTK first, each shell deleting its copy as it is ported, each step proved
by byte-identical `--render-to` frames:

Done so far: one motion is applied the same way in every grid (`nav::apply`), and what a key does
to the active cell is one table (`nav::motion_for`); the browser's copies of the core's
selection and motion types are gone, and a double-clicked word is `word::around` everywhere. A
text selection's two ends are `caret::selection`; the ignored grammar rules are the checker's
(`grind_grammar::checker`); a colour blend is `color::mix`; the header edge a pointer picks up
is `Sizes::edge_near`.

Left, and why: the rest of the grid's geometry follows each shell's scroll model — GTK and the
browser scroll by pixels, Windows by whole tracks, the Mac through `NSScrollView` — so sharing
it means changing how three of them scroll, each proved by frames. A text view's editor state
(caret, anchor, goal column, the style pending for the next character, a composition) and every
edit at it is `grind_text::caret::Editor`, which the Mac and Windows hold, with both platforms'
word rules (`caret::word`, `caret::word_start`); GTK and the browser still keep their own, each
with its own copy of where a motion lands.

1. **Input.** A portable `Key`/`Mods`, one `Intent` vocabulary and the CUA table per document
   type. The terminal keeps its vi table and emits the same intents.
2. **Editor state.** Selection, caret and edit mode, and what an intent does to them, in the core.
   A shell holds an editor rather than anchors.
3. **The grid's frame.** One geometry — visible tracks, hit testing, rectangles — and the grid as
   a list of marks, as a chart already is. A shell executes marks.

### Phase 5 — docs in the present tense

Each design document rewritten to its decisions and their reasons; journals, evidence logs and
dated status cells dropped (git has them). `CLAUDE.md` kept true rather than cut to a length —
read on 2026-11-20, it was current but for one claim, and what it lacked was a command;
`AGENTS.md` a pointer to it.

Done for the page model, PDF export, the Windows, macOS and GTK spreadsheet shells, the
projection, `grind init`, the docx filter, the formula workbench, the grammar checker, the view
modes, the server and loop E: each says what is built and what is not, and nothing cites a
milestone by number. `doc/xlsx-import.md` keeps its milestone labels: `xlsx/tests/ooxmlgen.rs` and the
corpus's README classify every fixture by them, so they are a vocabulary the tests check rather
than history. `AGENTS.md` points at `CLAUDE.md`. `doc/suite.md` and `doc/text-layout.md` say
what was decided and why, without the milestones and the argument that preceded it;
`doc/feature-matrix.md`, `doc/not-doing.md` and `doc/spelling.md` carry no dates other than evidence
dates. The format notes' dates are when a fact was observed, which is evidence and stays.

### Phase 6 — the big files

`ui_win32/src/win.rs`, `print/src/text/mod.rs`, `ui_tui/src/sheet/app.rs`,
`ui_sheet_gtk/src/grid.rs` split along the seams phase 4 leaves, not before.

## Keeping it simple

Every new feature answers three questions: who used it last month, what replaces it if it goes,
and which row of `doc/not-doing.md` it moves. Once a month the counts in
`cli/tests/simplicity.rs` and the two-ways table above are read and the ceilings lowered.
