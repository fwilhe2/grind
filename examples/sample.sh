#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
#
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Build a sample document that uses **every feature this build supports**, through the CLI
# and nothing else. The document itself is a small household budget — obviously made-up
# numbers, but shaped the way a real one is: a category table with a running total, a
# transaction pasted in after the fact, and a journal sheet that reaches back across to it.
#
#   examples/sample.sh [output-directory]        # default: ./sample
#   SHEET=target/debug/sheet examples/sample.sh  # a binary other than the one on PATH
#
# This is a living inventory, not a demo: `cli/tests/cli.rs` runs it and fails the build if
# it stops working, and a feature that lands without a line here is a feature nobody can
# see. When you add one, add it below — that is the whole maintenance rule.
#
# What it deliberately cannot show, because nothing can do it yet: moving a sheet; cell
# fonts; CSV.

set -euo pipefail

SHEET=${SHEET:-sheet}
out=${1:-sample}
mkdir -p "$out"
book="$out/sample.ods"
session="$out/session.json"
rm -f "$book" "$session" "$out/sample.fods"

say() { printf '\n=== %s\n' "$1"; }
run() { "$SHEET" "$@" >/dev/null; }

# --- the document ------------------------------------------------------------------------

say "a new document"
run new "$book" --force
run rename "$book" Sheet1 Budget

say "values: every type a cell can hold, one row per budget category"
run set "$book" A1 'Category'
run set "$book" B1 'Budgeted'
run set "$book" C1 'Actual'
run set "$book" D1 'Difference'
run set "$book" E1 '% of Total'
run set "$book" F1 'Paid'
run set "$book" G1 'Ref'
run set "$book" H1 'Notes'
run set "$book" A2 'Housing'
run set "$book" A3 'Groceries'
run set "$book" A4 'Transport'
run set "$book" A5 'Utilities'
run set "$book" A6 'Entertainment'
run set "$book" A7 'Savings'
run set "$book" B2 1800
run set "$book" B3 500
run set "$book" B4 220
run set "$book" B5 260
run set "$book" B6 80
run set "$book" B7 400
run set "$book" C2 1825.50
run set "$book" C3 612.34
run set "$book" C4 179.85
run set "$book" C5 238.10
run set "$book" C6 -12.50          # a cash-back refund; a leading hyphen is a number, not a flag
run set "$book" C7 400
run set "$book" F2 FALSE           # a logical: rent not fully settled yet
run set "$book" F3 TRUE
run set "$book" F4 TRUE
run set "$book" F5 TRUE
run set "$book" F6 TRUE
run set "$book" F7 TRUE
run set "$book" G2 '2091' --text   # forced to text, or it would be the number 2091
printf 'rent increase\nstarting July' | run set "$book" H2 -   # from stdin, newline and all

say "paste: tab-separated rows into a rectangle, in one undo step"
printf 'Subscriptions\t45\nCharity\t25\n' | run paste "$book" A21 -
"$SHEET" view "$book" A21:B22 --raw

say "formulas: OpenFormula syntax, stored verbatim"
run set "$book" A8 'Total'
run set "$book" B8 '=SUM([.B2:.B7])'                        # math
run set "$book" C8 '=SUM([.C2:.C7])'
run set "$book" D8 '=[.B8]-[.C8]'
run set "$book" D2 '=[.B2]-[.C2]'                            # per-category over/under, filled below
run set "$book" E2 '=[.C2]/[.$C$8]'                          # an absolute reference to the total
run set "$book" A9 'Largest expense'
run set "$book" B9 '=MAX([.C2:.C7])'                         # statistical
run set "$book" A10 'Groceries actual'
run set "$book" B10 '=VLOOKUP("Groceries";[.A2:.C7];3;FALSE())' # lookup, unsorted
run set "$book" A11 'Categories paid'
run set "$book" B11 '=COUNTIF([.F2:.F7];TRUE())'             # a criterion
run set "$book" A12 'Paid so far'
run set "$book" B12 '=SUMIF([.F2:.F7];TRUE();[.C2:.C7])'     # a criterion over a sum range
run set "$book" A13 'Average spend'
run set "$book" B13 '=AVERAGE([.C2:.C7])'
run set "$book" A14 'Label'
run set "$book" B14 '=[.A2]&" & "&[.A3]'                     # text, via the concatenation operator
run set "$book" A15 'Spend ratio'
run set "$book" B15 '=IF([.B8]=0;0;[.C8]/[.B8])'             # short-circuit
run set "$book" A16 'Mortgage payment'
run set "$book" B16 '=ROUND(PMT(0.0625/12;360;-350000);2)'   # financial, rounded
run set "$book" A17 'Statement date'
run set "$book" B17 '=DATE(2026;8;16)'                       # a date, as a serial number
run set "$book" A18 'Statement time'
run set "$book" B18 '=TIME(9;30;0)'
run set "$book" A19 'Is category text'
run set "$book" B19 '=ISTEXT([.A2])'                         # information

