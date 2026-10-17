// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `:dict` — the personal dictionary, in both halves: the words every spelling check accepts,
//! in every document and every language (`grind_spell::personal`, `doc/spelling.md` decision 5).
//! `:dict` lists them in a [`crate::pick`] pane where `Enter` removes one, and `:dict add WORD`
//! and `:dict rm WORD` do the rest from the command line — `grind words` from a terminal's other
//! half. What a word is and where the list lives are `grind_spell`'s; this file is the wording.

use grind_spell::{Speller, personal};

use crate::pick::{Pick, Row};

/// Open `pane` on the list.
pub fn open(pane: &mut Pick, here: Option<&str>) {
    let rows = personal::words()
        .into_iter()
        .map(|word| Row {
            address: word,
            label: String::new(),
            depth: 0,
        })
        .collect();
    pane.open("personal dictionary", rows, here);
    pane.says(
        "No words yet \u{2014} :dict add WORD adds one, :spell add the misspelt one at the cursor.",
        "Enter removes it",
    );
}

/// `:dict <rest>`: `add WORD` or `rm WORD`, with the running speller told at once so what is
/// underlined changes without reopening anything. The sentence for the status line.
pub fn command(rest: &str, speller: Option<&Speller>) -> String {
    let (verb, word) = rest.split_once(' ').unwrap_or((rest, ""));
    match verb {
        "add" => add(word, speller),
        "rm" | "remove" => remove(word, speller),
        _ => "usage: :dict, :dict add WORD, :dict rm WORD".to_string(),
    }
}

pub fn add(typed: &str, speller: Option<&Speller>) -> String {
    let word = match personal::word(typed) {
        Ok(word) => word,
        Err(why) => return why.to_string(),
    };
    match personal::add(&word) {
        Ok(_) => {
            if let Some(speller) = speller {
                speller.accept(&word);
            }
            format!("{word} added to your dictionary")
        }
        Err(e) => format!("{word} was not added: {e}"),
    }
}

pub fn remove(word: &str, speller: Option<&Speller>) -> String {
    let word = word.trim();
    match personal::remove(word) {
        Ok(true) => {
            if let Some(speller) = speller {
                speller.forget(word);
            }
            format!("{word} removed \u{2014} :dict add {word} puts it back")
        }
        Ok(false) => format!("{word} is not in your dictionary"),
        Err(e) => format!("{word} was not removed: {e}"),
    }
}
