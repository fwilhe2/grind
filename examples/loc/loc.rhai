// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later
//
// How big grind is, release by release — a spreadsheet that lives in git, built by a script.
//
//   examples/loc/update.sh              # count HEAD into history.json, then rebuild loc.fods
//   grind build examples/loc/loc.rhai -o examples/loc/loc.fods   # rebuild only
//   grind test examples/loc/loc.rhai    # and check it agrees with itself
//
// Three files and one rule. `history.json` is the data: `update.sh` counts a commit's lines and
// adds them, once per release. This script is the shape: which sheets, which formulas, which
// charts. `loc.fods` is what `grind build` makes of the two — committed so that `git diff`
// and any office suite can show it, and **never edited by hand**, because the next release
// regenerates it. Adding a snapshot adds one row to each table, and the diff of the `.fods`
// says exactly that.
//
//   Growth   the latest figures, one row per snapshot with the crates summed into what they
//            are for, and two charts: how it grew, and what it is made of now
//   Crates   one row per snapshot, one column per top-level directory, and a bar chart of
//            the latest row
//
// Nothing below knows how many snapshots or crates there are. A crate that appears for the
// first time gets a column and is filed under an area by its name; a snapshot gets a row, and
// every range — the totals, the growth, all three charts and where they sit — is written from
// counts measured here.
//
// Every chart is on the sheet it reads, or after it. LibreOffice draws a chart whose ranges
// are on a *later* sheet as empty, because this build does not write the cached copy of the
// data it would otherwise fall back on (doc/chart-format.md) — which is why there is no
// dashboard sheet in front of the tables.

let history = json("history.json").snapshots;
let n = history.len();
if n == 0 {
    throw "history.json has no snapshots yet — run examples/loc/update.sh";
}
let latest = history[n - 1];

// --- what a crate is for ----------------------------------------------------------------------
//
// The one opinion in this file. A crate nobody filed lands in "Other", which shows in the pie
// and is the reminder to file it.

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

// Every row gets a height, because a chart is placed in centimetres and has to know where the
// table above it ends.
let row_h = 0.5;
let header_h = 1.0;

