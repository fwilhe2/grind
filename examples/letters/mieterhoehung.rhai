// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later
//
// Rent-increase letters: a calculation with a rule in it — the *Kappungsgrenze*.
//
//   grind test examples/letters/mieterhoehung.rhai
//   grind build examples/letters/mieterhoehung.rhai -o letters --as pdf
//
// A landlord may raise the rent up to the local comparative rent (the Mietspiegel's figure per
// square metre times the flat's area), but not by more than a cap over the rent paid three years
// ago. Each tenancy lands in one of three outcomes — an increase, an increase held back by the
// cap, or none because the rent is already at the comparative rent — and the letter says which,
// with the figures that decided it, so the tenant can follow the arithmetic. That decision is the
// reason to compute rather than fill in: a template cannot choose its own sentence.
//
// The legal references are illustrative and the Mietspiegel is made up; this is an example of
// the mechanism, not advice. Everything in `mieterhoehung.json` is invented.


let source = json("mieterhoehung.json");

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


let landlord = source.landlord;

let bills = [];
for t in source.tenants {
    let now = cents(t.rent);
    let reference = cents(t.reference_per_m2.to_float() * t.area.to_float());
    let cap = cents(t.rent_3y.to_float() * (1.0 + t.cap.to_float() / 100.0));
    // The new rent is the lower of what the Mietspiegel allows and what the cap allows, and
    // never less than the rent already paid.
    let target = if reference < cap { reference } else { cap };
    let outcome = if reference <= now { "none" } else if cap < reference { "capped" } else { "full" };
    let new_rent = if target > now { target } else { now };
    bills.push(#{ t: t, now: now, reference: reference, cap: cap, new_rent: new_rent, outcome: outcome });
}

let letters = #{};

for b in bills {
    let d = text_from("brief-din5008.fodt");
    let t = b.t;

    d.para("Ruecksendeangabe", landlord.name + " · " + landlord.street + " · " + landlord.city);
    field(d, [t.name, source.building.street, source.building.city], [
        ["Zeichen:", "MH-" + t.id.to_upper()],
        ["Bearbeiterin:", landlord.signed],
        ["Telefon:", landlord.phone],
        ["", ""],
        ["Datum:", source.issued],
    ]);

    d.para("Betreff", (if b.outcome == "none" { "Ihre Miete – " } else { "Mieterhöhung – " }) + t.unit);
    d.para("Brieftext", t.greeting + ",");

    if b.outcome == "none" {
        d.para("Brieftext", "wir haben die Miete für **" + t.unit + "** mit dem " + source.mietspiegel.name
            + " verglichen. Ihre Miete von " + eur(b.now / 100.0) + " liegt auf der Höhe der ortsüblichen Vergleichsmiete von "
            + eur(b.reference / 100.0) + " oder darüber. **Wir erhöhen Ihre Miete deshalb nicht.**");
    } else {
        d.para("Brieftext", "wir bitten Sie, einer Erhöhung der Miete für **" + t.unit + "** ab dem " + source.effective
            + " zuzustimmen. Grundlage ist der " + source.mietspiegel.name + " (" + source.mietspiegel.note + "):");
        d.table([
            ["**Berechnung**", "**Betrag**"],
            ["Vergleichsmiete: " + de(t.area, 1) + " m² × " + eur(t.reference_per_m2), eur(b.reference / 100.0)],
            ["Obergrenze: Miete vor drei Jahren " + eur(t.rent_3y) + " + " + t.cap + " %", eur(b.cap / 100.0)],
            ["Ihre heutige Miete", eur(b.now / 100.0)],
        ], ["Tabelle", "Tabelle_20_Betrag"]);
        if b.outcome == "capped" {
            d.para("Brieftext", "Die Vergleichsmiete liegt über der Obergrenze. Wir erhöhen deshalb nur bis zur Obergrenze (" + source.cap_note + ").");
        }
        d.para("Brieftext", "Bitte stimmen Sie bis zum Ende des übernächsten Monats schriftlich zu. Antworten Sie uns gern unter " + landlord.email + ".");
        d.para("Ergebnis", "**Neue Miete: " + eur(b.new_rent / 100.0) + "** (+ " + eur((b.new_rent - b.now) / 100.0) + " im Monat, + "
            + de((b.new_rent - b.now).to_float() / b.now.to_float() * 100.0, 1) + " %).");
    }
    if b.outcome == "none" {
        d.para("Ergebnis", "**Miete unverändert: " + eur(b.now / 100.0) + ".**");
    }
    d.bookmark(t.id);

    d.para("Gruss", "Mit freundlichen Grüßen");
    d.para("Unterschrift", landlord.signed);
    letters[t.id] = d;
}

letters;

// --- checks --------------------------------------------------------------------------------

fn test_an_increase_up_to_the_comparative_rent(d) {
    // 58,0 m² × 9,40 € = 545,20 €; the cap is 470 × 1,2 = 564,00 €, so the Mietspiegel decides.
    assert(d.doc("e1").block("#e1").starts_with("Neue Miete: 545,20 € (+ 47,20 € im Monat, + 9,5 %)"), d.doc("e1").block("#e1"));
}

fn test_the_cap_holds_an_increase_back(d) {
    // 76,0 m² × 10,90 € = 828,40 €, but 640 × 1,2 = 768,00 €: the cap decides.
    assert(d.doc("e2").block("#e2").starts_with("Neue Miete: 768,00 €"), d.doc("e2").block("#e2"));
}

fn test_a_rent_above_the_comparative_rent_is_left_alone_at_all(d) {
    // 64,0 m² × 9,40 € = 601,60 €, below today's 640,00 €: no increase.
    assert(d.doc("e3").block("#e3").starts_with("Miete unverändert: 640,00 €"), d.doc("e3").block("#e3"));
}

fn test_a_rent_above_the_comparative_rent_is_left_alone(d) {
    // 82,5 m² × 8,80 € = 726,00 €, below today's 745,00 €.
    assert(d.doc("e4").block("#e4").starts_with("Miete unverändert: 745,00 €"), d.doc("e4").block("#e4"));
}

fn test_the_document_contradicts_itself_nowhere(d) {
    assert(d.lint().is_empty());
}
