<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: CC-BY-4.0
-->

# CLI parity — `grind calc`

The tape's half of R9 (`doc/cli-parity-sheet.md` says why this file exists). Every public method
of `grind_sheet::Tape` appears below once, with the `grind calc` spelling that reaches it.
`cli/tests/parity.rs` holds the two to each other. The tape is an experiment (`doc/calc.md`).

- `new` — `grind calc [--locale <tag>]`
- `len` — the rows `grind calc` prints, one per line
- `is_empty` — `grind calc` with no lines on stdin prints nothing
- `lines` — `grind calc <line>…`, or `--format json` for handles, names, text and values
- `cycles` — printed to stderr after the rows, and `cycles` in `--format json`
- `push` — each argument, each line of stdin, and each line typed at the interactive tape
- `insert` — `:insert N <line>` at the interactive tape
- `edit` — `:edit N <line>` at the interactive tape
- `delete` — `:del N` at the interactive tape
- `extract` — `grind calc … --extract '#N' <number> <name> [--nth K]`, or `:extract N <number> <name> [K]`
- `undo` — `:undo` at the interactive tape
- `redo` — `:redo` at the interactive tape
- `promote` — `grind calc … --save <file>`, which writes what it returns
- `save` — `grind calc … --save <file> [--force]`, or `:save <file> [!]`

## Beyond `App`

- `formula::ExprList` — the named expression list under the tape, crate-private until a second
  caller needs it, with no CLI of its own:
  everything it does is reached through the `Tape` method above that calls it.
