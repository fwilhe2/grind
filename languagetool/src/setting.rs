// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The remembered grammar setting (`doc/grammar.md` decision 5), in
//! `$XDG_CONFIG_HOME/grind/grammar`: a line each — `builtin on|off`, `languagetool
//! <url>|public|premium|off`, `style on|off` — and an `ignore <rule>` line per rule ignored. A
//! line this build cannot read is ignored rather than read as something else. Premium's key is
//! never here: it comes from the environment.

use std::path::PathBuf;

use crate::Server;

/// Which LanguageTool server, as remembered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Choice {
    SelfHosted(String),
    Public,
    Premium,
}

impl Choice {
    /// `public`, `premium`, or a URL; `None` for anything else.
    pub fn parse(word: &str) -> Option<Choice> {
        match word {
            "public" => Some(Choice::Public),
            "premium" => Some(Choice::Premium),
            url if url.starts_with("http://") || url.starts_with("https://") => {
                Some(Choice::SelfHosted(url.to_owned()))
            }
            _ => None,
        }
    }

    /// What a window's *LanguageTool Server…* is answered with: empty or `off` is no server, and a
    /// host typed without its scheme (`localhost:8010`) is taken as `http://`.
    pub fn answer(typed: &str) -> Result<Option<Choice>, String> {
        let typed = typed.trim();
        if typed.is_empty() || typed == "off" {
            return Ok(None);
        }
        let bare_host = typed.contains(':') && !typed.contains("://") && !typed.contains(' ');
        match bare_host {
            true => Choice::parse(&format!("http://{typed}")),
            false => Choice::parse(typed),
        }
        .map(Some)
        .ok_or_else(|| {
            format!("{typed}? An address such as http://localhost:8010, public or premium")
        })
    }

    pub fn tag(&self) -> &str {
        match self {
            Choice::SelfHosted(url) => url,
            Choice::Public => "public",
            Choice::Premium => "premium",
        }
    }

    /// The server, Premium's account read through `env` (`GRIND_LANGUAGETOOL_USER` and
    /// `GRIND_LANGUAGETOOL_KEY`).
    pub fn server_from(&self, env: impl Fn(&str) -> Option<String>) -> Result<Server, String> {
        Ok(match self {
            Choice::SelfHosted(url) => Server::SelfHosted(url.clone()),
            Choice::Public => Server::Public,
            Choice::Premium => {
                let names = ["GRIND_LANGUAGETOOL_USER", "GRIND_LANGUAGETOOL_KEY"];
                let [username, key] = names.map(|name| env(name).filter(|v| !v.is_empty()));
                match (username, key) {
                    (Some(username), Some(key)) => Server::Premium { username, key },
                    _ => {
                        return Err(format!(
                            "LanguageTool Premium needs {} and {} set",
                            names[0], names[1]
                        ));
                    }
                }
            }
        })
    }

    /// [`Choice::server_from`] the process's environment.
    pub fn server(&self) -> Result<Server, String> {
        self.server_from(|name| std::env::var(name).ok())
    }
}

/// Which checkers run, against what, and what is left alone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Setting {
    pub builtin: bool,
    pub languagetool: Option<Choice>,
    /// The style rules too.
    pub style: bool,
    /// Rule ids the person ignores, prefixed as findings carry them (`lt:…`, `builtin:…`).
    pub ignored: Vec<String>,
}

impl Default for Setting {
    fn default() -> Self {
        Setting {
            builtin: true,
            languagetool: None,
            style: false,
            ignored: Vec::new(),
        }
    }
}

fn on(word: &str) -> Option<bool> {
    match word {
        "on" => Some(true),
        "off" => Some(false),
        _ => None,
    }
}

impl Setting {
    pub fn parse(text: &str) -> Setting {
        let mut setting = Setting::default();
        for line in text.lines().map(str::trim) {
            let Some((name, value)) = line.split_once(char::is_whitespace) else {
                continue;
            };
            let value = value.trim();
            match name {
                "builtin" => setting.builtin = on(value).unwrap_or(setting.builtin),
                "style" => setting.style = on(value).unwrap_or(setting.style),
                "languagetool" if value == "off" => setting.languagetool = None,
                "languagetool" => {
                    if let Some(choice) = Choice::parse(value) {
                        setting.languagetool = Some(choice);
                    }
                }
                "ignore" => setting.ignored.push(value.to_owned()),
                _ => {}
            }
        }
        setting
    }

    pub fn to_text(&self) -> String {
        let flag = |b: bool| if b { "on" } else { "off" };
        let mut out = format!("builtin {}\n", flag(self.builtin));
        out += &format!(
            "languagetool {}\n",
            self.languagetool.as_ref().map_or("off", Choice::tag)
        );
        out += &format!("style {}\n", flag(self.style));
        for rule in &self.ignored {
            out += &format!("ignore {rule}\n");
        }
        out
    }

    /// Where it lives, or `None` with no home directory to put it in.
    pub fn path() -> Option<PathBuf> {
        Some(grind_spell::config_dir()?.join("grammar"))
    }

    /// The remembered setting — the built-in checker alone when nothing was ever chosen.
    pub fn load() -> Setting {
        Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map_or_else(Setting::default, |text| Setting::parse(&text))
    }

    pub fn save(&self) -> std::io::Result<PathBuf> {
        let path = Self::path().ok_or_else(|| std::io::Error::other("no home directory"))?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, self.to_text())?;
        Ok(path)
    }
}
