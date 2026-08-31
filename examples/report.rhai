// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later
//
// A text document, generated — doc/dsl.md layer 1, D7, the word processor's half.
//
//   grind build examples/report.rhai -o report.fodt
//
// The same argument as `examples/budget.rhai` makes for a spreadsheet: the *content* is data
// at the top, and the document is what the loop does with it. A section per region, each with
// the same shape, and adding one is adding a line to `REGIONS`.

const REGIONS = [
    ["North", "led on volume", 12],
    ["South", "held flat", 0],
    ["East",  "grew from a small base", 34],
    ["West",  "slipped on one large account", -6],
];

let d = text();

d.heading(1, "Quarterly report");
// A bookmark is an address that survives editing — `grind text view #summary` finds this
// paragraph wherever it ends up, which `p2` does not.
d.para("Revenue rose **12%** on the year, with growth concentrated in two regions.");
d.bookmark("summary");

d.heading(2, "By region");
for r in REGIONS {
    // The inline notation is the one the suite already reads while you type: `**bold**` and
    // `_italic_` mean here exactly what they mean in a shell's editor.
    d.item(1, "**" + r[0] + "** " + r[1] + " (" + r[2] + "%)");
}

d.heading(2, "What is not here");
d.para("A generator says structure and text. Character formatting beyond the notation, images and named styles reach the model through a caret rather than through a block, so a script cannot say them — see `build/src/text.rs`.");

d
