// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later
//
// A grade book — doc/dsl.md layer 1.
//
//   grind build examples/notenbuch/notenbuch.rhai -o notenbuch.fods
//   grind test  examples/notenbuch/notenbuch.rhai
//
// Three tables that have to agree: the **weights** (how much each kind of assessment counts),
// the **scale** (from which percentage a work is a 1, a 2 …) and the **gradebook** that reads
// both. Neither the weights nor the scale are typed into a formula: each grade is a lookup into
// the Notenschluessel sheet and each average multiplies by cells on the Gewichte sheet, so a
// teacher who changes a weight there changes every row, and the script's only job is to say
// the *shape* once.
//
// The point of the tests is the edges: a work of exactly 70 points is a 2, not a 3, and the
// weights must add up to 100 or the script refuses to build — a grade book whose weights add up
// to 110 gives everybody a better grade and nobody notices. The students are invented.

let source = json("notenbuch.json");

/// A header row: bold, and nothing else.
fn header(cells) { row(cells).bold() }

let points = format("number").decimals(1);
let percent = format("number").decimals(1);

// --- weights: refuse to build a book whose weights do not add up -----------------------------

let weight_sum = 0;
for w in source.weights { weight_sum += w.weight; }
if weight_sum != 100 {
    throw "the weights add up to " + weight_sum + ", not 100";
}

let weights = sheet("Gewichte");
weights.push(header(["Leistung", "Gewicht (%)"]));
for w in source.weights { weights.push([w.name, w.weight]); }
let weights_last = weights.rows;
weights.push(row(["Summe", sum_above()]).bold());
weights.width("A:A", "5cm");

// --- the scale: a lookup table, ascending, which is what an approximate VLOOKUP needs -------

let scale = sheet("Notenschluessel");
scale.push(header(["Ab Prozent", "Note"]));
for x in source.scale { scale.push([x.from, x.grade]); }
let scale_range = "Notenschluessel.$A$2:$B$" + scale.rows;

// --- the grade book -------------------------------------------------------------------------

let book = sheet("Notenspiegel");
let heads = ["Name"];
for w in source.weights { heads.push(w.name); }
heads.push("Durchschnitt (%)");
heads.push("Note");
heads.push("Hinweis");
book.push(header(heads));

let n = source.weights.len;
for st in source.students {
    let cells = [st.name];
    for s in st.scores { cells.push(s); }
    let at = book.push(cells);
    let r = at + 1;

    // The weighted average: each score times the weight *cell* it belongs to, over their sum.
    // Written out, because it is short and because a reader can see each term.
    let terms = [];
    for i in 0..n {
        terms.push(book.at(at, 1 + i) + "*Gewichte.$B$" + (2 + i));
    }
    let weighted = "(" + terms.reduce(|sum, t| sum + "+" + t, terms.shift()) + ")/Gewichte.$B$" + (weights_last + 1);
    book.set(book.at(at, 1 + n), formula(weighted));
    book.set(book.at(at, 2 + n), formula("VLOOKUP(" + book.at(at, 1 + n) + ";" + scale_range + ";2;TRUE())"));
    book.set(book.at(at, 3 + n), formula("IF(" + book.at(at, 2 + n) + ">=5;\"nicht ausreichend\";IF(" + book.at(at, 2 + n) + "=4;\"gefährdet\";\"\"))"));
}
let last = book.rows;
let avg_col = 1 + n;   // 0-based index of the average column
let grade_col = 2 + n;
let first_cell = book.at(1, 1);

book.format(book.at(1, 1) + ":" + book.at(last - 1, avg_col), percent);
book.style("A1:" + book.at(0, 3 + n), style().background("silver").align("center").border("0.5pt solid navy"));
book.width("A:A", "5cm");
let corner = book.at(0, 3 + n);
book.width("B:" + corner.sub_string(0, corner.len - 1), "3.4cm");
book.height("1:1", "8mm");

// --- the class at a glance -----------------------------------------------------------------

let grades = book.at(1, grade_col) + ":" + book.at(last - 1, grade_col);
let avg = book.at(1, avg_col) + ":" + book.at(last - 1, avg_col);
book.push([]);
let first_fact = book.push(["Klassendurchschnitt (%)", (), (), (), (), formula("AVERAGE(" + avg + ")")]);
book.push(["Notendurchschnitt", (), (), (), (), formula("AVERAGE(" + grades + ")")]);
book.push(["Anzahl nicht ausreichend", (), (), (), (), formula("COUNTIF(" + grades + ";\">=5\")")]);
book.format(book.at(first_fact, avg_col), percent);
book.format(book.at(first_fact + 1, avg_col), percent);
book.style("A" + (first_fact + 1) + ":A" + (first_fact + 3), style().italic());

let d = spreadsheet();
d.push(book);
d.push(weights);
d.push(scale);
d;

// --- checks ----------------------------------------------------------------------------------

fn test_a_weighted_average(d) {
    // Alex: 0,25 × 88 + 0,25 × 92 + 0,10 × 90 + 0,40 × 85 = 88.
    assert_near(d.cell("F2").value, 88.0, 0.0001);
    assert_eq(d.cell("G2").value, 1);
}

fn test_a_grade_boundary_belongs_to_the_better_grade(d) {
    // Gabi has 70 everywhere: exactly the lower edge of a 2, and so a 2 rather than a 3.
    assert_eq(d.cell("F8").value, 70);
    assert_eq(d.cell("G8").value, 2);
    // Hanno averages exactly 85 — the edge of a 1.
    assert_eq(d.cell("F9").value, 85);
    assert_eq(d.cell("G9").value, 1);
}

fn test_the_lowest_grades_are_flagged(d) {
    // Dana averages 42,25 — a 4, "gefährdet"; Finn 29,75 — a 5, which is not enough; Chris,
    // at 56,55, is a 3 and has no remark at all.
    assert_eq(d.cell("G5").value, 4);
    assert_eq(d.cell("H5").value, "gefährdet");
    assert_eq(d.cell("G4").value, 3);
    assert_eq(d.cell("H4").value, "");
    assert_eq(d.cell("H7").value, "nicht ausreichend");
}

fn test_the_weights_are_looked_up_not_copied(d) {
    // Change a cell on Gewichte and every average changes: the formula reads the cell.
    assert(d.cell("F2").formula.contains("Gewichte"), d.cell("F2").formula);
}

fn test_the_document_contradicts_itself_nowhere(d) {
    // German labels in a document whose language is not stated, and no spelling for a chart
    // — neither is a contradiction.
    for finding in d.lint() {
        assert(finding.contains("[misspelt]") || finding.contains("[unspellable]"), finding);
    }
}
