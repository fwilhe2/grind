#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
#
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Build a sample document that uses **every feature this build supports**, through the CLI
# and nothing else.
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

say "values: every type a cell can hold"
run set "$book" A1 'Region'
run set "$book" A2 'North'
run set "$book" A3 'South'
run set "$book" A4 'East'
run set "$book" B1 'Revenue'
run set "$book" B2 1234.5
run set "$book" B3 -19.99          # a leading hyphen is a number, not a flag
run set "$book" B4 20000
run set "$book" C1 'Share'
run set "$book" D1 'Open'
run set "$book" D2 TRUE            # a logical
run set "$book" D3 FALSE
run set "$book" D4 TRUE
run set "$book" E1 'Note'
run set "$book" E2 '007' --text    # forced to text, or it would be the number 7
printf 'first line\nsecond line' | run set "$book" E3 -   # from stdin, newline and all

say "paste: tab-separated rows into a rectangle, in one undo step"
printf 'West\t9999\nCentral\t8888\n' | run paste "$book" A16 -
"$SHEET" view "$book" A16:B17 --raw

say "formulas: OpenFormula syntax, stored verbatim"
run set "$book" B5 '=SUM([.B2:.B4])'                       # math
run set "$book" C2 '=[.B2]/[.$B$5]'                        # an absolute reference
run set "$book" C3 '=[.B3]/[.$B$5]'
run set "$book" C4 '=[.B4]/[.$B$5]'
run set "$book" A6 'Largest'
run set "$book" B6 '=MAX([.B2:.B4])'                       # statistical
run set "$book" A7 'Lookup north'
run set "$book" B7 '=VLOOKUP("North";[.A2:.B4];2;FALSE())' # lookup, unsorted
run set "$book" A8 'Open regions'
run set "$book" B8 '=COUNTIF([.D2:.D4];TRUE())'            # a criterion
run set "$book" A9 'Label'
run set "$book" B9 '=CONCATENATE([.A2];" & ";[.A3])'       # text
run set "$book" A10 'Guarded'
run set "$book" B10 '=IF([.B5]=0;0;[.B2]/[.B5])'           # short-circuit
run set "$book" A11 'Payment'
run set "$book" B11 '=PMT(0.0625/12;360;-350000)'          # financial
run set "$book" A12 'Today'
run set "$book" B12 '=DATE(2026;8;16)'                     # a date, as a serial number
run set "$book" A13 'Time'
run set "$book" B13 '=TIME(9;30;0)'
run set "$book" A14 'Is it text?'
run set "$book" B14 '=ISTEXT([.A2])'                       # information

say "number formats: display only, the value never moves"
run format "$book" B2:B4 currency --symbol '€' --grouping --decimals 2
run format "$book" C2:C4 percent --decimals 1
run format "$book" B12 date
run format "$book" B13 time
run format "$book" D2:D4 boolean
run format "$book" B5 number --decimals 2 --grouping
run format "$book" B6 general                              # back to the plain value
run format "$book" B11 number --decimals 2 --locale de-DE  # 2.155,01 rather than 2,155.01

# Colours may be named as well as spelled in hex: the names are the palette in
# `style::PALETTE`, which lives in the core so that a name here and a swatch in a GUI write
# the same attribute. Anything outside it is still a plain #rrggbb.
say "cell styling"
run style "$book" A1:E1 --bold --background silver --align center --border '0.5pt solid navy'
run style "$book" A2:A4 --italic --color navy
run style "$book" E3 --wrap --valign top --size 9pt
run style "$book" B5 --bold --align right

say "recalculate the whole document"
run recalc "$book"

# --- reading it back ---------------------------------------------------------------------

say "view: what a person sees"
"$SHEET" view "$book" A1:E5

say "view --raw: what the file stores"
"$SHEET" view "$book" B2:B4 --raw

say "get: one cell, its stored value, and its formula"
"$SHEET" get "$book" B12
"$SHEET" get "$book" B12 --raw
"$SHEET" get "$book" B5 --formula
"$SHEET" get "$book" B5 --input                             # what an editor would show
"$SHEET" get "$book" E2 --input                             # and the ' that keeps 007 text

