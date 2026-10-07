// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later
//
// How big grind is, release by release — a spreadsheet that lives in git, built by a script.
//
//   examples/loc/update.sh              # count HEAD into history.json, then rebuild loc.fods
//   grind build examples/loc/loc.rhai -o examples/loc/loc.fods   # rebuild only
//
// Three files and one rule. `history.json` is the data: `update.sh` counts a commit's lines and
// adds them, once per release. This script is the shape: which sheets, which formulas, which
// charts. `loc.fods` is what `grind build` makes of the two — committed so that GitHub, `git
// diff` and any office suite can show it, and **never edited by hand**, because the next
// release regenerates it. Adding a snapshot adds one row to two tables, and the diff of the
// `.fods` says exactly that.
//
//   Dashboard  the latest figures and three charts
//   Areas      one row per snapshot, the crates summed into what they are for
//   Crates     one row per snapshot, one column per top-level directory
//
// Nothing below knows how many snapshots or crates there are. A crate that appears for the
// first time gets a column and is filed under an area by its name; a snapshot gets a row, and
// every range — the totals, the growth, all three charts — is written from counts measured here.

let history = json("history.json").snapshots;
let n = history.len();
if n == 0 {
    throw "history.json has no snapshots yet — run examples/loc/update.sh";
}

// --- what a crate is for ----------------------------------------------------------------------
//
// The one opinion in this file. A crate nobody filed lands in "Other", which is visible in the
// pie and is the reminder to file it.

fn area_of(name) {
    if name.starts_with("ui_") { return "Shells"; }
    switch name {
        "sheet" => "Spreadsheet",
        "text" => "Word processor",
        "core" => "Core",
        "xlsx" | "docx" | "ooxml" => "Import filters",
        "print" | "spell" => "Print & spelling",
        "build" => "Generator",
        "cli" => "CLI",
        _ => "Other",
    }
}

let area_order = [
    "Spreadsheet", "Word processor", "Core", "Shells", "Import filters", "CLI", "Generator",
    "Print & spelling", "Other",
];

// Every crate any snapshot has, in name order — a Rhai map's keys come back sorted.
let seen = #{};
for snap in history {
    for name in snap.crates.keys() {
        seen[name] = true;
    }
}
let crates = seen.keys();

// The areas that have a crate in them, in `area_order`'s order.
let areas = [];
for area in area_order {
    for name in crates {
        if area_of(name) == area {
            areas.push(area);
            break;
        }
    }
}

// --- the look ---------------------------------------------------------------------------------

let ink = "#1f3a5f";
let band = "#eef3f8";
let palette = [
    "#4e79a7", "#f28e2b", "#e15759", "#76b7b2", "#59a14f", "#edc948", "#b07aa1", "#ff9da7",
    "#9c755f",
];
let count = format("number").grouping();
let share = format("percent").decimals(1);

/// A table's header row: white on the ink, centred.
fn header(s, cells, ink) {
    let at = s.push(row(cells));
    let look = style().bold().color("white").background(ink).align("center").valign("middle");
    s.style(`${s.at(at, 0)}:${s.at(at, cells.len() - 1)}`, look);
    s.height(`${at + 1}:${at + 1}`, "0.8cm");
}

/// Every other body row shaded, so a wide table can be read across.
fn banding(s, first, last, width, band) {
    let shade = style().background(band);
    for r in range(first + 1, last + 1, 2) {
        s.style(`${s.at(r, 0)}:${s.at(r, width - 1)}`, shade);
    }
}

/// `=DATE(y;m;d)` from `2026-10-07` — a date has no other spelling in a script (§7).
fn date_of(text) {
    let p = text.split("-");
    `=DATE(${p[0].parse_int()};${p[1].parse_int()};${p[2].parse_int()})`
}

// Rows 0 is the header in both tables, so snapshot i is row i + 1 in both — which is what lets
// an Areas cell name its Crates cells by the same row.
let first = 1;
let last = n;

// --- Crates -----------------------------------------------------------------------------------

