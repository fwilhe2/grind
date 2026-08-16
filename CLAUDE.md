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
| **B** — formula conformance | *parse half:* every formula in the corpus parses. *evaluate half:* recalculating each fixture matches the cached value already in the file | `core/tests/corpus_parse.rs`, `core/tests/corpus_eval.rs` | 509 per-function `.fods` in `functions/**/fods/`, plus loop A's 361 |
| **C** — round-trip differential | write → `soffice --headless --convert-to` → read back → semantically identical, and the reverse | `core/tests/roundtrip.rs` | hand-built cases + 20 densest value-only corpus files |

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

Loop B's evaluate half reports **13201 of 52213 formula cells matching LibreOffice**, with
37706 needing a function that does not exist yet, 1267 disagreeing and 39 reading the clock.
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
`doc/shared-core-native-shell.md`. All state and logic in `core/`; every future shell is a
renderer and event forwarder owning nothing. `cli/` exists so capabilities cannot hide in a
UI.

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
- **`funcs::implemented()` is checked against `doc/small-group.md` by a test.** A function
  outside the 110 fails the build, which is the anti-bloat rule made mechanical.

### `cli/` — the `sheet` binary, and the parity ratchet

Three files: `main.rs` (clap structs, one `match` arm per subcommand), `report.rs` (what gets
printed), `a1.rs` (addressing). A subcommand is a few lines that drive `App`; anything longer
belongs in the core. Conventions come from
[fwilhe2/editor](https://github.com/fwilhe2/editor)'s CLI: file as a positional, long flags
only, `--session`/`--format`/`--dry-run` global, results on stdout, diagnostics on stderr,
**errors are never JSON**.

- **`doc/cli-parity.md` + `cli/tests/parity.rs` are phase 6's exit criterion**, and the same
  mechanism `doc/small-group.md` uses: every public `App` method is listed with the command
  that reaches it or a *reason* it does not, and the test reads `core/src/lib.rs` to check
  both directions. Adding a core capability without exposing it fails CI; so does a stale
  row. The test also asserts it found ≥12 methods — a scanner matching nothing would pass
  vacuously and quietly retire the ratchet.
- **Addresses are ODF reference syntax minus the brackets** — `A1`, `$B$7`, `Data.B2`,
  `'Q3 Actuals'.A1:.C9`. `a1.rs` does not parse them: it wraps them in `[…]` and calls
  `lex::lex`, so the CLI and a formula cannot disagree about what an address means, and
  whole-column forms work because §5.8 already describes them. The one liberty taken is
  case: §5.8 spells a column `[A-Z]+`, so the *cell* half is upper-cased before lexing while
  the sheet name is left exactly as typed.
- **The 1-based/0-based conversion is `a1::format`, and it is the only index arithmetic in
  `cli/`.** There is no inbound `-1` at all — `lex::Axis` is already 0-based, so that half
  lives in the lexer, shared with the evaluator.
- **Formula text is stored verbatim in OpenFormula syntax**, brackets and `;` included. The
  CLI translates addresses but never formulas; a syntax translator is the thing this project
  exists not to have.
- **`recalc` reports `spoiled`** — cells that held a real value and now hold an error. The
  Small Group is complete, but a document is free to use any of the other ~370 functions in
  Part 4, and recalculating one that does is data loss; the CLI warns on stderr rather than
  writing it back quietly. The whole recalculation is one `Action::Batch`, so one `undo` is
  the way back.

## Conventions

- **Positions are 0-based in the core.** Only the CLI is 1-based, and it converts in exactly
  one place — `cli/src/a1.rs`.
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
one, but a document that has one keeps it.

What is left in the phase: locales — a preset date is the ISO spelling and nothing can ask
for `DD.MM.YYYY` yet, though the model holds one fine — and cell styles beyond the
data-style link (fonts, borders and backgrounds are read and dropped, as they always were).

**Phase 6 is done, out of order** — the CLI, because the ratchet cannot ratchet while it is a
stub and because phase 4's functions were reachable only from `cargo test`. It brought the
core operations the CLI needed and nothing else: `Action::SetFormula`, `Action::Batch`,
`App::{set_formula, clear_formula, formula, formula_count, names, recalc, session,
restore_session}`. `App::recalc` writing computed values back is also what loop C's `back`
direction needs, so phase 4's last deferral now has its machinery.

Deliberately still absent, and *not* parity gaps — nothing can do them at all, so no shell is
hiding a capability: adding, renaming and deleting sheets; editing named expressions; CSV.
The rest of phase 5 — setting a format, cell styles, `style:map`, locales — is next in the
plan's order.
