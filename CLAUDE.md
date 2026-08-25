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
across invocations), tables, footnotes, fields, style definitions, RTL layout, pages, and — in
any shell — a *selection*, so no copy, cut or paste of text.

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
| `doc/sheet-shell.md` | Phase 9's work plan for the **spreadsheet's** GTK shell — normative for that phase |
| `doc/text-shell.md` | S9 + S10 — what the word processor's GTK and browser shells do, what they deliberately do not, and what building them proved about `Metrics` |
| `doc/tui-shell.md` | **The terminal shell — normative for `ui_tui/`.** Its two decisions (vi rather than a menu; markdown is for *typing*, never for *showing*), what both halves do, and its gap list |
| `doc/web-shell.md` | **The browser shell — normative for `ui_web/`.** Its one design decision (a page, not a window: one verb bar, one tool row, Ctrl+K for the rest), what both panes do, and its gap list — which used to live in the two shell docs above and outgrew them |
| `doc/flat-first.md` | **In doubt, write the form that diffs.** Normative for every default choice between the package and flat forms — `Form::from_path`, save dialogs, new documents |
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
document's kind out of the file (`info`, `convert`). Every core capability is reachable from
it, enforced by
`cli/tests/parity.rs`:

```sh
cargo run -p grind-cli -- sheet new book.ods
cargo run -p grind-cli -- sheet set book.ods A1 1
cargo run -p grind-cli -- sheet set book.ods A2 '=[.A1]*2'   # ODF syntax, verbatim
cargo run -p grind-cli -- sheet recalc book.ods
cargo run -p grind-cli -- sheet view book.ods A1:A2
cargo run -p grind-cli -- --format json info book.ods    # suite level: reads the kind
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
| `grind-core` | `core/` | **\[GENERIC\]** — the container (`odf/package`), the namespace vocabulary (`odf/names`), the tolerant reading architecture (`odf/context`), `Form`, the styling primitives every family of style is built from, the locale, the build stamp, `Observer`, and `kind` (which document type some bytes are) |
| `grind-sheet` | `sheet/` | The spreadsheet: model, column store, ODS reader/writer, R6 splicing, number formats, cell styles, the OpenFormula engine, `App` |
| `grind-text` | `text/` | The word processor (phase 10): the block model, `loc.rs` addressing and carets, `style.rs`'s `CharStyle` (direct character formatting — bold, italic, family, size, colour), `markdown.rs`'s notation and `App::type_markdown` (`**bold**` read as it is typed, in the core so four shells cannot read `**` four ways), the ODT reader and writer, `App` with block *and* caret edits, and R6 splicing — a `.fodt` lives in git the way a `.fods` does, and one keystroke is one line of diff. Line layout is `grind_core::layout`'s and reaches a shell through `App::layout_block`/`caret_line`/`caret_line_bounds` (`doc/text-layout.md`, Path C) |
| `grind-cli` | `cli/` | The `grind` binary |
| `grind-sheet-gtk` | `ui_sheet_gtk/` | The spreadsheet's GTK shell |
| `grind-text-gtk` | `ui_text_gtk/` | The word processor's GTK shell (S9, minimal). Its own binary and app ID because a `.desktop` file's `MimeType=` is per application. `geom.rs` stacks blocks, `keymap.rs` names the motions, `metrics.rs` is Pango behind `Metrics`, `view.rs` is the widget |
| `grind-web` | `ui_web/` | The wasm shell, **both document types in one bundle** — `sheet/` and `text/` under it, panes picked by `grind_core::kind`. `text/mod.rs`'s `Face` is its layout contribution: how wide is this text, in CSS pixels, measured on a canvas. `command.rs` is every verb either pane has, as *data*, reached from the Ctrl+K palette, a key and a button alike (`doc/web-shell.md`) |
| `grind-tui` | `ui_tui/` | The terminal shell, **both document types in one binary** — `sheet/` and `text/` under it, picked by `grind_core::kind` from the file's bytes. `text/mod.rs`'s `Cells` is its whole layout contribution: how wide is this text, in terminal columns. Its formatting toolbar is `grind_text::markdown` — typed, never *drawn* as markers (`doc/tui-shell.md`) |

**R8: no document type's vocabulary reaches `grind-core`.** Checked by `core/tests/generic.rs`,
which asserts the manifest names no document-type crate and that no source dispatches on
`Ns::Table` or `Ns::Text`. `grind-sheet` re-exports the generic modules (`grind_sheet::odf`
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
- **`cli/`** — `main.rs` (clap, one arm per subcommand) + `report.rs`. A subcommand is a few
  lines driving `App`; anything longer belongs in the core. `doc/cli-parity-sheet.md` +
  `cli/tests/parity.rs` are the parity ratchet: every public `App` method needs a reaching
  command or a named reason it has none, checked by a test that reads `sheet/src/lib.rs`.

## Conventions

- **Positions are 0-based in the core.** Only a UI is 1-based; the whole workspace converts
  in exactly one place — `sheet/src/a1.rs`.
- **`ponytail:` comments** mark deliberate shortcuts with a known ceiling and upgrade path.
  They're a tracked ledger — don't silently "fix" one without reading the reason.
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

**M9** brought packaging under `ui_sheet_gtk/data/` (`.desktop`, AppStream metainfo, a scalable
icon — nothing builds or installs them yet, since this is a pure Cargo workspace), a
`gtk::ShortcutsWindow` built from the same accelerator table the window wires up, recent
files via `gtk::RecentManager` (which `gtk::FileDialog`'s own "Recent" section already reads,
so no custom menu was needed), and the a11y floor — `gtk::Accessible::announce` on every
selection move, which is why `ui_sheet_gtk/Cargo.toml`'s `gtk4` feature is now `v4_14`. The flatpak
manifest was the one "stretch" item and was skipped. `.github/workflows/packaging.yml` builds
`.deb` (`cargo deb`) and `.rpm` (`cargo generate-rpm`) packages for both `grind-cli` and
`grind-sheet-gtk`, reading the `[package.metadata.deb]`/`[package.metadata.generate-rpm]` blocks in
each crate's `Cargo.toml`, as artifacts on every push — not yet attached to a release.

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
`grind_text::lay_out`) and one core limitation, written down rather than worked around:
`App::caret_line` takes one width and one provider for a motion that may cross into a block set
in a different face.

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

**The shell half is not built.** No GTK toolbar, no selection to apply one to, and neither GUI
shell *draws* the formatting the core now measures — the gap list in `doc/text-shell.md` is the
up-to-date statement of that, and selection is the piece everything else waits on.

**What remains of the layout work is L3**: `ui_sheet_gtk`'s row auto-height measurement moves onto
the same trait, so one breaker serves both applications. Then S11 — packaging the suite, which
is where `grind-text-gtk` gets its `.desktop` file, its metainfo, its icon and its packages.
