// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Spelling in the browser (`doc/spelling.md`): the checker, rules and languages of every other
//! client, with two differences.
//!
//! **The dictionaries are fetched, not carried**: twelve megabytes nobody who never types a word
//! should download. `grind-spell` is linked without its bundled lists, and [`fetch`] asks for the
//! one language wanted — `dictionaries/<tag>.aff` and `.dic`, which `ui_web/build.sh` puts beside
//! the page — for [`grind_spell::Language::provide`]. Nothing is fetched until a document opens
//! with spelling on, or a language is asked for.
//!
//! **The setting is remembered in the browser**: `localStorage`, one entry per kind of document —
//! the person's, never the document's — as are the words they added.

use std::rc::Rc;

use grind_core::DocumentKind;
use grind_spell::{Language, Setting, Unchecked};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::{JsFuture, spawn_local};

use crate::{Mode, Shell, request_repaint};

/// Where the page's dictionaries are, beside it.
const DICTIONARIES: &str = "dictionaries/";

/// The `localStorage` key holding the setting for one kind of document.
fn key(kind: DocumentKind) -> &'static str {
    match kind {
        DocumentKind::Spreadsheet => "grind.spelling.spreadsheet",
        DocumentKind::Text => "grind.spelling.text",
        DocumentKind::Presentation => "grind.spelling.presentation",
    }
}

/// The personal word list's key: one list for every document and every language.
const WORDS: &str = "grind.spelling.words";

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

/// The remembered setting for `kind` — automatic when nothing was ever chosen, or when the
/// browser will not say (a private window, blocked site data).
pub fn load(kind: DocumentKind) -> Setting {
    storage()
        .and_then(|s| s.get_item(key(kind)).ok().flatten())
        .and_then(|tag| Setting::from_tag(&tag))
        .unwrap_or_default()
}

/// Remember `setting` for `kind`. `Err` carries the browser's refusal, which the caller says on
/// the status line: a choice that was not kept is one the next page will not know about.
pub fn save(kind: DocumentKind, setting: Setting) -> Result<(), String> {
    let storage = storage().ok_or("the browser keeps nothing for this page")?;
    match setting {
        Setting::Automatic => storage.remove_item(key(kind)),
        other => storage.set_item(key(kind), other.tag()),
    }
    .map_err(|_| "the browser refused to keep it".to_owned())
}

