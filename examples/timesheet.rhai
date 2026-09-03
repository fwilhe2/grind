// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later
//
// A month's job costing for a joinery workshop — doc/dsl.md layer 1, and the worked example
// `doc/generator-guide.md` builds up to.
//
//   grind build examples/timesheet.rhai -o month.fods
//
// `examples/budget.rhai` is one table from a list. This one is the shape a generator is
// actually for: **four sheets that have to agree with each other**, built out of an export
// nobody edits by hand.
//
//   Rates      the hourly rate per role — the table the Time sheet looks rates up in
//   Time       one row per timesheet line, its rate looked up and its value calculated
//   Materials  one row per delivery note
//   Summary    one row per job, its hours, labour and materials summed back out of the other two
//
// Nothing below is told how many jobs, entries or deliveries there are. Add a job to
// `timesheet.json` and it gets a summary row whose three SUMIFs cover the new rows, because
// every range in this script is written from a row count the script measured.
//
// What that buys, said plainly: **the twenty-third row cannot be the one where the formula was
// wrong.** There is one line here per *kind* of cell, not one per cell, and running the script
// again after the data changes is the whole of "updating the spreadsheet".

// --- the data ------------------------------------------------------------------------------
//
// Everything the document *contains* is in `timesheet.json` beside this file, which the
// workshop manager edits and which nobody needs to read Rhai to change. A script may read JSON
// and nothing else, from one directory — `doc/generator-spec.md` §3.5 is the four walls.

let source = json("timesheet.json");

let money = format("currency").symbol(source.currency).grouping().decimals(2);
let hours = format("number").decimals(1);
let share = format("percent").decimals(1);

// --- two shapes said once ---------------------------------------------------------------
//
// A Rhai `fn` cannot see this file's variables — it gets only its arguments, which is why the
// sheet is one of them. Builders are shared handles, so a sheet written to inside a function is
// the same sheet, not a copy.

/// A header row: bold, and nothing else. `doc/dsl.md` §4.2's definition of one.
fn header(cells) {
    row(cells).bold()
}

/// A labelled figure under a table — a name in column A and a formula in column B — answering
/// with the row it landed on.
fn fact(s, label, source) {
    let at = s.push([label]);
    s.set(s.at(at, 1), formula(source));
    at
}

// --- Rates: the table everything else looks a price up in --------------------------------
//
// Built first because the Time sheet's VLOOKUP has to name its range, and a range is a string
// that needs the row count of a table that exists.

let rates = sheet("Rates");
rates.push(header(["Role", "Hourly rate"]));
for r in source.roles {
    rates.push([r.role, r.rate]);
}

// `rates.rows` is how many rows have been written, which is also the 1-based number of the last
// one. Absolute, because every row of the Time sheet reads the same table.
let rate_table = "Rates.$A$2:$B$" + rates.rows;
rates.format("B2:B" + rates.rows, money);
rates.width("A:A", "4.5cm");

// --- Time: one row per timesheet line ------------------------------------------------------

let time = sheet("Time");
time.push(header(["Day", "Job", "Who", "Role", "Hours", "Rate", "Value", "Task"]));

for e in source.time {
    // The row goes in whole, with holes where a formula belongs: `()` is *nothing*, and leaves
    // the cell empty for the two lines below to fill.
    let at = time.push([e.day, e.job, e.who, e.role, e.hours, (), (), e.task]);

    // The rate is looked up **in the document** rather than substituted here. Both would put
    // the same number in the cell today; only this one still explains itself in six months,
    // and only this one changes when somebody edits the Rates sheet.
    time.set(
        time.at(at, 5),
        formula("VLOOKUP(" + time.at(at, 3) + ";" + rate_table + ";2;FALSE())"),
    );
    // Each row's arithmetic is written *for that row*, so nothing is anchored with a `$`. A
    // fill handle needs absolute references because it copies one formula down; a loop writes
    // each one where it goes.
    time.set(time.at(at, 6), formula(time.at(at, 4) + "*" + time.at(at, 5)));
}

let time_last = time.rows;
time.format("E2:E" + time_last, hours);
time.format("F2:G" + time_last, money);
time.width("C:C", "4cm");
time.width("D:D", "4.5cm");
time.width("H:H", "6cm");

// --- Materials: one row per delivery note --------------------------------------------------

let materials = sheet("Materials");
materials.push(header(["Job", "Supplier", "Item", "Cost"]));
for m in source.materials {
    materials.push([m.job, m.supplier, m.item, m.cost]);
}

let materials_last = materials.rows;
materials.format("D2:D" + materials_last, money);
materials.width("B:B", "5cm");
materials.width("C:C", "9cm");

// --- Summary: the sheet anybody actually opens ---------------------------------------------
//
// The three ranges the summary reads, spelled once. Absolute and sheet-qualified, because every
// job's row asks the same two sheets the same question about a different code.

let time_jobs = "Time.$B$2:$B$" + time_last;
let time_hours = "Time.$E$2:$E$" + time_last;
let time_value = "Time.$G$2:$G$" + time_last;
let materials_jobs = "Materials.$A$2:$A$" + materials_last;
let materials_cost = "Materials.$D$2:$D$" + materials_last;

