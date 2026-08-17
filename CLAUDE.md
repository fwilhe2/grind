# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

## What this is

An ODF-native spreadsheet: one Rust core, native shells, and a feature list that ends. It is
**not** a port of LibreOffice and contains none of its code. Read `README.md` for the pitch,
`CONTRIBUTING.md` for the rules that bind contributors, and `doc/plan.md` for the phase plan
and its exit criteria.

**`doc/plan.md`'s "The requirements" is normative — R1–R7, MUST/MAY, each naming what checks
it.** Independence and ODF-native semantics (R1); everything written validates against the
RELAX NG schema (R2, `jing -i`); minimal boilerplate (R3); `calcext:` allowed but opt-in and
outranked by R2, since `calcext:value-type` is not schema-valid (R4); LibreOffice's files
read and unknown properties are inert (R5); **writing changes as little XML as possible so a
flat file stays diffable (R6 — the one that is unmet, and phase 8)**; and eight named
documents that must work, vendored in `core/tests/data/kb/` so the requirement cannot skip
(R7). Strictness on the way out, tolerance on the way in: five of the eight R7 files are
*invalid* against the 1.4 schema and must still load.

## The two constraints that govern everything

**1. Clean room.** LibreOffice source may be *read* and *cited by `file:line`*, never copied —
not a function, not a table, not a reworded transcription. Every fact learned from reading it
goes into a spec document (`doc/ods-format.md` is the template, every entry cited) *before* it
reaches code. This is why the project can be licensed freely at all; a port would be bound to
MPL-2.0 permanently. LibreOffice is a **conformance oracle and test corpus**, never a source.

**2. ODF semantics are the product.** Do not reach for Excel-oriented libraries
(formualizer, IronCalc, calamine-for-writing) with a translation layer. The divergence is
semantic, not syntactic — error model, text→number coercion, empty-cell handling,
`table:null-date` vs the 1900 leap-year bug, `!`/`~` reference operators, collation — and a
syntax translator leaks all of it through. The normative specs are the source of truth:

| Source | Role |
|---|---|
| `doc/OpenDocument-v1.4-schema.rng` | ODF 1.4 **Part 3** — content schema |
| `doc/OpenDocument-v1.4-os-part4-formula.html` | ODF 1.4 **Part 4** — OpenFormula: per-function semantics, implicit conversions, error model |
| `doc/small-group.md` | The 110-function scope line, *extracted* from Part 4 §2.3.2, not estimated |
| `doc/ods-format.md` | Clean-room notes on what LibreOffice actually does where the specs leave room |
| `doc/cli-parity.md` | Every public `App` method and the CLI command reaching it — checked by a test |
| `doc/gtk-shell.md` | Phase 9's work plan for the GTK shell — core additions, widget design, milestones |
| `doc/not-doing.md` | The feature line as a product document — never, not yet, and where each capability stops |

The laziness ladder still applies to format-neutral plumbing (quick-xml, zip, petgraph,
chrono) — just never to semantics.

## Commands

```sh
cargo test                       # everything
cargo test --test read_values    # one test file
cargo test -- repeated_columns   # one test by name substring
cargo clippy --workspace --all-targets   # must be clean; CI does not gate on it yet
reuse lint                       # must stay compliant; CI DOES gate on this
```

The binary is `sheet`. Every core capability is reachable from it, and `cli/tests/parity.rs`
fails the build when one is not:

```sh
cargo run -p sheet-cli -- new book.ods
cargo run -p sheet-cli -- set book.ods A1 1
cargo run -p sheet-cli -- set book.ods A2 '=[.A1]*2'   # ODF syntax, verbatim
cargo run -p sheet-cli -- recalc book.ods
cargo run -p sheet-cli -- view book.ods A1:A2
cargo run -p sheet-cli -- --format json info book.ods
```

The GUI is `sheet-gtk`, and it needs `libgtk-4-dev` + `libadwaita-1-dev` to build. It is
**not** in `cargo build --workspace`'s path any more — the root CI job names crates for
exactly that reason — so it is built and run on its own:

```sh
cargo run -p sheet-gtk -- book.ods                  # .ods or .fods; no file = empty document
cargo run -p sheet-gtk -- book.fods --render-to /tmp/grid.png   # one frame, then exit
cargo test -p sheet-gtk                             # geom.rs, and no display needed
```

`--render-to` is the machine's eyes, not a user feature: a custom-drawn widget has no
other assertable output, so that is how a rendering change is checked, and how a refactor
is proved to be one (the PNG comes back byte-identical or it was not a refactor). It reads
its arguments positionally, file first.

The most interesting document to open is the one `examples/sample.sh` builds, because it
uses every feature this build has:

```sh
cargo build && SHEET=target/debug/sheet examples/sample.sh /tmp/demo
cargo run -p sheet-gtk -- /tmp/demo/sample.fods
```

The corpus tests need a LibreOffice checkout and **skip with a notice** without one, so
`cargo test` works on a machine that has none:

```sh
SHEET_LO_CORPUS=/path/to/libreoffice/core/sc/qa/unit/data cargo test
```

The default path is hardcoded to this machine's checkout, so locally it just works.

## The three loops

Correctness is checked against LibreOffice, not against our own opinion. `soffice` must be on
`PATH` for loop C.

| Loop | Asserts | Where | Corpus |
|---|---|---|---|
| **A** — read tolerance | every `.ods`/`.fods` loads without error | `core/tests/corpus_read.rs` | 361 files in `ods/` + `fods/` |
| **B** — formula conformance | *parse half:* every formula in the corpus parses. *display half:* each one survives canonical → display form → canonical. *evaluate half:* recalculating each fixture matches the cached value already in the file | `core/tests/corpus_parse.rs`, `core/tests/corpus_eval.rs` | 509 per-function `.fods` in `functions/**/fods/`, plus loop A's 361 |
| **C** — round-trip differential | write → `soffice --headless --convert-to` → read back → semantically identical, and the reverse | `core/tests/roundtrip.rs` | hand-built cases + 20 densest value-only corpus files |

`core/tests/kb.rs` is the fourth check and the only one that never skips: **R7's fourteen
vendored documents**, because a requirement that skips is a preference. Two corpora that pull
opposite ways — `data/kb/` is hand-written and sparse (an `office:version` of 1.3, a table
with no `table:table-column`, formulas with no cached value), `data/samples/` is
LibreOffice's own output normalised by `odslint-clean` and is dense (three-sheet workbooks,
137 formulas in a table, charts, a pivot table, conditional formatting, hundreds of elements
with no model here). Two upstream samples were dropped for adding zero new elements or
attributes; a corpus file that widens nothing is only slower.

