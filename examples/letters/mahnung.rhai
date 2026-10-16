// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later
//
// Dunning letters, in three tones — the same sender, template and mechanism as
// `nebenkosten.rhai`, a different calculation.
//
//   grind test examples/letters/mahnung.rhai
//   grind build examples/letters/mahnung.rhai -o letters --as pdf
//
// Level 1 is a friendly reminder, level 2 a second reminder that charges default interest and a
// fee, level 3 a last warning that says what the landlord may do next. What differs between
// them is *data* (`level`, the fee and the deadline per level) and one table of sentences here,
// so a fourth level is a line in each rather than a fourth letter template. The interest is
// worked out per open item from the days it has been late, in cents, rounded once per item.
//
// Everything in `mahnung.json` is made up, the base rate included.


let source = json("mahnung.json");

// --- German figures ------------------------------------------------------------------------
//
// A text document holds *text*, so a number has to be spelled out by the script, and the
// spelling is the reader's: 1.234,56 € rather than 1,234.56. One function, used for money,
// areas and consumption alike. It works in whole units of the last decimal place — an integer
// — so that two figures that are equal print equally.

/// `1234.5` with 2 decimals is `1.234,50`; a minus sign is the real one, U+2212.
fn de(x, decimals) {
    let scale = 1;
    for i in 0..decimals {
        scale *= 10;
    }
    let units = (x.to_float() * scale.to_float()).round().to_int();
    let negative = units < 0;
    if negative {
        units = -units;
    }
    let whole = (units / scale).to_string();
    let grouped = "";
    while whole.len > 3 {
        grouped = "." + whole.sub_string(whole.len - 3) + grouped;
        whole = whole.sub_string(0, whole.len - 3);
    }
    grouped = whole + grouped;
    if decimals > 0 {
        let fraction = (units % scale).to_string();
        while fraction.len < decimals {
            fraction = "0" + fraction;
        }
        grouped += "," + fraction;
    }
    if negative { "−" + grouped } else { grouped }
}

fn eur(x) { de(x, 2) + " €" }

/// A figure in cents, so that the shares of one bill are rounded once, each, and then *added* —
/// the way the tenants' letters will be checked by a person with a calculator.
fn cents(x) { (x.to_float() * 100.0).round().to_int() }

/// Up to the next whole euro; there is no `ceil` in the dialect.
fn ceil_euro(x) {
    let whole = x.to_int();
    if x > whole.to_float() { whole + 1 } else { whole }
}

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


const TONE = #{
    "1": #{ subject: "Zahlungserinnerung", opening: "wahrscheinlich ist die Zahlung der folgenden Miete bei Ihnen untergegangen. Wir bitten Sie, den offenen Betrag zu überweisen:" },
    "2": #{ subject: "2. Mahnung", opening: "trotz unserer Erinnerung haben wir die folgenden Mietzahlungen nicht erhalten:" },
    "3": #{ subject: "Letzte Mahnung", opening: "trotz zweier Aufforderungen sind die folgenden Mietzahlungen weiterhin offen:" },
};

let landlord = source.landlord;