/// The words somebody added to their own dictionary.
pub fn words() -> Vec<String> {
    storage()
        .and_then(|s| s.get_item(WORDS).ok().flatten())
        .map(|text| {
            text.lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Add a word to the personal dictionary, for good.
pub fn add_word(word: &str) -> Result<(), String> {
    let storage = storage().ok_or("the browser keeps nothing for this page")?;
    let mut all = words();
    if !all.iter().any(|w| w == word) {
        all.push(word.to_owned());
    }
    storage
        .set_item(WORDS, &all.join("\n"))
        .map_err(|_| "the browser refused to keep it".to_owned())
}

/// Take a word off the personal dictionary. `Ok(false)` when it was not on it.
pub fn remove_word(word: &str) -> Result<bool, String> {
    let storage = storage().ok_or("the browser keeps nothing for this page")?;
    let mut all = words();
    let before = all.len();
    all.retain(|w| w != word);
    if all.len() == before {
        return Ok(false);
    }
    storage
        .set_item(WORDS, &all.join("\n"))
        .map(|()| true)
        .map_err(|_| "the browser refused to keep it".to_owned())
}

/// The palette's rows for a misspelt word under the caret or in the active cell: its first five
/// `suggestions` as `spell.fix:N`, then Ignore and Add. Offered for an empty query, for the words
/// that name them, and for what somebody typing the correction they mean would be typing:
/// `receive`, over the word `recieve`.
pub fn offers(word: &str, suggestions: &[String], query: &str) -> Vec<crate::command::Entry> {
    use crate::command::Entry;
    let query = query.trim().to_lowercase();
    let asks = query.is_empty()
        || ["spelling", "replace", "correct", "ignore", "dictionary"]
            .iter()
            .any(|name| name.contains(query.as_str()))
        || word.to_lowercase().contains(&query)
        || suggestions
            .iter()
            .any(|with| with.to_lowercase().contains(&query));
    if !asks {
        return Vec::new();
    }
    let mut out: Vec<Entry> = suggestions
        .iter()
        .take(5)
        .enumerate()
        .map(|(n, with)| {
            Entry::target(
                format!("spell.fix:{n}"),
                format!("Replace \u{201c}{word}\u{201d} with \u{201c}{with}\u{201d}"),
                "Spelling",
            )
        })
        .collect();
    out.push(Entry::target(
        "spell.ignore",
        format!("Ignore \u{201c}{word}\u{201d}"),
        "Spelling",
    ));
    out.push(Entry::target(
        "spell.add",
        format!("Add \u{201c}{word}\u{201d} to your dictionary"),
        "Spelling",
    ));
    out
}

/// The palette's row prefix for a word on the personal dictionary — picking it removes it.
pub const FORGET: &str = "Remove from your dictionary: ";

/// The palette's rows for the personal dictionary, as `(command id, title)`: one per word, once
/// the query is a beginning of [`FORGET`] at least `Remove` long, or [`FORGET`] and then part of
/// a word — which is what `spell.words` opens the palette on, and what typing narrows.
pub fn forget(query: &str, words: &[String]) -> Vec<(String, String)> {
    let query = query.trim_start().to_lowercase();
    let prefix = FORGET.to_lowercase();
    let narrow = match query.strip_prefix(&prefix) {
        Some(rest) => rest.trim().to_owned(),
        None if query.trim().len() >= "remove".len() && prefix.starts_with(query.trim()) => {
            String::new()
        }
        None => return Vec::new(),
    };
    words
        .iter()
        .filter(|word| word.to_lowercase().contains(&narrow))
        .map(|word| (format!("spell.forget:{word}"), format!("{FORGET}{word}")))
        .collect()
}

/// The language a command id names, `spell.de-DE` — or `spell.auto`, `spell.off`.
pub fn setting_of(id: &str) -> Option<Setting> {
    id.strip_prefix("spell.").and_then(Setting::from_tag)
}

/// Fetch one language's Hunspell files and hand them to the checker. A no-op when the language
/// is here already. The parse is 40 to 300 milliseconds, once per visit.
pub async fn fetch(language: Language) -> Result<(), String> {
    if language.available() {
        return Ok(());
    }
    let window = web_sys::window().ok_or("no window")?;
    let get = |extension: &'static str| {
        let url = format!("{DICTIONARIES}{}.{extension}", language.tag());
        let window = window.clone();
        async move {
            let response: web_sys::Response = JsFuture::from(window.fetch_with_str(&url))
                .await
                .map_err(|_| format!("could not fetch {url}"))?
                .unchecked_into();
            if !response.ok() {
                return Err(format!("could not fetch {url}: {}", response.status()));
            }
            let text = JsFuture::from(response.text().map_err(|_| format!("{url} has no text"))?)
                .await
                .map_err(|_| format!("could not read {url}"))?;
            text.as_string().ok_or_else(|| format!("{url} is not text"))
        }
    };
    let (aff, dic) = (get("aff").await?, get("dic").await?);
    language.provide(&aff, &dic)
}

/// Apply the remembered setting to the pane for `kind`, from wherever a document just arrived.
pub(crate) fn spawn(kind: Option<DocumentKind>, announce: bool) {
    let Some(kind) = kind else { return };
    crate::with_rc_shell(move |shell| spawn_local(shell.apply_spelling(kind, announce)));
}

impl Shell {
    /// A `spell.*` command: choosing a language, or acting on the word at the caret.
    pub(crate) fn run_spelling(self: &Rc<Self>, id: &str) {
        if id == "spell.choose" {
            return self.open_palette_with("Spelling: ");
        }
        // The personal dictionary: its words as palette rows, each one's pick removing it.
        if id == "spell.words" {
            return self.open_palette_with(FORGET);
        }
        if id == "spell.add-word" {
            let typed = web_sys::window()
                .and_then(|w| {
                    w.prompt_with_message("A word spelling should accept in every document")
                        .ok()
                })
                .flatten();
            let Some(typed) = typed else { return };
            let said = match grind_spell::personal::word(&typed) {
                Err(why) => why.to_string(),
                Ok(word) => match add_word(&word) {
                    Err(why) => format!("{word} was not added: {why}"),
                    Ok(()) => {
                        for speller in [&self.sheet_speller, &self.text_speller] {
                            if let Some(speller) = speller.borrow().as_ref() {
                                speller.accept(&word);
                            }
                        }
                        format!("{word} added to your dictionary")
                    }
                },
            };
            self.set_message(said);
            return request_repaint(&self.pending);
        }
        if let Some(word) = id.strip_prefix("spell.forget:") {
            let said = match remove_word(word) {
                Ok(true) => {
                    for speller in [&self.sheet_speller, &self.text_speller] {
                        if let Some(speller) = speller.borrow().as_ref() {
                            speller.forget(word);
                        }
                    }
                    format!("{word} removed from your dictionary")
                }
                Ok(false) => format!("{word} is not in your dictionary"),
                Err(why) => format!("{word} was not removed: {why}"),
            };
            self.set_message(said);
            return request_repaint(&self.pending);
        }
        if let Some(setting) = setting_of(id) {
            let Some(kind) = self.mode.get().kind() else {
                return;
            };
            if let Err(why) = save(kind, setting) {
                self.set_message(format!("The setting was not remembered: {why}"));
            }
            return spawn_local(self.clone().apply_spelling(kind, true));
        }
        let sheet = match self.mode.get() {
            Mode::Sheet => true,
            Mode::Text => false,
            Mode::Welcome => return,
        };
        match id {
            "spell.next" if sheet => self.sheet.next_misspelling(),
            "spell.next" => self.text.next_misspelling(),
            "spell.ignore" | "spell.add" => {
                let word = match sheet {
                    true => self.sheet.misspelt_here().map(|wrong| wrong.word),
                    false => self.text.misspelt_here().map(|wrong| wrong.word),
                };
                let Some(word) = word else {
                    return self.set_message("No misspelt word here".to_owned());
                };
                let speller = match sheet {
                    true => &self.sheet_speller,
                    false => &self.text_speller,
                };
                if let Some(speller) = speller.borrow().as_ref() {
                    speller.accept(&word);
                }
                let said = match id == "spell.add" {
                    false => format!("{word} is ignored for this visit"),
                    true => match add_word(&word) {
                        Ok(()) => format!("{word} added to your dictionary"),
                        Err(why) => format!("{word} is ignored, but not kept: {why}"),
                    },
                };
                self.set_message(said);
                request_repaint(&self.pending);
            }
            _ => match id
                .strip_prefix("spell.fix:")
                .and_then(|n| n.parse::<usize>().ok())
            {
                Some(n) if sheet => self.sheet.fix_misspelt(n),
                Some(n) => self.text.fix_misspelt(n),
                None => self.set_message(format!("No such command: {id}")),
            },
        }
    }

    /// Attach what the remembered setting for `kind` means to the document in its pane — fetching
    /// the dictionary first if needed, the one asynchronous step — and repaint. `announce` says on
    /// the status line what it checks in; opening a document says nothing unless something is
    /// wrong.
    pub(crate) async fn apply_spelling(self: Rc<Self>, kind: DocumentKind, announce: bool) {
        let setting = load(kind);
        for _ in 0..2 {
            let outcome = match kind {
                DocumentKind::Spreadsheet => setting.apply(&*self.sheet.app),
                _ => setting.apply(&*self.text.app),
            };
            match outcome {
                Err(Unchecked::NotLoaded(language)) => {
                    self.set_message(format!("Fetching the {} dictionary…", language.name()));
                    if let Err(why) = fetch(language).await {
                        return self.set_message(format!("Spelling is not checked: {why}"));
                    }
                    continue;
                }
                Ok(Some((choice, speller))) => {
                    for word in words() {
                        speller.accept(&word);
                    }
                    match kind {
                        DocumentKind::Spreadsheet => {
                            *self.sheet_speller.borrow_mut() = Some(speller)
                        }
                        _ => *self.text_speller.borrow_mut() = Some(speller),
                    }
                    if announce {
                        self.set_message(format!("Checking spelling in {}", choice.describe()));
                    } else {
                        self.set_message(String::new());
                    }
                }
                Ok(None) => {
                    match kind {
                        DocumentKind::Spreadsheet => *self.sheet_speller.borrow_mut() = None,
                        _ => *self.text_speller.borrow_mut() = None,
                    }
                    if announce {
                        self.set_message("Spelling is off".to_owned());
                    }
                }
                Err(why) => {
                    if announce {
                        self.set_message(format!("Spelling is not checked: {why}"));
                    }
                }
            }
            break;
        }
        self.show_spelling_chip();
        request_repaint(&self.pending);
    }

    /// Put the remembered setting for the document showing on the status bar's flag.
    pub(crate) fn show_spelling_chip(&self) {
        let Some(kind) = self.mode.get().kind() else {
            return;
        };
        let setting = load(kind);
        let picker = &self.dom.spelling_picker;
        picker.set_text_content(Some(setting.chip(draws_flags())));
        picker.set_title(&format!("Spelling: {} — click to choose", setting.label()));
    }
}

/// Whether this browser draws a flag emoji as a flag. Windows ships no flag glyphs — every
/// browser on it draws the two letters (`DE`) instead — so there the chip says `DE` on purpose
/// rather than by accident; everywhere else the platform's own emoji font draws the flag.
fn draws_flags() -> bool {
    web_sys::window()
        .and_then(|window| window.navigator().user_agent().ok())
        .is_none_or(|agent| !agent.contains("Windows"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A command id carries a setting in the vocabulary a menu's state and `:spell` already
    /// speak, so the palette's rows are `Setting::ALL` and nothing else.
    #[test]
    fn the_dictionarys_words_are_rows_once_the_query_reaches_for_them() {
        let words = ["Grind".to_owned(), "grnid".to_owned(), "ODF".to_owned()];
        assert_eq!(forget(FORGET, &words).len(), 3);
        assert_eq!(forget("remove", &words).len(), 3);
        assert_eq!(
            forget(&format!("{FORGET}gr"), &words),
            [
                ("spell.forget:Grind".to_owned(), format!("{FORGET}Grind")),
                ("spell.forget:grnid".to_owned(), format!("{FORGET}grnid")),
            ]
        );
        assert!(forget("rem", &words).is_empty());
        assert!(forget("bold", &words).is_empty());
    }

    #[test]
    fn every_setting_has_a_command_id_and_back() {
        for setting in Setting::ALL {
            let id = format!("spell.{}", setting.tag());
            assert_eq!(setting_of(&id), Some(setting), "{id}");
        }
        assert_eq!(setting_of("spell.next"), None);
        assert_eq!(setting_of("edit.copy"), None);
    }
}
