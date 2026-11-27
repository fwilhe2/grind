<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: CC-BY-4.0
-->

# The suite — a word processor beside the spreadsheet

How one ODF-native spreadsheet is an ODF-native **suite**, and the rules that hold it: what the
things are called, how the core splits so that a second document type is a peer rather than a
guest, how the CLI is shaped, how every shell reaches every document type, and how it is
packaged. Normative for phase 10. **Status: built.** Where text is laid out is
`doc/text-layout.md`'s decision and outranks this document on it.

## Decisions

| Question | Answer |
|---|---|
| Suite command | **`grind`**: `grind sheet …`, `grind text …`, and verbs that read the kind from the file |
| Word processor | **`text`**, ODF's own word for it (`office:text`) |
| Relationship to the core | **Split, don't wrap**: a generic crate, one crate per document type |
| Two `App`s or one | **Two**, and no shared trait yet ("The two `App`s") |
| Shells | **Every document type reaches every shell.** GTK splits per type because a `.desktop` file must; every other shell is one binary |
| Layout | Line layout in the core with metrics injected; pages set by `grind-print` (`doc/text-layout.md`, `doc/page-model.md`) |
| Presentations | Out of scope. The shape must not preclude a third type; `slides` is reserved |

## The names, and why

The README promises that no trademark is used in the name of the project, its binaries, its
icon or its packaging. These names keep it.

| Name | Role | Why |
|---|---|---|
| **`grind`** | The suite, the first-level binary, the crate prefix | A plain English word with no software mark of consequence, and it reads as a sentence: `grind sheet new`, `grind text outline` |
| **`sheet`** | The spreadsheet | Generic |
| **`text`** | The word processor | **ODF's own word**: the body is `office:text`, the media type `application/vnd.oasis.opendocument.text` |
| **`slides`** | Reserved, unbuilt | `office:presentation`'s plain-language name |

Rejected, which matters more than the accepted list:

| Rejected | Because |
|---|---|
| **`odf`** | OpenDocument and ODF are OASIS trademarks. Descriptive use in prose is fine; a binary or crate named `odf` is a trademark in packaging |
| **`writer`** | LibreOffice Writer, and the most confusable choice available |
| **`word`, `office`** | Microsoft's |
| **`docs`, `pages`** | Google's and Apple's |
| **`document`** | Ambiguous: in a suite a spreadsheet is a document too |

Names that are one application's say so — `ui_sheet_gtk/` beside `ui_text_gtk/`,
`examples/sample-sheet.sh` beside `sample-text.sh`, `doc/sheet-shell.md` beside
`doc/text-shell.md`. `ui_tui/` and `ui_web/` hold both types and split inside, in `src/sheet/`
and `src/text/`. The `GRIND_*` variables keep unqualified names: loops B and E are the
spreadsheet's, and `CLAUDE.md`'s loop table says so. There is no `sheet` alias for the old
binary name: nothing had been released, and a second name kept forever for no users is what
`doc/not-doing.md` refuses.

## The requirements, extended

`doc/plan.md`'s R1–R7 apply **per document type**, unchanged in wording. Two of them weigh more
for text:

- **R2 and R5.** An ODT in the wild carries change tracking, fields, sections, frames, shapes,
  forms and bibliography markup; far more of the file is content with no model than in a
  spreadsheet. The tolerant reader is the same architecture; the regenerating writer produces
  valid ODF for the subset it owns.
- **R6 is the strongest reason a word processor exists here at all.** A `.fodt` that changes one
  line of XML when one paragraph is edited lives in git like a source file
  (`text/tests/diffable.rs`). Per `text:p` it is the spreadsheet's per-cell splice against a
  different element, and it is what carries every unmodelled construct through an edit.

LibreOffice Writer's `loext:` is read and ignored on R4's terms: it earns a place on the way out
only with a measured behaviour recorded in `doc/odt-format.md` that cannot be had otherwise.

Three requirements are added:

| | Requirement | Held by |
|---|---|---|
| **R8** | **No document type's vocabulary appears in the shared crate.** `grind-core` knows packaging, namespaces, the context stack, `fo:` primitives and line layout, and nothing about cells, sheets, paragraphs or headings | `core/tests/generic.rs`: its `Cargo.toml` names no app crate, and its sources carry none of the body vocabularies. A shared crate without a mechanical guard becomes a dumping ground |
| **R9** | **Whatever any GUI can do, the CLI can do — per application** | `cli/tests/parity.rs`, over a registry of `(core crate, parity document)` pairs: `doc/cli-parity-sheet.md`, `doc/cli-parity-text.md` |
| **R10** | **Every document type is reachable from every shell.** A per-shell feature gap is allowed and must be named; a shell that cannot open and edit a type at all is a defect | `doc/feature-matrix.md`, whose every ◐ and ○ carries its reason |

