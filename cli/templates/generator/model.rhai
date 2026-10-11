// A spreadsheet, built by a script — the starter `grind init` writes.
//
//   grind build model.rhai -o model.fods     the Build task, Ctrl+Shift+B in VS Code
//   grind test  model.rhai                   the tests at the bottom of this file
//
// The numbers live in data.json beside this file: change them there and rebuild, and every
// formula, total and format below follows without being touched. The script's value — its last
// expression, `s;` above the tests — is the document `grind build` writes.
//
// Type a few letters of `push`, `format` or `style` for completion with documentation, and see
// doc/generator-guide.md (https://github.com/fwilhe2/grind) for the whole language.

let data = json("data.json");
let money = format("currency").symbol(data.currency).grouping().decimals(2);

let s = sheet("Costs");
s.push(row(["Item", "Monthly", "Yearly", "Share"]).bold());

// Where the total will land, known before the rows are written because the data is known: one
// row per item, after the header.
let total = data.items.len + 1;

for item in data.items {
    let at = s.push([item.name, item.monthly]);
    // Each formula is written for the row it lands on, so nothing here needs a `$`.
    s.set(s.at(at, 2), formula(s.at(at, 1) + "*12"));
    s.set(s.at(at, 3), formula(s.at(at, 2) + "/" + s.at(total, 2)));
}

// `sum_above()` sums the run of numbers directly above wherever it lands — an AutoSum.
s.push(row(["Total", sum_above(), sum_above()]).bold());

s.format("B2:C" + (total + 1), money);
s.format("D2:D" + total, format("percent").decimals(1));
s.style("A1:D1", style().background("silver"));
s.width("A:A", "5cm");

s;

// --- what must stay true -------------------------------------------------------------------
//
// `grind test model.rhai` builds the document above, recalculates it, and calls every
// `test_…` function with it. `grind build` never calls them, so a model and its checks are one
// file. The addresses assume data.json's three items: the total is row 5.

fn test_a_year_is_twelve_months(d) {
    assert_near(d.cell("C5").value, d.cell("B5").value * 12, 0.005);
}

fn test_the_shares_add_up_to_the_whole(d) {
    assert_near(d.cell("D2").value + d.cell("D3").value + d.cell("D4").value, 1, 0.000001);
}

fn test_nothing_contradicts_itself(d) {
    assert(d.lint().is_empty(), "grind lint finds nothing");
}