# The twin of paste: replicate one cell's formula across a rectangle, the way a drag handle
# or Ctrl+D does. Relative references shift with the target; `$`-anchored ones do not — D2's
# [.B2]-[.C2] becomes [.B5]-[.C5] at row 5, but E2's [.$C$8] stays pinned to the total row
# wherever it lands.
say "fill: replicate a formula down a column, relative refs shifting and $ ones pinned"
run fill "$book" D2 D3:D7
run fill "$book" E2 E3:E7
"$SHEET" get "$book" D5 --formula
"$SHEET" get "$book" E5 --formula

say "number formats: display only, the value never moves"
run format "$book" B2:C8 currency --symbol '€' --grouping --decimals 2
run format "$book" E2:E7 percent --decimals 1
run format "$book" B17 date
run format "$book" B18 time
run format "$book" F2:F7 boolean
run format "$book" B9 general                               # back to the plain value
run format "$book" B11 number --decimals 0                   # a count, not money
run format "$book" B12 currency --symbol '€' --grouping --decimals 2
run format "$book" B13 currency --symbol '€' --grouping --decimals 2
run format "$book" B15 percent --decimals 1
run format "$book" B16 currency --symbol '€' --grouping --decimals 2 --locale de-DE  # 2.155,01 €

# Colours may be named as well as spelled in hex: the names are the palette in
# `style::PALETTE`, which lives in the core so that a name here and a swatch in a GUI write
# the same attribute. Anything outside it is still a plain #rrggbb.
say "cell styling"
run style "$book" A1:H1 --bold --background silver --align center --border '0.5pt solid navy'
run style "$book" A2:A7 --italic --color navy
run style "$book" H2 --wrap --valign top --size 9pt
run style "$book" B8 --bold --align right

# A length is ODF's own, in whatever unit it was written — LibreOffice respells them all in
# centimetres on the way through, which is its business rather than the document's.
say "column widths and row heights"
run width "$book" A:A 4cm
run width "$book" H:H 3.5cm
run height "$book" 1:1 8mm

say "hide: a column or row by hand"
run hide "$book" D
"$SHEET" hide "$book"
# Left hidden, so the GTK shell's own marker over D has something to show and unhide.

say "recalculate the whole document"
run recalc "$book"

# --- reading it back ---------------------------------------------------------------------

say "view: what a person sees"
"$SHEET" view "$book" A1:H8

say "view --raw: what the file stores"
"$SHEET" view "$book" B2:C4 --raw

say "get: one cell, its stored value, and its formula"
"$SHEET" get "$book" B17
"$SHEET" get "$book" B17 --raw
"$SHEET" get "$book" B8 --formula
"$SHEET" get "$book" B8 --input                              # what an editor would show
"$SHEET" get "$book" G2 --input                               # and the ' that keeps 2091 text

# Setting a style replaces it, so "bold as well" is a read, a field and a write — which is
# exactly what a toolbar's bold button does, and it needs this to read first.
say "--show: how a cell looks, and how its value is spelled"
"$SHEET" style "$book" A1 --show
"$SHEET" format "$book" B16 --show                           # the flags that recreate it
"$SHEET" width "$book" A:H                                    # only the columns that were sized

# A name is what makes a formula say what it means: `SUM(budgeted)` rather than
# `SUM([.B2:.B7])`. An address becomes a named *range*, written absolute and
# sheet-qualified so it means the same thing read from anywhere; a target starting with `=`
# becomes a named *expression* instead, and one name may build on another.

say "name: a named range, and an expression over it"
run name "$book" budgeted B2:B7
"$SHEET" name "$book" budgeted
run name "$book" biggestBudget '=MAX(budgeted)'
run set "$book" J1 '=SUM(budgeted)'
run set "$book" J2 '=biggestBudget'
"$SHEET" view "$book" J1:J2 --raw

# Which rows a filter hides is derived from the values, never stored — so it follows an
# edit without anyone recomputing it, and the file's `table:visibility="filter"` marks are
# written from the same answer a shell draws from.

say "filter: keep a set of values in one column, hide the other rows"
run filter "$book" A1:H7 A=Groceries A=Transport
"$SHEET" filter "$book"
run filter "$book" --clear
# Put it back, so the document this script leaves behind actually has one to look at: the
# GTK shell draws a dropdown button in every heading cell of the range.
run filter "$book" A1:H7 A=Groceries A=Transport