let summary = sheet("Summary");
summary.push(header([
    "Job", "Client", "Quoted", "Hours", "Labour", "Materials", "Cost", "Margin", "Margin %",
]));

for j in source.jobs {
    let at = summary.push([j.code, j.client, j.quoted]);
    let code = summary.at(at, 0);

    summary.set(summary.at(at, 3), formula("SUMIF(" + time_jobs + ";" + code + ";" + time_hours + ")"));
    summary.set(summary.at(at, 4), formula("SUMIF(" + time_jobs + ";" + code + ";" + time_value + ")"));
    summary.set(
        summary.at(at, 5),
        formula("SUMIF(" + materials_jobs + ";" + code + ";" + materials_cost + ")"),
    );
    // The overhead is a *name*, defined at the bottom of this file, so the cell says
    // `=(E2+F2)*overhead` rather than `=(E2+F2)*1.18`. Six months from now that is the
    // difference between a formula that explains itself and one that has to be decoded.
    summary.set(
        summary.at(at, 6),
        formula("(" + summary.at(at, 4) + "+" + summary.at(at, 5) + ")*overhead"),
    );
    summary.set(summary.at(at, 7), formula(summary.at(at, 2) + "-" + summary.at(at, 6)));
    summary.set(
        summary.at(at, 8),
        formula("IF(" + summary.at(at, 2) + "=0;0;" + summary.at(at, 7) + "/" + summary.at(at, 2) + ")"),
    );
}

let last_job = summary.rows;

// `sum_above()` is the one thing the script does not have to count for itself: it sums the
// contiguous run of numbers directly above wherever it lands, so this line is the same line
// whether there are four jobs or forty.
let total = summary.push(
    row(["Total", (), sum_above(), sum_above(), sum_above(), sum_above(), sum_above(), sum_above()])
        .bold(),
);
// The blended margin is not the sum of the margins, so it is written rather than summed.
summary.set(
    summary.at(total, 8),
    formula("IF(" + summary.at(total, 2) + "=0;0;" + summary.at(total, 7) + "/" + summary.at(total, 2) + ")"),
);

// --- what the month says about itself -------------------------------------------------------
//
// A blank row, then the four questions somebody opens this file to ask. `fact` is the function
// from the top: the label and the formula, one line each.

let blank = summary.rows + 1;
summary.set(summary.at(blank, 0), "Month");
summary.set(summary.at(blank, 1), source.month);

let margins = "I2:I" + last_job;
let first = fact(summary, "Jobs under target", "COUNTIF(" + margins + ";\"<" + source.target + "\")");
fact(summary, "Best margin", "MAX(" + margins + ")");
fact(summary, "Worst margin", "MIN(" + margins + ")");
fact(summary, "Hours in the month", "SUM(" + time_hours + ")");
fact(summary, "Average line value", "AVERAGE(" + time_value + ")");

// `first` is the *index* the first fact landed on. Two things are built from it below and they
// are not the same arithmetic: `s.at(first + 1, 1)` asks for the cell one row further down, and
// `"A" + (first + 1)` is the address of the first fact's own row, because a 0-based index names
// a row that wears the number one higher.
let facts = first + 1;

// --- how it looks -----------------------------------------------------------------------------

// `total` is an *index*, counted from zero; the row it names wears the number one higher. This
// is the one place a script does address arithmetic, and `s.at` does the rest of it.
let last = total + 1;
summary.format("C2:C" + last, money);
summary.format("D2:D" + last, hours);
summary.format("E2:H" + last, money);
summary.format("I2:I" + last, share);
summary.format(summary.at(first + 1, 1), share);
summary.format(summary.at(first + 2, 1), share);
summary.format(summary.at(first + 3, 1), hours);
summary.format(summary.at(first + 4, 1), money);

// Styling layers rather than replacing, so the header row keeps the bold `header` gave it.
summary.style("A1:I1", style().background("silver").align("center").border("0.5pt solid navy"));
summary.style("A2:A" + last_job, style().italic().color("navy"));
summary.style("A" + last + ":I" + last, style().border("0.5pt solid navy"));
summary.style("A" + facts + ":A" + (facts + 4), style().italic());
summary.width("A:A", "3cm");
summary.width("B:B", "6cm");
summary.height("1:1", "8mm");

// A name is what makes a formula say what it means, and it comes out qualified with the sheet
// that said it — `margins` is `[$Summary.$I$2:.$I$5]` read from anywhere in the document.
summary.name("margins", margins);
summary.name("quoted", "C2:C" + last_job);
summary.name("overhead", "=" + (1 + source.overhead));

// --- the document ------------------------------------------------------------------------------
//
// Sheets appear in the order they are pushed, which is not the order they were built: Summary is
// first because it is the one anybody opens, and it was last to be written because it needed the
// other three to exist.

let d = spreadsheet();
d.push(summary);
d.push(time);
d.push(materials);
d.push(rates);
d
