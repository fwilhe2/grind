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

- `enter` — `sheet set` (the typing rule: `=` a formula, `'` text, empty clears, `--recalc`
  recalculates the document in the same undo step)
- `enter_range` — `sheet paste <anchor> <tsv>`, or `-` to read the rows from stdin
- `preview` — `sheet eval <address> <formula>`, which stores nothing and writes nothing
- `set_cell` — not exposed directly: `sheet set` goes through `App::enter`, which is the
  typing rule every shell shares and which *replaces* whatever the cell held, formula
  included. `set_cell` is the primitive underneath it, and the one shape that can leave a
  value sitting beside a formula that disagrees with it — which is the state `stale` exists
  to report, not one a user asks for.
- `set_formula` — `sheet set` with a value starting `=`, through `App::enter`
- `clear_formula` — `sheet clear --formula-only`
- `clear_range` — `sheet clear <range>`
- `set_style` — `sheet style` (and `sheet style <range>` with no options to clear one)
- `set_format` — `sheet format` (and `sheet format <range> general` to clear one)
- `set_col_width` — `sheet width <columns> <length>` (and `--clear` to drop one)
- `set_row_height` — `sheet height <rows> <length>` (and `--clear` to drop one)
- `set_name` — `sheet name <name> <address-or-=expression>`
- `clear_name` — `sheet name <name> --delete`
- `add_sheet` — `sheet add <name>`
- `rename_sheet` — `sheet rename <sheet> <name>`
- `remove_sheet` — `sheet remove <sheet>`
- `recalc` — `sheet recalc`
- `stale` — every command that writes: the warning on stderr, and the `stale` field of its
  JSON report. Not a command of its own, because the answer is only interesting *after* an
  edit — asking it of an untouched document is what `sheet recalc --dry-run` is for.

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
- `input_text` — `sheet get --input`, which prints what an editor would show for the cell:
  the text that, given back to `sheet set`, leaves it exactly as it is
- `style_at` — `sheet style <cell> --show`, which prints the cell's styling under ODF's own
  attribute names. The read half of the read-merge-write a bold button is, since `set_style`
  replaces rather than merges.
- `format_at` — `sheet format <cell> --show`, printing the flags that would recreate the
  format and whether they can (`preset`) — a document may hold one this vocabulary cannot
  build.
- `col_widths` — `sheet width <columns>` with no length, printing one `A<TAB>2.258cm` line per
  sized column. A column the document never sized prints nothing, because ODF's own answer
  there is "whatever the application's default is".
- `row_heights` — `sheet height <rows>` with no length, the same shape
- `formula_count` — `sheet info`
- `sheet_count` — `sheet info`
- `sheet_name` — `sheet info`, and any sheet-qualified address
- `used_extent` — `sheet info`, and `sheet view` with no range
- `names` — `sheet info`, and `sheet name <name>` for one

## Not reachable, and why

- `new` — not exposed: constructing an `App` is what every subcommand does before anything
  else. `sheet new` writes an empty document, which is the user-visible half.
- `set_observer` — not exposed: the core pushes changes to shells that stay running
  (`doc/plan.md` rule 3). A CLI process exits before a notification could matter, and there
  is no user-facing behaviour behind it.

## Beyond `App`

Reachable from the CLI, but not `App` methods, so the test does not track them:

- `sheet fmt` — `formula::parse` plus the AST's `Display`; `--display` / `--from-display`
  are `formula::display`, checked against the whole corpus by loop B's third half.
  `--friendly` is `formula::friendly::explain` — a read-only, multi-line, aliased,
  parameter-labelled rendering; it never parses back, so it is not a fourth spelling of the
  formula, only an explanation of one. `--friendly --inline` is
  `formula::friendly::explain_inline`, the same rendering never unfolded, which is what the
  GTK shell's formula bar shows in place of a stored formula.
- `core::a1` — addressing, the only 0↔1 conversion in the workspace. Free functions, used
  by every shell and by every command that takes an address.
- `sheet functions` — `formula::funcs::implemented()`, already gated against
  `doc/small-group.md` by its own test; `--long` adds `formula::funcs::catalog()`, the
  spec's own signature and summary for each, which is what a GUI's autocomplete offers, plus
  `formula::friendly::signature` and `formula::funcs::category` for a help browser's friendly
  signature — the aliased name and one plain-English label per parameter, the same names the
  GTK shell's signature hint reads in — and its grouping