let by_crate = sheet("Crates");
let crate_cols = crates.len();
let crate_total = 2 + crate_cols;
header(by_crate, ["Date", "Commit"] + crates + ["Total"], ink);
for snap in history {
    let cells = [date_of(snap.date), snap.commit];
    for name in crates {
        // A crate that did not exist yet is an empty cell, not a zero: it was not small, it
        // was not there.
        cells.push(if name in snap.crates { snap.crates[name] } else { () });
    }
    let at = by_crate.push(cells);
    by_crate.set(by_crate.at(at, crate_total),
        formula("SUM(" + by_crate.at(at, 2) + ":" + by_crate.at(at, crate_total - 1) + ")"));
}
by_crate.format("A2:A" + (last + 1), "date");
by_crate.format(by_crate.at(first, 2) + ":" + by_crate.at(last, crate_total), count);
banding(by_crate, first, last, crate_total + 1, band);
by_crate.style(by_crate.at(first, crate_total) + ":" + by_crate.at(last, crate_total), style().bold());
by_crate.width("A:A", "2.4cm");
by_crate.width("B:B", "1.9cm");
by_crate.width(by_crate.at(0, 2) + ":" + by_crate.at(0, crate_total), "2.2cm");

// --- Areas ------------------------------------------------------------------------------------

let by_area = sheet("Areas");
let a0 = 3;                          // the first area column
let total = a0 + areas.len();        // then Total, Tests, Tests %, Docs, Growth
let tests = total + 1;
let tests_pct = total + 2;
let docs = total + 3;
let growth = total + 4;
header(by_area, ["Date", "Commit", "Release"] + areas + ["Total", "Tests", "Tests %", "Docs", "Growth"], ink);
for i in 0..n {
    let snap = history[i];
    let at = by_area.push([date_of(snap.date), snap.commit, snap.tag]);
    for j in 0..areas.len() {
        // Summed out of the Crates sheet rather than out of the data, so the two tables cannot
        // disagree: an area is a formula over the crates filed under it.
        // Only the crates this snapshot has: an empty cell would sum to the same answer, and
        // `grind lint` would rightly ask why a formula reads it.
        let terms = [];
        for c in 0..crate_cols {
            if area_of(crates[c]) == areas[j] && crates[c] in snap.crates {
                terms.push("Crates." + by_crate.at(at, 2 + c));
            }
        }
        let cell = by_area.at(at, a0 + j);
        if terms.is_empty() {
            by_area.set(cell, 0);
        } else {
            by_area.set(cell, formula(`SUM(${terms.reduce(|sum, t| sum + ";" + t)})`));
        }
    }
    by_area.set(by_area.at(at, total),
        formula("SUM(" + by_area.at(at, a0) + ":" + by_area.at(at, total - 1) + ")"));
    by_area.set(by_area.at(at, tests), snap.tests);
    by_area.set(by_area.at(at, tests_pct), formula(by_area.at(at, tests) + "/" + by_area.at(at, total)));
    by_area.set(by_area.at(at, docs), snap.docs);
    if at > first {
        by_area.set(by_area.at(at, growth), formula(by_area.at(at, total) + "-" + by_area.at(at - 1, total)));
    }
}
by_area.format("A2:A" + (last + 1), "date");
by_area.format(by_area.at(first, a0) + ":" + by_area.at(last, tests), count);
by_area.format(by_area.at(first, tests_pct) + ":" + by_area.at(last, tests_pct), share);
by_area.format(by_area.at(first, docs) + ":" + by_area.at(last, growth), count);
banding(by_area, first, last, growth + 1, band);
by_area.style(by_area.at(first, total) + ":" + by_area.at(last, total), style().bold());
by_area.width("A:A", "2.4cm");
by_area.width("B:C", "1.9cm");
by_area.width(by_area.at(0, a0) + ":" + by_area.at(0, growth), "2.6cm");
by_area.style(by_area.at(0, a0) + ":" + by_area.at(0, growth), style().wrap());

// --- Dashboard --------------------------------------------------------------------------------

let latest = history[n - 1];
let board = sheet("Dashboard");
board.push(row(["grind — lines of code"]).style(style().bold().size("20pt").color(ink)));
board.push(row(["Non-blank lines of Rust per crate, one snapshot per release. Rebuilt by examples/loc/update.sh from history.json — edit neither by hand."])
    .style(style().italic().color("gray")));
board.push([]);
board.push(row(["Lines of Rust", "Since " + history[0].date, "Of which tests", "Docs (Markdown)", "Crates"])
    .style(style().color("gray").size("9pt")));