# A second sheet, and a formula on it reaching across to the first. Renaming does **not**
# rewrite the formulas that mention the old name — they go stale instead, which the warning
# on stderr says out loud and `sheet recalc` turns into an error. Deleting is undoable: the
# inverse carries the whole sheet, so the cells come back.

say "sheets: add, write across one, rename, and delete"
run add "$book" Journal
run set "$book" Journal.A1 'category count check'
run set "$book" Journal.B1 '=COUNT([$Budget.$B$2:.$B$7])'
run rename "$book" Journal Archive
"$SHEET" view "$book" Archive.A1:B1 --raw
run --session "$session" remove "$book" Archive
run --session "$session" undo "$book"

say "info: sheets, extents, formula counts, named expressions"
"$SHEET" info "$book"

say "json: every command is machine-readable"
"$SHEET" --format json get "$book" E2

say "fmt: the stored form, the display form a formula bar shows, and back"
"$SHEET" fmt '=SUM([.A1:.A2])*-2^2'
"$SHEET" fmt --display '=SUM([.A1:.A2])*-2^2'
"$SHEET" fmt --from-display '=SUM(A1:A2)*-2^2'
"$SHEET" fmt --friendly '=RATE([.A1];-100;1000;0;0;0.05)'   # read-only: full names, labelled args
"$SHEET" fmt --friendly --inline '=RATE([.A1];-100;1000;0;0;0.05)'  # the same, on one line

say "eval: what a formula would say at a cell, storing nothing"
"$SHEET" eval "$book" B25 '=SUM([.B2:.B4])*2'

say "calculations: everything the document computes, and what it calls to do it"
"$SHEET" calculations "$book"
"$SHEET" calculations "$book" --filter sum    # by function name, address, sheet or formula text

say "functions: what this build implements"
"$SHEET" functions | tail -1
"$SHEET" functions --long --filter vlookup    # spec signature, friendly one, category, summary

# --- editing safely ----------------------------------------------------------------------

say "dry run: apply and report, write nothing"
"$SHEET" --dry-run set "$book" B2 0

say "session: undo across invocations"
run --session "$session" set "$book" B2 0
"$SHEET" --session "$session" get "$book" B2
run --session "$session" undo "$book"
"$SHEET" --session "$session" get "$book" B2
run --session "$session" redo "$book"
run --session "$session" undo "$book"

say "clear: a cell, a whole rectangle, and a formula while keeping its value"
run set "$book" J3 'scratch'
run clear "$book" J3
run clear "$book" A21:B22                                   # the pasted rows, in one step
run clear "$book" B9 --formula-only

say "convert: the same document as flat XML"
run convert "$book" "$out/sample.fods"

# --- R6: a flat document edits in place ---------------------------------------------------
# Editing a `.fods` rewrites the one element that changed and leaves every other byte alone,
# which is what makes these files live in git the way source files do. Shown against a copy,
# with `diff` counting the lines rather than a claim in a comment.

say "minimal diff: one cell changed, one element rewritten"
cp "$out/sample.fods" "$out/before.fods"
run set "$out/sample.fods" B2 2200                          # a rent increase, mid-year
printf 'changed lines: %s of %s\n' \
  "$(diff "$out/before.fods" "$out/sample.fods" | grep -c '^[<>]')" \
  "$(wc -l < "$out/before.fods")"

# B8 is `=SUM([.B2:.B7])`, so the edit above left the document disagreeing with itself: the
# formula is one claim and its cached value is another, and ODF has no dirty bit to write.
# The warning on stderr is the only thing standing between that and a file everyone —
# LibreOffice included — displays a stale total from. Recalculating is a separate command on
# purpose: this build implements the Small Group, and a document using anything outside it
# would lose good cached values to #NAME?.

say "stale: the edit above invalidated a total, and said so"
"$SHEET" get "$out/sample.fods" B8 --raw
run recalc "$out/sample.fods"
"$SHEET" get "$out/sample.fods" B8 --raw

# What a GUI does on every commit, and what the CLI does only when asked: the edit and the
# recalculation it causes land as one change, so one `undo` takes back both. It is skipped
# — with a warning — when recalculating would replace a cached value this build cannot
# reproduce, which is the same honesty `recalc` prints.

say "set --recalc: the edit and its ripple, in one step"
run set "$out/sample.fods" B2 2250 --recalc                 # a further adjustment
"$SHEET" get "$out/sample.fods" B8 --raw

printf '\n%s and %s\n' "$book" "$out/sample.fods"
