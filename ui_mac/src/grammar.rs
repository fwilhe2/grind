// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Grammar on the page (`doc/grammar.md`): Edit ▸ Spelling's *Check Grammar With Spelling*,
//! *Grammar Style Suggestions* and *Next Grammar Finding* (⌥⌘;), the rows leading the context
//! menu over a finding — its message, its replacements and *Ignore Grammar Rule*.

use std::sync::Arc;

use objc2::MainThreadMarker;
use objc2_app_kit::{NSMenu, NSMenuItem};

use grind_grammar::setting::Setting;
use grind_text::Caret;
use grind_text::grammar::Flag;

use crate::menu::{Command, Grammar, REPLACEMENTS};
use crate::page_view::TextPane;
use crate::prompt;
use crate::spelling::item;

impl TextPane {
    /// Attach the built-in checker as the remembered setting says — when the pane is made and
    /// whenever the setting changes.
    pub(crate) fn check_grammar(&self) {
        let setting = self.grammar.borrow().clone();
        let checker: Option<Arc<dyn grind_core::grammar::Checker>> = if setting.builtin {
            Some(Arc::new(grind_grammar::Builtin {
                picky: setting.style,
            }))
        } else {
            None
        };
        self.app.set_checker(checker);
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
        let title = if with.is_empty() {
            format!("Remove \u{201c}{covered}\u{201d}")
        } else {
            with.clone()
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
