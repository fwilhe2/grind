<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: CC-BY-4.0
-->

# CLI parity — `grind calc`

The tape's half of R9 (`doc/cli-parity-sheet.md` says why this file exists). Every public method
of `grind_sheet::Tape` appears below once, with the `grind calc` spelling that reaches it.
`cli/tests/parity.rs` holds the two to each other. The tape is `doc/calc.md`.

- `new` — `grind calc [--locale <tag>]`
- `len` — the rows `grind calc` prints, one per line
- `is_empty` — `grind calc` with no lines on stdin prints nothing
- `lines` — `grind calc <line>…`, or `--format json` for handles, names, text and values
- `cycles` — `cycles` in `--format json`
- `circles` — printed to stderr after the rows
- `assist` — not exposed: it is help for a line half typed, at a caret; a line given to
  `grind calc` is already whole, and the names it could offer are the ones `lines` prints
- `spans` — not exposed: it is colour for an editor, and `lines` prints the same words
- `push` — each argument, each line of stdin, and each line typed at the interactive tape
- `insert` — `:insert N <line>` at the interactive tape
- `edit` — `:edit N <line>` at the interactive tape
- `delete` — `:del N` at the interactive tape
- `extract` — `grind calc … --extract '#N' <number> <name> [--nth K]`, or `:extract N <number> <name> [K]`
- `can_undo` — `:undo` at the interactive tape answers *nothing to undo* when it is false
- `can_redo` — `:redo` answers *nothing to redo* when it is false
- `undo` — `:undo` at the interactive tape
- `redo` — `:redo` at the interactive tape
- `promote` — `grind calc … --save <file>`, which writes what it returns
- `save` — `grind calc … --save <file> [--force]`, or `:save <file> [!]`

## Beyond `App`

- `formula::ExprList` — the named expression list under the tape, crate-private until a second
  caller needs it, with no CLI of its own:
  everything it does is reached through the `Tape` method above that calls it.
- `Notebook` — which row of a tape's editor the caret goes to after Enter, an arrow or
  Backspace. It has no CLI of its own: every change it makes is a `push`, `insert`, `edit` or
  `delete` above, and `:insert`, `:edit` and `:del` are the interactive tape's spellings of them.