## How the core splits

`CLAUDE.md`'s crate table is the layout. The rule that drew it: **a seam is extracted when a
second caller pulls on it, never when a document predicts it will.** The package, manifest,
escaping and envelope moved into `grind-core` when the text writer needed them, and were proved
moves by the spreadsheet's R6 figures not changing by a digit. Three things predicted to move
have not, because no second caller pulled:

- **`numfmt` stays in `grind-sheet`.** The format *model* is ODF-generic; the module mostly
  renders a cell's value with one, and depends on the formula engine's dates and values. It
  moves when `grind-text` needs a data style (a table cell with `office:value`, a date field).
- **No `Editor` trait** (`core/src/observer.rs` says why): its error type cannot be chosen well
  while each `App` speaks its own crate's `Result`. The suite verbs dispatch on
  `grind_core::kind` in one `match` instead.
- **No shared GTK crate.** The two GTK shells share an observer bridge, the `--render-to`
  harness and the window-close latch; three small copies are not yet a seam. What the shells
  do share moves into the document type's crate instead (`doc/simplicity.md` rule 2).

## The two `App`s

`grind_sheet::App` and `grind_text::App`, each with its own state lock, `Action` enum and
inverse, undo stack and parity document.

| Option | Verdict |
|---|---|
| One generic `App<D: Document>` | **No.** A grid's viewport and a text flow's share nothing but the word; every caller would carry a type parameter to buy an abstraction with one method |
| One `App` enum dispatching on kind | **No.** Every method becomes a match with an unreachable arm, and the parity ratchet can no longer say which application is missing what |
| Two `App` types | **Yes** |

What they share is `grind_core::Observer`, the one part of the shell contract with nothing
document-shaped in it. `--session` is the spreadsheet's: `grind_text::App` has no session, so
undo does not reach across CLI invocations for text (`doc/cli-parity-text.md` names it).

## Addressing text — `loc.rs`

A1 notation is normative and universally understood; text has no such gift, so its addresses
are invented, written down and kept in one module. `grind_text::loc` is held to `a1.rs`'s rules:
the only 0↔1 conversion, parsed in one place, never re-derived by a shell.

| Spelling | Means |
|---|---|
| `p12` | the 12th block — paragraphs, headings, list items and table cells' blocks all count |
| `p12:p20` | a range of blocks |
| `p12+40` | a caret: 40 characters into block 12 |
| `#intro` | the block holding the `text:bookmark` named `intro`, which round-trips because ODF has the construct |
| `§2.1.3` (or `s2.1.3`) | an outline path, derived from heading levels |

`p12` is what a machine uses and is invalidated by every insertion above it; `#intro` and
`§2.1.3` are what a person or a script uses and survive edits elsewhere, so
`grind text set report.fodt §3.2 "…"` still works next week. An offset is its own axis, so
`#intro+5` and `§3.2+0` are carets too.

## The text model

`grind_text::Document` is an ordered `Vec<Block>` plus the document-level tables (bookmarks,
declared style names, the page it is set on, the source bytes R6 splices from).
`doc/text-core.md` lists every element it models. Three decisions inside it:

- **A `String` per run, not a rope.** A paragraph is the natural edit unit and rarely more than
  a few kilobytes, and a block *is* an element, which makes splicing per paragraph natural.
- **Blocks carry stable ids beside their index** (`BlockId`). An index is what a user types; an
  id is what an undo entry, the splice registry and a notification carry, so an insertion above
  does not retarget them.
- **A table is a run of blocks**, each carrying the coordinate of the cell it is in, and a cell
  holds blocks because `table-table-cell-content` is the body's own production. Nothing of the
  spreadsheet's reader is shared: an ODT cell holds blocks, not values. Whether Writer's
  `table:formula` is OpenFormula is unverified, so a cell holds text and never a formula.

## The CLI

```
grind <app> <verb> [args]      grind sheet set book.ods A1 1
                               grind text  outline report.fodt
grind <verb> [args]            grind info anything.odt
```

Global flags are global: `--format json`, `--session`, `--dry-run`. The suite-level verbs read
the kind from the bytes: `info`, `lint`, `project`, `diff`, `convert`, and the ones with no
document (`build`, `test`, `init`, `serve`). `grind <app> --help` lists each application's
verbs, and every verb in `grind text` has a spreadsheet analogue or a reason in
`doc/cli-parity-text.md`.

`grind text` has **no `recalc`**: a text document has no cached-value contract, so the
spreadsheet's staleness apparatus has no analogue and does not grow one by imitation.

