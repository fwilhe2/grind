// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later
//
// Invitations to an owners' meeting (Eigentümerversammlung) — an agenda that is data, and
// money items that are each owner's share by their ownership fraction.
//
//   grind test examples/letters/einladung.rhai
//   grind build examples/letters/einladung.rhai -o letters --as pdf
//
// Every owner gets the same agenda, numbered by the script rather than typed (so a topic added
// to `einladung.json` renumbers everything after it), with their own *Miteigentumsanteile* — and,
// under each topic that costs money, what it means for them. The script stops if the fractions
// do not add up to a thousand: an invitation that quietly gives a building 1,003 votes is worse
// than none. Everything in `einladung.json` is made up.


let source = json("einladung.json");

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

let total = 0;
for o in source.owners { total += o.mea; }
if total != 1000 {
    throw "the ownership fractions add up to " + total + "/1000, not 1000/1000";
}

let letters = #{};

for o in source.owners {
    let d = text_from("brief-din5008.fodt");

    d.para("Ruecksendeangabe", landlord.name + " · " + landlord.street + " · " + landlord.city);
    field(d, [o.name, o.street, o.city], [
        ["Zeichen:", "ETV-" + o.id.to_upper()],
        ["Bearbeiterin:", landlord.signed],
        ["Telefon:", landlord.phone],
        ["", ""],
        ["Datum:", source.issued],
    ]);

    d.para("Betreff", "Einladung zur Eigentümerversammlung am " + source.meeting.date);
    d.para("Brieftext", o.greeting + ",");
    d.para("Brieftext", "die " + source.community + " lädt Sie zur Eigentümerversammlung ein: **" + source.meeting.date + ", "
        + source.meeting.time + "**, " + source.meeting.place + ".");
    d.para("Brieftext", "Tagesordnung:");

    // The agenda as a table: its number, its topic, and — where it costs money — this owner's
    // share by their ownership fraction. Numbered by the script, so a topic added to the data
    // renumbers everything after it.
    let rows = [["**TOP**", "**Thema**", "**Ihr Anteil " + o.mea + "/1000**"]];
    let n = 0;
    for top in source.agenda {
        n += 1;
        let share = "";
        if "amount" in top {
            let cents_share = cents(top.amount.to_float() * o.mea.to_float() / 1000.0);
            share = eur(cents_share / 100.0) + (if top.per == "Jahr" { " im Jahr" } else { "" });
        }
        rows.push([n.to_string(), top.title, share]);
    }
    d.table(rows, ["Tabelle", "Tabelle", "Tabelle_20_Betrag"]);
    // The one-off cost's share is what a test reads: the last row of the agenda that has one.
    for top in source.agenda {
        if "amount" in top && top.per == "einmalig" {
            d.para("Brieftext", top.note + " " + eur(top.amount) + ": Ihr Anteil **" + eur(cents(top.amount.to_float() * o.mea.to_float() / 1000.0) / 100.0) + "**.");
            d.bookmark(o.id + "cost");
        }
    }

    d.para("Ergebnis", "**Ihre Miteigentumsanteile: " + o.mea + "/1000** (" + o.unit + ").");
    d.bookmark(o.id);

    d.para("Brieftext", "Wenn Sie nicht teilnehmen können, bevollmächtigen Sie bitte schriftlich eine Person Ihres Vertrauens. Rückfragen: " + landlord.phone + ".");
    d.para("Gruss", "Mit freundlichen Grüßen");
    d.para("Unterschrift", landlord.signed);
    letters[o.id] = d;
}

letters;

// --- checks --------------------------------------------------------------------------------

fn test_the_shares_of_a_cost_follow_the_fractions(d) {
    // 48.000,00 € × 270/1000 = 12.960,00 €, for the owner with two flats; × 118/1000 = 5.664,00 €.
    assert(d.doc("o3").block("#o3cost").contains("**12.960,00 €**") || d.doc("o3").block("#o3cost").contains("12.960,00 €"), d.doc("o3").block("#o3cost"));
    assert(d.doc("o1").block("#o1cost").contains("5.664,00 €"), d.doc("o1").block("#o1cost"));
}

fn test_every_owner_has_a_letter(d) {
    for id in ["o1", "o2", "o3", "o4", "o5", "o6"] {
        assert(d.doc(id).block("#" + id).starts_with("Ihre Miteigentumsanteile: "), id);
    }
}

fn test_the_document_contradicts_itself_nowhere(d) {
    assert(d.lint().is_empty());
}
