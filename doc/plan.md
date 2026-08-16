<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Rust ODF spreadsheet core with native shells

## Context

You want an app with LibreOffice's *substance* — real ODF documents, real formulas, real
interoperability — without its UX and without its feature surface. LO matters here because
it is **the leading implementation of ODF**, and ODF fidelity is the product. Two pieces of
groundwork already exist and decide most of the shape:

- **`/home/florian/code/github.com/fwilhe2/editor`** — a working Shared Core / Native Shell
  prototype. One Rust core, six shells, plus `doc/shared-core-native-shell.md`, which is a
  written-down architecture with rules, an order of work, and known traps. This project is
  that guide applied to a document type that actually justifies it.
- **`ODS_CLEAN_ROOM_SPEC.md`** (this repo root, 635 lines) — a clean-room ODS read/write
  spec derived from the OASIS RNG schema plus *cited, never copied* observations of LO's
  filter source. It covers packaging, the content model, styles, number formats, minimal
  templates, and — most valuably — LO's tolerant-reading architecture (§8–9).

Decisions made up front, which the rest of this assumes:

| Question | Answer |
|---|---|
| Relationship to LO source | **Clean-room + cite.** Read for facts, never copy. |
| First vertical slice | **Calc / spreadsheet.** |
| Endpoint | **Usable daily driver for the 90%.** Never parity; the missing 10% is the product. |
| Shells on day one | **Core + CLI only.** No GUI until the model and formulas are solid. |
| Formula semantics | **ODF-native throughout.** No Excel-semantics engine with a translation layer. |

## The requirements

The decisions above say what the project *is*. These say what it must **do**, and they are
normative — MUST / MAY in the RFC 2119 sense. Each one names the thing that checks it, because
a requirement nothing checks is a preference.

| | Requirement | Checked by |
|---|---|---|
| **R1** | The implementation is **fully independent**, and implements ODF natively rather than translating another format. | `CONTRIBUTING.md`'s clean-room rule; `funcs::implemented()` against `doc/small-group.md` |
| **R2** | Everything written **MUST be valid** against the ODF schema. | `jing -i doc/OpenDocument-v1.4-schema.rng`, in `core/tests/kb.rs` and over `examples/sample.sh`'s output in `cli/tests/cli.rs` |
| **R3** | Output **MUST carry minimal boilerplate** while staying compliant — nothing written that the document does not use. | the writer's own tests; a new document is 13 lines |
| **R4** | Output **MAY carry `calcext:`** items where LibreOffice needs them. | see below — nothing yet, because nothing needs it |
| **R5** | Files LibreOffice produces **MUST read**, and unknown elements and attributes **MUST be tolerated**. | loop A, 361 documents; `core/tests/kb.rs`'s three LibreOffice-authored files |
| **R6** | Writing **MUST change as little of the XML as possible**. Editing one number must not produce a hundred-line diff the way LibreOffice's own save does; a flat ODF file must stay easy to `git diff`. | **unmet** — phase 8 |
| **R7** | These eight documents **MUST work**, and are vendored so the requirement cannot skip: `filter` · `fizzbuzz` · `formula` · `minimal` · `minimal-libreoffice` · `minimal-libreoffice-cleanup` · `minimal-with-styles` · `named-range`, all `.fods`. | `core/tests/kb.rs` |

Three consequences worth stating rather than discovering:

**R2 outranks R4.** `calcext:value-type` is *not* valid against the ODF schema — the strict
grammar allows no such attribute on `table:table-cell`, and LibreOffice's own output fails
validation because of it (along with `loext:opacity` and `loext:theme`). So `calcext:` is
opt-in, never default, and an item only earns its place with a *measured* LibreOffice
behaviour recorded in `doc/ods-format.md` that cannot be had any other way. "LO writes it"
is not a reason; "LO gets it wrong without it" is.

**R2 and R5 are not the same direction.** We validate what we *write*; we do not validate
what we read, and must not — five of the eight R7 documents are invalid against the 1.4
schema (an `office:version` of 1.3, a `table:table` with no `table:table-column`) and every
one of them must still load. Strictness on the way out, tolerance on the way in.

