// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Grammar in the browser (`doc/grammar.md`): the built-in checker of every other client, and
//! its setting remembered in `localStorage` as `grind grammar` remembers it in a file.

use std::rc::Rc;

use grind_languagetool::setting::{Choice, Setting};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::{JsFuture, spawn_local};

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
            "grammar.languagetool" => {
                let typed = web_sys::window()
                    .and_then(|w| {
                        w.prompt_with_message(
                            "A LanguageTool server's address — one started with --allow-origin, \
                             such as http://localhost:8010 — or public",
                        )
                        .ok()
                    })
                    .flatten();
                match typed.as_deref().map(str::trim).and_then(Choice::parse) {
                    Some(Choice::Premium) => {
                        self.set_message(
                            "Premium needs an API key, which this page does not keep — use \
                             grind grammar premium"
                                .to_owned(),
                        );
                        false
                    }
                    Some(choice) => {
                        setting.languagetool = Some(choice);
                        true
                    }
                    None => {
                        self.set_message(
                            "That is neither public nor an http(s) address".to_owned(),
                        );
                        false
                    }
                }
            }
            "grammar.languagetool-off" => {
                setting.languagetool = None;
                true
            }
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

/// How long typing pauses before LanguageTool is asked.
const PAUSE_MS: i32 = 800;

/// After a paint: ask LanguageTool, once nothing has been painted for [`PAUSE_MS`], for the
/// paragraphs the core says are unanswered (`doc/grammar.md` decision 10).
pub(crate) fn after_paint() {
    crate::with_rc_shell(|shell| {
        let text = shell.text.clone();
        if !text.asks_languagetool() {
            return;
        }
        let painted = text.painted();
        spawn_local(async move {
            sleep(PAUSE_MS).await;
            if text.painted() == painted {
                ask(text).await;
            }
        });
    });
}

async fn sleep(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        if let Some(window) = web_sys::window() {
            let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
        }
    });
    let _ = JsFuture::from(promise).await;
}

/// One round: the unanswered paragraphs out, LanguageTool's answer into the core, a repaint.
async fn ask(text: Rc<crate::text::Ui>) {
    let Some((server, options)) = text.languagetool() else {
        return;
    };
    let pending = text.app.grammar_pending(0..text.app.block_count());
    if pending.is_empty() || !text.start_asking() {
        return;
    }
    let paragraphs: Vec<_> = pending.iter().map(|p| p.paragraph()).collect();
    let mut failure = None;
    let mut answered: Vec<Vec<grind_text::grammar::Finding>> = vec![Vec::new(); pending.len()];
    for batch in grind_languagetool::batches(&paragraphs, &options) {
        match post(&server, &batch.form)
            .await
            .and_then(|answer| batch.findings(&answer))
        {
            Ok(found) => {
                for (at, finding) in found {
                    answered[at].push(finding);
                }
            }
            Err(why) => failure = Some(why),
        }
    }
    match failure {
        None => {
            for (paragraph, findings) in pending.iter().zip(answered) {
                text.app.grammar_found(paragraph.key, findings);
            }
        }
        Some(why) => text.set_message(format!("LanguageTool did not answer: {why}")),
    }
    text.done_asking();
    text.request_repaint();
}

/// `POST` a form to `/v2/check`, answering the body.
async fn post(
    server: &grind_languagetool::Server,
    form: &[(String, String)],
) -> Result<String, String> {
    let window = web_sys::window().ok_or("no window")?;
    let body: Vec<String> = form
        .iter()
        .map(|(k, v)| {
            format!(
                "{}={}",
                js_sys::encode_uri_component(k),
                js_sys::encode_uri_component(v)
            )
        })
        .collect();
    let init = web_sys::RequestInit::new();
    init.set_method("POST");
    init.set_body(&wasm_bindgen::JsValue::from_str(&body.join("&")));
    let headers = js_sys::Object::new();
    let _ = js_sys::Reflect::set(
        &headers,
        &"Content-Type".into(),
        &"application/x-www-form-urlencoded".into(),
    );
    init.set_headers(&headers);
    let url = format!("{}/v2/check", server.base());
    let response: web_sys::Response = JsFuture::from(window.fetch_with_str_and_init(&url, &init))
        .await
        .map_err(|_| format!("{} could not be reached", server.host()))?
        .unchecked_into();
    if !response.ok() {
        return Err(format!("{} answered {}", server.host(), response.status()));
    }
    let text = JsFuture::from(response.text().map_err(|_| "no answer".to_owned())?)
        .await
        .map_err(|_| "no answer".to_owned())?;
    text.as_string().ok_or_else(|| "no answer".to_owned())
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
