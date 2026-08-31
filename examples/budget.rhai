// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later
//
// The household budget of `examples/sample-sheet.sh`, generated — doc/dsl.md layer 1, D7.
//
//   grind build examples/budget.rhai -o budget.fods
//
// The point is the top of this file: the categories are *data*, said once, and the table is
// what the loop does with them. Adding a row to the budget is adding a row to `CATEGORIES`;
// every formula, format and total follows without being touched, and nothing below has to
// know how many categories there are.
//
// What a script may not do: open a document, read a file, ask the clock, or run when somebody
// opens the result. `grind build` is a verb a person types (doc/dsl.md §2).

// --- the data ----------------------------------------------------------------------------

// Category, budgeted, actual. Made-up numbers, shaped like a real month.
const CATEGORIES = [
    ["Housing",       1800, 1825.50],
    ["Groceries",      500,  612.34],
    ["Transport",      220,  179.85],
    ["Utilities",      260,  238.10],
    ["Entertainment",   80,  -12.50],   // a cash-back refund
    ["Savings",        400,  400.00],
];

const MONEY = format("currency").symbol("€").grouping().decimals(2);

// --- the table ---------------------------------------------------------------------------

let s = sheet("Budget");

// Row 0. A header is a row that is bold, which is what `doc/dsl.md` §4.2 wrote it as.
s.push(row(["Category", "Budgeted", "Actual", "Difference", "% of Total"]).bold());

// Where the totals will land, known before the rows are written because the data is known.
// One `+ 1` for the header, and the sheet's own `at` does every other conversion.
let total_row = CATEGORIES.len + 1;
let total_actual = s.at(total_row, 2);

for c in CATEGORIES {
    let at = s.push([c[0], c[1], c[2]]);
    // Each row's formulas are written *for that row*, so nothing here is relative to anything
    // — the reason a generated sheet has no `$` in it. A fill handle needs absolute references
    // because it copies one formula; a loop writes each one where it goes.
    s.set(s.at(at, 3), formula(s.at(at, 1) + "-" + s.at(at, 2)));
    s.set(s.at(at, 4), formula(s.at(at, 2) + "/" + total_actual));
}

// `sum_above()` is the one thing the script cannot say for itself: it sums the contiguous run
// of numbers directly above wherever the cell lands, so it does not need telling how many
// categories there were either.
s.push(row(["Total", sum_above(), sum_above(), sum_above()]).bold());

// --- what the document says about itself ---------------------------------------------------

let facts = s.rows + 1;   // one blank row under the table
s.set(s.at(facts, 0), "Largest expense");
s.set(s.at(facts, 1), formula("MAX(C2:C" + (total_row) + ")"));
s.set(s.at(facts + 1, 0), "Average spend");
s.set(s.at(facts + 1, 1), formula("AVERAGE(C2:C" + (total_row) + ")"));
s.set(s.at(facts + 2, 0), "Spend ratio");
s.set(s.at(facts + 2, 1), formula("IF(B" + (total_row + 1) + "=0;0;C" + (total_row + 1) + "/B" + (total_row + 1) + ")"));
s.set(s.at(facts + 3, 0), "Statement date");
// The other formula spelling: ODF syntax, verbatim, exactly as the file stores it. A date
// value with no formula behind it has no spelling in the generator, which is why this is a
// `DATE()` under a date format — the same thing `examples/sample-sheet.sh` writes.
s.set(s.at(facts + 3, 1), "=DATE(2026;8;16)");

// --- how it looks ---------------------------------------------------------------------------

// Ranges are strings, so a row number computed above is part of one. `total_row + 1` is the
// 1-based row an address wears.
s.format("B2:D" + (total_row + 1), MONEY);
s.format("E2:E" + total_row, format("percent").decimals(1));
s.format(s.at(facts, 1), MONEY);
s.format(s.at(facts + 1, 1), MONEY);
s.format(s.at(facts + 2, 1), format("percent").decimals(1));
s.format(s.at(facts + 3, 1), "date");

// Styling layers: the header row is already bold from `row(…).bold()` above, and this adds to
// it rather than replacing it.
s.style("A1:E1", style().background("silver").align("center").border("0.5pt solid navy"));
s.style("A2:A" + total_row, style().italic().color("navy"));
s.width("A:A", "4cm");
s.height("1:1", "8mm");

// A name is what makes a formula say what it means, and it comes out qualified with the sheet
// that said it: `budgeted` is `[$Budget.$B$2:.$B$7]` read from anywhere.
s.name("budgeted", "B2:B" + total_row);
s.name("biggestBudget", "=MAX(budgeted)");

// --- a second sheet, reaching back across --------------------------------------------------

let journal = sheet("Journal");
journal.push(row(["Entry", "Amount"]).bold());
journal.push(["opening balance", 1250]);
journal.push(["closing balance", 830]);
journal.push(row(["Categories counted", formula("COUNT(Budget.B2:B" + total_row + ")")]).italic());
journal.format("B2:B3", MONEY);
journal.width("A:A", "5cm");

let d = spreadsheet();
d.push(s);
d.push(journal);
d
