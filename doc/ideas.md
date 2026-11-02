<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: CC-BY-4.0
-->

# Ideas — a document you can read like code

**Status: candidates, unbuilt, nothing decided.** Not normative for anything. `doc/plan.md`'s
requirements, `doc/not-doing.md` and the architecture rules in `CLAUDE.md` outrank every word
here; each item becomes a design document of its own before it becomes code.

## The direction

What already sets grind apart is not parity with LibreOffice but a theme: the projection (view
source), roles and inline names (`doc/view-modes.md`), formulas in plain English, the formula
workbench (`doc/formula-workbench.md`), a CLI that can do everything (R9) and the Rhai generator
(`doc/dsl.md`). Each treats a document as something to **read, check and refactor**, the way
code is. The candidates below push that theme further, mostly on machinery that exists already.
None of them reopens a row of `doc/not-doing.md` §1.

Ranked by what they give for what they cost.

## 1. `grind diff`, and a git driver

`doc/not-doing.md` answers change tracking with "version control over `.fods`", but a textual
diff of flat ODF is XML noise: a style renumbered, a repeated-row run split, an attribute
reordered. A diff that knows what a document *means* makes that answer true.

- **Sheet:** cells whose content changed, with old and new formula and value side by side, and
  how many cells downstream changed with them (`graph::Graph::dependents`). Sheets added, removed
  and renamed; names, rules, merges and charts as their own sections.
- **Text:** blocks added, removed, moved and reworded, matched by content rather than position,
  with a word-level diff inside a changed paragraph.
- **Git:** `grind diff` usable as git's external diff command, documented as a
  `.gitattributes` recipe (`*.ods diff=grind`), so `git diff book.ods` shows the above. A merge
  driver is a later and separate decision; a three-way merge of cells is easy, of blocks is not.
- `--format json` for agents and CI, like every other verb.

Built on: the projection's model readers, `App::get_viewport`, `graph.rs`. Both documents are
read, nothing is written. Verified by diffing every R7 document against itself (empty) and
against a scripted edit of it (exactly that edit).

## 2. Lint rules that catch real spreadsheet mistakes

`sheet/src/lint.rs` has stale values, missing sheets, empty single-cell references, off-palette
colours and spelling. The classic errors in real workbooks are elsewhere:

- **Inconsistent formula:** a cell in a filled run whose formula, read relatively, differs from
  its neighbours' — the overwritten-formula bug. The R1C1-style comparison is the whole rule.
- **Range one short:** a `SUM`/`AVERAGE`/… range that stops one row or column before a
  contiguous block of data of the same kind continues.
- **Hard-coded input:** a literal in a formula equal to a named single cell's value — the name
  was meant (`doc/view-modes.md` §4.2's magic constant, as a diagnostic rather than a role).
- **Unread result:** a computed cell nothing reads, outside the last row or column of its block
  — a hint, off unless asked for.

Diagnostics stay marks, never roles (`doc/view-modes.md` §4.3), so they reach every shell through
the lint surfaces that exist. Each rule needs a corpus measurement of its false-positive rate
before it is on by default.

## 3. Finish the formula workbench: extract (F4)

Select a sub-expression, *extract to cell* or *extract to name*, and the formula is rewritten to
read it. `doc/formula-workbench.md` §3.5 and §5 already specify `App::extract` and when it is
refused, and `doc/ods-format.md` §4.2 measured when extraction changes meaning, so the risky
part is designed. With `grind sheet name --inline`, extract and inline are the refactoring pair
an IDE has.

## 4. The dependency graph, drawn

Select a cell and see arrows to what it reads and what reads it, one level at a time, as a view
mode like roles and names. `graph.rs` answers both questions already; what is missing is the
overlay (`doc/view-modes.md` §2) and a CLI form (`grind sheet trace-deps A1 --depth 2`, R9).
The most visible of the five, and the cheapest once the overlay convention is settled.

## 5. Live editing through the server

`doc/server.md`'s Tier 4: two tabs, or two shells, editing one document, with the core's push
notifications carried over a WebSocket and the rebase loop in `grind_core::collab`. The only
candidate here that is a project rather than a few weekends, and it waits on `doc/server.md`'s
open questions.

## Where to start

1 and 2 together. They make "a spreadsheet is code" concrete for a person at a terminal and for
an agent alike, they mostly reuse what is built, and neither writes to a document — so neither
can break R6 or the never-worse rule.
