// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later
//
// A series of letters — Word's Serienbrief, with the arithmetic inside — doc/dsl.md layer 1.
//
//   grind build examples/letters/nebenkosten.rhai -o letters --as pdf   # a PDF per recipient
//   grind build examples/letters/nebenkosten.rhai -o letters            # or flat ODF
//   grind test  examples/letters/nebenkosten.rhai
//
// The yearly service-charge statement (Nebenkostenabrechnung) of a small block of flats: one
// letter per tenancy, each with that tenant's own share of every cost, what they prepaid, and
// whether they owe money or get some back — then a page for the property manager that says
// whether the letters add up to the bills.
//
// A mail merge fills *fields* in. This one **calculates**. The share of the water bill depends
// on how many people lived in the flat for how many months, and the person who moved out in
// June and the one who moved in in July each get a letter about the same flat. Doing that in a
// letter template means a spreadsheet beside it and somebody copying figures across; here the
// figures and the wording come out of one source, so the letter cannot say 3,412.50 € where
// the sum says 3,421.50 €.
//
// It comes out as **DIN 5008 letters** — a PDF per tenancy (`grind build … --as pdf`) — set in a
// template (`brief-din5008.fodt`): the page, the letterhead and the paragraph styles are
// designed once in a word processor, and this script says which style each block is set in.
//
// Everything that is a person, an address or a price is in `nebenkosten.json` — and **every
// bit of it is made up**: Musterstadt, example.org, an IBAN that is all zeros. Nothing here
// reads the clock (doc/generator-spec.md §2.5), so the date on the letters is data too. That
// is a feature: the same data builds the same letters, byte for byte, the day it is sent and
// the day an auditor asks what was sent.

let source = json("nebenkosten.json");

// --- German figures ------------------------------------------------------------------------
//
// A text document holds *text*, so a number has to be spelled out by the script, and the
// spelling is the reader's: 1.234,56 € rather than 1,234.56. The locale is an argument — never
// the machine's — so the same script writes the same letters anywhere.

fn de(x, decimals) { format_number(x, decimals, "de-DE") }
fn eur(x) { format_money(x, "€", "de-DE") }

/// A figure in cents, so that the shares of one bill are rounded once, each, and then *added* —
/// the way the tenants' letters will be checked by a person with a calculator.
fn cents(x) { (x.to_float() * 100.0).round().to_int() }

// --- the keys ------------------------------------------------------------------------------
//
// Each cost is split by a *key*, and each key is a weight per tenancy: the flat's area, the
// persons living in it, or one per flat. Every weight is multiplied by the months the tenancy
// lasted, so a flat that changed hands in July counts for twelve months in total, split six and
// six.

const KEYS = #{
    area:    "Wohnfläche",
    persons: "Personenzahl",
    units:   "Wohneinheiten",
};

fn weight(t, key) {
    let per_month = if key == "area" {
        t.area.to_float()
    } else if key == "persons" {
        t.persons.to_float()
    } else {
        1.0
    };
    per_month * t.months.to_float()
}

let tenancies = source.tenancies;

let totals = #{ area: 0.0, persons: 0.0, units: 0.0, kwh: 0.0 };
for t in tenancies {
    for key in ["area", "persons", "units"] {
        totals[key] += weight(t, key);
    }
    totals.kwh += t.kwh.to_float();
}

// --- the bills -----------------------------------------------------------------------------
//
// One map per tenancy, built before any letter is written: the property manager's page at the
// end needs all of them, and the checks need them before anything is allowed to leave.

let consumption_share = source.heating.consumption_share;
let heating = source.heating.amount;

let bills = [];
let allocated = 0;
for t in tenancies {
    let lines = [];
    let sum = 0;

    for c in source.costs {
        let part = weight(t, c.key) / totals[c.key];
        let share = cents(c.amount.to_float() * part);
        lines.push(#{ name: c.name, key: c.key, total: c.amount, part: part, share: share });
        sum += share;
    }

    // Heating is two bills in one: the part that follows what the meter says, and the base
    // charge that follows the floor space.
    let metered = heating * consumption_share;
    let base = heating - metered;
    let metered_part = t.kwh.to_float() / totals.kwh;
    let base_part = weight(t, "area") / totals.area;
    let metered_share = cents(metered * metered_part);
    let base_share = cents(base * base_part);
    sum += metered_share + base_share;

    let paid = cents(t.prepayment) * t.months;
    bills.push(#{
        t: t,
        lines: lines,
        metered: metered, metered_part: metered_part, metered_share: metered_share,
        base: base, base_part: base_part, base_share: base_share,
        sum: sum, paid: paid, balance: sum - paid,
    });
    allocated += sum;
}

