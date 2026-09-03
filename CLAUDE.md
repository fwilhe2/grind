# CLAUDE.md

Guidance for Claude Code in this repo. This file is kept short on purpose — the "why" behind
any given piece of code lives in that code's own doc comments and in `doc/*.md`; read those
when touching that area rather than expecting the full rationale here.

<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

## What this is

An ODF-native office **suite**: one shared core, one crate per document type, native shells,
a feature list that ends. Not a port of LibreOffice, contains none of its code. `README.md`
has the pitch, `CONTRIBUTING.md` the contributor rules, `doc/plan.md` phases 0–9,
`doc/suite.md` phase 10 — the split into `grind-core` + `grind-sheet` + `grind-text`, the
`grind <app> <verb>` CLI, and R8/R9/R10.

The spreadsheet (`grind sheet`) is complete through phase 9. The word processor
(`grind text`) is done through S10: it reads, writes, edits by block *and by caret*, lays text
out, and **has all three shells** — `grind-tui` opens both document types in one binary,
`grind-text-gtk` is its window, and `grind-web`'s one bundle holds both panes. The two GUI
shells are deliberately *minimal*; `doc/text-shell.md` lists what they do and do not do.
`examples/sample-text.sh` builds a document out of every feature it has, through the CLI only.
Loops A and C are both green. **Line layout lives in `grind-core`** (`doc/text-layout.md`,
decided on Path C), so `j`/`k`/Home/End mean one thing in every shell and the CLI can answer
them; a shell supplies only font metrics. What it does **not** have: a session (so no `undo`
across invocations), tables, footnotes, fields, style definitions, RTL layout and pages. Every
shell *does* now have a selection and a formatting toolbar over it; the clipboard is the uneven
one — `grind-tui` has a register and `grind-web` the browser's own, and `grind-text-gtk` has
neither.

`doc/plan.md`'s "The requirements" (R1–R7) is normative. In short: independence and
ODF-native semantics (R1); everything written validates against the RELAX NG schema (R2,
`jing -i`); minimal boilerplate (R3); `calcext:` opt-in, outranked by R2 (R4); LibreOffice's
files read, unknown properties inert (R5); writing changes as little XML as possible (R6,
phase 8, done); eight named documents that must load, vendored in `sheet/tests/data/kb/` (R7).
Strictness on the way out, tolerance on the way in — several R7 files are schema-invalid and
must still load.

## The two constraints that govern everything

**1. Clean room.** LibreOffice source may be *read* and *cited by `file:line`*, never
copied. Every fact learned from reading it goes into a cited spec document
(`doc/ods-format.md`) *before* it reaches code. LibreOffice is a conformance oracle and test
corpus, never a source — this is what keeps the project freely licensable.

**2. ODF semantics are the product.** No Excel-oriented libraries with a translation layer —
the divergence (error model, coercion, empty-cell handling, null-date, reference operators,
collation) is semantic, not syntactic, and a syntax translator leaks it. Normative sources:

| Source | Role |
|---|---|
| `doc/OpenDocument-v1.4-schema.rng` | ODF 1.4 Part 3 — content schema |
| `doc/OpenDocument-v1.4-os-part4-formula.html` | ODF 1.4 Part 4 — OpenFormula semantics |
| `doc/small-group.md` | The 110-function scope line, extracted from Part 4 §2.3.2 |
| `doc/suite.md` | Phase 10 — the suite plan; normative for that phase, incl. R8/R9/R10 |
| `doc/odt-format.md` | Clean-room notes for text documents. **§5 is `UNVERIFIED` and may not be implemented** |
| `doc/text-core.md` | The text scope line — *invented*, not extracted, and checked by `text/tests/scope.rs` |
| `doc/text-layout.md` | **Where layout lives — decided, Path C.** Normative for it, including the five answers at its end (RTL out, `Layout` in `grind-core`, the CLI's unit). Outranks `doc/suite.md`'s fork section, which is the record of an argument rather than the answer |
| `doc/ods-format.md` | Clean-room notes on undocumented LibreOffice behaviour |
| `doc/cli-parity-sheet.md`, `doc/cli-parity-text.md` | Every public `App` method and the CLI command reaching it — one per app (R9) |
| `doc/projection-sheet.md` | **The projection's grammar for the spreadsheet, executable rather than descriptive.** Every node with a one-line example the test really reads, and every field of `Document`/`Sheet` with the node that carries it or a named gap. `sheet/tests/projection_scope.rs` is `doc/dsl.md` §3.7 made mechanical |
| `doc/projection-text.md` | **The projection's grammar for the word processor** — the twin, and the one where §3.7 is kept *literally*: a text document has an element scope line (`doc/text-core.md`), so every element `grind_text::implemented()` returns must have a node, a piece of inline notation, or a named gap. `text/tests/projection_scope.rs` is that check. Images are the one gap, and the section on them is why |
| `doc/sheet-shell.md` | Phase 9's work plan for the **spreadsheet's** GTK shell — normative for that phase, and its "Four surfaces" section is normative for that window's chrome: which of the four surfaces a control may go in, and why only the command palette is allowed to grow |
| `doc/text-shell.md` | S9 + S10 — what the word processor's GTK and browser shells do, what they deliberately do not, and what building them proved about `Metrics` |
| `doc/tui-shell.md` | **The terminal shell — normative for `ui_tui/`.** Its two decisions (vi rather than a menu; markdown is for *typing*, never for *showing*), what both halves do, and its gap list |
| `doc/web-shell.md` | **The browser shell — normative for `ui_web/`.** Its one design decision (a page, not a window: one verb bar, one tool row, Ctrl+K for the rest), what both panes do, and its gap list — which used to live in the two shell docs above and outgrew them |
| `doc/flat-first.md` | **In doubt, write the form that diffs.** Normative for every default choice between the package and flat forms — `Form::from_path`, save dialogs, new documents |
| `doc/view-modes.md` | **What a document *means*, drawn — normative for `sheet/graph.rs`, `sheet/view.rs` and the overlays in all four shells.** Inline names and derived cell roles, neither of which is ever *written*: a stored classification goes stale and a derived one cannot |
| `doc/dsl.md` | **The projection — a document as plain text, and a generator that writes one.** Normative for `core/src/projection/`, `sheet/src/projection/`, `text/src/projection/` and `build/`. Two layers, and fusing them is the mistake it exists to prevent: layer 0 (`.grind`, KDL, bijective, round-trips — D0–D5, both document types) and layer 1 (a generator, one direction, `grind build` — D7 built, D8's `grind test` not) |
| `doc/generator-spec.md` | **The generator's language — normative for `build/`.** §8 is the editor half: `grind definitions` writes a Rhai `.d.rhai` file from the engine itself and `--snippets` writes the same vocabulary as VS Code snippets, so completion and hover cover it with or without a language server, and `build/src/hint.rs` is why every function has documentation to show — registering one *takes* the comment, so an undocumented function cannot be added by forgetting. The Rhai dialect (which features are taken, what is removed and *how*, every limit, what determinism rests on), the whole host API for both document types, how the returned tree becomes a document, and §7's list of what a script cannot say and what to do instead. `doc/dsl.md` §4 is the argument; this is the reference, and `build/tests/spec.rs` holds it to the code in both directions |
| `doc/projection-guide.md` | **The projection's guide (D12)** — writing a `.grind` by hand, in the order the problems arrive, built on `examples/quote.grind`. Task-shaped where `doc/projection-sheet.md` is a vocabulary; §1 is the answer to *why would anybody want this*, and §10 the honest list of what converting *to* a projection drops |
| `doc/generator-guide.md` | **The generator's guide (D14)** — from `examples/first.rhai` to `examples/timesheet.rhai`, four sheets that have to agree with each other. `cli/tests/editor.rs` builds every script it names |
| `doc/editor-setup.md` | **VS Code for `.grind` and `.rhai`** — what each extension actually does, measured as installed rather than as described, and why the vocabulary ships as *snippets* as well as a `.d.rhai`: the published Rhai extension runs no language server. `.vscode/` is the shipped configuration and `cli/tests/editor.rs` holds both snippet files to the vocabularies they describe |
| `doc/not-doing.md` | The feature line as a product document |

Format-neutral plumbing (quick-xml, zip, petgraph, chrono) can be lazy; semantics never are.

## Commands

```sh
cargo test                       # everything
cargo test -p grind-sheet --test read_values   # one test file
cargo test -- repeated_columns   # one test by name substring
cargo clippy --workspace --all-targets   # CI gates on it, warnings as errors
cargo fmt --all                          # rustfmt.toml is its config; CI gates on --check
cargo doc --no-deps                      # CI gates on it with RUSTDOCFLAGS=-D warnings
reuse lint                               # must stay compliant; CI gates on this too
```

`grind` is the CLI — `grind <app> <verb>`, plus a few suite-level verbs that read the
document's kind out of the file (`info`, `convert`, `lint`) and one that has no document to read
(`build`, which runs a generator script — `doc/dsl.md` layer 1). Every core capability is reachable from
it, enforced by
`cli/tests/parity.rs`:

```sh
cargo run -p grind-cli -- sheet new book.ods
cargo run -p grind-cli -- sheet set book.ods A1 1
cargo run -p grind-cli -- sheet set book.ods A2 '=[.A1]*2'   # ODF syntax, verbatim
cargo run -p grind-cli -- sheet recalc book.ods
cargo run -p grind-cli -- sheet view book.ods A1:A2
cargo run -p grind-cli -- --format json info book.ods    # suite level: reads the kind
cargo run -p grind-cli -- build examples/budget.rhai -o book.fods   # a script returns a document
cargo run -p grind-cli -- build examples/timesheet.rhai -o month.fods  # four sheets that agree
cargo run -p grind-cli -- lint examples/quote.grind       # a hand-written projection
```

The two GTK shells need `libgtk-4-dev` + `libadwaita-1-dev`, and are **not** in
`cargo build --workspace`'s path — built and run on their own:

```sh
cargo run -p grind-sheet-gtk -- book.ods                  # .ods or .fods; no file = empty document
cargo run -p grind-sheet-gtk -- book.fods --render-to /tmp/grid.png   # one frame, then exit
cargo test -p grind-sheet-gtk                             # geom.rs, no display needed

cargo run -p grind-text-gtk -- report.fodt                # the word processor's window
cargo run -p grind-text-gtk -- report.fodt --render-to /tmp/page.png
cargo test -p grind-text-gtk   # geom + keymap always; the widget tests skip with no display
```

`--render-to` is how a custom-drawn widget gets an assertable output (a refactor is proved
one when the PNG comes back byte-identical). Not a user feature.

`grind-tui` is the terminal shell and needs no system packages. **One binary, both document
types**, chosen by `grind_core::kind` from the file's bytes:

```sh
cargo run -p grind-tui -- book.fods       # the spreadsheet
cargo run -p grind-tui -- report.fodt     # the word processor
cargo run -p grind-tui -- --text          # a new document, empty
cargo test -p grind-tui                   # both keymaps, `Cells`, the markdown notation, and rendering via TestBackend
```

```sh
cargo build && GRIND=target/debug/grind examples/sample-sheet.sh /tmp/demo
cargo run -p grind-sheet-gtk -- /tmp/demo/sample.fods
```

`examples/sample-sheet.sh` and `examples/sample-text.sh` build a document out of **every feature
this build supports**, through the CLI only, and `cli/tests/cli.rs` runs both — a feature
without a line there is invisible. Add one when adding a capability.

The corpus tests need a LibreOffice checkout and skip with a notice without one:

```sh
GRIND_LO_CORPUS=/path/to/libreoffice/core cargo test
```

## The loops

Correctness is checked against LibreOffice, not our own opinion. `soffice` must be on `PATH`
for loops C and E; loops A and B want a LibreOffice source checkout (`GRIND_LO_CORPUS`, the
**checkout root** — Calc's corpus is at `sc/qa/unit/data` and Writer's at `sw/qa`, and one
clone serves both). CI's `corpus` job gets that with a blobless sparse clone of just those two
directories rather than the whole tree. The oracle is pinned to a container
image by digest (`ci/libreoffice-image`, "Pinning LibreOffice" in `doc/differential-fuzz.md`)
— loop E's `FLOOR` is a fact about one `soffice` build. `scripts/soffice-docker/soffice` is a
shim that runs that image, so putting it first on `PATH` pins the oracle without any test
knowing about Docker; `scripts/soffice-tests.sh` does that and runs loops C and E locally
against exactly what CI uses.

| Loop | Asserts | Where |
|---|---|---|
| **A** — read tolerance | every `.ods`/`.fods` loads without error | `sheet/tests/corpus_read.rs` |
| **A** — read tolerance, text | every `.odt`/`.fodt` in `sw/qa` loads without error | `text/tests/corpus_read.rs` |
| **B** — formula conformance | parse / display round-trip / evaluate-matches-cached-value | `sheet/tests/corpus_parse.rs`, `corpus_eval.rs` |
| **C** — round-trip differential | write → `soffice --convert-to` → read back → identical, and reverse | `sheet/tests/roundtrip.rs` |
| **C** — round-trip differential, text | the same, both directions, over `sw/qa` | `text/tests/roundtrip.rs` |
| **E** — generated differential | formulas generated from the catalog's signatures, evaluated by us and by LO | `sheet/tests/loop_e.rs`, `doc/differential-fuzz.md` |
| **F** — projection differential | project → read back → the two models are identical, both directions | `sheet/tests/loop_f.rs`, `text/tests/loop_f.rs`, `doc/dsl.md` §8 |

`sheet/tests/kb.rs` is the fourth check and never skips: R7's vendored documents. It also
validates the writer against the schema (`jing -i`) and measures R3/R6.

`text/tests/libreoffice.rs` is its counterpart for text and never skips either: documents
LibreOffice Writer actually wrote, vendored in `text/tests/data/` in **both forms** of the same
file. Everything there is globbed rather than listed, so adding a Writer document is dropping a
file in — it must load, an untouched flat save must return its bytes exactly, and the package
and flat readers must agree. It is also where the price of the package boundary is written
down: saving a `.odt` regenerates it, so `styles.xml`, `settings.xml`, `meta.xml` and the
thumbnail are lost on a plain open-and-save (`text/src/odf/source.rs` states the rule; the test
states the cost).

Current scoreboard (see each test's own comments for what each column means and why):
loop A (sheet) 359 read / 3 password-protected / 0 failed; loop A (text) 1755 read /
4 password-protected / 4 independently confirmed non-documents / 0 failed; loop B parse 75845/77061 (1216 named
syntactic exclusions); loop B display 75845 round-trip, 271 named ambiguity; loop B evaluate
13327/52213 matching LO (`FLOOR` in the test is the ratchet — raise it, never lower it; run
`GRIND_LOOP_B_DUMP=LOG cargo test -p grind-sheet --test corpus_eval -- --nocapture` for the scoreboard).
Loop C is green both directions for the sheet and for text (16 documents out, 20 corpus
documents / 5095 blocks back, 0 differences) and gates CI in all four. The text loop compares
structure, text, bookmarks and — **out only** — the character formatting of every character;
formatting is excluded on the way back because LibreOffice hoists a character style covering a
whole paragraph onto the paragraph, which is measured rather than assumed
(`a_character_style_over_a_whole_paragraph_is_hoisted_into_it`, `doc/odt-format.md` §5b).
`ci/libreoffice-image`
was a Calc-only LibreOffice when the text loop was written, so `oracle_ready` in
`text/tests/roundtrip.rs` probes whether the `soffice` on `PATH` can convert a text document at
all rather than assuming it; the image has since been rebuilt with Writer, and that half started
gating with no file changed. The probe stays for a developer whose own `soffice` has no Writer.
Loop F is green for **both document types**: R7's fourteen vendored spreadsheets and
`text/tests/data/`'s vendored Writer documents, neither of which skips, and — as D3 —
**359/359 of loop A's spreadsheet corpus and 1755/1755 of its text corpus, with nothing
differing** (`FLOOR = 359` and `FLOOR = 1755`, and they only go up). It costs no corpus of its
own, which is the point of building it on loop A's. It has exactly one named exclusion per
application — charts for the sheet, images for text — each with a test that fails the day it is
projected. The text half compares runs after the normalisation `text/src/odf/write.rs` already
performs (a literal tab in a run *is* a `text:tab`), and proves that equivalence directly rather
than assuming it.
Loop E is at 913/1000 on the pinned
image at its default seed (same binary locally and in CI, so the figure should match), with
the untriaged disagreements classified in `doc/differential-fuzz.md`. All four
loops now run in CI (`build`, `roundtrip`, `loop_e`, `corpus` jobs) rather than only where a
developer's machine happens to have a LibreOffice checkout.

Each loop has exactly one documented loosening (loop A accepts `Error::Encrypted`; loop C
compares doubles at 15 significant digits, all LibreOffice writes). A third exception is a
bug in the code, not the loop.

## Architecture

Shared Core / Native Shell (see
[fwilhe2/editor](https://github.com/fwilhe2/editor)'s `doc/shared-core-native-shell.md`).
All state and logic in `core/`; every shell is a renderer and event forwarder owning
nothing. `cli/` exists so capabilities cannot hide in a UI; `ui_sheet_gtk/` is held to the same
rule by `cli/tests/parity.rs`.

Rules that are cheap now, expensive to break later:

1. **Reads go through `App::get_viewport`.** No getter hands out the whole document.
2. **Undo/redo lives in the core.** `Document::apply` returns the action's inverse; that is
   the whole mechanism. Shells never implement history.
3. **The core pushes, shells never poll.** `App::mutate` drops the write lock *before*
   notifying observers, because an observer calls straight back in to re-read.
4. **Whatever any GUI can do, the CLI can do.** A UI-only feature is a bug.
5. **No filesystem assumptions.** Every `*_file` has a `*_bytes` twin.
6. **Every feature must survive a LibreOffice round-trip** (loop C).

### The crates

Phase 10 (`doc/suite.md`) split the core in two, so that a second document type is a peer
rather than a guest:

| Crate | Directory | Holds |
|---|---|---|
| `grind-core` | `core/` | **\[GENERIC\]** — the container (`odf/package`), the namespace vocabulary (`odf/names`), the tolerant reading architecture (`odf/context`), `Form`, the styling primitives every family of style is built from, the locale, the build stamp, `Observer`, `kind` (which document type some bytes are), and `projection/` — the KDL container, the kind header, the token and span maps of `doc/dsl.md`'s third physical form, and `projection/source.rs`, which is R6 for it |
| `grind-sheet` | `sheet/` | The spreadsheet: model, column store, ODS reader/writer, R6 splicing, number formats, cell styles, the OpenFormula engine, `App`, and `projection/` — the same document as plain text (`doc/dsl.md`) |
| `grind-text` | `text/` | The word processor (phase 10): the block model, `loc.rs` addressing and carets, `style.rs`'s `CharStyle` (direct character formatting — bold, italic, family, size, colour), `markdown.rs`'s notation and `App::type_markdown` (`**bold**` read as it is typed, in the core so four shells cannot read `**` four ways), the ODT reader and writer, `App` with block *and* caret edits, `projection/` — the same document as plain text, with `inline.rs`'s bidirectional notation (`doc/dsl.md` §3.6) — and R6 splicing — a `.fodt` lives in git the way a `.fods` does, and one keystroke is one line of diff. Line layout is `grind_core::layout`'s and reaches a shell through `App::layout_block`/`caret_line`/`caret_line_bounds` (`doc/text-layout.md`, Path C) |
| `grind-build` | `build/` | **The generator** (`doc/dsl.md` layer 1, D7): a Rhai script that *returns* a document, and the sandbox it runs in. `sheet.rs` and `text.rs` are the two host vocabularies — the projection's own nouns — `engine.rs` is every restriction §2 promises, in one screen, and `data.rs` is the one exception to them: `json(…)`, which reads **data and never code** from one directory a person named, with `..`, absolute paths and symlinks out all refused. **Nothing that opens a document may depend on this crate** (R11), which `build/tests/manifest.rs` reads the manifests to enforce |
| `grind-cli` | `cli/` | The `grind` binary |
| `grind-sheet-gtk` | `ui_sheet_gtk/` | The spreadsheet's GTK shell |
| `grind-text-gtk` | `ui_text_gtk/` | The word processor's GTK shell (S9, minimal). Its own binary and app ID because a `.desktop` file's `MimeType=` is per application. `geom.rs` stacks blocks, `keymap.rs` names the motions, `metrics.rs` is Pango behind `Metrics`, `view.rs` is the widget |
| `grind-web` | `ui_web/` | The wasm shell, **both document types in one bundle** — `sheet/` and `text/` under it, panes picked by `grind_core::kind`. `text/mod.rs`'s `Face` is its layout contribution: how wide is this text, in CSS pixels, measured on a canvas. `command.rs` is every verb either pane has, as *data*, reached from the Ctrl+K palette, a key and a button alike (`doc/web-shell.md`) |
| `grind-tui` | `ui_tui/` | The terminal shell, **both document types in one binary** — `sheet/` and `text/` under it, picked by `grind_core::kind` from the file's bytes. `text/mod.rs`'s `Cells` is its whole layout contribution: how wide is this text, in terminal columns. Its formatting toolbar is `grind_text::markdown` — typed, never *drawn* as markers (`doc/tui-shell.md`) |

**R8: no document type's vocabulary reaches `grind-core`.** Checked by `core/tests/generic.rs`,
which asserts the manifest names no document-type crate, that no source dispatches on
`Ns::Table` or `Ns::Text`, and — since the projection — that no source spells a projection node
name either (`doc/dsl.md` §3.2: the projection splits exactly where `odf/` splits). `grind-sheet` re-exports the generic modules (`grind_sheet::odf`
hands on `context`, `names`, `package`, `Form`), so the reader and writer reach them by the
paths they always used — the crate boundary is the seam, those aliases are ergonomics.

Two things stayed in `grind-sheet` on purpose and move when a second caller pulls on them:
`numfmt/` (it depends on `formula::date` and `model::CellValue`, so the generic part is the
format *model*, not applying one to a cell) and `odf/source.rs` (R6's splice registry, whose
shape is spreadsheet-addressed until a text writer needs one). There is deliberately **no
`Editor` trait** yet; `core/src/observer.rs` records the open question and what has to exist
before it can be answered.

### Where things live

- **`sheet/src/a1.rs`** — addressing. The *only* 0↔1 conversion in the workspace; a shell
  never does its own index arithmetic. Parses by wrapping in `[…]` and calling `lex::lex`.
- **`sheet/src/model.rs`** — `Sheet::used_rows`/`used_cols` are the extent of **everything a
  cell can carry**, not just of the values: a formula, a date/time spelling, a number format or a
  cell style all count. `odf::write` emits exactly that rectangle (`carries` is the same five
  questions on the way out), so a narrower answer is silent data loss — regenerating
  `kb/fizzbuzz.fods`, eighteen formulas with no cached values, used to write a sheet with no rows
  in it. A `TODO:` in `odf/read.rs` holds the remaining half: a *styled empty* cell is still
  dropped on read, and the measurement that says why is with it.
- **`sheet/src/grid.rs`** — the column store: a run-length sequence of typed blocks
  (LibreOffice's `mdds` shape). Invariants restored by `normalize()`, asserted by `check()`.
- **`sheet/src/numfmt/`** — number formats (§5.2). Display only, never touches the value. No
  format-code strings (Excel's spelling, not ODF's) — a format is an ordered sequence of
  pieces. `preset`/`is_preset`/`preset_params` are the whole "set/read a format" vocabulary
  and live here so no shell invents a second one.
- **`core/src/locale.rs`** — decimal point and grouping separator only (two characters).
  Everything else (month names, CLDR tables) is a deliberate, named gap.
- **`core/src/kind.rs`** — which document type some bytes are, decided *before* parsing,
  because §8's reader is tolerant by construction and would hand back an empty document
  rather than an error. Sniffed from content, never from the file name.
- **`core/src/style.rs`** / **`sheet/src/style.rs`** / **`text/src/style.rs`** — the split: the
  `fo:` primitives, ODF lengths, three-part borders, `TextStyle` (the four properties that
  change how *wide* text is) and `PALETTE` (the clrs.cc colours, a default a shell offers and
  never a limit) are generic; `CellStyle` — which of those pieces a *cell* carries — is the
  spreadsheet's, and `CharStyle` — which a *run of text* carries — is the word processor's. ODF
  values kept verbatim in all three, with one named exception (`fo:font-family`'s XSL-FO
  quoting, decoded on read and re-encoded on write). `CharStyle` is **direct** formatting only:
  an `office:automatic-styles` entry is resolved onto the run and its generated name forgotten,
  an `office:styles` name is kept as a name and never resolved — `doc/text-core.md`'s Styles
  section is that line and why it is there.
- **`sheet/src/odf/`** — the reader. Tolerance is structural: an unrecognised element gets
  `Ignore` for its whole subtree, so unknown content is inert by construction rather than by
  special-casing. Dispatch is on `(namespace-uri, local-name)`, never prefixes.
  `sheet/src/odf/source.rs` is R6's diffable-write machinery: it retains the original bytes
  and splices edits into them instead of regenerating, falling back to a full regenerate
  whenever a change can't be spliced (format/style edits, a package, a repeated row, …).
  `sheet/src/odf/write.rs` is the regenerating writer, minimal by intent, pooling formats and
  cell styles so equal ones share one automatic style.
- **`text/src/model.rs`** — the text model. A body is a **flat sequence**, not a tree
  (rng:16938): a heading does not contain what follows it, outline structure is implied by
  `text:outline-level` alone, and lists are flattened into the sequence with a depth. Blocks
  carry stable `BlockId`s because an index is invalidated by any insertion above it.
  `split_runs`/`coalesce` are the run surgery every caret edit is built on — the `grid.rs`
  `normalize()` of this crate.
- **`text/src/loc.rs`** — addressing, the `a1.rs` of that crate and its only 0↔1 conversion.
  A `Loc` is a `Target` (`p12`, `#bookmark`, `§2.1.3`) plus an optional character `offset`, and
  the offset is a separate axis *on purpose*: `#intro+5` and `§2.1+0` are addresses, not just
  `p12+40`. The named targets survive edits elsewhere in the document, which is what makes a
  caret scriptable; `p12` does not.
- **`sheet/src/formula/`** — the OpenFormula engine, built value model → lexer/parser/
  serializer → eval → functions, all cited to Part 4 by section. `value.rs` is the single
  point of failure for the value model, error set and §6.3 conversions. `eval.rs` recurses
  over the dependency graph rather than sorting one. `display.rs` is the formula bar's A1
  syntax layered on top of the same lexer/parser, not a second grammar. All 110 Small Group
  functions are implemented; `funcs::implemented()` is checked against `doc/small-group.md`
  by a test, which is the anti-bloat rule made mechanical.
- **`ui_sheet_gtk/`** — the spreadsheet's GNOME shell (phase 9, `doc/sheet-shell.md` is
  normative here). Owns no data — every paint reads `App::get_viewport` and throws it away.
  Custom `gtk::Widget` drawing in `snapshot()`, not `GtkColumnView`. `geom.rs` holds all
  layout arithmetic and no GTK types, so it's unit-testable with no display. Every colour
  comes from the theme, never a literal, except the reference palette and a colour the
  document itself chose.
- **`core/src/projection/`** + **`sheet/src/projection/`** + **`text/src/projection/`** —
  `doc/dsl.md` layer 0, the third
  physical form: the same document as plain KDL a person can write in any editor. The core half
  is the container and the two maps a code view is made of — the token map (highlighting comes
  from the *writer*, never from a highlighter) and the span map (address ⇄ byte range, both
  ways, plus the line-shaped questions a code view asks) — and `source.rs`, R6 for the third form: the retained text plus the byte range of every
  address in it, so one edit is one line and an untouched save is the bytes that were read. Each
  app half is that app's node vocabulary; the text one adds `inline.rs`, a
  paragraph's runs as one string and back, whose marker table is `markdown.rs`'s so no fifth
  reading of `**` exists anywhere in the suite. A text block anchors *every* address `loc.rs`
  gives it — `p12`, `#intro` and `§2.1.3` are one line and three rows of the span map. Reached as `grind sheet project`, as
  **`Form::Projection`** — the third arm of the form enum, so `grind convert book.fods
  book.grind` writes one and every shell's save dialog offers it — and by `read_bytes`, which
  sniffs the form from the first line rather than from the name. Each app's `projection::save` is
  the one door out — splice if it can, regenerate if it cannot — so no caller can write a
  projection without R6. **Charts are the one named gap for the sheet, images for text.**
  The grammar cannot drift from the model: `doc/projection-sheet.md` lists every node with an
  example the test executes, and every field of `Document`/`Sheet` with its node or a reason
  there is none — read out of `sheet/src/model.rs` at compile time, so a new side table fails
  the build until it has a spelling.
- **`core/src/lint.rs`** + **`sheet/src/lint.rs`** + **`text/src/lint.rs`** — `doc/dsl.md` §4.3,
  D6. The core half is what a *diagnostic* is — a rule id, a severity, an address in the
  application's own spelling, a sentence — and nothing about documents (R8). Each app half is its
  `RULES` array, checked against §4.3's table in both directions by `cli/tests/lint.rs`. A rule
  never writes and never stores: linting leaves a file's bytes exactly as they were, which is
  `doc/view-modes.md`'s promise for the same reason. `Document::styles` on the text side is read
  and never written, and exists only so `undeclared-style` can ask whether a name points at
  anything.
- **`cli/`** — `main.rs` (clap, one arm per subcommand) + `report.rs`. A subcommand is a few
  lines driving `App`; anything longer belongs in the core. `doc/cli-parity-sheet.md` +
  `cli/tests/parity.rs` are the parity ratchet: every public `App` method needs a reaching
  command or a named reason it has none, checked by a test that reads `sheet/src/lib.rs`.

## Conventions

- **Positions are 0-based in the core.** Only a UI is 1-based; the whole workspace converts
  in exactly one place — `sheet/src/a1.rs`.
- **`ponytail:` comments** mark deliberate shortcuts with a known ceiling and upgrade path.
  They're a tracked ledger — don't silently "fix" one without reading the reason.
- **`TODO:` comments** are different: something *reported or suspected wrong* and not yet
  reproduced. Each one says what is already checked, what that check did not cover, and the
  named suspects — so the next person starts where the evidence stops rather than re-proving
  the half that works. `grep -rn 'TODO:' --include='*.rs'` is the list.
- Deferred on purpose, documented where it lives: corrupt-zip recovery (no corpus file needs
  it; belongs with the spec's explicit repair mode).

## REUSE / licensing

**AGPL-3.0-or-later**, REUSE compliance enforced in CI. New files need annotating:

```sh
reuse annotate --copyright "Florian Wilhelm <fwilhelm.wgt+github@gmail.com>" \
               --license AGPL-3.0-or-later path/to/file
```

`Cargo.lock` is declared in `REUSE.toml` instead of annotated. **Never annotate
`doc/OpenDocument-v1.4-*` in place** — they're OASIS specs redistributed verbatim under terms
that forbid modification; they carry `.license` sidecar files instead. Anything *derived*
from them (`doc/small-group.md`) carries the OASIS copyright alongside ours.

**Test documents are data, not code.** The fixtures this project authored are **CC0-1.0** —
`sheet/tests/data/samples/**` and `text/tests/data/**` — declared in `REUSE.toml` rather than
annotated, because a header inside the XML would change the document under test. AGPL on a
fixture makes no sense: the point of one is that it can be copied into a bug report or into
somebody else's ODF test suite. `sheet/tests/data/kb/**` stays MIT — its upstream repository
states MIT, and a vendored copy does not restate somebody else's terms.

## Where the work is

Phases 0–8 are done: harness, column store, undo/redo, reader (loop A green), writer (loop C
green both directions), the OpenFormula engine (all 110 Small Group functions, dates, named
expressions), number formats and cell styling, the CLI (with the parity ratchet), sheet
add/rename/delete, `doc/not-doing.md` (the product stop-line), and R6's diffable writer.

**Phase 9 (the shells) is done through M9**, planned in `doc/sheet-shell.md`. Done: M0–M8 — CI
split, the read-only grid, core prep (`a1`, `formula::display`, `enter`/`preview`/
`clear_range`/`enter_range`), selection/navigation, editing (in-cell editor + formula bar,
undo/redo, spoilage banner), clipboard, formula UX (point mode, autocomplete, signature
hints), styling + the format strip, and column widths/row heights (drag to resize,
double-click to autofit a column or clear a row, both surviving a LibreOffice round-trip).
M10 adds row auto-height (a row without a height of its own is measured from what is in it,
so a wrapped cell and an oversized font are drawn whole) and zoom (Ctrl+wheel,
Ctrl+`+`/`-`/`0`) — one factor in `Grid::geom`, with nothing measured ever stored zoomed.

**The chrome was then reworked, and `doc/sheet-shell.md`'s "Four surfaces" is normative for
it.** The problem it answers: every new feature was becoming another toolbar button. The cause
was not inattention — it was that the mode-switched tool row's three pages were three
*different kinds* of control (a property inspector, view state, and a list of verbs with no
membership rule), so every verb landed on the third page. The rework gives each kind the
surface its kind wants and gives verbs one that is *meant* to grow: a **header bar** fixed at
five slots, one **format bar** whose admission test is "reads and writes a property of the
selection" (so `CellStyle` + `numfmt::Format` bound it), **context menus** on the cells, the
sheet tab, the headers and the charts, and a **Ctrl+K command palette** (`ui_sheet_gtk/src/palette.rs`)
for everything else. The palette is a view over `main.rs`'s own `actions()` table, so a verb
cannot be added without becoming findable — the growth rule is structural, not a habit — and
`main.rs`'s `chrome_tests` walk every `gio::Menu` in the window to check it, with no display
needed. The tab strip's removal also takes away the one piece of this window that resembled a
ribbon at all. `grind_core::search::score` came out of `ui_web/src/command.rs` in the same
change, so the two shells with a palette rank a query the same way.

**M9** brought packaging under `ui_sheet_gtk/data/` (`.desktop`, AppStream metainfo, a scalable
icon — nothing builds or installs them yet, since this is a pure Cargo workspace), a
`gtk::ShortcutsWindow` built from the same accelerator table the window wires up, recent
files via `gtk::RecentManager` (which `gtk::FileDialog`'s own "Recent" section already reads,
so no custom menu was needed), and the a11y floor — `gtk::Accessible::announce` on every
selection move, which is why `ui_sheet_gtk/Cargo.toml`'s `gtk4` feature is now `v4_14`. The flatpak
manifest was the one "stretch" item and was skipped. `.github/workflows/packaging.yml` builds
`.deb` (`cargo deb`) and `.rpm` (`cargo generate-rpm`) packages for **every binary the suite
has** — `grind-cli`, `grind-sheet-gtk`, `grind-text-gtk` and `grind-tui` — reading the
`[package.metadata.deb]`/`[package.metadata.generate-rpm]` blocks in each crate's `Cargo.toml`, as
artifacts on every push — not yet attached to a release. A shell that is not named there is
invisible in exactly the way a feature with no line in `examples/sample-*.sh` is: adding one
means adding its two manifest blocks, its `data/` (`.desktop`, metainfo, icon under its own app
ID) and its two lines in that workflow. The **meta-package** is what S11 still owes.

`ui_web/` is the wasm shell — rule 5's honest test, and it needed no core change: a document
arrives from the file picker as bytes (`App::open_bytes`) and leaves as a download
(`App::save_bytes`), with no path anywhere. Built by `ui_web/build.sh` (needs
`wasm32-unknown-unknown` and a version-matched `wasm-bindgen-cli`), checked without a browser
by `ui_web/smoke.sh` (jsdom) and by `cargo test -p grind-web` — the command table and its
fuzzy match, both keymaps, the viewport and track arithmetic, the chart's geometry, and the
line-cutting that turns formatting, a selection and a caret into `<span>`s.

**`doc/web-shell.md` is normative for it**, and is where its gap list lives. Its one design
decision: a browser tab has no menu bar and inventing one makes a worse desktop application, so
there is one bar of verbs, one tool row per document type, and **Ctrl+K** for everything else —
a palette over `ui_web/src/command.rs`, which is every verb as data. A command id is the one
vocabulary a button, a key and a palette row all speak. The palette is also the go-to box (an
address, a sheet, a defined name, a heading, a bookmark), which is why this shell has no go-to
or outline dialog. `doc/sheet-shell.md`'s "The gaps, written down" section is the up-to-date
list of everything deferred by decision in phase 9 for the *GTK* window.

**Phase 10 (the suite) is done through S10**, planned in `doc/suite.md` — every cell of R10's
shell matrix is now filled, some of them minimally and every gap named. S1–S6 split
`grind-core` out, built the `grind` CLI, wrote the two normative text documents, and gave the
word processor its model, reader, writer, R6 splicing and `App`.

S7 landed the *caret edits* — `insert_text`, `erase`, `split_block`, `join_block`, plus an
offset axis on `Loc` so `#intro+5` is as good as `p12+40` and survives an edit above it where
`p12+40` does not. Each has a CLI verb (`type`, `erase`, `split`, `join`) because rule 4 has no
exception for operations that feel like a UI's. **All of that is correct under every option in
`doc/text-layout.md` and none of it is at risk.**

What S7 also did was close the layout fork on Path A, and that was **reopened immediately**.
The objection: `CLAUDE.md`'s own architecture rule puts all logic in the core, and line layout
is not rendering — Down-arrow, Home/End, click-to-caret and selection extents are every one of
them defined in terms of a *line*, so Path A hands a piece of the editing model to three shells
that will disagree, and leaves the CLI unable to answer Down-arrow at all. `doc/text-layout.md`
separates two questions `doc/suite.md` had fused (line layout vs. pagination), and it **closed
on Path C**: line layout in `grind-core` (`core/src/layout.rs`), font metrics injected through
a `Metrics` trait, pagination still gated, RTL excluded by explicit decision.
L1 and L2 built it and gave `grind_text::App` the caret operations defined in terms of a line —
`layout_block`, `caret_x`, `caret_line`, `caret_line_bounds` — each reachable from the CLI
(`grind text view --width`, `grind text caret --down/--home/--end`).

**S8 is done: `grind-tui` is a word processor as well as a spreadsheet.** One binary, both
types, dispatched on the file's bytes. It is the payoff of Path C and the proof of it — the
shell implements `Metrics` in terminal cells (`ui_tui/src/text/mod.rs`, about twenty lines using
`unicode-width`) and gets line breaking, `j`/`k` by wrapped line, Home/End and hit-testing from
the core. **S9 and S10 are done, minimally** (`doc/text-shell.md`): `grind-text-gtk` is the word
processor's window — a custom widget drawing from `App::layout_block`, Pango behind `Metrics`,
typing through `GtkIMMulticontext`, an outline dialog and a go-to-address popover for `p12` /
`#intro` / `§2.1.3`, and a banner offering to hand a spreadsheet to `grind-sheet-gtk`. The
browser shell gained a second pane the same way `grind-tui` gained a second mode, dispatching
on `grind_core::kind`, with the canvas as a third `Metrics`. Three implementations of that
trait now exist and the engine needed no change, which is Path C's evidence. Building them
found one core bug (an empty paragraph laid out one *unit* tall, since fixed in
`grind_text::lay_out`) and one core limitation, written down rather than worked around and
**since fixed in the core**: `App::caret_line` took one width and one provider for a motion that
may cross into a block set in a different face, so Down-arrow out of a heading measured the
paragraph below it with the heading's font. The three caret operations now take a
`grind_text::Faces` — which measure and which `Metrics` *this* block is set in, asked as the
motion arrives at each block — with `Uniform` for the every-block-alike case the CLI and the
terminal want. The trait is `grind-text`'s rather than `grind_core::layout`'s because a block is
the word processor's vocabulary (R8), and it is handed a kind and a style name rather than the
block because `App` holds its read lock for the whole motion.

**After S10, character formatting landed in the core** — the half of a rich-text editor that is
not a shell. A `Run` carries a `CharStyle` (`text/src/style.rs`): bold, italic, underline,
strike, family, size, colour, highlight, all ODF values verbatim. The reader resolves an
`office:automatic-styles` text style onto the run and forgets its generated name; the writer
pools distinct formattings back into `T1`, `T2`, … and declares them, so a formatting edit
survives a regenerate where a style *name* does not. `App::set_char_style` replaces (one
`Action::Batch`, one Ctrl+Z) and `App::char_style` reports what a span agrees about, which is
what a toolbar reads before it writes; `grind text format <range>` reaches both (R9). `lay_out`
now measures each run with its own metrics, so a bold word is measured bold. Loop C compares it
character by character on the way out, and `doc/odt-format.md` §5b gained three measured
LibreOffice facts: the `style:font-name` rewrite, the font-family requoting, and the paragraph
hoist.

**The shell half followed, in all three shells.** Selection was the piece everything else waited
on, and each has one: an anchor beside the caret, grown by Shift+arrow, Shift+click and dragging,
erased first by typing or Enter (`Doc::selection` in `ui_text_gtk/src/view.rs`, `Ui::selection` in
`ui_web/src/text/mod.rs`, Visual mode in `grind-tui`). Over it sits a toolbar of the four toggles
— a second top bar of `gtk::ToggleButton`s, the browser's tool row plus colour and highlight,
`*`/`/`/`_`/`~` in the terminal — every one of them reading `App::char_style` and writing
`set_char_style`, so no shell has an idea of what bold means that the document does not share. And
each now *draws* what the core measures rather than one plain string per line:
`ui_text_gtk/src/metrics.rs`'s `run_attributes` builds a Pango attribute list per line out of the
block's own runs, `ui_web/src/text/runs.rs` cuts a line into `<span>`s at every boundary the
formatting, the selection and the caret introduce, and the terminal uses its own attributes. Both
GUI shells give `Title` and `Subtitle` a face of their own. **What is still uneven** is how much
of a `CharStyle` reaches the screen: the browser pane carries colour, highlight, family and size
as inline CSS, and the GTK window emits the four booleans and paints each line in one theme ink,
so a coloured run draws in the theme's foreground there. That, its missing clipboard and its
missing lists UI are `doc/text-shell.md`'s gap list, which is the up-to-date statement of all of
it.

**View modes are built, V0 through V7** (`doc/view-modes.md`): `sheet/src/graph.rs` is the
reference index — the forward and reverse dependency answers, resolved through `Engine::area` so
it cannot disagree with the evaluator — and `sheet/src/view.rs` turns it into a `CellRole` for
every cell and a `NameAnchor` for every named place. `App::get_viewport_with` carries both to a
shell, `view::Names` reads a formula through the names it uses (`=tax_rate*subtotal`), and
`grind sheet view --roles/--names/--formulas` and `grind text view --names` print all of it.
**Nothing here is ever written to a document**, and the headline check is exactly that: open
every R7 document, ask for every overlay, read the whole sheet, save, assert the bytes are
identical. Loop C cannot verify a feature that writes nothing, which is the point rather than a
gap. All four shells draw it; `CellRole::marker` is in the core so no shell invents a glyph
table.

**The projection is built for both document types, D0 through D5** (`doc/dsl.md` layer 0): a
`.grind` is a **third physical form** beside the package and the flat file — `Form::Projection`,
so `grind convert book.fods book.grind` and back both work, for either application, and every
shell opens one because `read_bytes` sniffs the form from the first line rather than the name.
`grind <app> project` prints it, with `--tokens` and `--anchors` for the two maps §6 is built
on. Each grammar is held to its own scope line by a document with executable examples
(`doc/projection-sheet.md` against `sheet/src/model.rs`'s fields, `doc/projection-text.md`
against `doc/text-core.md`'s elements) and loop F holds the bijection over both corpora. **D5 is
R6 for it**: a `.grind` is read with a `grind_core::projection::Source` beside it — the text, and
the byte range of every address in it — so saving splices rather than regenerates. One cell or
one block edited changes one line; comments, blank lines and hand alignment survive; an untouched
save returns the bytes that were read, asserted over both corpora. It is **not** `kdl-rs`'s
mutation API, which reprints nothing and loses the alignment when forced to — `doc/dsl.md` §3.1
records the measurement. **D9 is the code view**: every shell now shows the document as its projection, read-only, with the
line the selection is on marked and moving in it selecting what that line projects — `:source` in
`grind-tui`, *Show the source* in `grind-web`, Ctrl+Shift+U on the other page of a `gtk::Stack` in
both GTK windows. The four line-shaped questions a code view asks (`line_count`, `line_span`,
`line_pieces`, `address_on_line`) are `Projection`'s, so four shells cannot answer them four ways;
each shell contributes one file and the drawing. **D6 is `grind lint`**: eight rules over the two
applications (§4.3's table, which `cli/tests/lint.rs` executes — a rule with no row fails the
build and so does a row naming no rule), reached as `grind sheet lint`, `grind text lint` and
`grind lint` at the suite level, which reads the kind out of the file. What a diagnostic *is* is
`grind_core::lint` and nothing else (R8); every rule is asked through the machinery that already
answers its question — `RefIndex` for what a formula reads, one recalculation walk for whether a
cached value is still true — so the linter cannot disagree with the document's own behaviour.
**Nothing is written**, and an *error* exits non-zero so CI can gate on it. **All four shells
show the findings**: F8 opens a *Check Document* dialog in both GTK windows
(`ui_*_gtk/src/lint.rs`), `:lint` opens a pane in `grind-tui` and Ctrl+K → *Check the document*
one in `grind-web` (`ui_tui/src/problems.rs`, `ui_web/src/problems.rs`, each shared by both
document types). Every row is a jump, because a diagnostic's address is a string the shell's own
parser already takes. **D10's first row is done**: renaming a sheet now carries every reference
that named it — formulas, named expressions and chart ranges — in one `Action::Batch`, so it is
one Ctrl+Z and `doc/not-doing.md`'s row says *deleting* rather than "renaming or deleting". The
rewrite is `formula::rename`, an AST substitution re-serialised by the printer, never a textual
one; a formula this build cannot parse is left alone and `grind lint` finds it.

**D7 is `grind build` — layer 1 has begun.** `grind build model.rhai -o model.fods` runs a Rhai
script that **returns** a document and writes it, for either document type;
`examples/budget.rhai` and `examples/report.rhai` are the two smallest worked examples, `examples/timesheet.rhai` (with `timesheet.json`) is the one `doc/generator-guide.md` builds up to, and the first is
D7's exit criterion — `examples/sample-sheet.sh`'s budget, said once with the categories as data
and a loop for the rows. The arrow points one way: a script produces a document and is never
recovered from one, so this is `build` rather than a form `convert` reaches. Three things hold
the macro line (`doc/not-doing.md` §1, unchanged): **R11** — no crate that opens a document may
depend on `grind-build`, checked by `build/tests/manifest.rs` against every manifest in the
workspace *and* against the workspace's own member list; the language has no filesystem,
network, environment, clock or randomness, two of those by feature flags rather than by
unregistering; and everything is bounded, so a script that does not terminate is an error with a
line number. The host vocabulary is `doc/projection-sheet.md`'s rather than a third spelling of
one model, and the same source produces the same bytes — a test builds the budget twice and
compares them. **D8 (`grind test`) is not built**, and it plus the rest of §6.5's table are the
open list. A script may read JSON data beside itself (`json("prices.json")`,
`examples/prices.rhai` + `prices.json`), which is the one amendment to §2's "no I/O" and is
narrower than the rule it replaces — `doc/generator-spec.md` §3.5 has the four walls and
`build/tests/data.rs` tests each of them, including a symlink pointing out. **`grind
definitions`** prints the vocabulary as a Rhai definition file for an editor (§8);
`examples/grind.d.rhai` is a generated copy kept in the tree, with a test that fails when it
goes stale.

**The two languages owed four documents, and three of them are written** (§7, D11–D14). A design
record is not a specification somebody can implement against, nor a guide somebody can learn
from. **D13** is `doc/generator-spec.md`, the generator's language — the dialect, its limits, the
whole host API, what materialisation does, and what a script cannot say — held to the code by
`build/tests/spec.rs` in both directions plus the limits. That check was the reason to write it
first: adding to the host API is three lines in `register()`, so it was the one vocabulary in the
project with no §3.7-style guard.

**D12 and D14 are the two guides**, and each carries the check its genre allows.
`doc/projection-guide.md` is writing a `.grind` by hand, built on `examples/quote.grind` — a
joinery quote whose formulas carry no answers until `grind sheet recalc` fills them in, asserted
cell by cell in `cli/tests/cli.rs` along with the two properties the guide is *for*: one edit is
one line of diff, and an untouched save returns the bytes that were read.
`doc/generator-guide.md` runs from `examples/first.rhai` to `examples/timesheet.rhai` — four
sheets that have to agree with each other, built out of `examples/timesheet.json`, whose
cross-sheet ranges are measured from tables another loop wrote rather than counted.
`cli/tests/editor.rs` builds every script either guide names and fails when one moves.

Writing them found three things worth keeping (`doc/dsl.md` §7 records all three): the existing
examples were too small to make the case, the two vocabularies disagree about one word
(`percentage` in the projection, `percent` in the generator), and **`doc/generator-spec.md` §8's
editor answer was wrong** — the published `rhaiscript.vscode-rhai` ships no language server, so
the `.d.rhai` file nothing reads is now joined by `grind definitions --snippets`, the same engine
metadata as a VS Code snippet file. `doc/editor-setup.md` is what was measured and how to set it
up; `.vscode/` holds both snippet files and `cli/tests/editor.rs` holds them to their
vocabularies. **Still owed**: the projection's *specification* (D11), for which the two guides
are the evidence of what it would have to settle.

**What remains of the layout work is L3**: `ui_sheet_gtk`'s row auto-height measurement moves onto
the same trait, so one breaker serves both applications. Then S11 — packaging the suite. Its
per-app half is done (`grind-text-gtk` has its `.desktop` file, metainfo, icon and packages
beside the spreadsheet's); what remains is the meta-package that depends on the four, the
container, and the README as a suite pitch.
