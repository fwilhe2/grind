<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# sheet

An ODF-native spreadsheet: one Rust core, native shells, and a feature list that ends.

**Phases 0–4 and 6 of 7, and phase 5's number formats.** It reads and writes ODF
spreadsheets, in both the package (`.ods`) and flat (`.fods`) forms. All 361 documents in LibreOffice's own Calc test corpus load — the
three it declines are password-protected — and documents written here survive a round trip
through LibreOffice unchanged, checked in CI. It evaluates **all 110 of OpenFormula's
Small Group functions**, reads, sets and preserves **number formats** so a date prints as a
date, and **cell styling** — weights, colours, borders, alignment. The `sheet` CLI drives all
of it, and [`examples/sample.sh`](examples/sample.sh) builds a document out of every feature
there is. No fonts yet, and formats are the ISO spellings since nothing has a locale (the
rest of phase 5). See [`doc/plan.md`](doc/plan.md).

```sh
sheet new book.ods
sheet set book.ods A1 1
sheet set book.ods A2 2
sheet set book.ods A3 '=SUM([.A1:.A2])'   # OpenFormula syntax, stored verbatim
sheet recalc book.ods
sheet format book.ods A3 currency --symbol '€' --grouping
sheet style book.ods A1 --bold --background '#dddddd'
sheet view book.ods A1:A3                 # tab-separated, pipes into anything
sheet view book.ods A1:A3 --raw           # stored values, not formatted display text
```

Cells are addressed the way ODF references them, minus the brackets — `A1`, `$B$7`,
`Data.B2`, `'Q3 Actuals'.A1:.C9`. `--format json` makes every command machine-readable, and
`--session` carries undo across invocations. [`doc/cli-recipes.md`](doc/cli-recipes.md) has
worked scripts — CSV import, a PMT model, a CI gate on error cells, git diffs of `.ods`.
Whatever the core can do, the CLI can do:
[`doc/cli-parity.md`](doc/cli-parity.md) lists every public method against the command that
reaches it, and a test fails the build when one is missing.

## Why

LibreOffice is the leading implementation of OpenDocument, and ODF is worth having. Its UX
and its feature surface are not. This is not a port of LibreOffice and contains none of its
code: it implements ODF from the OASIS specifications, and uses LibreOffice as a conformance
oracle and test corpus. See [`CONTRIBUTING.md`](CONTRIBUTING.md) for the clean-room rule.

The architecture is [Shared Core, Native Shell](https://github.com/fwilhe2/editor) — all
state and logic in one Rust crate, every UI a renderer and event forwarder that owns
nothing. Shells come after the core is solid; the CLI comes first and stays the ratchet that
keeps capabilities out of shells.

## What it will and will not do

**In:** multiple sheets · cell values and types · OpenFormula **Small Group** (110 functions,
[`doc/small-group.md`](doc/small-group.md)) · formatting and number formats · sort and filter ·
find/replace · freeze panes · one chart type · CSV · print to PDF.

**Out, permanently:** macros · extensions · pivot tables · change tracking · OLE embedding ·
xlsx *writing* · scenarios · solver · sparklines · OpenFormula Large Group.

The "out" list is the product. Items move off it one at a time by explicit decision, and only
if they survive a round-trip through LibreOffice.

## Specs

| Document | What |
|---|---|
| `doc/OpenDocument-v1.4-schema.rng` | ODF 1.4 Part 3 — content schema |
| `doc/OpenDocument-v1.4-os-part4-formula.html` | ODF 1.4 Part 4 — OpenFormula: per-function semantics, conversions, errors |
| `doc/ods-format.md` | Clean-room notes on what LibreOffice actually does, cited `file:line` |
| `doc/small-group.md` | The 110-function Small Group list, extracted from Part 4 §2.3.2 |
| `doc/plan.md` | Phases, exit criteria, and the three verification loops |

## Building

```sh
cargo test
```

The corpus tests want a LibreOffice checkout, and skip with a notice without one:

```sh
SHEET_LO_CORPUS=/path/to/libreoffice/core/sc/qa/unit/data cargo test
```

## License

**AGPL-3.0-or-later.** Full text in [`LICENSES/`](LICENSES/).

If you modify this and offer it to others over a network, §13 requires you to publish your
source. That is deliberate: a spreadsheet core is exactly the thing someone embeds in a
hosted service, and plain GPL would ask nothing of them.

The repository is [REUSE](https://reuse.software) compliant — every file carries its
copyright and license, machine-readably:

```sh
reuse lint
```

The two OASIS specifications under `doc/` are **not** AGPL and **not** open source. They are
redistributed verbatim under the OASIS IPR Policy, which permits copying but forbids
modification of any kind — including adding an SPDX header. They are marked with `.license`
sidecar files for that reason; do not annotate them in place.
