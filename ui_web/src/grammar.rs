// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Grammar in the browser (`doc/grammar.md`): the built-in checker of every other client, and
//! its setting remembered in `localStorage` as `grind grammar` remembers it in a file.

use grind_languagetool::setting::Setting;

use crate::command::Entry;

/// The `localStorage` key the setting is kept under, in the setting file's own text.
const KEY: &str = "grind.grammar";

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

/// The remembered setting — the built-in checker, when nothing was chosen or the browser will
/// not say.
pub fn load() -> Setting {
    storage()
        .and_then(|s| s.get_item(KEY).ok().flatten())
        .map_or_else(Setting::default, |text| Setting::parse(&text))
}

pub fn save(setting: &Setting) -> Result<(), String> {
    storage()
        .ok_or("the browser keeps nothing for this page")?
        .set_item(KEY, &setting.to_text())
        .map_err(|_| "the browser refused to keep it".to_owned())
}

/// The palette's rows for the grammar finding at the caret: each replacement as
/// `grammar.fix:N`, then Ignore the rule — offered for an empty query, the words that name them,
/// or what the finding covers.
pub fn offers(finding: &grind_text::grammar::Finding, covered: &str, query: &str) -> Vec<Entry> {
    let query = query.trim().to_lowercase();
    let asks = query.is_empty()
        || ["grammar", "replace", "correct", "ignore", "remove"]
            .iter()
            .any(|name| name.contains(query.as_str()))
        || covered.to_lowercase().contains(&query)
        || finding.message.to_lowercase().contains(&query);
    if !asks {
        return Vec::new();
    }
    let mut out: Vec<Entry> = finding
        .replacements
        .iter()
        .take(5)
        .enumerate()
        .map(|(n, with)| {
            let title = match with.trim().is_empty() && with.len() < 2 {
                true if with.is_empty() => format!("Remove \u{201c}{covered}\u{201d}"),
                _ => format!("Replace \u{201c}{covered}\u{201d} with \u{201c}{with}\u{201d}"),
            };
            Entry::target(
                format!("grammar.fix:{n}"),
                format!("{title} \u{2014} {}", finding.message),
                "Grammar",
            )
        })
        .collect();
    out.push(Entry::target(
        "grammar.ignore",
        format!("Ignore this rule ({})", finding.rule),
        "Grammar",
    ));
    out
}

impl crate::Shell {
    /// A `grammar.*` command: switching the checker or its style rules, going to the next
    /// finding, or acting on the one at the caret. Only the word processor has grammar.
    pub(crate) fn run_grammar(&self, id: &str) {
        let mut setting = load();
        let changed = match id {
            "grammar.on" => {
                setting.builtin = true;
                true
            }
            "grammar.off" => {
                setting.builtin = false;
                setting.languagetool = None;
                true
            }
            "grammar.style-on" | "grammar.style-off" => {
                setting.style = id == "grammar.style-on";
                true
            }
            "grammar.ignore" => match self.text.grammar_here() {
                Some(flag) => {
                    setting.ignored.push(flag.finding.rule.clone());
                    self.set_message(format!("{} is ignored", flag.finding.rule));
                    true
                }
                None => {
                    self.set_message("No grammar finding here".to_owned());
                    false
                }
            },
            "grammar.next" => {
                self.text.next_grammar();
                false
            }
            _ => {
                match id
                    .strip_prefix("grammar.fix:")
                    .and_then(|n| n.parse::<usize>().ok())
                {
                    Some(n) => self.text.fix_grammar(n),
                    None => self.set_message(format!("No such command: {id}")),
                }
                false
            }
        };
        if changed {
            if let Err(why) = save(&setting) {
                self.set_message(format!("The setting was not remembered: {why}"));
            }
            self.text.apply_grammar(&setting);
        }
        crate::request_repaint(&self.pending);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grind_text::grammar::{Finding, Kind};

    #[test]
    fn a_finding_offers_its_replacements_and_ignoring_its_rule() {
        let finding = Finding {
            range: 3..7,
            rule: "builtin:repeated-word".into(),
            kind: Kind::Grammar,
            message: "\u{201c}the\u{201d} twice".into(),
            replacements: vec![String::new()],
            url: None,
        };
        let rows: Vec<(String, String)> = offers(&finding, " the", "")
            .into_iter()
            .map(|e| (e.id, e.title))
            .collect();
        assert_eq!(
            rows,
            [
                (
                    "grammar.fix:0".to_owned(),
                    "Remove \u{201c} the\u{201d} \u{2014} \u{201c}the\u{201d} twice".to_owned()
                ),
                (
                    "grammar.ignore".to_owned(),
                    "Ignore this rule (builtin:repeated-word)".to_owned()
                ),
            ]
        );
        assert!(offers(&finding, " the", "zebra").is_empty(), "not asked");
        assert_eq!(offers(&finding, " the", "grammar").len(), 2);
    }
}
