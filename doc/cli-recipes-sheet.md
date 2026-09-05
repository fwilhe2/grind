<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# CLI recipes

Scripted uses of the `sheet` binary. Every snippet here was run against the build in this
repository; the outputs shown are real.

Three facts the rest of the page leans on:

- Every command loads the file, applies one change and writes it back, so a loop of `set`
  calls is a loop of round trips (~200 in 0.8 s on a debug build — fine for a report, not for
  a bulk import of 100 000 rows).
- Mutating commands print a document report on stdout. In a script that means `>/dev/null`.
- Errors go to stderr and never to stdout, in either format, and exit non-zero. `--format
  json` therefore parses or the command failed; there is no error object to check for.

## Build a sheet from a CSV

One command, and the delimiter comes out of the file rather than out of its name:

```sh
grind sheet new report.ods
grind sheet import-csv report.ods data.csv >/dev/null
grind sheet set report.ods B4 '=SUM([.B1:.B3])' --recalc >/dev/null
grind sheet view report.ods
```

```
a	1
b	2
c	3
	6
```

A field becomes what you would have **typed** into the cell — a number, `TRUE`/`FALSE`, or
text, in that order — with four guards that keep what a real file carries: `007` stays a
product code rather than becoming 7, `NaN` and `inf` stay text (they are names and
abbreviations far more often than they are numbers), a leading `=` stays text unless you pass
`--formulas`, and `--locale de-DE` reads `1.234,50` as a number instead of as punctuation.
`--text` turns the whole file into strings; `--trim` drops the padding in `a, b, c`.

A semicolon file with German numbers, which is what most European exports are:

```sh
grind sheet import-csv book.ods supplier.csv --locale de-DE --at Data.A1
```

Dates are **ISO only**, and behind a flag:

```sh
grind sheet import-csv book.ods invoices.csv --dates
```

`2026-03-15` and `10:30` become a date and a time, formatted so they stay ones; `15/03/2026`
does not, because it and `03/15/2026` are the same characters meaning two different days and
nothing in the file says which. The whole import is one undo step, formats included.

The file may come down a pipe, which is how anything that prints a table gets in:

```sh
psql -Atc 'select name, amount from ledger' --csv | grind sheet import-csv book.ods - --at A2
```

## And back out again

```sh
grind sheet export-csv book.ods                     # everything the sheet uses
grind sheet export-csv book.ods A1:D20 --out q3.csv
grind sheet export-csv book.ods --delimiter tab | column -t
```

What each cell **shows**, so a date leaves as a date and a percentage as a percentage — the
number format is the only place a document says which of those a number is. `--formulas`
writes the formulas instead, in the spelling a formula bar shows:

```console
$ grind sheet export-csv book.ods B8:D8 --formulas
=SUM(B2:B7),=SUM(C2:C7),=B8-C8
```

A field is quoted only where leaving it bare would change what a reader sees. For a program
that wants every field quoted, or CRLF, or the byte-order mark Excel looks for before it
believes a file is UTF-8:

```sh
grind sheet export-csv book.ods --quote-all --crlf --bom --out for-excel.csv
```

Reading is UTF-8 only, and a file that is not says so rather than arriving as mojibake:

```console
$ grind sheet import-csv book.ods legacy.csv
grind: legacy.csv: not UTF-8 — convert it first, e.g. iconv -f windows-1252 -t utf-8
```

## Values in from another program

`-` reads the value from stdin, so anything that prints goes into a cell without quoting
games:

```sh
git log -1 --format=%H | grind sheet set report.ods C1 - >/dev/null
grind sheet set report.ods C2 - <<< "$(uname -sr)" >/dev/null
```

## Values back out

`view` is tab-separated and nothing else, so `cut`, `awk` and `paste` work as usual:

```sh
grind sheet view report.ods A1:B3 --raw | cut -f2 | paste -sd+ | bc     # 6
```

`view` and `get` print what the cell *displays* — its number format applied, so a date
prints as a date rather than as a five-digit serial. Pass `--raw` for the stored value, which
is what a script computing with the number wants:

```sh
grind sheet view from-libreoffice.ods A1    # 08/16/2026, in the format the document carries
grind sheet view from-libreoffice.ods A1 --raw   # 46250, the serial the file stores
```

For anything that needs types rather than text, `--format json` carries `ref`, `value` (the
stored value), `text` (the display), `type` and the formula source — both spellings, always,
so a consumer picks rather than re-running the command:

```sh
sheet --format json view report.ods A1:B4 |
  jq -r '.cells[] | select(.type == "float") | .value' |
  paste -sd+ | bc
```

```sh
sheet --format json get report.ods B4
```

```json
{"path":"report.ods","sheet":"Sheet1","cells":[{"ref":"B4","value":"6","text":"6","type":"float","formula":"=SUM([.B1:.B3])"}],"rows":1,"cols":1}
```

## A model driven by shell variables

The formula is stored verbatim in OpenFormula syntax — bracketed references, `;` between
arguments — so a script assembling one is assembling text, not translating a dialect:

```sh
rate=0.0625 years=30 principal=350000

grind sheet new loan.ods
for row in "1 rate $rate" "2 years $years" "3 principal $principal"; do
  set -- $row
  grind sheet set loan.ods "A$1" "$2" >/dev/null
  grind sheet set loan.ods "B$1" "$3" >/dev/null
done
grind sheet set loan.ods A4 payment >/dev/null
grind sheet set loan.ods B4 '=PMT([.B1]/12;[.B2]*12;-[.B3])' >/dev/null

grind sheet get loan.ods B4      # 2155.01020149237
```

`sheet fmt` parses a formula and prints it back normalised, which is the cheap way to check
one a script built before it reaches a cell — it exits non-zero on a syntax error:

```sh
grind sheet fmt '=SUM([.A1:.A2])*-2^2'     # =SUM([.A1:.A2])*-2^2
```

## Format a column

A number format is display only — the value underneath never moves, so a formatted cell
still sums:

```sh
grind sheet format report.ods B2:B40 currency --symbol '€' --grouping >/dev/null
grind sheet format report.ods C2:C40 percent --decimals 1 >/dev/null
grind sheet format report.ods A2:A40 date >/dev/null
grind sheet format report.ods B2:B40 general >/dev/null        # back to the plain value
```

One command over a range is one undo step, and `A:A` works — it is clamped to the rows the
sheet actually uses. `date`, `datetime` and `time` are the ISO spellings; `number`, `percent`
and `currency` take `--decimals`, `--grouping`, `--symbol` and `--locale`:

```sh
grind sheet format report.ods B2:B40 currency --symbol '€' --grouping --locale de-DE   # 1.234,50 €
```

A date a formula computes shows as its serial until the cell says otherwise, which is the
one place this bites:

```sh
grind sheet set report.ods A1 '=DATE(2026;8;16)' >/dev/null
grind sheet get report.ods A1                                  # 46250
grind sheet format report.ods A1 date >/dev/null
grind sheet get report.ods A1                                  # 2026-08-16
```

## Style a header row

```sh
grind sheet style report.ods A1:E1 --bold --background '#dddddd' --align center \
      --border '0.5pt solid #000000' >/dev/null
grind sheet style report.ods A1:E1 >/dev/null      # no options: plain again
```

`style` *replaces* a cell's styling rather than adding to it, so one command says everything
that cell should look like. Fonts are deliberately absent — LibreOffice rewrites a font
family into a reference nothing here follows yet.

[`examples/sample-sheet.sh`](../examples/sample-sheet.sh) builds a document using every
feature at once, and is run by the test suite, so it is always a working example of the
current CLI.

## Gate a repository's spreadsheets in CI

An error cell is text beginning with `#`. This fails the build when a committed document
carries one:

```sh
fail=0
for f in *.ods; do
  sheet --format json view "$f" |
    jq -e '[.cells[] | select(.value | startswith("#"))] | length == 0' >/dev/null ||
    { echo "$f has error cells" >&2; fail=1; }
done
exit $fail
```

Pair it with `recalc` to catch a cached value that no longer matches its formula. `recalc`
writes only when something changed, so a clean run reports `(no change)` and leaves the file's
mtime alone:

```sh
sheet --format json recalc book.ods | jq -e '.changed == false' >/dev/null ||
  { echo "book.ods was stale" >&2; exit 1; }
```

`recalc` warns on stderr when a cell that held a real value became an error — that is a
function outside this build's 110, and it is data loss. `sheet functions` lists what is
implemented, and `undo` is the way back.

## Edit under a session, and roll back

Undo history lives in the session file, never in the document, so `--session` is what makes a
sequence of invocations one transaction:

```sh
sheet --session tx.json --dry-run set book.ods B4 5 >/dev/null || exit 1   # nothing written
sheet --session tx.json set book.ods D1 '=1/0' >/dev/null
grind sheet get book.ods D1                                    # #DIV/0!
sheet --session tx.json undo book.ods >/dev/null
grind sheet get book.ods D1                                    # empty
```

`--dry-run` applies the command and reports the result without touching the disk — the report
carries `"changed":true,"written":false`, which is how a script asks "would this do anything?"

## Flat files for git

`.fods` is one XML file, so it diffs — and it is what you get unless you ask for otherwise
(`doc/flat-first.md`). `grind sheet new book.fods` and `grind sheet new book` both write flat
XML; only `book.ods` writes a zip. Convert an existing package on the way into review:

```sh
for f in *.ods; do grind convert "$f" "${f%.ods}.fods" >/dev/null; done
```

Or leave the `.ods` in place and teach git to read it, which makes `git diff` and `git log -p`
show cell values:

```sh
git config diff.ods.textconv 'grind sheet view'
echo '*.ods diff=ods' >> .gitattributes
```

## Inspect without opening anything

```sh
sheet --format json info book.ods
```

```json
{"path":"book.ods","changed":false,"written":false,"sheets":[{"name":"Sheet1","rows":4,"cols":2,"formulas":1}],"names":[],"can_undo":false,"can_redo":false}
```

`sheets` is per sheet — name, used extent and formula count — and `names` is the document's
named expressions. `sheet get book.ods A3 --formula` prints one cell's source instead of its
value.
