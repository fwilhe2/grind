<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# What this does not do

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
| **Medium Group, wholesale** | Not never — but never *as a block*. It moves category by category on evidence that a real week of use needed one, which is the same gate as anything else here. |

### Presentation

| Not doing | Because |
|---|---|
| **Fonts** | LibreOffice rewrites `fo:font-family` into an `office:font-face-decls` reference, and nothing here can draw a glyph anyway (`doc/ods-format.md` §5.4). It comes back when a renderer exists, not before. |
| **Conditional formatting beyond one rule type** | The general form is a rule engine with its own evaluation order. One rule type is the whole of the demonstrated need. |
| **Draw objects beyond images** | Shapes, connectors, and their layout model are a drawing application. |
| **Pivot tables** | Revisit *once*, later, as `plan.md` says — and only against a real use, because the honest version is an aggregation engine plus a UI plus a serialisation format. |
| **More than one chart type** | One that round-trips proves the mechanism. The second is taste. |

---

## 2. Not yet

Inside the feature line, unbuilt, and each one is a gate rather than a wish. Nothing here is
half-built and hidden: **no shell can do any of it either**, which is why none of them are
CLI parity gaps.

| Not yet | Owner | Gate |
|---|---|---|
| **Adding, renaming, deleting sheets** | Phase 7 shells | The core has no action for it; the CLI is not hiding one. |
| **Editing named expressions** | Phase 7 shells | Read, resolved and written today; not editable. |
| **CSV import/export** | Phase 7 shells | Format-neutral, and `doc/cli-recipes.md` already drives the import half from the shell. |
| **Sort and filter** | Phase 7 shells | Needs a collation decision first — `eval.rs:503` is code-point order after case folding, not locale collation. |
| **Find/replace** | Phase 7 shells | Trivial over the column store; there is nothing to type into yet. |
| **Freeze panes** | Phase 7 shells | Purely a view concern, and there is no view. |
| **One chart type** | Phase 7 shells | Must round-trip through LibreOffice like everything else. |
| **Print to PDF** | Phase 7 shells | Via the platform, so it needs a platform. |
| **Incremental recalculation** | When a UI makes whole-document recalc feel slow | `eval.rs:16`. `graph.rs` is in the plan and unbuilt on purpose; recursion-with-memoisation is the topological order today. |
| **Reading `.xlsx`** | Never scheduled, always allowed | Read-only, via `calamine`, if a real week of use demands it. |

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
| **`NOW` / `TODAY`** | UTC, not the host's local time. | `date.rs:283` |
| **String comparison** | Code-point order after case folding, not locale collation. §6.4.9 permits it. | `eval.rs:503` |
| **Corrupt-zip recovery** | Unbuilt. No corpus file needs it, and it belongs with the spec's explicit repair mode. | `CLAUDE.md` |
| **Loop C's `back` direction** | Skips formula-bearing documents. | `roundtrip.rs` |
| **Named expressions** | One flat map, so a sheet-local name is visible document-wide. | `model.rs:258` |

The `ponytail:` comments are the full ledger and outnumber this table; these are the ones a
*user* would notice.

---

## The point

Every row above is a thing that will not be in the way — of the code, of a reviewer, or of
someone deciding whether this program does what they need. A feature list that ends is only
credible if the ending is written down, and this is where it is written down.
