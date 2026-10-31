// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Grammar on the page (`doc/grammar.md`): Edit ▸ Spelling's *Check Grammar With Spelling*,
//! *Grammar Style Suggestions* and *Next Grammar Finding* (⌥⌘;), the rows leading the context
//! menu over a finding — its message, its replacements and *Ignore Grammar Rule* — and
//! LanguageTool asked through `curl` on a thread, its answer taken on the caret's blink.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use objc2::MainThreadMarker;
use objc2_app_kit::{NSMenu, NSMenuItem};

use grind_languagetool::setting::Setting;
use grind_text::Caret;
use grind_text::grammar::{Finding, Flag};

use crate::menu::{Command, Grammar, REPLACEMENTS};
use crate::page_view::TextPane;
use crate::prompt;
use crate::spelling::item;

/// What a LanguageTool round comes back with: each paragraph's key and findings, or why not.
pub(crate) type Answer = Result<Vec<(u64, Vec<Finding>)>, String>;

/// How long typing pauses before LanguageTool is asked.
const PAUSE: Duration = Duration::from_millis(800);

impl TextPane {
    /// Attach the built-in checker as the remembered setting says, and name the LanguageTool
    /// server to ask — when the pane is made and whenever the setting changes.
    pub(crate) fn check_grammar(&self) {
        let setting = self.grammar.borrow().clone();
        let checker: Option<Arc<dyn grind_core::grammar::Checker>> = match setting.builtin {
            true => Some(Arc::new(grind_grammar::Builtin {
                picky: setting.style,
            })),
            false => None,
        };
        self.app.set_checker(checker);
        let remote = setting
            .languagetool
            .as_ref()
            .and_then(|choice| choice.server().ok())
            .map(|server| {
                let options = grind_languagetool::Options {
                    picky: setting.style,
                    disabled_rules: setting
                        .ignored
                        .iter()
                        .filter_map(|rule| rule.strip_prefix("lt:"))
                        .map(str::to_owned)
                        .collect(),
                    max_chars: server.max_chars(),
                };
                (server, options)
            });
        self.app.set_background_grammar(remote.is_some());
        *self.languagetool.borrow_mut() = remote;
        *self.asking.borrow_mut() = None;
        self.edited_at.set(Instant::now() - PAUSE);
    }

    /// Change the remembered setting, and apply it.
    fn set_grammar(&self, change: impl FnOnce(&mut Setting), mtm: MainThreadMarker) {
        change(&mut self.grammar.borrow_mut());
        if let Err(why) = self.grammar.borrow().save() {
            prompt::tell(mtm, "The setting was not remembered.", &why.to_string());
        }
        self.check_grammar();
        self.act_on(|_, _, _| Ok(()));
    }

    /// *LanguageTool Server…* — asked for, remembered, and where the text goes said.
    fn choose_server(&self, mtm: MainThreadMarker) {
        let current = self.grammar.borrow().languagetool.clone();
        let Some(typed) = prompt::ask(
            mtm,
            "LanguageTool Server",
            "An address such as http://localhost:8010, public, premium (with \
             GRIND_LANGUAGETOOL_USER and GRIND_LANGUAGETOOL_KEY set), or off.",
            "Use",
            current.as_ref().map_or("", |choice| choice.tag()),
        ) else {
            return;
        };
        let choice = match grind_languagetool::setting::Choice::answer(&typed) {
            Ok(choice) => choice,
            Err(why) => return prompt::tell(mtm, "Not a LanguageTool server.", &why),
        };
        self.set_grammar(|g| g.languagetool = choice.clone(), mtm);
        match choice.map(|choice| choice.server()) {
            Some(Ok(server)) => prompt::tell(
                mtm,
                "Grammar checked by LanguageTool.",
                &format!("{} — {}.", server.base(), server.describe()),
            ),
            Some(Err(why)) => prompt::tell(mtm, "LanguageTool is not asked.", &why),
            None => {}
        }
    }

    /// Every grammar finding in `blocks`, but the ignored rules'.
    fn grammar_flags(&self, blocks: std::ops::Range<usize>) -> Vec<Flag> {
        let ignored = self.grammar.borrow().ignored.clone();
        self.app
            .grammar(blocks)
            .into_iter()
            .filter(|flag| !ignored.contains(&flag.finding.rule))
            .collect()
    }

    /// The grammar finding at `at`, if one covers it.
    pub(crate) fn grammar_at(&self, at: Caret) -> Option<Flag> {
        self.grammar_flags(at.block..at.block + 1)
            .into_iter()
            .find(|f| f.finding.range.start <= at.offset && at.offset <= f.finding.range.end)
    }

