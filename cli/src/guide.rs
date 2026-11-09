// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `grind guide`: one page for whoever drives grind from a script, an agent included — how the
//! commands fit together, read before the help of any one of them. Every line in it that starts
//! with `  grind ` or `  printf ` is a command `cli/tests/cli.rs` runs, in order.

pub(crate) const GUIDE: &str = "\
Driving grind from a script

grind edits OpenDocument files: spreadsheets (.ods, .fods) and text documents (.odt, .fodt).
Each command reads the file, does one thing and writes it back. Text output is tab-separated;
every command also takes --format json.

A spreadsheet, start to finish:

  grind sheet new book.ods
  printf 'Region\\tSales\\nNorth\\t4200\\nSouth\\t3100\\n' | grind sheet paste book.ods A1 -
  grind sheet set book.ods B4 '=SUM([.B2:.B3])'
  grind sheet set book.ods C4 --display '=B4*1.19'
  grind sheet set book.ods B2 4300 --recalc
  grind sheet view book.ods A1:C4
  grind --format json sheet get book.ods B4
  grind sheet eval book.ods A1 '=AVERAGE([.B2:.B3])'
  grind lint book.ods

  An address on the command line is A1, Data.A1, B2:D40, Data.B2:D40 or C:C. A formula is
  OpenFormula, the ODF standard's language, where references are bracketed — [.A1], [Data.A1],
  [.A1:.B9] — and arguments are separated by ;. `grind sheet set --help` has the rules;
  --display takes =SUM(B2:B4) as a formula bar spells it instead. `grind sheet functions
  --long` lists every function and how it is called. A formula grind would read wrongly is
  refused with what to write, and nothing is stored.

  A change leaves the formulas reading it with their old values until something recalculates:
  pass --recalc, or run `grind sheet recalc` once at the end.

A text document:

  grind text new report.odt
  grind text set report.odt p1 'Quarterly report'
  grind text kind report.odt p1 --heading 1
  grind text insert report.odt --text 'Revenue rose 12% on the year.'
  grind text view report.odt
  grind text outline report.odt

  A block is addressed by position (p2), by a bookmark (#intro) or by its place in the
  outline (§1); the last two keep finding it after an edit above. `grind text --help` lists
  every command.

Many changes at once:

  Each command rewrites the whole file, so a loop of `set` is slow: `paste` writes a
  rectangle of tab-separated rows in one call. `grind build` writes a whole document from a
  Rhai script (`grind build --help` shows one, `grind definitions` lists what a script can
  call). `grind project` prints a document as plain text that `grind convert` reads back:

  grind project book.ods > book.grind
  grind lint book.grind
  grind convert book.grind copy.ods

  A .grind is stored as written, so lint it before converting it.

Undo across commands:

  grind --session book.session sheet set book.ods B3 3000 --recalc
  grind --session book.session sheet undo book.ods

  --dry-run on any command reports what it would do and writes nothing.

When something goes wrong:

  An error is one line on standard error, and stdout is empty. Under --format json it is one
  object: {\"error\":{\"kind\":…,\"message\":…},\"exit\":…}, with \"at\" for a script's line and
  column. The exit status says which kind: 1 the input was refused, 2 the command line is
  wrong, 3 a file cannot be read, 4 a check ran and failed (lint, test, --strict).
  `grind --help` has the list.";
