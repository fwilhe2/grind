<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# CLI parity — `grind sheet`

> **Whatever any GUI can do, the CLI can do. A UI-only feature is a bug.**
> — `doc/plan.md` rule 4, `CONTRIBUTING.md` §4

This file is that rule made mechanical, **for the spreadsheet**. Every public method of
`grind_sheet::App` appears below exactly once, with the command that reaches it or the reason
it is not reachable. `cli/tests/parity.rs` reads `sheet/src/lib.rs` and this file and fails the
build when they disagree — so adding a capability to the core without exposing it breaks CI,
and so does deleting a command that this file still claims.

One of these per application (doc/suite.md's **R9**): the test drives a registry of
(application, core source, parity document) triples, so `grind-text` arrives as three lines
there and a `doc/cli-parity-text.md` beside this one — never as a second section here, because
two apps sharing one document would pass every check while covering half as much.

It is the same mechanism `doc/small-group.md` uses against `funcs::implemented()`: a
checked-in document that the code is tested against, rather than a promise in prose.

Format: one bullet per method, `` - `method` — how ``. "not exposed:" must be followed by a
reason; the test checks that too, because an unexplained exemption is how a ratchet quietly
stops ratcheting.

## Editing

- `enter` — `grind sheet set` (the typing rule: `=` a formula, `'` text, empty clears, `--recalc`
  recalculates the document in the same undo step)
- `enter_range` — `grind sheet paste <anchor> <tsv>`, or `-` to read the rows from stdin
- `fill` — `grind sheet fill <source> <range>`, replicating one cell across a rectangle with its
  relative references shifted and its absolute ones left alone — "extend a calculation into
  the next cell"
- `preview` — `grind sheet eval <address> <formula>`, which stores nothing and writes nothing
- `set_cell` — not exposed directly: `grind sheet set` goes through `App::enter`, which is the
  typing rule every shell shares and which *replaces* whatever the cell held, formula
  included. `set_cell` is the primitive underneath it, and the one shape that can leave a
  value sitting beside a formula that disagrees with it — which is the state `stale` exists
  to report, not one a user asks for.
- `set_formula` — `grind sheet set` with a value starting `=`, through `App::enter`
- `clear_formula` — `grind sheet clear --formula-only`
- `clear_range` — `grind sheet clear <range>`
- `set_style` — `grind sheet style` (and `grind sheet style <range>` with no options to clear one)
- `set_format` — `grind sheet format` (and `grind sheet format <range> general` to clear one)
- `set_col_width` — `grind sheet width <columns> <length>` (and `--clear` to drop one)
- `set_row_height` — `grind sheet height <rows> <length>` (and `--clear` to drop one)
- `set_col_hidden` — `grind sheet hide <columns>` (and `--unhide` to show it again)
- `set_row_hidden` — `grind sheet hide <rows>` (and `--unhide` to show it again)
- `set_filter` — `grind sheet filter <range> COLUMN=VALUE…` (and `grind sheet filter --clear`)
- `filter` — `grind sheet filter` with no range, which prints each sheet's filtered range
- `hidden_rows` — the same listing's `hides` column, in 1-based row numbers
- `set_name` — `grind sheet name <name> <address-or-=expression>`
- `clear_name` — `grind sheet name <name> --delete`
- `add_sheet` — `grind sheet add <name>`
- `rename_sheet` — `grind sheet rename <sheet> <name>`
- `remove_sheet` — `grind sheet remove <sheet>`
- `recalc` — `grind sheet recalc`
- `stale` — every command that writes: the warning on stderr, and the `stale` field of its
  JSON report. Not a command of its own, because the answer is only interesting *after* an
  edit — asking it of an untouched document is what `grind sheet recalc --dry-run` is for.
- `add_chart` — `grind sheet chart-add --type bar|line|pie --categories <range> --series
  <range>[=<label-range>] … [--x-axis-label <text>] [--y-axis-label <text>]
  [--x-tick-labels <bool>] [--y-tick-labels <bool>] [--x-gridlines <bool>]
  [--y-gridlines <bool>]` (`doc/chart-format.md`)
- `edit_chart` — `grind sheet chart-edit <index> [--type bar|line|pie]
  [--categories <range>] [--series <range>[=<label-range>]]… [the same axis flags]` — what a
  chart *is*, changed after the fact in the vocabulary a user types; every flag left off keeps
  what the chart already has, and its position stays `chart-reshape`'s
- `charts` — `grind sheet chart-list`
- `remove_chart` — `grind sheet chart-remove <index>`
- `reshape_chart` — `grind sheet chart-reshape <index> --x --y --width --height`
- `set_chart_style` — `grind sheet chart-style <index> [--x-axis-label <text>]
  [--y-axis-label <text>] [--x-tick-labels <bool>] [--y-tick-labels <bool>]
  [--x-gridlines <bool>] [--y-gridlines <bool>] [--series-color <series>=<color>]…
  [--point-color <series>.<point>=<color>]…` — everything an axis carries, and the colour a
  line series or a bar/pie point gets, overriding `series_color`'s default cycle
  (`doc/chart-format.md`)
- `chart_data` — not exposed: nothing here draws a chart. `chart-list`'s ranges are read
  back through `charts`; `grind-sheet-gtk` is the shell that resolves them against the live
  sheet and draws one, and the CLI has no drawing surface of its own to reach this from.

## History

- `undo` — `grind undo`
- `redo` — `grind redo`
- `can_undo` — `grind info`, and the `can_undo` field of every JSON report
- `can_redo` — `grind info`, and the `can_redo` field of every JSON report
- `session` — written by every command when `--session` is given
- `restore_session` — read by every command when `--session` is given

## Documents

- `open_file` — every subcommand that takes a file
- `open_bytes` — not exposed: the CLI has a filesystem, and this is its twin for shells that
  do not (`doc/plan.md` rule 5). `grind convert` covers the same ground for a user.
- `save_file` — `grind sheet new`, `grind sheet set`, `grind sheet clear`, `grind sheet recalc`, `grind convert`
- `save_bytes` — not exposed: the `*_bytes` twin of `save_file`, for shells without a
  filesystem. Nothing a user can ask for is missing while `save_file` is here.

## Reading

- `get` — `grind sheet get`
- `get_viewport` — `grind sheet view` (its display text is what `view` prints; `--raw` prints the
  stored values instead)
- `get_viewport_with` — `grind sheet view --roles` / `--names` / `--formulas`, which print
  `doc/view-modes.md`'s derived overlays instead of the values: what each cell *is*, what it is
  *called*, and what computes it. `--format json` carries every column of every cell at once, so
  "find every magic constant in this repository" is a shell loop rather than a feature request.
  The view mode is a **reading** of a document and writes nothing to it, which is why the CLI is
  not a formality here — it is the accessible surface for a feature whose entire output in a GUI
  is colour (`doc/view-modes.md` §4.6).
- `formula` — `grind sheet get --formula`
- `input_text` — `grind sheet get --input`, which prints what an editor would show for the cell:
  the text that, given back to `grind sheet set`, leaves it exactly as it is
- `value_text` — `grind sheet get --value`, a formula's calculated, formatted result rather than
  its source — what "Copy Value" puts on the clipboard
- `style_at` — `grind sheet style <cell> --show`, which prints the cell's styling under ODF's own
  attribute names. The read half of the read-merge-write a bold button is, since `set_style`
  replaces rather than merges.
- `format_at` — `grind sheet format <cell> --show`, printing the flags that would recreate the
  format and whether they can (`preset`) — a document may hold one this vocabulary cannot
  build.
- `col_widths` — `grind sheet width <columns>` with no length, printing one `A<TAB>2.258cm` line per
  sized column. A column the document never sized prints nothing, because ODF's own answer
  there is "whatever the application's default is".
- `row_heights` — `grind sheet height <rows>` with no length, the same shape
- `hidden_cols` — `grind sheet hide` with no track, which lists every column and row hidden by
  hand across every sheet
- `manually_hidden_rows` — the same listing's rows
- `formula_count` — `grind info`
- `sheet_count` — `grind info`
- `sheet_name` — `grind info`, and any sheet-qualified address
- `used_extent` — `grind info`, and `grind sheet view` with no range
- `names` — `grind info`, and `grind sheet name <name>` for one
- `calculations` — `grind sheet calculations`, one line per calculated cell (address, formula,
  result, the functions it calls) plus a tally of which functions the document uses.
  `--filter` narrows it by sheet, address, formula text or function name, through
  `Calculation::matches` — the same rule the GTK shell's search box uses, so the two cannot
  disagree about what a search finds.

## Not reachable, and why

- `new` — not exposed: constructing an `App` is what every subcommand does before anything
  else. `grind sheet new` writes an empty document, which is the user-visible half.
- `set_observer` — not exposed: the core pushes changes to shells that stay running
  (`doc/plan.md` rule 3). A CLI process exits before a notification could matter, and there
  is no user-facing behaviour behind it.

## Beyond `App`

Reachable from the CLI, but not `App` methods, so the test does not track them:

- `grind sheet fmt` — `formula::parse` plus the AST's `Display`; `--display` / `--from-display`
  are `formula::display`, checked against the whole corpus by loop B's third half.
  `--friendly` is `formula::friendly::explain` — a read-only, multi-line, aliased,
  parameter-labelled rendering; it never parses back, so it is not a fourth spelling of the
  formula, only an explanation of one. `--friendly --inline` is
  `formula::friendly::explain_inline`, the same rendering never unfolded, which is what the
  GTK shell's formula bar shows in place of a stored formula.
- `core::a1` — addressing, the only 0↔1 conversion in the workspace. Free functions, used
  by every shell and by every command that takes an address.
- `grind sheet functions` — `formula::funcs::implemented()`, already gated against
  `doc/small-group.md` by its own test; `--long` adds `formula::funcs::catalog()`, the
  spec's own signature and summary for each, which is what a GUI's autocomplete offers, plus
  `formula::friendly::signature` and `formula::funcs::category` for a help browser's friendly
  signature — the aliased name and one plain-English label per parameter, the same names the
  GTK shell's signature hint reads in — and its grouping