A word processor completely drivable from a shell falls straight out of R9 rather than being a
feature anybody had to want, and it is the half of the project the README's "point a script —
or an agent — at it" cares about most. `examples/sample-text.sh` builds a document from every
text feature through the CLI alone, as `sample-sheet.sh` does for the spreadsheet.

## The shells

### Every document type reaches every shell

This is R10, the rule the Shared Core / Native Shell architecture exists to make cheap: a suite
whose word processor only has a window has decided the window is the real product. Full does
not mean identical. `doc/feature-matrix.md` is the matrix, and its gaps are named there.

### One binary per document type — for GTK only

`grind-sheet-gtk` and `grind-text-gtk` are separate binaries with separate app IDs
(`io.github.fwilhe2.Sheet`, `io.github.fwilhe2.Text`), `.desktop` files, AppStream components and
icons:

- A `.desktop` file's `MimeType=` is how a desktop associates an application with a file type,
  and one entry claiming both spreadsheets and text documents makes *Open With* useless.
- GNOME's HIG and Software assume an application is one thing; two entries in Software is the
  correct outcome.
- LibreOffice's single process amortises a startup cost this project does not have.

**Every other shell is one binary**, and the reason is the desktop file rather than taste.
`grind-tui book.ods` and `grind-tui notes.fodt` are the same invocation with different bytes,
so the binary sniffs `grind_core::kind`. The browser receives bytes with no path and no mime
association; Windows associates files by ProgID and the Mac by document type, neither per
application. Splitting any of them would be consistency for its own sake.

### Cross-app handoff

Opening a spreadsheet in `grind-text-gtk` does not fail obscurely: `grind_core::kind` answers
before any parsing, and the window says *This is a spreadsheet.* with a button that launches
`grind-sheet-gtk`, or says plainly that it is not installed. The same in reverse. The CLI's
equivalent is a one-line error naming the right subcommand.

### The terminal

The terminal edits text on the same vi-modal design as its grid, so the two types feel like one
program: `hjkl`, `g`/`G`, Visual mode over a range, `i`/`a`/`o` into Insert, `]`/`[` and `}`/`{`
to the next misspelt word and grammar finding; `:w`, `:q`, `:outline`, `:style`, and a bare
address — `p12`, `#intro`, `§2.1.3` — to jump. It was built before the GTK text shell on purpose. It is the cheapest complete editor
there is — no font metrics, no shaping, no zoom, no page — so what is left is exactly the part
that has to be right: the editing model, the addressing, undo granularity, the observer round
trip. And it is what forced line layout to take its metrics as a parameter
(`doc/text-layout.md`): `ui_tui/src/text/` answers "how wide is this, in terminal columns", and
breaking and every line motion come from the core.

## Packaging

| Package | Contains |
|---|---|
| `grind-cli` | `/usr/bin/grind`; no runtime dependencies beyond libc |
| `grind-tui` | `/usr/bin/grind-tui`, both document types; no runtime dependencies beyond libc |
| `grind-sheet-gtk` | the binary, `.desktop`, metainfo and icon; `depends = "$auto"` |
| `grind-text-gtk` | the same, under `io.github.fwilhe2.Text` |
| `grind` | the meta-package: depends on the four and contains nothing |

`.deb` through `cargo deb`, `.rpm` through `cargo generate-rpm`. The meta-package is `suite/`, a
stub crate with no code, so that one mechanism builds all five; `generate-rpm` has no `$auto`,
so its `requires` are spelled out. `cargo deb` takes the architecture from the build target and
has no `all`, so there is a `.deb` per architecture (`amd64`, `arm64`), each beside the four it
depends on, while the `.rpm` is `noarch` and built once. `cli/tests/packaging.rs` holds `artifacts.yml` to all of it.

The container (`Containerfile.distroless-cli`) carries `grind`; the GUI applications are not
containerised. Flatpak is not built; if it is, Flathub prefers one app ID per manifest, so it
would be two manifests, for the reason the GTK binaries are two.

### Mime types

The detail that decides whether double-clicking a file works:

```
sheet: application/vnd.oasis.opendocument.spreadsheet
       application/vnd.oasis.opendocument.spreadsheet-flat-xml
text:  application/vnd.oasis.opendocument.text
       application/vnd.oasis.opendocument.text-flat-xml
```

The flat types are the ones distributions most often get wrong, and `.fods`/`.fodt` are the
forms this project cares most about. `shared-mime-info` knows all four; the `.desktop`
`MimeType=` lines name them, and `StartupWMClass` matches what GTK sets. The `-flat-xml`
spellings belong only there: inside a document the media type is the plain one
(`grind_core::kind`, `doc/ods-format.md` §10).

