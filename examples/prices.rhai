// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later
//
// Data and code, separated — doc/generator-spec.md §3.5.
//
//   grind build examples/prices.rhai -o prices.fods
//
// Everything this document *says* is here; everything it *contains* is in `prices.json` beside
// it, which somebody who has never read a line of Rhai can edit. That is the whole point: a
// price list changes weekly and its shape changes yearly, and they should not be the same file.
//
// A script reads JSON and nothing else, from one directory a person named — the script's own,
// or `--data`. `..`, an absolute path and a symlink pointing out are all refused; there is no
// writing, no network, and no way to reach anything else.

let source = json("prices.json");

let money = format("currency").symbol(source.currency).grouping().decimals(2);

let s = sheet("Prices");
s.push(row(["SKU", "Item", "Unit", "Price", "Qty", "Value"]).bold());

for item in source.items {
    let at = s.push([item.sku, item.name, item.unit, item.price, item.quantity]);
    // The line's value, written for the row it landed on.
    s.set(s.at(at, 5), formula(s.at(at, 3) + "*" + s.at(at, 4)));
}

let total = s.push(row(["Total", (), (), (), (), sum_above()]).bold());

// VAT and the gross total, from the same file's own rate.
let vat = s.rows;
s.set(s.at(vat, 1), "VAT at " + (source.vat * 100) + "%");
s.set(s.at(vat, 5), formula(s.at(total, 5) + "*" + source.vat));
s.set(s.at(vat + 1, 1), "Gross");
s.set(s.at(vat + 1, 5), formula(s.at(total, 5) + "+" + s.at(vat, 5)));

s.format("D2:D" + (total), money);
s.format("F2:F" + (vat + 2), money);
s.style("A1:F1", style().background("silver").align("center"));
s.style("A2:A" + (total), style().color("navy"));
s.width("B:B", "6cm");
s.width("C:C", "3cm");
s.name("lineValues", "F2:F" + total);

s
