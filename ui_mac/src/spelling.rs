// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Spelling on the page (`doc/spelling.md`): Edit ▸ Spelling's *Check Document Now* (⌘;) and
//! *Spelling Language…*, and the rows that lead the context menu over a misspelt word — what it
//! might have been, *Ignore Spelling* and *Learn Spelling*, the names every Mac text view gives
//! them.
//!
//! The decisions are the core's and `grind_spell`'s — which words are wrong
//! (`App::misspellings`), what they might have been (`App::suggest`), which dictionary
//! (`grind_spell::Setting`), the person's own list (`grind_spell::personal`) — and the Windows
//! pane's popup is the same rows in the same order. What is here is AppKit: an `NSMenu` led by
//! items made at the moment the menu opens, since a word's suggestions are not a table.

use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSMenu, NSMenuItem};
use objc2_foundation::NSString;

use grind_text::{Caret, Misspelling};

use crate::menu::{COMMAND_SELECTOR, Command, SUGGESTIONS, Spell};
use crate::page_view::TextPane;
use crate::prompt;

impl TextPane {
    /// Attach what [`TextPane::spelling`] says to the document — on the first spelling verb and
    /// whenever the choice changes, so *Ignore Spelling* has a dictionary of this page's own to
    /// add to.
    pub fn check_spelling(&self) {
        *self.speller.borrow_mut() = match self.spelling.get().apply(&self.app) {
            Ok(Some((_, speller))) => Some(speller),
            Ok(None) | Err(_) => None,
        };
    }

    /// The misspelt word `at` is in, from its first character to just past its last.
    pub fn misspelt_at(&self, at: Caret) -> Option<Misspelling> {
        self.app
            .misspellings(at.block..at.block + 1)
            .into_iter()
            .find(|m| m.offset <= at.offset && at.offset <= m.offset + m.len)
    }

    /// One of Edit ▸ Spelling's verbs, or a context-menu row's.
    pub fn spell(&self, spell: Spell, mtm: MainThreadMarker) {
        if self.speller.borrow().is_none() && self.spelling.get() != grind_spell::Setting::Off {
            self.check_spelling();
        }
        match spell {
            Spell::Next => self.next_misspelling(mtm),
            Spell::Language => self.choose_language(mtm),
            Spell::Ignore | Spell::Learn => {
                let Some((wrong, _)) = self.offers.borrow_mut().take() else {
                    return;
                };
                if let Some(speller) = self.speller.borrow().as_ref() {
                    speller.accept(&wrong.word);
                }
                if spell == Spell::Learn
                    && let Err(error) = grind_spell::personal::add(&wrong.word)
                {
                    prompt::tell(mtm, "The word could not be learned.", &error.to_string());
                }
                self.act_on(|_, _, _| Ok(()));
            }
            Spell::Correct(index) => {
                let Some((wrong, offers)) = self.offers.borrow_mut().take() else {
                    return;
                };
                let Some(with) = offers.get(usize::from(index)).cloned() else {
                    return;
                };
                self.act_on(|page, app, _| {
                    app.correct(wrong.caret(), &wrong.word, &with)
                        .map_err(|e| e.to_string())?;
                    page.place(
                        Caret {
                            block: wrong.block,
                            offset: wrong.offset + with.chars().count(),
                        },
                        false,
                    );
                    Ok(())
                });
            }
        }
    }

    /// *Check Document Now* — the next misspelt word after the caret selected, wrapping as Find
    /// Next does (`grind_text::find::step`); right-click it for what it might have been.
    fn next_misspelling(&self, mtm: MainThreadMarker) {
        if self.speller.borrow().is_none() {
            prompt::tell(
                mtm,
                "Spelling is off.",
                "Edit ▸ Spelling ▸ Spelling Language… turns it on.",
            );
            return;
        }
        let all = self.app.misspellings(0..usize::MAX);
        let carets: Vec<Caret> = all.iter().map(Misspelling::caret).collect();
        let at = self.state.borrow().caret;
        let Some(index) = grind_text::find::step(&carets, at, grind_text::find::Towards::Next)
        else {
            prompt::tell(
                mtm,
                "No misspelt words.",
                "Every word is in the dictionary.",
            );
            return;
        };
        let wrong = all[index].clone();
        self.act_on(|page, _, _| {
            page.place(wrong.caret(), false);
            page.place(
                Caret {
                    block: wrong.block,
                    offset: wrong.offset + wrong.len,
                },
                true,
            );
            Ok(())
        });
    }

    /// *Spelling Language…* — Automatic, a dictionary by name, or Off. The session's choice: the
    /// document's own language is read and never written (`doc/spelling.md`).
    fn choose_language(&self, mtm: MainThreadMarker) {
        let settings = grind_spell::Setting::ALL;
        let items: Vec<String> = settings.iter().map(|s| s.label().to_owned()).collect();
        let current = settings
            .iter()
            .position(|s| *s == self.spelling.get())
            .unwrap_or(0);
        let Some(picked) = prompt::pick(
            mtm,
            "Spelling Language",
            "Which dictionary checks this document, for as long as it is open.",
            "Choose",
            &items,
            current,
        ) else {
            return;
        };
        self.spelling.set(settings[picked]);
        self.check_spelling();
        if let Err(why) = self.spelling.get().apply(&self.app) {
            prompt::tell(mtm, "This document is not checked.", &why.to_string());
        }
        self.act_on(|_, _, _| Ok(()));
    }
}

/// The rows that lead the page's context menu over the misspelt word at `caret`: up to
/// [`SUGGESTIONS`] of what it might have been (or *No Guesses Found*), *Ignore Spelling*, *Learn
/// Spelling*, and a separator — nothing at all over a word spelled right. The word and its
/// suggestions are kept on the page, which is what a `Spell::Correct(n)` row later means.
pub fn lead_menu(pane: &TextPane, menu: &NSMenu, caret: Caret, mtm: MainThreadMarker) {
    let Some(wrong) = pane.misspelt_at(caret) else {
        *pane.offers.borrow_mut() = None;
        return;
    };
    let offers: Vec<String> = pane
        .app
        .suggest(&wrong.word)
        .into_iter()
        .take(usize::from(SUGGESTIONS))
        .collect();
    let mut rows: Vec<Retained<NSMenuItem>> = Vec::new();
    for (index, word) in offers.iter().enumerate() {
        rows.push(item(
            word,
            Some(Command::Spell(Spell::Correct(index as u8))),
            mtm,
        ));
    }
    if offers.is_empty() {
        rows.push(item("No Guesses Found", None, mtm));
    }
    rows.push(NSMenuItem::separatorItem(mtm));
    rows.push(item(
        "Ignore Spelling",
        Some(Command::Spell(Spell::Ignore)),
        mtm,
    ));
    rows.push(item(
        "Learn Spelling",
        Some(Command::Spell(Spell::Learn)),
        mtm,
    ));
    rows.push(NSMenuItem::separatorItem(mtm));
    for (at, row) in rows.iter().enumerate() {
        menu.insertItem_atIndex(row, at as isize);
    }
    *pane.offers.borrow_mut() = Some((wrong, offers));
}

/// One row: a command's item, or a disabled one with no action.
fn item(title: &str, command: Option<Command>, mtm: MainThreadMarker) -> Retained<NSMenuItem> {
    let action = command.map(|_| crate::app::selector(COMMAND_SELECTOR));
    // SAFETY: the selector names an action method, which takes one sender argument.
    let made = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str(title),
            action,
            &NSString::from_str(""),
        )
    };
    match command {
        Some(command) => made.setTag(command.tag()),
        None => made.setEnabled(false),
    }
    made
}