**R6 is the one that is not met today.** The writer regenerates a document from the model, so
opening a 482-line LibreOffice file and setting one cell writes 13 lines: correct ODF,
correct values, and everything the model does not carry silently gone. That is both a diff
problem and a fidelity problem, and phase 8 is where it is fixed.

### One reframing, then I'll stop

"Rewrite LibreOffice core in Rust" is not what this plan does. LO's core is ~226k lines in
`sc/source/core` alone, plus VCL, UNO, sfx2, svl, editeng and a 30-year service
architecture — and its shape is a consequence of goals you explicitly reject. Clean-room
means you are not porting anything; you are **building a new spreadsheet that implements
ODF directly, using the OASIS specs as the source of truth and LO as the conformance oracle
and test corpus.** Smaller, more likely to finish, and it's what your ODS spec is already
written for.

Consequence of clean-room: the license is a free choice, not an inheritance — a port would
have locked the project to MPL-2.0 forever. It is **AGPL-3.0-or-later**: the strongest
copyleft, and compatible with the MIT/Apache-2.0 Rust ecosystem (GPL-*2.0* would not be —
Apache-2.0's patent clause conflicts with it).

The reason for AGPL over plain GPL is specific to *this* project. A spreadsheet **core** is
exactly the asset someone embeds in a hosted service, and plain GPL asks nothing of them:
running modified code on a server is not distribution. AGPL §13 closes that — anyone
offering a modified version over a network must publish their source. The planned wasm shell
makes the hosted case concrete rather than theoretical.

Three consequences to plan around:

- **The Mac App Store is out for `ui_mac/`.** Apple's terms conflict with GPLv3 §6 — the
  reason VLC was pulled. It ships as a direct download. This applies to plain GPL-3.0 too;
  it is not a cost of choosing AGPL.
- **AGPL deters some corporate users and contributors** — a number of companies ban it by
  policy. That is the price of §13, paid knowingly.
- **Code cannot flow to LibreOffice**, which takes MPL-2.0. Not a loss: clean-room means
  nothing flows either way anyway.

Do not use the LibreOffice name or branding. The repository is REUSE-compliant; see
`CONTRIBUTING.md`.

---

## The missing input: ODF 1.4 Part 4 (OpenFormula)

`OpenDocument-v1.4-schema.rng` in this repo is **Part 3** — the content schema. It does not
specify formulas; `table:formula` is an unrestricted string as far as it's concerned
(§4 of your ODS spec says exactly this). The formula language is a **separate normative
specification**:

> [OpenDocument v1.4 Part 4: Recalculated Formula (OpenFormula) Format](https://docs.oasis-open.org/office/OpenDocument/v1.4/os/part4-formula/OpenDocument-v1.4-os-part4-formula.html)
> ([PDF](https://docs.oasis-open.org/office/OpenDocument/part4-formula/OpenDocument-v1.4-os-part4-formula.pdf))

Download it alongside the RNG. It gives you, normatively and per-function: syntax, argument
types and counts, return types, **implicit conversion operators** (Text/Number/Logical/
DateSequence coercion), error semantics, and constraints. This is the ODF-native equivalent
of what the RNG did for the content model, and it's what makes a from-scratch engine a
transcription job rather than a research project.

**It also defines your scope line for you.** Part 4 specifies three normative evaluator
tiers:

| Tier | Functions | Contents |
|---|---|---|
| **Small Group** | **110** — exact list in `doc/small-group.md` | Arithmetic, logical, text, date/time, lookup, information, *and* 12 database + 10 financial |
| **Medium Group** | +~150 | The rest of financial (54 total), statistical (65 total), matrix |
| **Large Group** | +~50 | Complex numbers (26), inline arrays, specialised conversions |

The spec ships 409 function definitions across: Mathematical 69 · Statistical 65 ·
Financial 54 · Information 32 · Text 26 · Complex 26 · Date/Time 23 ·
Number-representation conversion 17 · Database 13 · Lookup 12 · Rounding 8 · Logical 7 ·
Matrix 6 · Byte-position text 6 · Bit ops 5 · External 2.

**Small Group conformance is the Phase 4 exit criterion.** §2.3.2 enumerates it exactly, so
`doc/small-group.md` is generated from the spec rather than estimated: 110 functions with a
normative section reference each, plus §2.3.2's requirements on syntax (B), implicit
conversions (C) and operators (D — all of them except reference union `~`). It explicitly
*excludes* inline arrays, complex numbers and `~`, and need not evaluate multi-area
references.

Small Group is broader than a first guess suggests — it already contains the 12 database
functions and 10 of the financial ones. Medium Group is a later decision, made category by
category on evidence. Large Group is probably never — 26 complex-number functions is
precisely the bloat you're building this to avoid.

---

## Formulas: own the whole thing

I initially proposed building on [formualizer](https://github.com/psu3d0/formualizer) or
[IronCalc](https://github.com/ironcalc/IronCalc) (both MIT/Apache-2.0, 400+ functions, and
formualizer is nicely split into parse/eval/workbook). **Don't.** Both are Excel-syntax and
xlsx-oriented, and the divergence between Excel and OpenFormula is not confined to syntax
where a translation layer could contain it. It's in the semantics:

- error model and propagation (OpenFormula's error set vs Excel's)
- text→number coercion in arithmetic contexts
- empty-cell vs zero vs empty-string handling
- `table:null-date` (per-document, default 1899-12-30) vs Excel's 1900/1904 systems and the
  1900 leap-year bug
- reference intersection/union operators (`!`, `~`) vs Excel's space and comma
- comparison case-sensitivity and locale collation
- per-function argument optionality and edge-case returns

A syntax translator lets all of that leak through, and leaves you fighting an upstream whose
conformance target is Excel — on the one axis where this project has no room to compromise.
Depending on them would mean the thing that distinguishes the app is the thing you don't
control.

So: **write the engine, driven by Part 4 and verified by LO's corpus.** The scale is far
smaller than LO's 50k lines suggests, because LO's number includes OpenCL paths, the
`ScInterpreter` machinery, xls/xlsx compatibility layers and 30 years of accreted edge
cases. Small Group is ~90 functions, most 5–30 lines of Rust against a good value model.
The value model and coercion rules are where the real thinking goes, and they're maybe
1,500 lines — but they're also the entire reason for doing this.

The pieces, and where the effort actually lands:

| Piece | Size | Note |
|---|---|---|
| Tokenizer + Pratt parser + AST | ~1.5k | OpenFormula grammar directly. `[.A1]`, `[Sheet1.A1:.B2]`, `of:=`, `;` separators, inline arrays |
| Value model + implicit conversions + error set | ~1.5k | **The load-bearing part.** Part 4's conversion operators, verbatim |
| Dependency graph, dirty propagation, cycle → iterative calc | ~500 | Format-agnostic. `petgraph` for the graph |
| Small Group function library | ~3k | ~90 functions, corpus-driven, in usage order |

Reuse where it's genuinely neutral: `petgraph` for the dependency graph, `chrono` for
calendar arithmetic under your own serial-date layer. Read formualizer's and IronCalc's
evaluator structure as prior art — they solved real problems around recalc scheduling and
array handling. Read; don't depend.

---

## What to reuse elsewhere

Ladder, in order — the plan is deliberately lazy everywhere formula semantics aren't at stake:

| Need | Use | Not |
|---|---|---|
| Streaming XML, namespace-aware | `quick-xml` — resolves to `(uri, local_name)`, exactly §8.1's requirement | a DOM, XPath, serde-xml |
| ODF package | `zip` | hand-rolled ZIP |
| Dependency graph | `petgraph` | your own graph |
| Calendar arithmetic | `chrono`, under your own `table:null-date` serial layer | your own calendar |
| CLI | `clap` | anything else |
| Existing ODS crates | **read as reference**, don't depend on | `spreadsheet-ods` / `spsheet` — honest about being partial, and tolerant reading (§9) is the whole point |
| Number formatting | **write it** — ~20k lines in `svl/source/numbers`, no Rust equivalent, and §5.2 already specs the ODF half | |
| Sparse column storage | **write it** — see below | |

**Column storage is worth building carefully.** LO Calc stores each column as
`mdds::mtv::soa::multi_type_vector` — run-length-blocked, struct-of-arrays, type-segmented
(`sc/inc/mtvelements.hxx:141-153`, `sc/inc/column.hxx:164`). Blocks of homogeneous type
(numeric, string, formula, empty), O(log n) position lookup, cheap bulk ops. This is why
Calc handles a million rows, there's no Rust equivalent, and it's a few hundred lines. A
`HashMap<(row,col), Cell>` works fine until it doesn't, and then it's everywhere.

---

## Crate layout

Fresh repo. Do **not** fork this one — 18GB of git history and 519k commits you'll never
use. Keep `core/` checked out separately as a read-only reference and test corpus.

```
core/          document model, ODF I/O, formula engine, number formats. No UI.
cli/           the `sheet` binary — the parity ratchet and how agents/CI drive everything
```

That's it for now. `ffi/`, `ui_*` come later, copied from `editor`'s working shapes.
Empty crates now are scaffolding for later; later can scaffold for itself.

```
core/src/
  grid.rs        the multi-type column store — the one hand-built data structure
  model.rs       Document, Sheet, Cell, CellValue, styles-by-reference
  action.rs      Action enum + inverse (undo/redo, command pattern)
  odf/
    package.rs   zip + mimetype + manifest (spec §1) — GENERIC, reused by odt later
    names.rs     (uri, local_name) -> token dispatch table (spec §8.1)
    context.rs   the ElementContext stack + default-ignore context (spec §8)
    read.rs      per-element contexts for the ODS content model (spec §3)
    write.rs     serializer + style pooling (spec §5.3)
  numfmt/        number:*-style parse + apply (spec §5.2)
  formula/
    lex.rs       OpenFormula tokenizer
    parse.rs     Pratt parser -> AST (Part 4 grammar)
    value.rs     Value, error set, implicit conversions (Part 4) -- the load-bearing file
    graph.rs     dependency graph, dirty propagation, iterative calc
    funcs/       Small Group, one module per Part 4 category
    serialize.rs AST -> of:= text (write path)
  lib.rs         pub struct App — Arc<App>, RwLock inside, observer trait
```

The `odf/` split follows the spec's own `[GENERIC]`/`[ODS]` marking, so odt later reuses
`package.rs`, `names.rs`, `context.rs` unchanged (spec §10).

---

## Rules carried over from `editor`

From `doc/shared-core-native-shell.md` §2. Cheap now, expensive later:

1. **Reads go through a windowed API.** `get_viewport(sheet, rows, cols)`. Never a getter
   that hands a shell the whole document — the grid store exists precisely so this is the
   only sane access pattern.
2. **Undo/redo lives in the core**, command pattern, inverse falls out of `Action`. Cell
   edits, structural changes, style changes alike.
3. **The core pushes, shells never poll.** Observer trait, `Send + Sync`. Critically:
   **drop the write lock before notifying** — `editor`'s
   `an_observer_may_read_the_editor_without_deadlocking` test exists because this deadlocks
   otherwise. Write that test on day one.
4. **Whatever any GUI can do, the CLI can do.** A UI-only feature is a bug. Free to maintain
   while there's no GUI, and it'll save you later.
5. **Do not assume a filesystem.** Pair `load_file`/`save_file` with
   `load_bytes(name, &[u8])` / `save_to_bytes()` from the start. A wasm shell can't be
   retrofitted without it.
6. **Shape the core's API for Rust; FFI annotations go in a separate facade crate** later.

One new rule, and it's the anti-bloat ratchet:

7. **Every feature must survive a round-trip through LibreOffice unchanged.** If you can't
   write it such that LO reads it back correctly, and read what LO writes, it isn't done.
   Machine-checkable, which is what makes it a rule rather than an aspiration.

---

## Verification: specs define, LibreOffice verifies

This is what makes the project tractable, and why doing this *in* the LO checkout was a good
instinct even though the code isn't the part you need.

`/usr/bin/soffice` is installed. `cargo` 1.99.0-nightly is installed. The corpus:

| Corpus | Size | Use |
|---|---|---|
| `sc/qa/unit/data/functions/**/fods/` | **509 files**, categorised (mathematical, text, statistical, financial, date_time, logical, information, lookup, array, database) | Formula conformance. Per-function `.fods` with expected values embedded — in the exact format the reader must parse. The formula test suite, pre-written, in ODF. |
| `sc/qa/unit/data/ods/` | 303 files | Reader tolerance against real documents. |
| `sc/qa/unit/data/fods/` | 17 files | Ground truth for flat-form writing (spec §1.2 already cites these). |

Note the categories map almost one-to-one onto Part 4's function groups — so the corpus
directory structure *is* a conformance report layout.

Three loops, each a script plus a test:

**Loop A — read tolerance.** Every file in `ods/` + `fods/`: read it, assert no panic, no
error. A smoke test of §8/§9's default-ignore architecture; it should pass *by construction*
if the context stack is right. If a file needs special-casing, the architecture is wrong —
fix the architecture, not the file.

**Loop B — formula conformance.** For each of the 509 fixtures: load, recalculate every
formula cell from scratch, compare against the cached `office:value` already in the file.
Produces a per-function pass/fail table, grouped by Part 4 category, with Small/Medium tier
membership marked. This is the formula scoreboard and the project plan for Phase 4 — you
implement in the order the scoreboard says, and it tells you when Small Group is done. Runs
from day one of the formula phase; the number goes up and never down.

**Loop C — round-trip differential.** The strictest:

```
your_file.ods --[soffice --headless --convert-to fods]--> lo_reread.fods
```

Write a document, have LO convert it, read LO's output back, assert semantic identity. Then
the reverse: take an LO-authored file, read it, write it, convert both, diff. Catches the
whole class of "schema-legal but LO renders it differently" bugs that reading the spec
cannot. Also the enforcement mechanism for rule 7 — a feature that doesn't round-trip fails
CI, so the feature line is defended by a machine instead of by discipline.

Plus ordinary `cargo test`: the grid store's block splitting/merging, the OpenFormula
parser/serializer (property-tested on AST round-trip), the conversion operators from Part 4,
number format parse/apply.

---

## Phases

Each phase has an exit criterion. Don't start the next before it's met. Phases 8 and 9 were
one phase until the requirements above were written down; R6 turned out to be a phase of its
own, and one that has to land before anything types into the core.

### Phase 0 — Specs in hand, harness standing (small)

Download **ODF 1.4 Part 4** next to the Part 3 RNG. Re-read `ODS_CLEAN_ROOM_SPEC.md`
§§1–10 with code in mind. Stand up the repo and workspace, and **Loop A's runner as a stub
that fails** — harness before code, so progress is measurable from the first commit.

Write the clean-room rule into `CONTRIBUTING.md`: LO source may be read and cited by
file+line; never pasted. Every derived fact goes through a spec document first (your ODS
spec is the template). This is what keeps the license choice honest and an agent's work
auditable.

**Exit:** `cargo test` runs, Loop A enumerates 320 files and reports 320 failures.

### Phase 1 — Grid + model + actions, no I/O

`grid.rs` (multi-type column store), `model.rs`, `action.rs` with undo/redo, the observer
trait, the deadlock test. Pure in-memory. `App` API takes `&self` throughout, `RwLock`
inside. Get `get_viewport` right here — every future shell depends on it, and it's the
reason the column store exists.

**Exit:** store block splitting/merging tested, undo/redo round-trips arbitrary action
sequences, deadlock test passes.

### Phase 2 — ODF read (spec §§1, 3, 8, 9)

`package.rs`, `names.rs`, `context.rs`, `read.rs`. Both flat and package form. Build the
context stack with the **default-ignore fallback first**, before any element-specific
context — tolerance then falls out of the design instead of being a feature.

Values, value types, repeated rows/columns (`table:number-columns-repeated` is a correctness
trap, not an optimisation), sheets. No styles or formula evaluation yet: a formula cell
reads as its cached value, and its `table:formula` string is retained verbatim.

**Exit:** Loop A green on all 320 files. Cell values correct on a hand-checked subset.

### Phase 3 — ODF write + round-trip (spec §§1.3, 5.3, 7)

`write.rs` with style pooling. Flat form first (fewer moving parts; §7.1/7.2 give minimal
templates verbatim), then package form (§7.3).

**Exit:** Loop C green for value-only documents, both directions.

### Phase 4 — The formula engine (Part 4 + spec §4)

The biggest phase, and the one that decides whether the project is real. Order matters:

1. **`value.rs` first** — the value model, error set and Part 4 implicit conversions, with
   unit tests straight out of the spec's conversion tables. Everything downstream inherits
   its correctness from this file. Do not shortcut it.
2. **`lex.rs` + `parse.rs` + `serialize.rs`** — OpenFormula grammar. Reference syntax per
   your ODS spec §4 (`[.A1]`, `[Sheet1.A1:.B2]`, `$` absolutes), `of:=` prefix,
   `COM.MICROSOFT.*` vendor names parsed and preserved even when unevaluable. Property-test
   parse→serialize→parse.
3. **`graph.rs`** — dependency graph, dirty propagation, topological recalc, cycle
   detection feeding `table:calculation-settings` iterative mode.
4. **`funcs/`** — Small Group, one module per Part 4 category, implemented in the order
   Loop B's scoreboard prioritises. Cached-result emission on write is not optional (spec §4
   — an omitted cached value renders blank in LO).
5. Named ranges (`table:named-expressions`).

**Exit:** **conforming Small Group evaluator** — Loop B green across the ~90 Small Group
functions. Medium Group categories reported honestly as a published table, not hidden.

### Phase 5 — Styles and number formats (spec §5)

Cell styles, the ~7 `number:*-style` families, `style:data-style-name` indirection, the
pooling rule (§5.3). This is where "usable daily driver" starts being true — a spreadsheet
that can't show a date as a date isn't one.

**Exit:** Loop C green including styles and formats. Documents written by the tool look right
in LO — checked by eye once, then by Loop C forever.

### Phase 6 — The CLI (`sheet`)

The parity ratchet. Non-interactive, parseable stdout, diagnostics to stderr, non-zero exit
on failure — copy `editor`'s CLI conventions wholesale, including `--format json` and the
0-based-core / 1-based-CLI conversion happening in exactly one file.

Subcommands roughly: `new`, `set`, `get`, `view`, `recalc`, `import`/`export`, `info`, `fmt`.
Plus `--session` for undo across stateless invocations, as `editor` does.

**Exit:** every core capability reachable from the CLI, verified by a test that walks the
public API and fails on anything unexposed.

### Phase 7 — Stop

Before any GUI, write down what the app does **not** do, as a product document rather than a
backlog. That list is the reason this project exists.

**Exit:** `doc/not-doing.md` — never, not yet with the gate that owns it, and where each
capability that exists stops. Done.

### Phase 8 — The diffable writer (R6)

The writer regenerates a document from the model, which is right for a document this program
authored and wrong for one it opened. Everything the model does not carry — `office:meta`,
`office:settings`, styles nothing references, an `office:font-face-decls`, a chart, another
vendor's extension — is dropped, and the diff is the whole file. R6 says that is not
acceptable, and R6 is also the fidelity requirement wearing different clothes: a writer that
touches only what changed cannot lose what it never understood.

**The approach is retain-and-splice, not a fuller model.** Carrying every unknown element as
a shadow tree means the model grows to the size of ODF, which is the thing this project is
built to avoid. Instead:

1. The reader keeps the **source bytes** on the `Document`, and records the byte span of each
   `table:table-cell` element it read. `quick-xml` already reports buffer positions, so this
   is bookkeeping rather than a second parse.
2. `write_bytes` with a retained source and a change set that is only cell edits **splices**
   freshly serialised cells into the original bytes at their spans. Every other byte —
   indentation included — is the file that came in. One number changed is one line changed.
3. It **falls back to full regeneration**, loudly and by one named rule, when there is no
   source, when the form changed, when a cell arrived that did not exist, or when the target
   sits inside a `number-columns-repeated` / `number-rows-repeated` element that would have
   to be split. A fallback that fires silently would make the requirement untestable.
4. The package form is the same operation against `content.xml`, rewriting the zip with every
   other entry passed through untouched.

**Exit:** opening each of R7's eight documents, setting one cell, and writing it back
produces a diff of a **single element** against the original, and the file still validates
(R2) and still round-trips through LibreOffice (loop C). The fallback path is asserted
directly rather than left to be inferred.

### Phase 9 — Shells

One native shell, following `editor` §3's order of work with its GTK shell as the worked
example. Then the wasm shell, which is the honest test of rule 5.

Phase 8 comes first on purpose: a shell is a thing that saves on every keystroke, and a
writer that rewrites the whole file is a diff problem that only gets worse once something is
typing into it.

---

## The feature line (starting proposal — yours to set)

**In (the 90%):** multiple sheets · cell values and types · OpenFormula Small Group, growing
into Medium by category on evidence · cell/row/column formatting and number formats · sort
and filter · find/replace · freeze panes · one chart type that round-trips · CSV
import/export · print to PDF via the platform.

**Out, permanently, and named so nobody has to ask:** macros and Basic · extensions ·
database ranges and data sources · pivot tables (revisit once, later) · change tracking ·
OLE embedding · `.xls`/`.xlsx` write (read-only via `calamine` if ever) · Draw objects beyond
images · scenarios · goal seek and solver · sparklines · conditional formatting beyond a
single rule type · OpenFormula Large Group (complex numbers, 26 functions of pure bloat).

Anything on the "out" list that a real week of use proves necessary can move — by explicit
decision, one item at a time, and it must pass Loop C. That's the anti-bloat mechanism: not
taste, a gate.

---

## Risks, honestly

- **The formula engine is the schedule.** Phases 0–3 are weeks; Phase 4 is months. Mitigated
  by Small Group being a bounded, normative, ~90-function target with a 509-file corpus
  proving it, rather than an open-ended "implement spreadsheet functions." Loop B makes
  progress a number from the first week.
- **`value.rs` is the single point of failure.** Get the conversion operators and error
  semantics wrong and every function inherits the bug. It's the one file to over-test.
- **Number formats are a swamp.** ~20k lines in LO. But the ODF *declarative* half (§5.2) is
  far smaller than LO's format-code parser, and you only need the declarative half. Implement
  `number:*-style`, not LO's format-string language.
- **Interop expectations.** Users will hand it `.xlsx`. Read-only via `calamine` is a cheap
  escape hatch; writing xlsx is not, and is on the "out" list for a reason.
- **This is multi-year even scoped this way.** Phases 1–4 prove it's real. If Loop B hits
  Small Group and Loop C round-trips, the rest is work rather than risk. If Phase 4 stalls,
  that's the honest signal to stop.

---

## First commit

New repo. Workspace with `core/` and `cli/`. `CONTRIBUTING.md` with the clean-room rule.
`ODS_CLEAN_ROOM_SPEC.md` copied in as `doc/ods-format.md`, ODF 1.4 Part 4 downloaded
alongside it. Loop A's runner, pointed at this checkout's `sc/qa/unit/data/`, failing on all
320 files.

Then Phase 1, which touches no XML at all.