// The shares are rounded one by one, so the letters can be a few cents off the bills. A few
// cents is rounding. More than that is a wrong key or a missing tenancy, and a generator that
// writes six letters anyway is worse than one that stops: this is `throw`, and `grind build`
// reports it with the line.
let billed = 0;
for c in source.costs {
    billed += cents(c.amount);
}
billed += cents(heating);
let drift = allocated - billed;
if drift.abs() > bills.len * 2 * (source.costs.len + 2) {
    throw "the letters add up to " + eur(allocated / 100.0) + " but the bills are "
        + eur(billed / 100.0);
}

// --- the letters ---------------------------------------------------------------------------

// The letterhead, the page, the footer and the paragraph styles are a *template*: a flat ODF
// document beside this script, which a generator could not declare and a person can design in
// any word processor. `brief-din5008.fodt` is DIN 5008 Form B — its header says which
// measurement is which. The script says blocks and names the style each one is set in.
let landlord = source.landlord;
let letters = #{};

/// The nine lines of DIN 5008's address field, each also a line of the information block beside
/// it. The left of the first tab is the address — empty for the three lines of the additional
/// and remark zone, then the name, the street and the town — and what follows is a label, a tab
/// and its value. One paragraph per line, because the block shares the field's rows.
fn field(d, address, info) {
    for row in 0..9 {
        let line = if row >= 3 && row - 3 < address.len { address[row - 3] } else { "" };
        if row < info.len && info[row][0] != "" {
            line += "\t" + info[row][0] + "\t" + info[row][1];
        }
        d.para("Anschriftzeile", line);
    }
}

for b in bills {
    let t = b.t;
    let d = text_from("brief-din5008.fodt");

    d.para("Ruecksendeangabe", landlord.name + " · " + landlord.street + " · " + landlord.city);
    field(d, [t.name, source.building.street, source.building.city], [
        ["Zeichen:", "NK-" + source.year + "-" + t.id.to_upper()],
        ["Bearbeiterin:", landlord.signed],
        ["Telefon:", landlord.phone],
        ["", ""],
        ["Datum:", source.issued],
    ]);

    d.para("Betreff", "Betriebskostenabrechnung " + source.year + " – " + t.unit);

    d.para("Brieftext", t.greeting + ",");
    d.para("Brieftext", "hiermit erhalten Sie die Abrechnung der Betriebskosten für **" + t.unit
        + "** im Zeitraum " + t.start + " bis " + t.end + " (" + t.months + " Monate, "
        + de(t.area, 1) + " m², " + t.persons + (if t.persons == 1 { " Person" } else { " Personen" })
        + "). Ihre Anteile im Einzelnen:");

    // The shares as a table: what, the bill, by which key, and the share — the money on the
    // right. Four short columns, because the columns are equal and a wrapped cell doubles a row.
    let rows = [["**Kostenart**", "**Rechnung**", "**Verteilung**", "**Ihr Anteil**"]];
    for l in b.lines {
        rows.push([l.name, eur(l.total), KEYS[l.key] + ", " + de(l.part * 100.0, 2) + " %", eur(l.share / 100.0)]);
    }
    rows.push(["Heizung, Verbrauch", eur(b.metered), de(t.kwh, 0) + " von " + de(totals.kwh, 0) + " kWh", eur(b.metered_share / 100.0)]);
    rows.push(["Heizung, Grundkosten", eur(b.base), "Wohnfläche, " + de(b.base_part * 100.0, 2) + " %", eur(b.base_share / 100.0)]);
    rows.push(["**Summe**", "", "", "**" + eur(b.sum / 100.0) + "**"]);
    d.table(rows, ["Tabelle", "Tabelle_20_Betrag", "Tabelle", "Tabelle_20_Betrag"]);

    d.para("Brieftext", "Dem stehen Ihre Vorauszahlungen von " + t.months + " × " + eur(t.prepayment)
        + " = **" + eur(b.paid / 100.0) + "** gegenüber.");

    if b.balance > 0 {
        d.para("Ergebnis", "**Nachzahlung: " + eur(b.balance / 100.0) + ".** Bitte überweisen Sie den Betrag bis zum "
            + source.due + " auf das Konto " + landlord.iban + ".");
    } else if b.balance < 0 {
        d.para("Ergebnis", "**Guthaben: " + eur(-b.balance / 100.0) + ".** "
            + (if t.active {
                "Wir überweisen den Betrag in den nächsten Tagen auf Ihr Konto."
            } else {
                "Da Ihr Mietverhältnis am " + t.end + " endete, überweisen wir den Betrag in den nächsten Tagen auf das uns bekannte Konto."
            }));
    } else {
        d.para("Ergebnis", "**Die Abrechnung ist ausgeglichen.**");
    }
    // `#w3a` finds this letter whatever order the letters end up in, and the result line is
    // what a test wants to read.
    d.bookmark(t.id);

    if t.active {
        // Next year's prepayment follows this year's cost, per month, rounded up to a whole
        // euro — the way a landlord would put it, and what the tenant will find easy to set up.
        let monthly = ((b.sum.to_float() / 100.0) / t.months.to_float()).ceiling().to_int();
        d.para("Brieftext", "Wir empfehlen, die monatliche Vorauszahlung ab dem nächsten Monat auf **"
            + eur(monthly) + "** anzupassen (bisher " + eur(t.prepayment) + ").");
    }

    d.para("Gruss", "Mit freundlichen Grüßen");
    d.para("Unterschrift", landlord.signed);
    letters[t.id] = d;
}