let bills = [];
for t in source.tenants {
    let open = 0;
    let interest = 0;
    let lines = [];
    for i in t.items {
        let o = cents(i.due.to_float() - i.paid.to_float());
        // Default interest is charged from the second reminder on, on what is still open.
        let z = if t.level >= 2 { cents((o.to_float() / 100.0) * source.interest_percent / 100.0 * i.days.to_float() / 365.0) } else { 0 };
        open += o;
        interest += z;
        lines.push(#{ month: i.month, due: i.due, paid: i.paid, open: o, interest: z, days: i.days });
    }
    let fee = cents(source.fees[t.level.to_string()]);
    bills.push(#{ t: t, lines: lines, open: open, interest: interest, fee: fee, total: open + interest + fee });
}

let letters = #{};

for b in bills {
    let d = text_from("brief-din5008.fodt");
    let t = b.t;
    let tone = TONE[t.level.to_string()];
    let deadline = source.deadlines[t.level.to_string()];

    d.para("Ruecksendeangabe", landlord.name + " · " + landlord.street + " · " + landlord.city);
    field(d, [t.name, source.building.street, source.building.city], [
        ["Zeichen:", "MA-" + t.id.to_upper()],
        ["Bearbeiterin:", landlord.signed],
        ["Telefon:", landlord.phone],
        ["", ""],
        ["Datum:", source.issued],
    ]);

    d.para("Betreff", tone.subject + " – Miete " + t.unit);
    d.para("Brieftext", t.greeting + ",");
    d.para("Brieftext", tone.opening);

    // The open rents as a table — with a column of interest from the second reminder on, and the
    // fee as its own row. Money on the right.
    let interest = t.level >= 2;
    let head = ["**Miete**", "**Fällig**", "**Bezahlt**", "**Offen**"];
    if interest { head.push("**Verzugszinsen**"); }
    let rows = [head];
    for l in b.lines {
        let row = [l.month, eur(l.due), eur(l.paid), eur(l.open / 100.0)];
        if interest { row.push(eur(l.interest / 100.0) + " (" + l.days + " Tage)"); }
        rows.push(row);
    }
    if b.fee > 0 {
        let row = ["Mahngebühr", "", "", ""];
        row.push(eur(b.fee / 100.0));
        rows.push(row);
    }
    let total = ["**Summe**", "", "", ""];
    if interest { total.push("**" + eur(b.total / 100.0) + "**"); } else { total[3] = "**" + eur(b.total / 100.0) + "**"; }
    rows.push(total);
    let money = ["Tabelle", "Tabelle_20_Betrag", "Tabelle_20_Betrag", "Tabelle_20_Betrag"];
    if interest { money.push("Tabelle_20_Betrag"); }
    d.table(rows, money);

    d.para("Ergebnis", "**Offen: " + eur(b.total / 100.0) + ".** Bitte überweisen Sie den Betrag bis zum " + deadline
        + " auf das Konto " + landlord.iban + ".");
    d.bookmark(t.id);

    if t.level == 2 {
        d.para("Brieftext", "Die Verzugszinsen berechnen wir mit " + de(source.interest_percent, 2) + " % im Jahr ("
            + source.interest_note + ").");
    }
    if t.level == 3 {
        d.para("Brieftext", "Bei einem Rückstand von mindestens zwei Monatsmieten sind wir berechtigt, das Mietverhältnis fristlos zu kündigen. "
            + "Bitte zahlen Sie deshalb rechtzeitig, oder rufen Sie uns an, damit wir eine Lösung finden: " + landlord.phone + ".");
    }
    if t.level == 1 {
        d.para("Brieftext", "Hat sich Ihre Zahlung mit diesem Schreiben überschnitten, betrachten Sie es bitte als gegenstandslos.");
    }

    d.para("Gruss", "Mit freundlichen Grüßen");
    d.para("Unterschrift", landlord.signed);
    letters[t.id] = d;
}

letters;

// --- checks --------------------------------------------------------------------------------

fn test_level_one_is_a_reminder_without_interest_or_fee(d) {
    assert_eq(d.doc("m1").block("#m1"), "Offen: 842,00 €. Bitte überweisen Sie den Betrag bis zum 23. Oktober 2026 auf das Konto DE00 0000 0000 0000 0000 00.");
}

fn test_level_two_adds_interest_and_a_fee(d) {
    // 290,00 € open for 46 days and 690,00 € for 16 days, both at 7,27 % a year:
    // 290 × 0,0727 × 46 / 365 = 2,66 and 690 × 0,0727 × 16 / 365 = 2,20, and a 5,00 € fee.
    assert(d.doc("m2").block("#m2").starts_with("Offen: 989,86 €"), d.doc("m2").block("#m2"));
}

fn test_level_three_carries_the_largest_claim(d) {
    // Three rents of 755,50 €, interest of 6,92 + 2,41 + 1,81, and a 10,00 € fee.
    assert(d.doc("m3").block("#m3").starts_with("Offen: 2.287,64 €"), d.doc("m3").block("#m3"));
}

fn test_the_document_contradicts_itself_nowhere(d) {
    assert(d.lint().is_empty());
}
