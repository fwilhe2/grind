// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: MIT
//
// Typesetting on show: a justified, hyphenated German paragraph on a narrow measure, and the
// pairs kerning closes up, at display size.
//
//   grind build examples/typesetting/satz.rhai -o satz.pdf --as pdf
//
// The styles are the template's (`satz.fodt`): a script names a style, a person declares it.

let d = text_from("satz.fodt");

d.heading(1, "Satz");
d.para("Blocksatz", "Die Donaudampfschifffahrtsgesellschaft veröffentlichte ihre Jahresrechnung "
    + "zusammen mit einer ausführlichen Begründung der Fahrpreiserhöhung, die der Aufsichtsrat "
    + "nach langwierigen Verhandlungen mit den Gewerkschaftsvertretern beschlossen hatte. "
    + "Unverhältnismäßig hohe Instandhaltungskosten der Landungsbrücken, gestiegene "
    + "Treibstoffpreise und die Personalaufwendungen der Sommersaison machten die "
    + "Kostenrechnung unumgänglich.");
d.para("Schaugroesse", "AVATAR To Wa fi");

d;

// --- checks --------------------------------------------------------------------------------

fn test_both_styles_are_the_templates(d) {
    assert(d.lint().is_empty());
}
