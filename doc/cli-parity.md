<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# CLI parity

> **Whatever any GUI can do, the CLI can do. A UI-only feature is a bug.**
> — `doc/plan.md` rule 4, `CONTRIBUTING.md` §4

This file is that rule made mechanical. Every public method of `App` appears below exactly
once, with the command that reaches it or the reason it is not reachable. `cli/tests/parity.rs`
reads `core/src/lib.rs` and this file and fails the build when they disagree — so adding a
capability to the core without exposing it breaks CI, and so does deleting a command that
this file still claims.

It is the same mechanism `doc/small-group.md` uses against `funcs::implemented()`: a
checked-in document that the code is tested against, rather than a promise in prose.

Format: one bullet per method, `` - `method` — how ``. "not exposed:" must be followed by a
reason; the test checks that too, because an unexplained exemption is how a ratchet quietly
stops ratcheting.

## Editing

- `set_cell` — `sheet set`
- `set_formula` — `sheet set` with a value starting `=`
- `clear_formula` — `sheet clear --formula-only`
- `set_style` — `sheet style` (and `sheet style <range>` with no options to clear one)
- `set_format` — `sheet format` (and `sheet format <range> general` to clear one)
- `recalc` — `sheet recalc`

## History

- `undo` — `sheet undo`
- `redo` — `sheet redo`
- `can_undo` — `sheet info`, and the `can_undo` field of every JSON report
- `can_redo` — `sheet info`, and the `can_redo` field of every JSON report
- `session` — written by every command when `--session` is given
- `restore_session` — read by every command when `--session` is given

## Documents

- `open_file` — every subcommand that takes a file
- `open_bytes` — not exposed: the CLI has a filesystem, and this is its twin for shells that
  do not (`doc/plan.md` rule 5). `sheet convert` covers the same ground for a user.
- `save_file` — `sheet new`, `sheet set`, `sheet clear`, `sheet recalc`, `sheet convert`
- `save_bytes` — not exposed: the `*_bytes` twin of `save_file`, for shells without a
  filesystem. Nothing a user can ask for is missing while `save_file` is here.

## Reading

- `get` — `sheet get`
- `get_viewport` — `sheet view` (its display text is what `view` prints; `--raw` prints the
  stored values instead)
- `formula` — `sheet get --formula`
- `formula_count` — `sheet info`
- `sheet_count` — `sheet info`
- `sheet_name` — `sheet info`, and any sheet-qualified address
- `used_extent` — `sheet info`, and `sheet view` with no range
- `names` — `sheet info`

## Not reachable, and why

- `new` — not exposed: constructing an `App` is what every subcommand does before anything
  else. `sheet new` writes an empty document, which is the user-visible half.
- `set_observer` — not exposed: the core pushes changes to shells that stay running
  (`doc/plan.md` rule 3). A CLI process exits before a notification could matter, and there
  is no user-facing behaviour behind it.

## Beyond `App`

Reachable from the CLI, but not `App` methods, so the test does not track them:

- `sheet fmt` — `formula::parse` plus the AST's `Display`
- `sheet functions` — `formula::funcs::implemented()`, already gated against
  `doc/small-group.md` by its own test
