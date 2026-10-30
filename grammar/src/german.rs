// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The rules for German.

use grind_core::grammar::{Finding, Kind};

use crate::tokens::{Token, next_word, pairs, previous_word, replace};

/// Words that say when, which `seit` comes before and `seid` never does.
const TIME: &[&str] = &[
    "gestern",
    "heute",
    "langem",
    "kurzem",
    "jahren",
    "monaten",
    "wochen",
    "tagen",
    "stunden",
    "wann",
    "dem",
    "der",
    "damals",
    "jeher",
    "ewigkeiten",
    "neuestem",
    "anfang",
    "beginn",
];

/// `ihr seit` for `ihr seid`, and `seid gestern` for `seit gestern`. `ihr seit Jahren` is
/// *her* friend *for* years, so not before a word of time.
pub fn seid_seit(tokens: &[Token], out: &mut Vec<Finding>) {
    for (_, seit) in pairs(tokens, &["ihr"], &["seit"]) {
        let after = next_word(tokens, seit).map(|i| tokens[i].lower());
        if after.as_deref().is_some_and(|w| TIME.contains(&w)) {
            continue;
        }
        replace(
            "seid-seit",
            &tokens[seit],
            "seid",
            "\u{201e}ihr seid\u{201c}: von sein",
            out,
        );
    }
    for (seid, _) in pairs(tokens, &["seid"], TIME) {
        replace(
            "seid-seit",
            &tokens[seid],
            "seit",
            "\u{201e}seit\u{201c}: zeitlich",
            out,
        );
    }
}

/// A comma before `dass`, `weil` and `obwohl` where it is missing. Not after a word that joins
/// the conjunction (`so dass`, `ohne dass`) or takes the comma in front of itself (`und dass`,
/// `nur weil`, `gerade weil`), nor at a sentence's start.
pub fn comma_before_conjunction(tokens: &[Token], out: &mut Vec<Finding>) {
    const BEFORE: &[&str] = &[
        "so",
        "als",
        "ohne",
        "statt",
        "anstatt",
        "außer",
        "bis",
        "und",
        "oder",
        "sowie",
        "bzw",
        "nur",
        "auch",
        "gerade",
        "eben",
        "nicht",
        "allem",
        "insbesondere",
        "besonders",
        "aber",
        "sondern",
        "selbst",
        "zumal",
        "vielleicht",
        "wohl",
        "doch",
    ];
    for (at, token) in tokens.iter().enumerate() {
        if !matches!(token.lower().as_str(), "dass" | "weil" | "obwohl") {
            continue;
        }
        let Some(before) = previous_word(tokens, at) else {
            continue;
        };
        if BEFORE.contains(&tokens[before].lower().as_str()) {
            continue;
        }
        let space = &tokens[at - 1];
        out.push(crate::finding(
            "comma-before-conjunction",
            Kind::Punctuation,
            (space, space),
            format!("Ein Komma vor \u{201e}{}\u{201c}", token.text),
            vec![", ".to_owned()],
        ));
    }
}
