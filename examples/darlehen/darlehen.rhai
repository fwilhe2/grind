// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: MIT
//
// A loan, compared across scenarios — doc/dsl.md layer 1.
//
//   grind build examples/darlehen/darlehen.rhai -o darlehen.fods
//   grind test  examples/darlehen/darlehen.rhai
//
// What a generator is for, in the shape of a question people really ask: "what does the same
// loan cost over 15, 25 or 30 years?" One sheet per scenario holds the whole repayment schedule,
// a row a month, as live formulas — so editing a rate in a sheet's header recalculates it —
// and a comparison sheet reads each schedule's totals back out. The schedule has as many rows as
// the scenario has months, which the script measures from the data rather than counting; adding
// a scenario to `darlehen.json` adds a sheet, a chart and a row in the comparison.
//
// The numbers in `darlehen.json` are invented.

let source = json("darlehen.json");

let money = format("currency").symbol(source.currency).grouping().decimals(2);
let rate_format = format("percent").decimals(2);

/// A header row: bold, and nothing else.
fn header(cells) { row(cells).bold() }

let sheets = [];
let schedules = [];

for sc in source.scenarios {
    let s = sheet(sc.name);
    let months = sc.years * 12;

    // The loan's terms, in cells a person can change. Everything below reads them.
    s.push(["Darlehen", sc.principal]);          // B1
    s.push(["Zinssatz", sc.rate / 100.0]);       // B2
    s.push(["Laufzeit (Jahre)", sc.years]);      // B3
    s.push(["Monatsrate", formula("PMT(B2/12;B3*12;-B1)")]);  // B4
    s.format("B1", money);
    s.format("B2", rate_format);
    s.format("B4", money);
    s.style("A1:A4", style().bold());

    s.push([]);
    s.push(header(["Monat", "Anfangsschuld", "Zinsen", "Tilgung", "Restschuld"]));  // row 6
    let first = s.rows + 1;  // the first schedule row's number, 7

    for m in 1..=months {
        let at = s.push([m]);
        let r = at + 1;  // the row's own number
        // The first month starts from the loan; every later one from the last month's balance.
        s.set(s.at(at, 1), formula(if m == 1 { "$B$1" } else { "E" + (r - 1) }));
        s.set(s.at(at, 2), formula("B" + r + "*$B$2/12"));
        s.set(s.at(at, 3), formula("$B$4-C" + r));
        s.set(s.at(at, 4), formula("B" + r + "-D" + r));
    }
    let last = s.rows;
    s.format("B" + first + ":E" + last, money);
    s.style("A6:E6", style().background("silver").align("center").border("0.5pt solid navy"));
    s.width("A:A", "4cm");
    s.width("B:E", "3.2cm");

    // The balance falling to nothing, drawn on the sheet that holds it.
    s.chart(chart("line").series("E" + first + ":E" + last, "E6").title("Restschuld – " + sc.name)
        .position("0cm", "0cm").size("14cm", "7cm"));

    schedules.push(#{ name: sc.name, first: first, last: last, months: months, sc: sc });
    sheets.push(s);
}

// --- the comparison -----------------------------------------------------------------------

let cmp = sheet("Vergleich");
cmp.push(header(["Szenario", "Zinssatz", "Laufzeit (Jahre)", "Monatsrate", "Zinsen gesamt", "Gezahlt gesamt", "Restschuld am Ende"]));
for x in schedules {
    let at = cmp.push([x.name]);
    let r = at + 1;
    let from = "$" + x.name + ".";
    cmp.set(cmp.at(at, 1), formula(x.name + ".B2"));
    cmp.set(cmp.at(at, 2), formula(x.name + ".B3"));
    cmp.set(cmp.at(at, 3), formula(x.name + ".B4"));
    cmp.set(cmp.at(at, 4), formula("SUM(" + x.name + ".C" + x.first + ":C" + x.last + ")"));
    cmp.set(cmp.at(at, 5), formula("D" + r + "*" + x.months));
    cmp.set(cmp.at(at, 6), formula(x.name + ".E" + x.last));
}
let last_row = cmp.rows;
cmp.format("B2:B" + last_row, rate_format);
cmp.format("D2:G" + last_row, money);
cmp.style("A1:G1", style().background("silver").align("center").border("0.5pt solid navy"));
cmp.width("A:A", "4cm");
cmp.width("B:G", "3.4cm");
cmp.height("1:1", "8mm");

// The first sheet anybody opens, though the last one written.
let book = spreadsheet();
book.push(cmp);
for s in sheets { book.push(s); }
book;

fn test_a_monthly_payment_pays_the_loan_off(d) {
    // The closing balance of the last month is zero to the cent: a schedule that does not end
    // at zero has the wrong payment or the wrong interest, and every figure below is wrong.
    assert_near(d.cell("Vergleich.G2").value, 0.0, 0.01);
    assert_near(d.cell("Vergleich.G3").value, 0.0, 0.01);
    assert_near(d.cell("Vergleich.G4").value, 0.0, 0.01);
}

fn test_the_payment_matches_the_annuity_formula(d) {
    // 250.000 € at 3,6 % over 25 years: 750 / (1 − 1,003^−300) = 1.264,77 € a month.
    let rate = 0.036 / 12.0;
    let pay = 250000.0 * rate / (1.0 - 1.0 / ((1.0 + rate) ** 300.0));
    assert_near(d.cell("Vergleich.D2").value, pay, 0.005);
}

fn test_a_shorter_loan_costs_less_interest(d) {
    assert(d.cell("Vergleich.E3").value < d.cell("Vergleich.E2").value);
    assert(d.cell("Vergleich.E2").value < d.cell("Vergleich.E4").value);
}

fn test_the_document_contradicts_itself_nowhere(d) {
    // The labels are German in a document whose language is not stated, so the spell checker
    // has opinions about them, and a chart has no spelling in the projection yet (the named
    // gap in `doc/projection-sheet.md`). Neither is a contradiction.
    for finding in d.lint() {
        assert(finding.contains("[misspelt]") || finding.contains("[unspellable]"), finding);
    }
}
