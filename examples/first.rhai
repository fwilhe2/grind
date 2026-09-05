// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later
//
// The first script — `doc/generator-guide.md` §2, and the whole of it.
//
//   grind build examples/first.rhai -o first.fods
//
// A script is a sequence of statements, and **its value is the value of its last expression**.
// That value has to be a document, which is why the last line is a bare `s`: returning a sheet
// means a one-sheet spreadsheet.

let s = sheet("Sales");

// A header is a row that is bold. `push` takes a bare array or a `row(…)`; the second is what
// you want when the row needs an adjective.
s.push(row(["Region", "Q1", "Q2"]).bold());

s.push(["North", 4200, 4800]);
s.push(["South", 3100, 3300]);

// `sum_above()` is told nothing: it sums the contiguous run of numbers directly above wherever
// it lands, which is an AutoSum's rule and the only one that does not reach up into a header.
s.push(row(["Total", sum_above(), sum_above()]).bold());

s
