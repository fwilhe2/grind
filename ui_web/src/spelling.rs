// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Spelling in the browser (`doc/spelling.md`): the same checker, the same rules and the same
//! seven languages as every other client, with two differences the page forces.
//!
//! **The dictionaries are fetched, not carried.** They are twelve megabytes, and the native
//! binaries compile them in; a page that did the same would charge everybody who never types a
//! word. `grind-spell` is linked here *without* its bundled lists, and [`fetch`] asks the server
//! for the one language that is wanted — `dictionaries/<tag>.aff` and `.dic`, which
//! `ui_web/build.sh` puts beside the page — and hands the text to
//! [`grind_spell::Language::provide`], the way `print_module` and `fonts` fetch the PDF stack.
//! Nothing is fetched until a document is opened with spelling on, or somebody asks for a
//! language.
//!
//! **The setting is remembered in the browser**, not in a file: `localStorage`, one entry per
//! kind of document — the page's counterpart of `$XDG_CONFIG_HOME/grind/spelling`, and like it
//! the person's, never the document's. So are the words somebody added to their dictionary.

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
        if let Some(setting) = setting_of(id) {
            let Some(kind) = self.mode.get().kind() else {
                return;
            };
            if let Err(why) = save(kind, setting) {
                self.set_message(format!("The setting was not remembered: {why}"));
            }
            return spawn_local(self.clone().apply_spelling(kind, true));
        }
        if self.mode.get() != Mode::Text {
            return;
        }
        match id {
            "spell.next" => self.text.next_misspelling(),
            "spell.ignore" | "spell.add" => {
                let Some(wrong) = self.text.misspelt_here() else {
                    return self.set_message("No misspelt word here".to_owned());
                };
                if let Some(speller) = self.text_speller.borrow().as_ref() {
                    speller.accept(&wrong.word);
                }
                let said = match id == "spell.add" {
                    false => format!("{} is ignored for this visit", wrong.word),
                    true => match add_word(&wrong.word) {
                        Ok(()) => format!("{} added to your dictionary", wrong.word),
                        Err(why) => format!("{} is ignored, but not kept: {why}", wrong.word),
                    },
                };
                self.set_message(said);
                request_repaint(&self.pending);
            }
            _ => match id
                .strip_prefix("spell.fix:")
                .and_then(|n| n.parse::<usize>().ok())
            {
                Some(n) => self.text.fix_misspelt(n),
                None => self.set_message(format!("No such command: {id}")),
            },
        }
    }

    /// Attach what the remembered setting for `kind` means to the document in its pane —
    /// fetching the dictionary first if the language has not arrived, which is the one
    /// asynchronous step — and repaint. `announce` says what it is checking in on the status
    /// line; opening a document says nothing unless something is wrong.
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
                        self.set_message(format!(
                            "Checking spelling in {} ({})",
                            choice.language.name(),
                            choice.source.label()
                        ));
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
        request_repaint(&self.pending);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A command id carries a setting in the vocabulary a menu's state and `:spell` already
    /// speak, so the palette's rows are `Setting::ALL` and nothing else.
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