It also validates the writer against the schema with `jing -i` (`-i` because the ODF RNG's
own `draw:control` ID-types make jing reject the schema otherwise, and `xmllint --relaxng`
cannot handle the grammar at all). That check found a real bug loop C structurally could
not: `table:named-expressions` belongs in the spreadsheet's *epilogue*, after the tables, and
LibreOffice reads it in either place. Two more tests hold the requirements that are
*measurements* rather than behaviours: `a_written_document_carries_no_boilerplate` puts a
ceiling on the lines before the first cell (R3 — 7 where LibreOffice spent 200, and it grows
with the distinct formats used, never with the file), and
`the_samples_measure_what_regenerating_still_loses` prints how much of each sample survives
being written back (9–61%), which is R6's before-number for phase 8.

Loop C compares number formats as **the text the cell displays**, not as a `Format` struct.
Style names are LibreOffice's to renumber, and the struct is one step too literal in the
other direction — a bare date cell comes back carrying the ISO style the writer supplies for
it, which is not a difference in what the document says. What must not change is the
rendering.

Loop A currently reports **358 read, 3 password-protected, 0 failed**. Encrypted documents are
its *one* accepted non-success outcome, named explicitly in the test rather than filtered away
— every other error still fails the loop.

Loop B's parse half reports **75845 of 77061 formulas parsed, 1216 excluded, 0 failed**. The
exclusions are four *syntactic* classes named in `excused()` — inline arrays, `~`, quoted
labels, and formulas the corpus contains that §5.2's `Expression` production does not
describe (`of:=NOT(0)NOT(0)` and `of:=(…)AND(…)`, which LO reads but the grammar does not
allow). Never excuse a file; excuse a construct, or fix the parser.