/// A table's header row: white on the ink, centred, wrapping a long name onto two lines.
fn header(s, cells, ink, height) {
    let at = s.push(row(cells));
    let look = style().bold().color("white").background(ink).align("center").valign("middle").wrap();
    s.style(`${s.at(at, 0)}:${s.at(at, cells.len() - 1)}`, look);
    s.height(`${at + 1}:${at + 1}`, `${height}cm`);
    at
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

/// `A2:A9`, from two corners given as indices.
fn span(s, r1, c1, r2, c2) {
    `${s.at(r1, c1)}:${s.at(r2, c2)}`
}

// --- Crates -----------------------------------------------------------------------------------
//
// Built first because the Growth sheet's formulas name its cells. Header on row 0, so snapshot
// i is on row i + 1.

let by_crate = sheet("Crates");
let crate_cols = crates.len();
let crate_total = 2 + crate_cols;
header(by_crate, ["Date", "Commit"] + crates + ["Total"], ink, header_h);
for snap in history {
    let cells = [date_of(snap.date), snap.commit];
    for name in crates {
        // A crate that did not exist yet is an empty cell, not a zero: it was not small, it
        // was not there.
        cells.push(if name in snap.crates { snap.crates[name] } else { () });
    }
    let at = by_crate.push(cells);
    by_crate.set(by_crate.at(at, crate_total),
        formula(`SUM(${span(by_crate, at, 2, at, crate_total - 1)})`));
}
by_crate.format(span(by_crate, 1, 0, n, 0), "date");
by_crate.format(span(by_crate, 1, 2, n, crate_total), count);
banding(by_crate, 1, n, crate_total + 1, band);
by_crate.style(span(by_crate, 1, crate_total, n, crate_total), style().bold());
by_crate.height(`2:${n + 1}`, `${row_h}cm`);
by_crate.width("A:A", "2.8cm");
by_crate.width("B:B", "1.9cm");
by_crate.width(span(by_crate, 0, 2, 0, crate_total), "2.5cm");

by_crate.chart(chart("bar")
    .title(`Lines per crate at ${latest.commit}`)
    .categories(span(by_crate, 0, 2, 0, crate_total - 1))
    .series(span(by_crate, n, 2, n, crate_total - 1))
    .legend("none")
    .gridlines()
    .colors([ink])
    .position("0cm", `${header_h + n * row_h + 0.6}cm`)
    .size("24cm", "10cm"));

// --- Growth -----------------------------------------------------------------------------------

let growth = sheet("Growth");

// The title, and the five figures a reader came for. Each is a formula into the table below,
// so the summary is never a second copy of a number. Labels are text and spill across the empty
// cells beside them; the values sit in column D, which is wide enough for them.
growth.push(row(["grind — lines of code"]).style(style().bold().size("20pt").color(ink)));
growth.push(row(["Non-blank lines of Rust per crate, one snapshot per release. Rebuilt by examples/loc/update.sh from history.json — edit neither by hand."])
    .style(style().italic().color("gray")));
let facts = growth.rows;
let fact_labels = ["Lines of Rust", `Growth since ${history[0].date}`, "Of which tests", "Docs (Markdown)", "Crates"];
for label in fact_labels {
    growth.push([label]);
}
growth.push([]);

// The table: header on `top`, snapshot i on `top + 1 + i`.
let a0 = 3;                          // the first area column, after Date, Commit, Release
let total = a0 + areas.len();
let tests = total + 1;
let tests_pct = total + 2;
let docs = total + 3;
let delta = total + 4;
let top = header(growth,
    ["Date", "Commit", "Release"] + areas + ["Total", "Tests", "Tests %", "Docs", "Growth"],
    ink, header_h);
let first = top + 1;
let last = top + n;
for i in 0..n {
    let snap = history[i];
    let crate_row = i + 1;
    let at = growth.push([date_of(snap.date), snap.commit, snap.tag]);
    for j in 0..areas.len() {
        // Summed out of the Crates sheet rather than out of the data, so the two tables cannot
        // disagree: an area is a formula over the crates filed under it. Only the crates this
        // snapshot has — an empty cell would sum the same, and `grind lint` would rightly ask
        // why a formula reads it.
        let terms = [];
        for c in 0..crate_cols {
            if area_of(crates[c]) == areas[j] && crates[c] in snap.crates {
                terms.push("Crates." + by_crate.at(crate_row, 2 + c));
            }
        }
        let cell = growth.at(at, a0 + j);
        if terms.is_empty() {
            growth.set(cell, 0);
        } else {
            growth.set(cell, formula(`SUM(${terms.reduce(|sum, t| sum + ";" + t)})`));
        }
    }
    growth.set(growth.at(at, total), formula(`SUM(${span(growth, at, a0, at, total - 1)})`));
    growth.set(growth.at(at, tests), snap.tests);
    growth.set(growth.at(at, tests_pct), formula(`${growth.at(at, tests)}/${growth.at(at, total)}`));
    growth.set(growth.at(at, docs), snap.docs);
    if at > first {
        growth.set(growth.at(at, delta), formula(`${growth.at(at, total)}-${growth.at(at - 1, total)}`));
    }
}
growth.format(span(growth, first, 0, last, 0), "date");
growth.format(span(growth, first, a0, last, tests), count);
growth.format(span(growth, first, tests_pct, last, tests_pct), share);
growth.format(span(growth, first, docs, last, delta), count);
banding(growth, first, last, delta + 1, band);
growth.style(span(growth, first, total, last, total), style().bold());

// The figures, now that the table they read has addresses.
let now = growth.at(last, total);
let then = growth.at(first, total);
growth.set(growth.at(facts, 3), formula(now));
growth.set(growth.at(facts + 1, 3), formula(`(${now}-${then})/${then}`));
growth.set(growth.at(facts + 2, 3), formula(growth.at(last, tests_pct)));
growth.set(growth.at(facts + 3, 3), formula(growth.at(last, docs)));
growth.set(growth.at(facts + 4, 3),
    formula(`COUNT(Crates.${span(by_crate, n, 2, n, crate_total - 1)})`));
growth.set(growth.at(facts, 5),
    `at ${latest.commit}${if latest.tag == "" { "" } else { " (" + latest.tag + ")" }}, ${latest.date}`);
growth.format(growth.at(facts, 3), count)
    .format(growth.at(facts + 1, 3), share)
    .format(growth.at(facts + 2, 3), share)
    .format(growth.at(facts + 3, 3), count);
growth.style(span(growth, facts, 0, facts + 4, 5), style().background(band));
growth.style(span(growth, facts, 0, facts + 4, 2), style().color("gray"));
growth.style(span(growth, facts, 3, facts + 4, 3), style().bold().color(ink).align("right"));
growth.style(growth.at(facts, 5), style().italic().color("gray"));

growth.height("1:1", "1.2cm");
growth.height(`2:${top}`, `${row_h}cm`);
growth.height(`${first + 1}:${last + 1}`, `${row_h}cm`);
growth.width("A:A", "2.8cm");
growth.width("B:C", "1.9cm");
growth.width(span(growth, 0, a0, 0, delta), "2.6cm");

// Under the table: how it grew, and what it is made of now.
let below = `${1.2 + (top - 1) * row_h + header_h + n * row_h + 0.6}cm`;
let grew = chart("line")
    .title("Lines of Rust by area")
    .categories(span(growth, first, 0, last, 0))
    .legend("bottom")
    .gridlines()
    .colors(palette)
    .position("0cm", below)
    .size("24cm", "11cm");
for j in 0..areas.len() {
    grew = grew.series(span(growth, first, a0 + j, last, a0 + j), growth.at(top, a0 + j));
}
growth.chart(grew);

growth.chart(chart("pie")
    .title(`Share by area at ${latest.commit}`)
    .categories(span(growth, top, a0, top, total - 1))
    .series(span(growth, last, a0, last, total - 1))
    .colors(palette)
    .position("24.4cm", below)
    .size("13cm", "11cm"));

let book = spreadsheet();
book.push(growth);
book.push(by_crate);
book;

// --- checks: `grind test examples/loc/loc.rhai` -----------------------------------------------

/// A column's letters from its index — a test has the built document, not the sheet builder,
/// and so no `s.at`.
fn column(i) {
    let letters = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    if i < 26 { letters[i].to_string() } else { column(i / 26 - 1) + letters[i % 26] }
}

/// The row a sheet's table header is on (1-based), found by its first cell.
fn header_row(d, sheet) {
    let r = 1;
    while d.cell(`${sheet}.A${r}`).value != "Date" {
        r += 1;
        if r > 100 { throw `${sheet} has no table`; }
    }
    r
}

/// Where a sheet's table header says `Total`.
fn total_column(d, sheet) {
    let r = header_row(d, sheet);
    let c = 0;
    while d.cell(`${sheet}.${column(c)}${r}`).value != "Total" {
        c += 1;
        if c > 200 { throw `${sheet} has no Total column`; }
    }
    column(c)
}

/// The areas are a regrouping of the crates, so the two totals agree on every row — a crate
/// missing from every area, or filed under two, would show here.
fn test_areas_and_crates_agree(d) {
    let g = total_column(d, "Growth");
    let c = total_column(d, "Crates");
    let gr = header_row(d, "Growth") + 1;
    let cr = header_row(d, "Crates") + 1;
    while d.cell(`Crates.A${cr}`).value != () {
        assert_eq(d.cell(`Growth.${g}${gr}`).value, d.cell(`Crates.${c}${cr}`).value);
        gr += 1;
        cr += 1;
    }
    assert(cr > 2, "no snapshot rows");
}

/// The headline is the last row's total, not a number of its own.
fn test_the_headline_is_the_latest_total(d) {
    let g = total_column(d, "Growth");
    let r = header_row(d, "Growth") + 1;
    while d.cell(`Growth.A${r + 1}`).value != () { r += 1; }
    assert_eq(d.cell("Growth.D3").value, d.cell(`Growth.${g}${r}`).value);
}

/// Nothing `grind lint` would complain about — except that a chart has no `.grind` spelling
/// yet, which is true of every chart and is the projection's gap rather than this document's.
fn test_the_document_is_clean(d) {
    assert_eq(d.lint().filter(|line| !line.contains("[unspellable]")), []);
}