// Every figure is a formula into the tables, so the dashboard is never a second copy of a number.
let l = "Areas." + by_area.at(last, total);
let f = "Areas." + by_area.at(first, total);
board.push([
    formula(l),
    formula("(" + l + "-" + f + ")/" + f),
    formula("Areas." + by_area.at(last, tests_pct)),
    formula("Areas." + by_area.at(last, docs)),
    formula("COUNT(Crates." + by_crate.at(last, 2) + ":" + by_crate.at(last, crate_total - 1) + ")"),
]);
board.format("A5", count).format("B5", share).format("C5", share).format("D5", count);
board.style("A5:E5", style().bold().size("22pt").color(ink).align("left"));
board.push(row([
    "at " + latest.commit + (if latest.tag == "" { "" } else { " (" + latest.tag + ")" }) + ", " + latest.date,
]).style(style().color("gray").size("9pt")));
board.style("A4:E6", style().background(band));
board.height("1:1", "1.2cm").height("3:3", "0.3cm").height("5:5", "1.2cm");
board.width("A:E", "5cm");

// Three charts under the figures: how it grew, where it is now, and what it is made of.
let dates = "Areas.A2:A" + (last + 1);
let grew = chart("line")
    .title("Lines of Rust by area")
    .categories(dates)
    .legend("bottom")
    .gridlines()
    .colors(palette)
    .position("0cm", "5.4cm")
    .size("25cm", "11cm");
for j in 0..areas.len() {
    grew = grew.series("Areas." + by_area.at(first, a0 + j) + ":" + by_area.at(last, a0 + j),
        "Areas." + by_area.at(0, a0 + j));
}
board.chart(grew);

board.chart(chart("bar")
    .title("Lines per crate at " + latest.commit)
    .categories("Crates." + by_crate.at(0, 2) + ":" + by_crate.at(0, crate_total - 1))
    .series("Crates." + by_crate.at(last, 2) + ":" + by_crate.at(last, crate_total - 1))
    .legend("none")
    .gridlines()
    .colors([ink])
    .position("0cm", "16.8cm")
    .size("15cm", "10cm"));

board.chart(chart("pie")
    .title("Share by area")
    .categories("Areas." + by_area.at(0, a0) + ":" + by_area.at(0, total - 1))
    .series("Areas." + by_area.at(last, a0) + ":" + by_area.at(last, total - 1))
    .colors(palette)
    .position("15.4cm", "16.8cm")
    .size("9.6cm", "10cm"));

let book = spreadsheet();
book.push(board);
book.push(by_area);
book.push(by_crate);
book;

// --- checks: `grind test examples/loc/loc.rhai` -----------------------------------------------

/// A column's letters from its index — enough for a test, which has no `s.at`.
fn column(i) {
    let letters = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    if i < 26 { letters[i].to_string() } else { column(i / 26 - 1) + letters[i % 26] }
}

/// Where a sheet's header row says `Total`.
fn total_column(d, sheet) {
    let c = 0;
    while d.cell(sheet + "." + column(c) + "1").value != "Total" {
        c += 1;
        if c > 200 { throw sheet + " has no Total column"; }
    }
    column(c)
}

/// The areas are a regrouping of the crates, so the two totals must agree on every row — a
/// crate missing from every area, or filed under two, would show here.
fn test_areas_and_crates_agree(d) {
    let areas = total_column(d, "Areas");
    let crates = total_column(d, "Crates");
    let r = 2;
    while d.cell("Crates.A" + r).value != () {
        assert_eq(d.cell("Areas." + areas + r).value, d.cell("Crates." + crates + r).value);
        r += 1;
    }
    assert(r > 2, "no snapshot rows");
}

/// The dashboard's headline is the last row's total, not a number of its own.
fn test_the_headline_is_the_latest_total(d) {
    let areas = total_column(d, "Areas");
    let r = 2;
    while d.cell("Areas.A" + (r + 1)).value != () { r += 1; }
    assert_eq(d.cell("Dashboard.A5").value, d.cell("Areas." + areas + r).value);
}

/// Nothing `grind lint` would complain about — except that a chart has no `.grind` spelling
/// yet, which is true of every chart and is the projection's gap rather than this document's.
fn test_the_document_is_clean(d) {
    assert_eq(d.lint().filter(|line| !line.contains("[unspellable]")), []);
}