Loop B's *display* half rides in the same walk (one read of the corpus, two checks): **75845
formulas round-trip through display form, 75552 identically, 271 ambiguous, 0 failed**. The
one ambiguity class is named rather than excused per file — unbracketed, a defined name and
a cell address are the same shape, so a document whose name is cell-shaped (`database1`, and
LibreOffice's own `of:Err:502` marker) comes back as a reference. `App::set_name` refuses to
create such a name, which is what keeps the class to documents written elsewhere.

Loop B's evaluate half reports **13327 of 52213 formula cells matching LibreOffice**, with
37600 needing a function that does not exist yet, 1247 disagreeing and 39 reading the clock.
`FLOOR` in the test is the ratchet — raise it, never lower it — and the printed scoreboard is
the work list: per category, and then the fixtures with the most disagreements. To look at
one row of it:

```sh
SHEET_LOOP_B_DUMP=LOG cargo test --test corpus_eval -- --nocapture
```

Read the columns honestly. `volatile` is `NOW`/`TODAY`: a fixture's cached value for one is
the instant LO last recalculated it, so those cells *cannot* agree and counting them as
`wrong` would make the one meaningful column lie. `missing` is a function we have not
written (the whole `addin` category is outside the Small Group and always will be); `wrong`
is a function we claim to have and get wrong, and is the only number that means something
is broken. Much of `wrong` is *cascade* — a fixture's summary row is `AND(<every check>)`,
so one missing function turns a column of otherwise-correct cells red.

`wrong` is also not all ours. Three classes in it are known and named where they live: the
criteria language has wildcards but no regular expressions (`wildcard.rs`), text→number
conversion is ISO-only where LO reads a locale's `0,005` (`date.rs`, and phase 5 owns it),
and a handful of `PV` cells disagree because **LibreOffice is the less accurate one** — its
`PV(0.075/12;24;250)` is `−5555.60584593376` where the exact value rounds to
`−5555.60584593369`, which is what `fin.rs`'s `growth` computes. Check a disagreement against
the arithmetic before assuming the oracle is right.

Loop C runs both directions and is green. Its `out` direction needs only the `soffice`
binary, so **CI runs it** — which is what makes the anti-bloat rule a gate: a feature that
does not survive a LibreOffice round-trip fails CI, and the feature line is defended by a
machine instead of by discipline. Its `back` direction additionally wants the corpus.

Each loop has exactly one documented loosening, and both are named in the test rather than
buried in a constant: loop A accepts `Error::Encrypted`, and loop C compares doubles at 15
significant digits **because that is all LibreOffice writes** (`sal/rtl/math.cxx:364-366`;
see `doc/ods-format.md` §3.4). Comparing exactly there tests LO's serialiser, not ours.
Loop C's `back` direction also skips formula-bearing documents — LO recalculates on load, so
their values are a claim about an evaluator that does not exist until phase 4. If you find
yourself adding a *third* exception, that is a bug in the code, not in the loop.

## Architecture

Shared Core / Native Shell, from [fwilhe2/editor](https://github.com/fwilhe2/editor)'s
`doc/shared-core-native-shell.md`. All state and logic in `core/`; every shell is a renderer
and event forwarder owning nothing. `cli/` exists so capabilities cannot hide in a UI, and
`ui_gtk/` is the first one that does have a UI — it is held to rule 4 by `cli/tests/parity.rs`
rather than by good intentions.

Rules that are cheap now and expensive later — breaking one quietly ends the pattern:

1. **Reads go through `App::get_viewport`.** Never add a getter that hands a caller the whole
   document; the column store stops mattering the moment one exists.
2. **Undo/redo lives in the core.** `Document::apply` returns the action's *inverse* — that is
   the entire mechanism. The undo stack is a stack of inverses; redo is the same trick
   reversed. Shells never implement history.
3. **The core pushes, shells never poll.** `App::mutate` drops the write lock **before**
   notifying observers, because an observer is expected to call straight back in to re-read.
   Notifying under the lock hangs rather than fails, which is why
   `an_observer_may_read_the_app_without_deadlocking` exists before any UI does.
4. **Whatever any GUI can do, the CLI can do.** A UI-only feature is a bug.
5. **No filesystem assumptions.** Every `*_file` has a `*_bytes` twin — the browser has no
   filesystem and this is not retrofittable.
6. **Every feature must survive a LibreOffice round-trip** (loop C).

### `core/src/a1.rs` — addressing, and the only `+ 1`

Addresses as a person writes them — `A1`, `$B$7`, `Data.B2`, `'Q3 Actuals'.A1:.C9` — are ODF
reference syntax minus the brackets. **The only 0↔1 conversion in the workspace lives here**,
which is the point of it being in the core rather than in a shell: a second shell doing its
own would be a second chance to be off by one.

- **It does not parse an address.** `parse` wraps the string in `[…]` and calls `lex::lex`,
  so a shell and a formula cannot disagree about what an address means, and whole-column
  forms work because §5.8 already describes them.
- The inbound half of the conversion is not here either: `lex::Axis` is already 0-based. The
  outbound `+ 1` in `format` is the only index arithmetic outside the lexer.
- The one liberty is case: §5.8 spells a column `[A-Z]+`, so the *cell* half is upper-cased
  before lexing while the sheet name is left exactly as typed.

### `core/src/grid.rs` — the column store

The one deliberately hand-built data structure. A column is a run-length sequence of typed
blocks, so `Empty(1_000_000)` costs four bytes and a million numbers cost one `Vec<f64>`.
Same shape LibreOffice Calc arrived at (`mdds::mtv::soa::multi_type_vector`, cited in the
module docs) reached from the same constraints.

Three invariants — no zero-length blocks, no two adjacent blocks of the same kind, no trailing
empty run — are restored by `normalize()` and asserted by a `check()` the tests run *after
every mutation*, including a 4000-step randomised sequence diffed against a dumb `Vec`
reference. Values reading back correctly while blocks silently fragment is the failure mode
this store has; test structure, not just values.

### `core/src/numfmt/` — number formats

Phase 5's model, and **display only**: a format never touches a value, which is why a
formatted cell still sums and still round-trips as the number it is (§5.2).

- **The shape is the spec's**: a `number:*-style` is an *ordered sequence of pieces*, and
  rendering is a walk over it. There is deliberately **no format-code string** (`"#,##0.00"`)
  anywhere in the core — that spelling is Excel's, ODF has no such attribute, and a code
  parser would be the translation layer this project exists not to have.
- **No XML in the module.** `odf/read.rs` builds a `Format` from elements and `odf/write.rs`
  puts it back, so the vocabulary has exactly one consumer on each side and they fail
  together rather than drifting.
- `numfmt::general` is what a cell with *no* format displays as, and it is where "a date
  prints as a date" comes from without any style in the document: the value type is enough.
- **`style:map` is the sign, not the renderer.** A two-branch format (§5.1's red-negative
  currency, and 9986 cells of the corpus) *is* how ODF spells a negative: the base style
  carries a literal `-` and a map switches to the plain branch at zero. So the renderer
  supplies a minus only for a format that has neither a map nor a leading `-` of its own —
  otherwise `-19.99` renders as `--19.99`. Branches are followed one level, which is all
  LibreOffice writes, and a map whose target the document does not define is dropped.
- Two deliberate gaps, named in the module docs: the locale (month and weekday names are
  English; `HOST-LOCALE` has no home in the core yet) and
  `number:truncate-on-overflow="false"`.
- A date a *formula* computes has no format and no `NumberKind`, so `=DATE(2026;8;16)`
  displays as its serial until the cell is formatted. The subtype belongs in `formula::value`
  (Part 4 §4.3.3), not here.
- **`preset` is the whole "set a format" vocabulary, and it lives in the core.** A shell that
  built its own would make a document formatted from the CLI display differently in a GUI.
  `App::set_format` takes a rectangle (formatting a column is the normal case) as one
  `Action::Batch`, and bounds it: a format costs an entry per cell, so `A1:ZZ100000` is an
  exhaustion request rather than an intent and is refused by size.

### `core/src/locale.rs` — two characters, and where the rest of a locale is not

A number format carries its own locale (§5.2), and `1234.5` displays as `1,234.50` in
`en-US` and `1.234,50` in `de-DE` from the *same* `number:number` element. That is all this
module decides: the decimal point and the grouping separator.

- **The language and country are kept verbatim** as the document spells them, so a locale
  this build has never heard of round-trips and merely falls back to `.`/`,`.
- ponytail: one table, three groups, keyed on the decimal-separator convention rather than
  on CLDR. Switzerland's apostrophe and India's lakh grouping are wrong. The upgrade is a
  real CLDR table or `icu`, and the reason not to have one is that nothing here needs
  collation, plurals or calendars — a dependency that size for two characters is the trade
  this project exists not to make.
- **Month and weekday names stay English**, whatever the locale says. Same argument, bigger
  table.
- **Text→number conversion stays ISO-only** and is *not* this module's gap to close. Part 4
  §6.3.6 makes it `HOST-LOCALE`-dependent, so LO reads `"0,005"` as a number in a German
  document and this build does not. It is a phase 4 conformance item: the locale has to
  reach the evaluator's value model, where nothing carries a document today.

### `core/src/style.rs` — cell styling

§5's other half: a number format says how a *value* is spelled, this says how the cell looks
around it. Both live on one `style:style` and are pooled together, which is why the writer's
pool keys on the **pair**.

- **The values are ODF's own, kept verbatim** — `"bold"`, `"#ffff00"`, `"0.5pt solid
  #000000"`. Parsing XSL-FO strings into a typed model buys nothing until something *renders*
  them, and nothing does yet. When a renderer needs a colour as three bytes it parses one
  string in one place.
- **Validation lives at the edge that has a user**: `sheet style` takes an enum for
  alignment and checks a colour's syntax. A *document's* value is whatever the document said,
  because rejecting it would lose the cell rather than the attribute.
- **`fo:border` is a shorthand for four edges, not a fifth field.** It is expanded on the way
  in and collapsed when the edges agree, or two spellings of one style would be unequal and
  the pool would emit both. ODF's `"none"` is read as *absent*, for the same reason.
- Two properties are deliberately not carried, both measured rather than assumed
  (doc/ods-format.md §5.4): **`fo:font-family`**, which LO rewrites into an
  `office:font-face-decls` reference, and **exact border widths**, which LO re-quantises
  (`0.5pt` comes back `0.51pt`) — so loop C compares widths numerically and everything else
  exactly.

### `core/src/odf/` — the reader

`package`, `names` and `context` are format-agnostic (`[GENERIC]` in the spec) and will be
reused unchanged for text documents; only `read.rs` knows what a spreadsheet is.

**Tolerance is structural, not a feature.** A context that does not recognise a child returns
`None` and the driver installs `Ignore` — whose callbacks all do nothing — for that element
and its entire subtree. Unknown elements, unknown attributes, foreign vendor namespaces and
newer-ODF features are therefore inert *by construction*; nothing detects junk. Whitespace
falls out the same way: `text()` defaults to a no-op, so pretty-printing indentation never
becomes cell content. **If a corpus file ever needs special-casing to load, the architecture is
wrong — fix the architecture, not the file.**

Contexts never talk to each other. A child is told where it lives when created, and all
mutation flows through the shared `Builder`, so there is no child-to-parent channel and no
downcasting.

Non-obvious things a change here can break:

- **Dispatch on `(namespace-uri, local-name)`, never on prefixes.** Prefixes vary in the wild
  and can be redeclared on any element. `prefixes_are_irrelevant_only_the_namespace_uri_matters`
  pins this with a document written using hostile prefixes.
- **`number-columns-repeated` / `number-rows-repeated` are positioning, not optimisation.**
  Mishandle them and every later cell silently lands in the wrong place. Rows are buffered and
  replayed; the empty case (a trailing row repeated ~1M times, which is how a sheet's extent is
  conventionally bounded) must stay O(1).
- **Clamping counts is necessary but not sufficient.** Clamping bounds each *count* while the
  cost is their *product*; 1048576 × 16384 is legal under both clamps and asks for seventeen
  billion writes. `MAX_MATERIALISED_CELLS` bounds the work itself. Truncation must never shift
  addresses — the cursor still advances by the full repeat.
- Every value-parse failure degrades to a safe default **scoped to its own cell**, never to a
  rejected document. A `date` cell whose `office:date-value` will not parse keeps the text and
  loses only its type; the document still loads.
- **A cell's format is reached through two indirections and three defaults.**
  `table:style-name` names a `style:style`, which names a `style:data-style-name`, which is
  the `number:*-style` (§5.1). A cell with no style of its own inherits the row's
  `table:default-cell-style-name`, then the column's — which is how a whole formatted column
  is written, and dropping it loses most real formatting. Column defaults are applied only to
  cells that exist, so a formatted column costs its cells and not its million rows.
- **`styles.xml` is parsed too, before `content.xml`.** A package keeps named styles there,
  and a cell in the content may reference one. It carries no cells, so nothing else depends
  on the order, and a part that will not parse costs its styles rather than the document.
- **A `date` or `time` cell becomes a `CellValue::Number`** — Part 4 §4.3.3 says "Date is a
  subtype of Number", so the serial *is* the value and `[.A1]+1` is the next day. Which
  spelling it had rides in `Sheet::kind` (a side table, like `formulas`) purely so the writer
  can put it back as a date; loop C's `dates` case fails if that is dropped.

### `core/src/odf/source.rs` — retain and splice (R6)

The file a document came from, kept so that saving can **edit** it rather than replace it.
Regenerating is right for a document this program authored and wrong for one it opened.

- **The model is not made fuller.** A shadow tree of every unknown element would grow the
  model to the size of ODF, which is the trade this project exists not to make. Instead the
  reader keeps the bytes and remembers where each cell element sat; the writer drops new
  cells into those exact ranges. Everything it does not understand it never touches, so
  fidelity comes from *not looking* rather than from modelling.
- **`Attrs::span` carries the position, not the `Context` trait.** Widening `start_child`
  and `end` would touch every context in the reader for the sake of the one that wants it.
  `Attrs` is already built per element and handed over, so it is one struct field.
  `Context::end` still gets nothing, which is why `read.rs`'s `cell_extent` scans forward
  for the close tag — and refuses when it meets another `<table:table-cell` first, because a
  cell may legally contain a subtable.
- **A repeated *cell* is split; a repeated *row* is not.** LibreOffice writes a row of five
  empty cells as one element, so refusing to split one would leave R6 true only for cells
  that already held a value. Writing into a run re-emits the element as before/cell/after.
  A row's element stands for many rows, and splitting that means emitting whole rows, which
  is no longer a small diff. `Source::rows` is keyed per `(sheet, row)` rather than per cell
  for the same reason a repeat exists: one trailing `repeated="16384"` would otherwise be
  sixteen thousand map entries.
- **`Cell::keep` is the unmanaged attributes, sliced out verbatim.** Not just the style name:
  re-deriving a start tag from the model dropped `table:number-columns-spanned` and silently
  un-merged a merged cell. Only what the writer produces itself is re-derived — the value,
  its type, the formula, the repeat count — plus `office:currency` and `calcext:value-type`,
  which *describe* a value that just changed. Verbatim by slicing rather than by
  re-serialising, because `Attrs` has already resolved prefixes to namespaces and rebuilding
  would respell the document's attributes in our prefixes.
- **`Document::edits` decides, and it is filled by `Action::apply`.** Every mutation goes
  through one function, so tracking what changed is one insert rather than a diffing pass.
  `only_values` goes false on a format or style change, because those need a `style:style`
  the source file does not contain — a second splice site and a pool to merge, which is not
  built.
- **Every refusal falls back to regenerating**, which is always correct and is what the
  writer did before any of this existed. Refusals are: no source, a changed form, a package
  (a zip has no diff to preserve), a format or style edit, a cell in a row the file does not
  spell, and a repeated row. `what_cannot_be_spliced_regenerates` asserts them by name.
- **R2 means what this build *generates*.** A spliced document keeps bytes that may not be
  schema-valid — most of R7's are not — and re-deriving them to fix that is exactly what R6
  forbids. `kb.rs` validates the regenerating writer, reaching it by setting
  `Document::source` to `None`. That same trick is why a write→read test is not vacuous:
  saving an unedited document returns its bytes byte for byte.

### `core/src/odf/write.rs` — the writer

One content writer for both forms; they differ only in the root element name and whether
`office:mimetype` sits on it (§7.3). Minimal by intent (§1.4): no `styles.xml`, no
`meta.xml`, no automatic styles, because there is nothing yet to put in them.

Whitespace is the trap here, and it is not symmetric with reading. **A conforming reader
collapses runs of whitespace inside `text:p`**, so a literal `"a    b"` comes back as
`"a b"` and a leading or trailing space vanishes — which is why `text:s`/`text:tab` exist
and why `paragraph()` emits them. Our own reader does not collapse, so this bug is invisible
until a document meets LibreOffice. Reading has the mirror-image rule: `text:s` carries a
**count**, and ignoring it flattens every multi-space string.

**Formats are pooled, and the pool is the only construct there is.** ODF has no way to put a
format on a cell — §5.3's indirection is mandatory — so the writer collects the distinct
formats, writes each once as `N{i}` with a `ce{i}` cell style pointing at it, and puts
`table:style-name` on the cells. Pooling that silently reindexes is worse than none: the
second cell then displays through *another* document's format, which is why the test asserts
that two cells sharing a format share a name.

A date or time cell with **no** format is written with an ISO default anyway. LibreOffice
requires a date cell to have a date style and invents one from its own locale when a file
omits it, so writing nothing means the document comes back with `M/D/YY` bolted on — and the
round trip is no longer an identity. A date carrying a fraction is a DateTime (§4.3.4) and
takes a style that shows both halves, because a format cannot look at the value it is given.

Structural invariants worth not breaking: a table needs at least one `table:table-column`
and one `table:table-row` even when empty (§3.2); a formula always travels with its cached
value, since an omitted one renders blank until recalculation (§4); and in the package form
`mimetype` must be the first entry, stored uncompressed, with no extra field — readers sniff
it at a fixed byte offset before unzipping anything, which is what the byte-level assertions
in the writer's tests are pinning.

### `core/src/formula/` — the OpenFormula engine

Built in the plan's order: `value.rs`, then `lex.rs` + `parse.rs` + `serialize.rs`, then the
dependency graph and the functions. Everything cites ODF 1.4 **Part 4** by section, because
that spec is the source of truth and a rule without a citation is a guess.

- **`value.rs` is the single point of failure** (doc/plan.md's own words): the value model,
  the seven-name error set, and §6.3's implicit conversions. Its three
  "implementation-defined" points are named as choices in the module docs, not left as
  accidents — text→number and text→logical are strict, and `Value::number` turns a non-finite
  result into `#NUM!` at the one place every operator returns through.
- **Referenced cells and scalar arguments convert differently.** §6.3.7's sequence conversion
  keeps only numbers, so `SUM(A1:A3)` skips a cell holding `"7"` while `SUM("7")` converts it.
  That asymmetry is the semantics, not an optimisation.
- **`§5.5` Table 1 has two traps**, both encoded in `parse.rs`'s binding powers and both the
  opposite of most languages: prefix `-` binds tighter than `^` (`-2^2` is `4`), and `^` is
  left-associative (`2^3^2` is `64`).
- **References are lexed, not parsed.** `[` … `]` is a terminating rule (§5.14); the parser
  never looks at a character. The second end of a range inherits the first's sheet (§5.8), and
  getting that wrong reads the wrong sheet silently rather than failing.
- **Serialisation parenthesises by precedence rather than by memory**, so an AST that nobody
  parsed — one the CLI or a reference rewrite built — still prints as itself. Explicit
  `Expr::Paren` nodes are kept on top of that, because §5.5 Note 3 asks for it.
- Deliberately unparsed: inline arrays (§5.13), quoted labels and automatic intersection
  (§5.10), and `~` is parsed but out of scope for evaluation. §2.3.2 excludes all of them from
  the Small Group.
- **`eval.rs`: recursion is the dependency graph.** A formula reading a formula evaluates it
  first, memoised per cell, so the topological order is arrived at by asking rather than by
  sorting — and a cell already being visited is a cycle, which is `#NUM!`. The plan's
  `graph.rs` with dirty propagation earns its place when *incremental* recalculation exists;
  nothing needs it yet, and the `ponytail:` note in the module says so.
- **Functions get their arguments unevaluated**, which is what lets `IF` short-circuit
  (§6.15.4) so that `IF([.A1]=0;0;1/[.A1])` is a working guard. Conversions live in `Args`,
  not in each function.
- **A whole-column reference is bounded by the sheet's used extent**, not by 1048576. Cells
  past it are empty and contribute nothing to any Small Group function, so the answer is the
  same and the reads are not.
- Two functions deliberately break the house rules, each because its spec section says to:
  `COUNT` does not propagate errors (§6.13.6 in as many words), and the `IS*` family reads an
  error rather than converting it.
- **`date.rs` holds every calendar rule, and the reader and writer share it.** A serial date
  is days from `HOST-NULL-DATE` (§3.4 item 8), whose default is **1899-12-30** — §4.3.3
  Note 3's own recommendation, and the reason 1900 is correctly not a leap year here. Text
  parsing is **ISO 8601 only** on purpose: `DATEVALUE` is locale-dependent (`HOST-LOCALE`),
  and a locale belongs to phase 5's number formats, not to a guess. Both `table:null-date`
  and `table:null-year` are read from the document — a corpus file really does set the
  latter to 1919.
- **`DATE` has two rules that are not in §6.10.2's prose**, both from the corpus: a
  two-digit year expands through `HOST-NULL-YEAR` *before* the month/day roll-over, and a
  year before §7.4's 1583 is `#VALUE!` rather than a proleptic guess.
- **`NOW` and `TODAY` are the only two functions that are not a pure function of the
  document.** Loop B counts them in its own `volatile` column, and `date::now` carries a
  `ponytail:` for reading UTC rather than the host's locale.
- **`display.rs` is the formula bar's syntax, and it is not a second parser.** A document
  stores `of:=SUM([.B2:.B4])`; a shell shows `=SUM(B2:B4)`. `to_display` re-serialises
  through `serialize::Bare` — one printer with the brackets left off, so precedence is not
  computed twice. `from_display` *scans* for reference-shaped runs, re-brackets them and
  hands the result to the existing lexer and parser, so the grammar that validates a typed
  formula is the grammar that reads the file. `spans` is that same scanner exposed in **byte**
  ranges (Pango attribute indices are bytes), which is why an editor's colourer and its
  committer cannot disagree about what a reference is. Two forms stay bracketed on purpose:
  an external-source reference, and the range *operator* between two references
  (`[Sheet2.C22]:[.C33]`, whose second end does **not** inherit the first's sheet the way
  `[Sheet2.C22:.C33]` does). Function names keep the case they were typed in, as `sheet set`
  stores them.
- **A parse error now carries a character offset, not a token index.** `lex::lex_spans` keeps
  where each token started and `parse` maps the parser's token index back through it, which is
  what lets `DisplayError` put a caret on the problem.
- **`criterion.rs` is where empty cells stop behaving.** §4.11.8 spells out that `"=0"` does
  *not* match an empty cell even though an empty cell converts to 0 everywhere else, that
  `"<>7"` does, and that a criterion which is a *reference* to an empty cell means the number
  0 rather than "blank". Each of those is one line, and each was a loop B disagreement first.
- **The lookups' default is the sorted search**, not the exact one — `VLOOKUP` without its
  fourth argument takes the largest value ≤ the key, and the **last** of a run of equals.
  A sorted search that lands on another type is `#N/A` (§6.14.9), which is what stops a text
  key from "finding" the last number in a column.
- **`db.rs` is twelve functions and one selection.** §6.9's Database/Field/Criteria triple is
  a *shape*: the first row of the database names the fields, the criteria range's first row
  names them again, cells within one criteria row are ANDed and the rows are ORed. Three
  things about blankness are load-bearing and each was a loop B disagreement — a blank
  criterion cell constrains nothing, a criteria row that is blank all the way across is not a
  disjunct at all (it is the unfilled part of a rectangle), and a formula that returned `""`
  counts as blank in both of those and in `criterion.rs`'s bare `"="` and `"<>"`.
- **`fin.rs` never writes `(1 + rate).powf(nper)`.** `growth()` is `ln_1p`/`exp_m1`, because
  forming `1 + rate` for a monthly rate throws away three digits of it before the power
  begins, and cached values are compared at fifteen. It is also what makes `PMT` work at a
  rate of `1e-16`. `PayType` is a flag, so any non-zero value is 1 (§6.12.36 defines 0 and 1
  and nothing else), and `RATE`/`IRR` iterate from `Guess` first — that is which root
  LibreOffice reports — falling back to a sign-change sweep of the whole domain only when the
  secant lands on nothing.
- **`funcs::catalog()` is the spec quoted, not paraphrased.** Autocomplete needs a list, a
  signature hint needs the parameter names, and a tooltip needs one line — all three are
  already in Part 4, so `catalog.rs` carries each section's `Syntax:` and `Summary:`
  verbatim with its section number, and the OASIS copyright alongside ours for the same
  reason `doc/small-group.md` does. Two tests hold it: it names exactly `implemented()`, and
  every section matches `doc/small-group.md`. The first of them found **three errata in the
  spec** — §6.13.20 `ISNA`, §6.20.18 `REPT` and §6.12.45 `SLN` each have a `Syntax:` line
  naming a *different* function — which are corrected in place and noted at the entry.
  `sheet functions --long` prints it.
- **`funcs::implemented()` is checked against `doc/small-group.md` by a test.** A function
  outside it fails the build, which is the anti-bloat rule made mechanical. The document has
  **two halves** and the test splits on the `# Beyond the Small Group` heading: above it is
  §2.3.2 E) verbatim and asserted to be exactly 110, so the conformance claim keeps meaning
  what it says; below it is the plan's one-at-a-time escape hatch, and each entry carries the
  evidence that moved it. `ROW` and `COLUMN` (§6.13.29, §6.13.4) are the only two, moved in
  because R7's `fizzbuzz.fods` is eighteen copies of `IF(MOD(ROW();15)=0;…)` — eight lines in
  `info.rs`, no new machinery, and worth 126 loop B cells besides.

### `ui_gtk/` — the GNOME shell

Phase 9's first shell, planned in `doc/gtk-shell.md`, which is normative for this phase the
way `doc/plan.md` is for the rest. Crate `sheet-gtk`, binary `sheet-gtk`. It reads and draws
today and does not edit; the milestones are in the plan.

It owns no data. Every paint asks `App::get_viewport` for the cells that fit on screen and
throws them away — which is what makes a 1048576 × 16384 sheet cost a screenful. A field
here that is not a presentation concern (selection, scroll, an in-progress edit) means the
core is missing something.

- **A custom `gtk::Widget` implementing `gtk::Scrollable`, drawing in `snapshot()` — not
  `GtkColumnView`.** That widget is row-oriented, wants to own a list model, has no
  rectangular selection and does not virtualise 16384 columns; a widget that owns the
  document is rule 1's trap in its nastiest form. The four Scrollable properties are
  overridden by hand rather than by the `Properties` derive, whose `override_interface`
  spelling churns between gtk4-rs releases.
- **`geom.rs` holds the arithmetic and no GTK types**, so it unit-tests with no display,
  no compositor, and no CI runner that has either — the only cheaply testable part of a
  GUI, which is why everything decidable belongs there. `hit` is `cell_rect`'s inverse and
  the two are tested *against each other*: an off-by-one otherwise shows up only as a grid
  that looks very slightly wrong. Two coordinate spaces (content and widget) and mixing
  them is the bug the module exists to prevent.
- **The scrollbar's `upper` is the *used* extent plus a screenful, never 1048576 rows.** A
  thumb sized against the whole sheet is a few pixels tall and one click lands in row
  800000. It grows as the view moves into it.
- **Text overflows into empty neighbours and stops at the first occupied cell; a number
  that will not fit becomes `###`.** The asymmetry is the convention every spreadsheet user
  expects and it carries information — a wrong magnitude read as a right one is worse than
  no reading, and a number that reads as *text* is visibly left-aligned, which is how a
  bad import is spotted. Columns are fetched with a margin either side so a label anchored
  off-screen still reaches into view and "is the neighbour empty" needs no second read.
- **Every colour comes from the theme** (`theme.rs`), never a literal, or the grid is white
  in a dark session. `lookup_color` is deprecated with no replacement, so it is called in
  exactly that one place, with fallbacks.
- **GTK 4 has no partial invalidation** — no `queue_draw_area`, so any change redraws the
  widget. That is fine because the cost is bounded by the cells on screen, and it means the
  performance lever is text shaping (one reused `pango::Layout`, `ponytail:` noted) rather
  than damage tracking.
- **`keymap.rs` is pure too**, and holds the selection as well as the keys: an anchor and an
  active cell, plus every rule for moving them. `moved` takes an `occupied` closure rather
  than an `App`, which is what keeps the *rule* headless-testable and the *reads* in the
  widget. Ctrl+arrow bounds its scan by the **used extent** rather than by row 1048576 — the
  same bound the scrollbar uses, and the reason Ctrl+Down in an empty column lands on data
  rather than a million rows into nothing.
- **A key the keymap does not claim returns `Propagation::Proceed`**, which is what leaves
  the toolkit's bindings — and later the editor's input method — working. There is a test
  that a Ctrl-modified key never also moves one cell.
- **The status bar's aggregates are `App::preview` over generated formulas** (`SUM`,
  `COUNTA`, `AVERAGE`), so what the bar says and what a cell would say cannot differ. The
  range is clamped to the used extent first — a whole-column selection must not walk a
  million rows — and evaluated at a cell one *past* it, because a formula evaluated inside
  its own range is a circular reference.
- **`state.rs` is the editing machine, and pure**: Ready → Enter | Edit. The difference
  between the two editing modes is one rule — an arrow key *commits* in Enter and moves the
  caret in Edit — and it exists because M6's Point mode grows out of it.
- **The in-cell editor is a real `gtk::Text` child of the grid**, allocated over the active
  cell by the same `cell_rect` that draws it, sharing **one `gtk::EntryBuffer`** with the
  formula bar so the two stay in step while each keeps its own caret. A custom-drawn editor
  is rejected outright: `gtk::Text` brings the input method, the caret, selection and
  clipboard, and hand-rolling `GtkIMContext` is the classic way to ship broken input. The
  cell under it is skipped by `draw_cells`, or its stored value shows through.
- **The key controller is in Capture phase**, so the grid decides before the editor child
  does and everything it does not claim travels on untouched.
- **A commit that changes nothing writes nothing.** Opening a cell and closing it again must
  not push an undo entry, and a click inside the editor is a caret move rather than a commit
  — the grid compares against `App::input_text` and checks the editor's own rectangle.
- **Every edit runs `RecalcMode::Document`**, so a GUI feels live; when that would spoil a
  cached value the edit still lands and an `adw::Banner` offers *Recalculate Anyway*. A
  banner rather than a toast because it is a state the document is in, not an event.
- **The observer bridge is `async-channel` + one `spawn_future_local`** (rule 3: the core
  pushes, shells never poll). `Observer` is `Send + Sync` and a widget is neither, so what
  crosses is a token; the loop drains a burst into one refresh, *after* the mutation that
  sent it released the lock.
- **The clipboard carries `App::input_text`, not the display text**: pasted back it
  reproduces the cells exactly, and pasted elsewhere `1234.5` is a number where
  `1,234.50 €` is a guess about that program's locale. Paste is asynchronous because the
  clipboard is, and lands as one `App::enter_range` — one undo step.
- **Point mode is a predicate, not a mode.** `state::ref_eligible(text, caret)` asks whether
  a reference could go where the caret is — a formula, and an operator, `(`, `;` or the `=`
  itself before it — and while a pending reference exists the arrows keep moving it. A flag
  would go stale the moment someone moved the caret. Pointing borrows the *keymap's* motion
  vocabulary, so Ctrl+arrow points at a data edge and PgDn points a screenful, for free.
- **A pending reference is a byte range plus two positions.** Every point event re-renders
  `a1::reference` through `display::reference_text` and replaces that range, so the text
  never accumulates halves of a reference. Anything the user types clears the pending — the
  buffer's own change signal does it, guarded by an `applying` flag so the widget's own
  writes do not count.
- **The caret moving is its own signal.** The buffer's change signal fires *inside*
  `set_text`, before the caret is repositioned, so a signature hint driven by it alone is one
  keystroke behind — and an arrow through a formula changes no text at all.
  `connect_caret_moved` is what the hint and the completion listen to.
- **References are coloured by `display::spans`**, the same scanner that decides what a
  reference is when the formula is committed — one per *distinct* reference, in both editors
  and as outlines on the grid, with the pending one drawn thicker. The eight colours are the
  one place a colour is not the theme's: they are data colours, and the theme only decides
  which of the two sets reads on this background.
- **The autocomplete and the signature hint are `funcs::catalog()`** — the spec's own list,
  signature and summary. A shell keeping its own would offer a function the evaluator does
  not have.
- **`chrome.rs` is the parts made of ordinary widgets** — formula bar, name box, sheet tabs,
  status bar — and owns nothing either. The name box resolves through `core::a1`, so what it
  means by `Data.B2:C9` is what a formula means by it.

### `cli/` — the `sheet` binary, and the parity ratchet

Two files: `main.rs` (clap structs, one `match` arm per subcommand) and `report.rs` (what
gets printed); addressing moved down to `core::a1`, where every shell shares it. A
subcommand is a few lines that drive `App`; anything longer belongs in the core. Conventions come from
[fwilhe2/editor](https://github.com/fwilhe2/editor)'s CLI: file as a positional, long flags
only, `--session`/`--format`/`--dry-run` global, results on stdout, diagnostics on stderr,
**errors are never JSON**.

- **`doc/cli-parity.md` + `cli/tests/parity.rs` are phase 6's exit criterion**, and the same
  mechanism `doc/small-group.md` uses: every public `App` method is listed with the command
  that reaches it or a *reason* it does not, and the test reads `core/src/lib.rs` to check
  both directions. Adding a core capability without exposing it fails CI; so does a stale
  row. The test also asserts it found ≥12 methods — a scanner matching nothing would pass
  vacuously and quietly retire the ratchet.
- **Addresses are `core::a1`'s**, which is where the 1-based/0-based conversion lives for
  every shell at once — see its section above. Nothing in `cli/` does index arithmetic.
- **`sheet set` is `App::enter`, the typing rule**, not `set_cell`: a leading `=` is a
  formula, a leading `'` (which is what `--text` spells) forces text, an empty value clears,
  and whatever the cell held — a formula included — is replaced. `--recalc` recalculates the
  document in the same undo step, which is what a GUI does on every commit.
- **`sheet eval`, `sheet paste`, `sheet clear <range>`** are the CLI halves of what the grid
  needs: a formula previewed without storing it, a rectangle filled from TSV, a rectangle
  emptied — each one `App` call, each one undo step.
- **`sheet fmt --display` / `--from-display`** convert between stored and display form
  (`formula::display`), which is how a shell's formula bar and the file agree.
- **Formula text is stored verbatim in OpenFormula syntax**, brackets and `;` included. The
  CLI translates addresses but never formulas; a syntax translator is the thing this project
  exists not to have.
- **`sheet name` defines a named expression (§5.11), which is what lets a formula say what
  it means** — `SUM(expenses)` rather than `SUM([.B2:.B4])`. Same rule as `set`: a leading
  `=` is an expression, anything else is an address. An address is stored **absolute and
  sheet-qualified** (`a1::as_definition`), because a name is document-level — a relative one
  would shift with the formula mentioning it and an unqualified one would mean a different
  range on another sheet. `App::set_name` **parses before it stores** and validates the name
  in the *core*, because a name that will not lex is unreachable: the document would carry it
  and every formula naming it would still say `#NAME?`. A name that is also a **cell**
  address is refused (`A1`, `Q1` — LibreOffice refuses those too); a whole-column one is
  *not*, because OpenFormula always brackets a reference, and refusing `SALES` would rule out
  most of the words anyone wants. `Action::SetName` sets `edits.only_values = false`, or R6's
  splice would write the cells and silently drop the name.
- **Every command that writes reports `stale`.** A cached value and a formula are two claims
  about the same cell, and editing a cell a formula *reads* makes them disagree without
  touching the formula's own cell — `set B2 4321` leaves `B5 = SUM([.B2:.B4])` on disk beside
  the total it used to have. ODF has no dirty bit and every reader including LibreOffice
  shows the cached value, so the file is quietly wrong until someone recalculates. `App::stale`
  is the *same walk* `recalc` does with nothing written (`recalculated()` is shared), so
  "what recalculating would do" and "what it does" cannot drift; the CLI warns on stderr from
  `finish`, the one place every mutating command passes through, and carries the count in the
  JSON report. **Warned about rather than fixed**, for the reason below — recalculating is
  the user's call.
- **`recalc` reports `spoiled`** — cells that held a real value and now hold an error. The
  Small Group is complete, but a document is free to use any of the other ~370 functions in
  Part 4, and recalculating one that does is data loss; the CLI warns on stderr rather than
  writing it back quietly. The whole recalculation is one `Action::Batch`, so one `undo` is
  the way back.

## The sample document

`examples/sample.sh` builds a document out of **every feature this build supports**, through
the CLI and nothing else. It is a living inventory rather than a demo: `cli/tests/cli.rs`
runs it, so a feature that breaks or a flag that changes fails the build. **A feature that
lands without a line there is a feature nobody can see — add one when you add a capability.**

```sh
SHEET=target/debug/sheet examples/sample.sh /tmp/demo
```

## Conventions

- **Positions are 0-based in the core.** Only a user interface is 1-based, and the whole
  workspace converts in exactly one place — `core/src/a1.rs`.
- **`ponytail:` comments** mark deliberate shortcuts with a known ceiling and their upgrade
  path (e.g. text and bool sharing one block; linear block lookup). They are a tracked ledger,
  not apologies — do not silently "fix" one without reading the reason.
- Deferred on purpose, and documented where it lives: corrupt-zip recovery is unbuilt
  because no corpus file needs it and it belongs with the spec's explicit repair mode.
  (Dates were the other entry here; phase 4's `date.rs` closed it — see below.)

## REUSE / licensing

**AGPL-3.0-or-later**, and REUSE compliance is enforced in CI. Every file carries
machine-readable copyright and license. New files need annotating:

```sh
reuse annotate --copyright "Florian Wilhelm <fwilhelm.wgt+github@gmail.com>" \
               --license AGPL-3.0-or-later path/to/file
```

Generated files that would lose a header (`Cargo.lock`) are declared in `REUSE.toml` instead.

**Never annotate `doc/OpenDocument-v1.4-*` in place.** They are OASIS specifications
redistributed verbatim, and their terms state the document *"may not be modified in any way,
including by removing the copyright notice"* — adding an SPDX header is a modification. They
carry `.license` **sidecar** files; if you re-download them, re-add the sidecars. Anything
derived from them (`doc/small-group.md` is the worked example) carries the OASIS copyright
alongside ours, because the OASIS grant for derivative works is conditional on passing the
notice along.

The `.html` spec will not hash-match a fresh download — Cloudflare rewrites the authors'
mailto: addresses per request. Its sidecar records the verification command; this is not
tampering.

## Where the work is

Phases 0–3 are done: specs and harness, the column store and undo/redo, the reader with loop
A green, and the writer with loop C green in both directions. Documents open and save through
`App` — never by handing out the `Document`, which is why saving is `App::save_bytes` rather
than a getter.

**Phase 4 has its function line complete** — the formula engine, whose exit criterion is a
conforming OpenFormula Small Group evaluator, and the phase that decides whether the project
is real. `doc/plan.md` has the ordering within it. Done: `value.rs` (the value model, error
set and §6.3 conversions), the front end (`lex.rs`, `parse.rs`, `serialize.rs`) with loop B's
parse half green across the whole corpus, `eval.rs`, **all 110 of the Small Group's
functions** — the last two groups in are database (§6.9, `db.rs`) and financial (§6.12,
`fin.rs`) — named expressions (§5.11) read, resolved and written, and **dates and times**.
Loop C gates the last two, so a named range and a date both survive a LibreOffice round-trip.

What is left in phase 4 is conformance, not coverage, and loop B names each piece: criteria
and unsorted lookups take wildcards (`wildcard.rs`) but regular expressions are off, which
is LibreOffice's own either/or, array/matrix formulas are read as
ordinary ones, and `table:database-range` names are not read, so a formula naming one gets
`#NAME?`. Still deferred *in code*: loop C's `back` direction skips formula-bearing
documents.

**Phase 5 has its number formats done** (`core/src/numfmt/`). Formats are read through
§5.1's indirection — cell style, row default, column default, and `styles.xml` as well as
`content.xml` — rendered, pooled back out as automatic styles, and checked by loop C in both
directions. `sheet view` and `sheet get` print what a cell displays, with `--raw` for the
stored value, and `--format json` carries both. `sheet format` sets one, over a rectangle and
in one undo step; `general` clears it. The vocabulary is `numfmt::preset` in the **core**, so
a GUI's format picker cannot invent a second one.

Two-branch formats (`style:map`) read, render and round-trip; `sheet format` cannot build
one, but a document that has one keeps it. **Cell styling** — weights, colours, borders,
alignment, wrapping — reads, writes and round-trips too (`core/src/style.rs`), and `sheet
style` sets it.

A format also carries its **locale** (`core/src/locale.rs`), so a German document's
`1.234,50` stays that, and `sheet format --locale de-DE` writes one.

**Phase 5 is done.** What it deliberately does not have, each named where it lives: fonts,
which LibreOffice rewrites into an `office:font-face-decls` reference (§5.4) and which
nothing can draw anyway; locale-specific month and weekday names; and a preset date is the
ISO spelling, though the model holds `DD.MM.YYYY` fine — nothing can ask for one.

**Phase 6 is done, out of order** — the CLI, because the ratchet cannot ratchet while it is a
stub and because phase 4's functions were reachable only from `cargo test`. It brought the
core operations the CLI needed and nothing else: `Action::SetFormula`, `Action::Batch`,
`App::{set_formula, clear_formula, formula, formula_count, names, recalc, session,
restore_session}`. `App::recalc` writing computed values back is also what loop C's `back`
direction needs, so phase 4's last deferral now has its machinery.

Deliberately still absent, and *not* parity gaps — nothing can do them at all, so no shell is
hiding a capability: reordering sheets; CSV.

**Sheets are add, rename and delete** (`App::{add_sheet, rename_sheet, remove_sheet}`,
`sheet add|rename|remove`), built ahead of phase 9 because a grid UI wants them within the
hour. Three things carry the weight: `Action::InsertSheet` carries a whole `Sheet`, so
undoing a deletion brings the cells back rather than an empty sheet with the right name;
adding or removing shifts every later index and the undo stack survives it *because it is
strictly ordered* — an older entry is only applied once the sheet action above it has been
undone — which is why there are no sheet handles; and a rename does **not** rewrite the
formulas naming the old sheet, so they go stale and `finish`'s existing warning is what says
so. A `Sheet` is `Serialize` for the session file, which is why `Pos`-keyed side tables are
written as pairs (`model.rs`'s `pairs`): JSON has no key but a string.

**Phase 7 is done — the stop is written down.** `doc/not-doing.md` is the product document
the plan asks for before any GUI: what is never (macros, xlsx writing, Large Group, arrays,
pivot tables), what is not yet and which gate owns it, and where each capability that *does*
exist stops — each row pointing at the `ponytail:` comment or module doc that owns the limit
rather than restating it. Adding a capability means moving a row, not just writing code.

**Phase 8 is done — R6, the diffable writer** (`core/src/odf/source.rs`). Setting one cell in
a 482-line LibreOffice file now changes one element and leaves every other byte alone,
indentation included. Phase 9 is the shells.

**Phase 9 is in progress: the shells**, planned in `doc/gtk-shell.md` — normative for this
phase the way `doc/plan.md` is for the rest, and holding the three decisions taken up
front: A1 display-form formulas, recalculation that is automatic only when it cannot spoil
a cached value, and column widths in scope.

Done: **M0** (the CI split), **M1, the read-only grid** — see `ui_gtk/` under Architecture
for what it is and what not to break — and **M2, the core work the editing milestones need**,
CLI-first as rule 4 requires: `core::a1` (C1), `formula::display` (C2) with loop B's third
half over the corpus, and `App::{enter, preview, clear_range, enter_range}` (C3–C6) reached
by `sheet set --recalc`, `sheet eval`, `sheet clear <range>` and `sheet paste`.

**M3 is done too**: selection and navigation — `keymap.rs` (pure, tested), click and drag
including whole columns and rows from the headers, Ctrl+arrows to the data edges, Ctrl+A,
Home/End/PgUp/PgDn, scroll-into-view, and a status bar showing Sum · Count · Average of the
selection.

**M4 is done**: editing. `state.rs` (pure, tested), the in-cell editor and the formula bar
over one shared buffer, commit through `App::enter` with the ripple in the same undo entry,
Delete over a selection, undo/redo, the spoilage banner, sheet tabs with an undo toast for a
deletion, name-box navigation, and open/save/save-as with a close confirmation. It brought
one core capability: `App::input_text` — what an editor shows for a cell, which is
`App::enter`'s inverse and therefore belongs beside it rather than in a shell (`sheet get
--input`).

**M5 is done**: the clipboard. Ctrl+C/X/V over a selection, tab-separated, through
`enter_range` and `clear_range`.

**M6 is done**: the formula UX. Point mode (arrows and drags build a reference where one
could go), F4 cycling `$`, Tab-column memory, reference colouring in both editors and as
outlines on the grid, an autocomplete popover over `funcs::catalog()` and the document's
names, a signature hint with the current argument bold, and a live preview chip — errors
included, which is half the value. It brought C9 (`funcs::catalog()`) and two core helpers,
`a1::reference` and `display::reference_text`.

Next is M7 — styles and the formatting UI (C7's getters and C8's viewport styles), then M8's
column widths and M9's packaging. The wasm shell after that is the honest test of rule 5.