// --- the page for the property manager ----------------------------------------------------

{
    let d = text_from("brief-din5008.fodt");
    d.para("Ruecksendeangabe", "Interne Aufstellung · nicht zum Versand");
    field(d, ["Hausverwaltung Mustermann GmbH", "Abteilung Abrechnung", landlord.street, landlord.city], [
        ["Zeichen:", "NK-" + source.year],
        ["Bearbeiterin:", landlord.signed],
        ["", ""],
        ["", ""],
        ["Datum:", source.issued],
    ]);
    d.para("Betreff", "Übersicht der Betriebskostenabrechnungen " + source.year);
    d.para("Brieftext", "Eine Abrechnung je Mietverhältnis; Nachzahlungen und Guthaben:");
    let owed = 0;
    for b in bills {
        owed += b.balance;
        let t = b.t;
        d.item(1, t.unit + ", " + t.name + ": "
            + (if b.balance >= 0 { "Nachzahlung " + eur(b.balance / 100.0) } else { "Guthaben " + eur(-b.balance / 100.0) })).style("Aufzaehlung");
    }
    d.para("Brieftext", "Umgelegt wurden **" + eur(allocated / 100.0) + "** von **" + eur(billed / 100.0)
        + "** Gesamtkosten (Rundungsdifferenz " + eur(drift / 100.0) + "). Saldo aller Abrechnungen: **"
        + (if owed >= 0 { "Nachzahlung " + eur(owed / 100.0) } else { "Guthaben " + eur(-owed / 100.0) })
        + "**.");
    d.bookmark("uebersicht");
    letters["uebersicht"] = d;
}

letters;

// --- checks --------------------------------------------------------------------------------
//
// `grind test` builds the letters exactly as above and asks about what came out. The text
// model has no cells, so a test reads a block — the bookmarked line of each letter — and what
// it holds is the sentence a tenant will read.

fn test_every_tenancy_has_a_letter_with_the_right_result(d) {
    // Each of these is a figure worked out by hand from `nebenkosten.json`, so a change to the
    // keys or to the rounding shows up here before it shows up in a letter somebody posts.
    assert(d.doc("w1").block("#w1").starts_with("Nachzahlung: 73,77 €"), d.doc("w1").block("#w1"));
    assert(d.doc("w2").block("#w2").starts_with("Guthaben: 145,16 €"), d.doc("w2").block("#w2"));
    assert(d.doc("w3a").block("#w3a").starts_with("Nachzahlung: 88,74 €"), d.doc("w3a").block("#w3a"));
    assert(d.doc("w3b").block("#w3b").starts_with("Guthaben: 81,31 €"), d.doc("w3b").block("#w3b"));
    assert(d.doc("w4").block("#w4").starts_with("Nachzahlung: 71,95 €"), d.doc("w4").block("#w4"));
    assert(d.doc("w5").block("#w5").starts_with("Guthaben: 136,47 €"), d.doc("w5").block("#w5"));
}

fn test_the_summary_reconciles_to_the_bills(d) {
    // 1.240,00 + 1.496,00 + 2.310,50 + 862,40 + 1.080,00 + 372,60 + 7.420,00, and the letters
    // add up to two cents more, which is the rounding of six shares each of eight costs.
    assert(d.doc("uebersicht").block("#uebersicht").contains("von 14.781,50 € Gesamtkosten (Rundungsdifferenz 0,02 €)"), d.doc("uebersicht").block("#uebersicht"));
    assert(d.doc("uebersicht").block("#uebersicht").contains("Guthaben 128,48 €"), d.doc("uebersicht").block("#uebersicht"));
}

fn test_the_document_contradicts_itself_nowhere(d) {
    assert(d.lint().is_empty(), "lint found something");
}
