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
| **B** — formula conformance | recalculating each fixture matches the cached value already in the file | phase 4 | 509 per-function `.fods` in `functions/**/fods/` |
| **C** — round-trip differential | write → `soffice --headless --convert-to` → read back → semantically identical, and the reverse | phase 3 | — |

Loop A currently reports **358 read, 3 password-protected, 0 failed**. Encrypted documents are
its *one* accepted non-success outcome, named explicitly in the test rather than filtered away
— every other error still fails the loop.

Loop C is also the enforcement mechanism for the anti-bloat rule: a feature that does not
survive a LibreOffice round-trip fails CI, so the feature line is defended by a machine
instead of by discipline.

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
  rejected document.

## Conventions

- **Positions are 0-based in the core.** Only the CLI is 1-based, and it converts in exactly
  one place.
- **`ponytail:` comments** mark deliberate shortcuts with a known ceiling and their upgrade
  path (e.g. text and bool sharing one block; linear block lookup). They are a tracked ledger,
  not apologies — do not silently "fix" one without reading the reason.
- Deferred on purpose, and documented where it lives: dates/times keep their ISO strings until
  `table:null-date` exists in phase 4, because guessing an epoch now bakes in exactly the
  Excel-shaped assumption this project exists to avoid. Corrupt-zip recovery is unbuilt
  because no corpus file needs it and it belongs with the spec's explicit repair mode.

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

Phases 0–2 are done: specs and harness, the column store and undo/redo, and the reader with
loop A green. **Phase 3 is next** — the writer plus loop C. Phase 4 (the formula engine, whose
exit criterion is a conforming OpenFormula Small Group evaluator) is the phase that decides
whether the project is real; `doc/plan.md` has the ordering within it, and `value.rs` first is
not negotiable since every function inherits its correctness.
