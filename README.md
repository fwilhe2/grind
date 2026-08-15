<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# sheet

An ODF-native spreadsheet: one Rust core, native shells, and a feature list that ends.

**Phase 0 of 7.** The harness exists; the code does not. `cargo test` currently reports 361
failures, which is the intended state — see [`doc/plan.md`](doc/plan.md).

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
