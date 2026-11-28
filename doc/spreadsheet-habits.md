<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: CC-BY-4.0
-->

# Spreadsheet habits

Joel Spolsky's talk *You Suck at Excel* (2015) is a list of habits that make spreadsheets
brittle and of the features that avoid them. Most of its advice is already the shape of this
program: the habits it warns against are what `grind lint`, the roles overlay and the names
machinery exist to catch. This document checks the talk point by point against what is built,
and names what is worth building and what stays out.

## 1. Already here

| The talk | Here |
|---|---|
| R1C1 mode, to see whether a column of formulas really is one formula | The `inconsistent-formula` lint finds the cell that breaks its line's pattern and says what it computes instead. Nobody has to read R1C1 by eye, so there is no second formula grammar. |
| Absolute references with `$` | `short-range` and `hard-coded-input` catch what a wrong `$` usually leads to. |
| Ctrl+` shows every formula | *Show Formulas* in every client, `grind sheet view --formulas` (`doc/view-modes.md`). |
| Blue for inputs, black for formulas | The roles overlay derives each cell's role from the document — input, computed, cross-sheet, unnamed constant — so nothing is styled by hand and nothing goes stale (`doc/view-modes.md` Part II). |
| Named ranges | Define, rename carrying every use, inline, the name-anchor overlay, formulas read through their names, a number extracted into a named input cell (`doc/calc.md`). |
| `INDEX` + `MATCH` over `VLOOKUP` | Both, and `XLOOKUP`/`XMATCH` (`doc/small-group.md`). |
| Ctrl+D, Ctrl+R, Ctrl+Enter over a range, Ctrl+Arrow | Fill down/right, one cell filled across a rectangle, jump to a block's edge — every window. |
| Paste Values | *Copy Value*, and *Replace Formulas with Values*. |
| Tables | *Format as table* and the autofilter, both `table:database-range`. |
| `ROUND` for money | The function, as Part 4 defines it. |

## 2. Worth building

In the order they are built.

1. **F4 in every window** — built. `formula::assist::cycle_absolute` is the core's; each shell
   binds the key over its own edit line.
2. **Filling a series** — built: `App::extend` (`sheet/src/series.rs`), the GNOME window's fill
   handle and `grind sheet extend`. Two or more numbers with one difference between each pair
   go on by it, rounded to their own decimals; one date steps by a day; dates on the same day of
   each month go on by months (the month's last day when it is shorter), and dates on months'
   last days stay on them. Anything else — text, formulas, an uneven line, one number — is
   repeated, a formula's relative references shifted. Each new cell takes the format and style
   of the cell it continues. Ctrl+D and Ctrl+R still copy: a series is the drag's, as in Excel.
3. **Paste Special.** A pasted rectangle transposed; and a pasted number added to, subtracted
   from, multiplied into or divided into the cells under it — the talk's price rise. Each one
   undo step, with a CLI verb (R9).
4. **A lint for one fact per cell.** A text cell that is a number with a unit or currency beside
   it (`500 USD`, `12 kg`) in a column of numbers: arithmetic over the column skips it.
5. **Double-clicking the fill handle** fills down as far as the column beside the selection
   goes.

A rounding lint — a total whose shown digits differ from the sum of the shown digits above it —
is the talk's `ROUND` advice checked after the fact. It is left until the four above are done,
because it depends on what a number format shows rather than on what a cell holds.

## 3. Staying out

- **Goal Seek and pivot tables** are in `doc/not-doing.md` §1. Goal Seek is the smallest of the
  three it names — one input, one target, a root-finder over the evaluator — but moving it is a
  product decision, not a ticket.
- **Structured references** (`Orders[Total]`) have no OpenFormula syntax; LibreOffice turns them
  into plain ranges. A named range over the column says the same in ODF.
- **R1C1 display**: see the first row of §1.
- **Dragging the fill handle back to clear**: Delete does it.