# Setting a style replaces it, so "bold as well" is a read, a field and a write — which is
# exactly what a toolbar's bold button does, and it needs this to read first.
say "--show: how a cell looks, and how its value is spelled"
"$SHEET" style "$book" A1 --show
"$SHEET" format "$book" B11 --show                          # the flags that recreate it

# A name is what makes a formula say what it means: `SUM(expenses)` rather than
# `SUM([.B2:.B4])`. An address becomes a named *range*, written absolute and
# sheet-qualified so it means the same thing read from anywhere; a target starting with `=`
# becomes a named *expression* instead, and one name may build on another.

say "name: a named range, and an expression over it"
run name "$book" expenses B2:B4
"$SHEET" name "$book" expenses
run name "$book" biggest '=MAX(expenses)'
run set "$book" F1 '=SUM(expenses)'
run set "$book" F2 '=biggest'
"$SHEET" view "$book" F1:F2 --raw

# A second sheet, and a formula on it reaching across to the first. Renaming does **not**
# rewrite the formulas that mention the old name — they go stale instead, which the warning
# on stderr says out loud and `sheet recalc` turns into an error. Deleting is undoable: the
# inverse carries the whole sheet, so the cells come back.

say "sheets: add, write across one, rename, and delete"
run add "$book" Notes
run set "$book" Notes.A1 'sales region count'
run set "$book" Notes.B1 '=COUNT([$Sheet1.$B$2:.$B$4])'
run rename "$book" Notes Summary
"$SHEET" view "$book" Summary.A1:B1 --raw
run --session "$session" remove "$book" Summary
run --session "$session" undo "$book"

say "info: sheets, extents, formula counts, named expressions"
"$SHEET" info "$book"

say "json: every command is machine-readable"
"$SHEET" --format json get "$book" C2

say "fmt: the stored form, the display form a formula bar shows, and back"
"$SHEET" fmt '=SUM([.A1:.A2])*-2^2'
"$SHEET" fmt --display '=SUM([.A1:.A2])*-2^2'
"$SHEET" fmt --from-display '=SUM(A1:A2)*-2^2'

say "eval: what a formula would say at a cell, storing nothing"
"$SHEET" eval "$book" B20 '=SUM([.B2:.B4])*2'

say "functions: what this build implements"
"$SHEET" functions | tail -1
"$SHEET" functions --long --filter vlookup    # signature and summary, from the spec

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
run set "$book" G1 'scratch'
run clear "$book" G1
run clear "$book" A16:B17                                  # the pasted rows, in one step
run clear "$book" B6 --formula-only

say "convert: the same document as flat XML"
run convert "$book" "$out/sample.fods"

# --- R6: a flat document edits in place ---------------------------------------------------
# Editing a `.fods` rewrites the one element that changed and leaves every other byte alone,
# which is what makes these files live in git the way source files do. Shown against a copy,
# with `diff` counting the lines rather than a claim in a comment.

say "minimal diff: one cell changed, one element rewritten"
cp "$out/sample.fods" "$out/before.fods"
run set "$out/sample.fods" B2 4321
printf 'changed lines: %s of %s\n' \
  "$(diff "$out/before.fods" "$out/sample.fods" | grep -c '^[<>]')" \
  "$(wc -l < "$out/before.fods")"

# B5 is `=SUM([.B2:.B4])`, so the edit above left the document disagreeing with itself: the
# formula is one claim and its cached value is another, and ODF has no dirty bit to write.
# The warning on stderr is the only thing standing between that and a file everyone —
# LibreOffice included — displays a stale total from. Recalculating is a separate command on
# purpose: this build implements the Small Group, and a document using anything outside it
# would lose good cached values to #NAME?.

say "stale: the edit above invalidated a total, and said so"
"$SHEET" get "$out/sample.fods" B5 --raw
run recalc "$out/sample.fods"
"$SHEET" get "$out/sample.fods" B5 --raw

# What a GUI does on every commit, and what the CLI does only when asked: the edit and the
# recalculation it causes land as one change, so one `undo` takes back both. It is skipped
# — with a warning — when recalculating would replace a cached value this build cannot
# reproduce, which is the same honesty `recalc` prints.

say "set --recalc: the edit and its ripple, in one step"
run set "$out/sample.fods" B2 1234.5 --recalc
"$SHEET" get "$out/sample.fods" B5 --raw

printf '\n%s and %s\n' "$book" "$out/sample.fods"
