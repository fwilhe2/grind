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

```sh
sheet new report.ods
r=1
while IFS=, read -r name n; do
  sheet set report.ods "A$r" "$name" >/dev/null
  sheet set report.ods "B$r" "$n"    >/dev/null
  r=$((r + 1))
done < data.csv
sheet set report.ods "B$r" "=SUM([.B1:.B$((r - 1))])" >/dev/null
sheet view report.ods
```

```
a	1
b	2
c	3
	6
```

`set` takes the value as a number, `TRUE`/`FALSE`, or text, in that order — pass `--text` to
force a field like `007` or `-` to stay a string.

## Values in from another program

`-` reads the value from stdin, so anything that prints goes into a cell without quoting
games:

```sh
git log -1 --format=%H | sheet set report.ods C1 - >/dev/null
sheet set report.ods C2 - <<< "$(uname -sr)" >/dev/null
```

## Values back out

`view` is tab-separated and nothing else, so `cut`, `awk` and `paste` work as usual:

```sh
sheet view report.ods A1:B3 --raw | cut -f2 | paste -sd+ | bc     # 6
```

`view` and `get` print what the cell *displays* — its number format applied, so a date
prints as a date rather than as a five-digit serial. Pass `--raw` for the stored value, which
is what a script computing with the number wants:

```sh
sheet view from-libreoffice.ods A1    # 08/16/2026, in the format the document carries
sheet view from-libreoffice.ods A1 --raw   # 46250, the serial the file stores
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

sheet new loan.ods
for row in "1 rate $rate" "2 years $years" "3 principal $principal"; do
  set -- $row
  sheet set loan.ods "A$1" "$2" >/dev/null
  sheet set loan.ods "B$1" "$3" >/dev/null
done
sheet set loan.ods A4 payment >/dev/null
sheet set loan.ods B4 '=PMT([.B1]/12;[.B2]*12;-[.B3])' >/dev/null

sheet get loan.ods B4      # 2155.01020149237
```

`sheet fmt` parses a formula and prints it back normalised, which is the cheap way to check
one a script built before it reaches a cell — it exits non-zero on a syntax error:

```sh
sheet fmt '=SUM([.A1:.A2])*-2^2'     # =SUM([.A1:.A2])*-2^2
```

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
sheet get book.ods D1                                    # #DIV/0!
sheet --session tx.json undo book.ods >/dev/null
sheet get book.ods D1                                    # empty
```

`--dry-run` applies the command and reports the result without touching the disk — the report
carries `"changed":true,"written":false`, which is how a script asks "would this do anything?"

## Flat files for git

`.fods` is one XML file, so it diffs. Convert on the way into review:

```sh
for f in *.ods; do sheet convert "$f" "${f%.ods}.fods" >/dev/null; done
```

Or leave the `.ods` in place and teach git to read it, which makes `git diff` and `git log -p`
show cell values:

```sh
git config diff.ods.textconv 'sheet view'
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