    /// What a finding covers, as text.
    fn covered(&self, flag: &Flag) -> String {
        let range = flag.finding.range.clone();
        self.app
            .get_viewport(flag.block..flag.block + 1)
            .get(flag.block)
            .map(|view| {
                view.text
                    .chars()
                    .skip(range.start)
                    .take(range.len())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// One of Edit ▸ Spelling's grammar verbs, or a context-menu row's.
    pub fn grammar(&self, verb: Grammar, mtm: MainThreadMarker) {
        match verb {
            Grammar::Toggle => self.set_grammar(|g| g.builtin = !g.builtin, mtm),
            Grammar::Style => self.set_grammar(|g| g.style = !g.style, mtm),
            Grammar::Next => self.next_grammar(mtm),
            Grammar::Server => self.choose_server(mtm),
            Grammar::Ignore => {
                let Some(flag) = self.grammar_offer.borrow_mut().take() else {
                    return;
                };
                self.set_grammar(|g| g.ignored.push(flag.finding.rule.clone()), mtm);
            }
            Grammar::Correct(index) => {
                let Some(flag) = self.grammar_offer.borrow_mut().take() else {
                    return;
                };
                let Some(with) = flag.finding.replacements.get(usize::from(index)).cloned() else {
                    return;
                };
                let covered = self.covered(&flag);
                let app = self.app.clone();
                self.act_on(move |page, _, _| {
                    app.correct(flag.caret(), &covered, &with)
                        .map_err(|e| e.to_string())?;
                    page.place(
                        Caret {
                            block: flag.block,
                            offset: flag.finding.range.start + with.chars().count(),
                        },
                        false,
                    );
                    Ok(())
                });
            }
        }
    }

    /// *Next Grammar Finding* — the next finding after the caret selected, wrapping; right-click
    /// it for what it might be instead.
    fn next_grammar(&self, mtm: MainThreadMarker) {
        let all = self.grammar_flags(0..usize::MAX);
        let carets: Vec<Caret> = all.iter().map(Flag::caret).collect();
        let at = self.state.borrow().caret;
        let Some(index) = grind_text::find::step(&carets, at, grind_text::find::Towards::Next)
        else {
            prompt::tell(mtm, "No grammar findings.", "Nothing to check here.");
            return;
        };
        let flag = all[index].clone();
        self.act_on(|page, _, _| {
            page.place(flag.caret(), false);
            page.place(
                Caret {
                    block: flag.block,
                    offset: flag.finding.range.end,
                },
                true,
            );
            Ok(())
        });
    }

    /// On the caret's blink: take LanguageTool's answer if it has come, or ask it once typing
    /// has paused — one request at a time, on a thread of its own.
    pub(crate) fn tick_grammar(&self) {
        let slot = self.asking.borrow().clone();
        if let Some(slot) = slot {
            let answer = slot.lock().ok().and_then(|mut s| s.take());
            let Some(answer) = answer else {
                return;
            };
            *self.asking.borrow_mut() = None;
            if let Ok(found) = answer {
                for (key, findings) in found {
                    self.app.grammar_found(key, findings);
                }
                self.act_on(|_, _, _| Ok(()));
            }
            return;
        }
        let Some((server, options)) = self.languagetool.borrow().clone() else {
            return;
        };
        if self.edited_at.get().elapsed() < PAUSE {
            return;
        }
        let pending = self.app.grammar_pending(0..usize::MAX);
        if pending.is_empty() {
            return;
        }
        let slot: Arc<Mutex<Option<Answer>>> = Arc::new(Mutex::new(None));
        *self.asking.borrow_mut() = Some(slot.clone());
        std::thread::spawn(move || {
            let paragraphs: Vec<_> = pending.iter().map(|p| p.paragraph()).collect();
            let mut answered: Vec<Vec<Finding>> = vec![Vec::new(); pending.len()];
            let mut failure = None;
            for batch in grind_languagetool::batches(&paragraphs, &options) {
                match crate::curl::post(&server, &batch.form).and_then(|a| batch.findings(&a)) {
                    Ok(found) => {
                        for (at, finding) in found {
                            answered[at].push(finding);
                        }
                    }
                    Err(why) => failure = Some(why),
                }
            }
            let answer = match failure {
                None => Ok(pending.iter().map(|p| p.key).zip(answered).collect()),
                Some(why) => Err(why),
            };
            if let Ok(mut slot) = slot.lock() {
                *slot = Some(answer);
            }
        });
    }
}

/// The rows that lead the page's context menu over the grammar finding at `caret`: what is
/// wrong (a row with no action), each replacement, *Ignore Grammar Rule*, and a separator —
/// nothing at all elsewhere. The finding is kept on the page, which is what a
/// `Grammar::Correct(n)` row later means.
pub fn lead_menu(pane: &TextPane, menu: &NSMenu, caret: Caret, mtm: MainThreadMarker) {
    let Some(flag) = pane.grammar_at(caret) else {
        *pane.grammar_offer.borrow_mut() = None;
        return;
    };
    let covered = pane.covered(&flag);
    let mut rows: Vec<objc2::rc::Retained<NSMenuItem>> =
        vec![item(&flag.finding.message, None, mtm)];
    for (index, with) in flag
        .finding
        .replacements
        .iter()
        .take(usize::from(REPLACEMENTS))
        .enumerate()
    {
        let title = match with.is_empty() {
            true => format!("Remove \u{201c}{covered}\u{201d}"),
            false => with.clone(),
        };
        rows.push(item(
            &title,
            Some(Command::Grammar(Grammar::Correct(index as u8))),
            mtm,
        ));
    }
    rows.push(item(
        "Ignore Grammar Rule",
        Some(Command::Grammar(Grammar::Ignore)),
        mtm,
    ));
    rows.push(NSMenuItem::separatorItem(mtm));
    for (at, row) in rows.iter().enumerate() {
        menu.insertItem_atIndex(row, at as isize);
    }
    *pane.grammar_offer.borrow_mut() = Some(flag);
}
