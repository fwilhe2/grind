// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The German rules (`doc/grammar.md` §4).

mod common;
use common::{found, nothing};

const DE: Option<&str> = Some("de-DE");

#[test]
fn ihr_seid_and_seit_gestern() {
    assert_eq!(
        found("Ich glaube, ihr seit zu spät.", DE),
        ["seid-seit:seit"]
    );
    assert_eq!(
        found("Ich bin seid gestern hier und nicht dort.", DE),
        ["seid-seit:seid"]
    );
    nothing("Ihr seid zu spät, und das ist nicht gut.", DE);
    nothing("Er ist ihr seit Jahren treuer Freund und Helfer.", DE);
    nothing("Seid ihr da? Das ist die Frage.", DE);
}

#[test]
fn a_comma_before_dass_weil_and_obwohl() {
    assert_eq!(
        found("Ich weiß dass es regnet und die Straße nass ist.", DE),
        ["comma-before-conjunction: "]
    );
    assert_eq!(
        found("Er blieb zu Hause weil er krank war, sagt sie.", DE),
        ["comma-before-conjunction: "]
    );
    nothing("Ich weiß, dass es regnet und die Straße nass ist.", DE);
    nothing("Es regnet, so dass die Straße nass ist.", DE);
    nothing(
        "Er ging, ohne dass es jemand merkte, und kam nicht wieder.",
        DE,
    );
    nothing("Er sagte, dass er kommt und dass er bleibt.", DE);
    nothing("Nur weil es regnet, ist die Straße nicht immer nass.", DE);
    nothing("Er kam, gerade weil er krank war, und blieb auch.", DE);
    nothing("Dass es regnet, ist nicht schlimm für die Felder.", DE);
}

#[test]
fn german_rules_do_not_run_on_english() {
    nothing(
        "I know dass is a German word and the seid one too.",
        Some("en-GB"),
    );
}

/// What German Wikipedia drew that was an abbreviation, not a sentence's end.
#[test]
fn an_abbreviation_or_a_regnal_number_does_not_end_a_sentence() {
    nothing("Das Buch wurde hrsg. von der Stadt und ist gut.", DE);
    nothing("Die Mittheilungen der kais. Gesellschaft sind alt.", DE);
    nothing("Ein Name (arab. farīd) ist das und nicht mehr.", DE);
    nothing("In der Charta Jakob VI. für den Ort ist es so.", DE);
    nothing("Davon gingen 5384 Rs. an die Briten und nicht mehr.", DE);
    nothing("Die Angaben sind bei binnenschifferforum.de zu finden.", DE);
    nothing("Das Werk ist zus. mit ihm und Übers. von ihr.", DE);
}

#[test]
fn a_lowercase_start_after_a_sentence_ending_in_a_short_word_is_still_found() {
    assert_eq!(
        found("Er war nicht da und das ist. dann kam er.", DE),
        ["sentence-start-lowercase:dann"]
    );
}