Sheet also lists the workbooks it imports (`doc/xlsx-import.md`):
`application/vnd.openxmlformats-officedocument.spreadsheetml.sheet` and
`application/vnd.ms-excel.sheet.macroEnabled.12`, the two types GIO reports for `.xlsx` and
`.xlsm` (measured with `gio info`; `xdg-mime query filetype` falls back to `file` and calls both
the first), with `Keywords=Excel;xlsx` and an AppStream `<provides>` block. **CSV and TSV**
(`text/csv`, `text/tab-separated-values`) are listed the same way, because Sheet opens one as a
new, unsaved spreadsheet (`grind_sheet::csv::open`). The word processor lists none of these and
hands such a file to Sheet.

**Listing is the whole of it.** No package writes a `mimeapps.list`, runs `xdg-mime default` or
otherwise names a default: which program a double-click opens is the user's or the
distribution's to say. Measured with GIO against scratch `XDG_DATA_DIRS` holding Sheet's entry
and LibreOffice's: where a `mimeapps.list` names a default — the user's after any *always open
with*, or a distribution's — that default keeps the double-click and Sheet appears under
*Recommended*. Where nothing names one, the freedesktop fallback is the first entry in
`mimeinfo.cache`, which `update-desktop-database` writes in alphabetical order of desktop-file
ID, so `io.github.fwilhe2.Sheet` is picked over `libreoffice-calc`, and for `.csv` over any
editor whose ID sorts later (`org.gnome.TextEditor`). No desktop-entry field lowers an
application's rank for GIO (KDE's `InitialPreference` is per entry, not per type, and GIO
ignores it), so changing that would mean writing a default after all or not listing the type.
Stock Debian ships no default list for either.

## Verification

The loops apply per document type where they have an analogue (`CLAUDE.md`'s loop table):

| Loop | Text |
|---|---|
| **A**, read tolerance | Writer's corpus under `sw/qa`, nothing special-cased |
| **B**, formula conformance | **No analogue.** Text has no functions and no normative per-function corpus, which is why its scope line is invented rather than derived |
| **C**, round trip through LibreOffice | Both directions, both forms; one documented loosening, style names, because this writer declares no styles (`doc/odt-format.md` §5b) |
| **E**, generated differential | No analogue |
| **F**, projection round trip | `text/tests/loop_f.rs` |
| R7's vendored documents | `text/tests/libreoffice.rs`: hand-written sparse `.fodt` on the thin side, LibreOffice's own output on the thick side, in both forms |

## The scope line for text

`doc/small-group.md` was extracted from a normative document; text has nothing to extract, so
`doc/text-core.md` is a judgement made element by element, each with its schema line, and made
mechanical: `grind_text::implemented()` is checked against it by `text/tests/scope.rs`. That
test is the anti-bloat rule for this half of the suite, without which the word processor would
become Writer one reasonable-sounding element at a time. What is never built is in
`doc/not-doing.md` §1, written before the first caret appeared because the first person to see
one is the first to ask for mail merge.

## The differentiators

The spreadsheet has formulas in plain English and *find everything that is calculated*. The
word processor's:

1. **A word processor whose files live in git.** R6 on `text:p`: editing one paragraph of a
   500-page `.fodt` changes one line of XML, and opening a document to read it is not a commit.
2. **Find everything that is hand-formatted.** `grind text formatting` lists every block whose
   own formatting overrides its named style, with its address: the analogue of
   `sheet calculations`, answering "why is this paragraph different?"
3. **The outline is the document.** `grind text outline` and an outline pane in every window;
   `grind text move §3.2 §1` moves a whole subtree by an address that survives edits elsewhere.

## What does not change

The clean-room rule and its spec-first discipline, R1–R7, the loops and their documented
loosenings, `ponytail:` as a ledger, REUSE and AGPL-3.0-or-later, and the Shared Core / Native
Shell rules in `CLAUDE.md`. None of those is a spreadsheet rule, which is why a second document
type was a list of steps rather than a second project.

## Open

- **The name.** A binary-name collision search for `grind` in Debian and Fedora
  (`apt-file search bin/grind`, `dnf provides '*/bin/grind'`) and a trademark check for GRIND in
  Nice classes 9 and 42 have not been run; a file conflict in `/usr/bin` is a packaging bug, not
  a preference. If either comes back badly, `desk` and `suite` were the runners-up.
- **`table:formula`**: whether Writer writes OpenFormula or a vendor dialect, by observation,
  cited in `doc/odt-format.md` before any code.
- **Data styles in text**: that `number:*-style` is reachable from an ODT (a table cell with
  `office:value`, a date field) against the schema, before `numfmt` moves.
