// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Which grammar checker a command uses (`doc/grammar.md`): the remembered setting, or what the
//! command line says this once.

use std::sync::Arc;

use grind_core::grammar::{Both, Checker};
use grind_grammar::Builtin;
#[cfg(feature = "languagetool")]
use grind_languagetool::{
    LanguageTool, Server,
    setting::{Choice, Setting},
};

/// The checker to attach, the rules to leave out of what it finds, and the LanguageTool client
/// among it, whose failure is worth reporting.
pub struct Chosen {
    pub checker: Option<Arc<dyn Checker>>,
    pub ignored: Vec<String>,
    #[cfg(feature = "languagetool")]
    pub languagetool: Option<Arc<LanguageTool>>,
}

/// What `grind text grammar` asks for: the remembered setting, unless `builtin_only` or
/// `languagetool` says otherwise. The public server is not sent a whole document unless
/// `public_ok`.
#[cfg(feature = "languagetool")]
pub fn chosen(
    picky: bool,
    builtin_only: bool,
    languagetool: Option<&str>,
    public_ok: bool,
) -> Result<Chosen, String> {
    let setting = Setting::load();
    let style = picky || setting.style;
    let (builtin, server) = match (builtin_only, languagetool) {
        (true, _) => (true, None),
        (false, Some(word)) => (
            false,
            Some(Choice::parse(word).ok_or_else(|| {
                format!("{word:?} is none of public, premium, or a server's http(s) URL")
            })?),
        ),
        (false, None) => (setting.builtin, setting.languagetool.clone()),
    };
    let mut checkers: Vec<Arc<dyn Checker>> = Vec::new();
    if builtin {
        checkers.push(Arc::new(Builtin { picky: style }));
    }
    let mut client = None;
    if let Some(choice) = server {
        let server = choice.server()?;
        if server == Server::Public && !public_ok {
            return Err(
                "LanguageTool's public server is not for checking whole documents from a script; \
                 pass --public-ok to send it this one anyway"
                    .to_owned(),
            );
        }
        let ignored = setting
            .ignored
            .iter()
            .filter_map(|rule| rule.strip_prefix("lt:"))
            .map(str::to_owned)
            .collect();
        let lt = Arc::new(LanguageTool::new(server, style).ignoring(ignored));
        eprintln!(
            "grind: grammar by LanguageTool — {}",
            lt.server().describe()
        );
        checkers.push(lt.clone());
        client = Some(lt);
    }
    Ok(Chosen {
        checker: one(checkers),
        ignored: setting.ignored,
        languagetool: client,
    })
}

/// The built-in checker alone, in a build without LanguageTool.
#[cfg(not(feature = "languagetool"))]
pub fn chosen(picky: bool) -> Result<Chosen, String> {
    Ok(Chosen {
        checker: Some(Arc::new(Builtin { picky })),
        ignored: Vec::new(),
    })
}

fn one(mut checkers: Vec<Arc<dyn Checker>>) -> Option<Arc<dyn Checker>> {
    match checkers.len() {
        0 | 1 => checkers.pop(),
        _ => Some(Arc::new(Both(checkers))),
    }
}

/// `grind grammar`: change the remembered setting as asked, then print it — or with
/// `check_server`, ask the chosen server what it is.
#[cfg(feature = "languagetool")]
pub fn setting(
    choice: Option<&str>,
    with_builtin: bool,
    style: Option<&str>,
    (ignore, unignore): (Option<&str>, Option<&str>),
    check_server: bool,
) -> Result<Vec<String>, String> {
    let mut setting = Setting::load();
    let before = setting.clone();
    match choice {
        Some("off") => {
            setting.builtin = false;
            setting.languagetool = None;
        }
        Some("builtin") => {
            setting.builtin = true;
            setting.languagetool = None;
        }
        Some(word) => {
            setting.languagetool = Some(Choice::parse(word).ok_or_else(|| {
                format!("{word:?} is none of builtin, off, public, premium, or an http(s) URL")
            })?);
            setting.builtin = with_builtin;
        }
        None if with_builtin => setting.builtin = true,
        None => {}
    }
    match style {
        Some("on") => setting.style = true,
        Some("off") => setting.style = false,
        Some(other) => return Err(format!("--style takes on or off, not {other:?}")),
        None => {}
    }
    if let Some(rule) = ignore
        && !setting.ignored.iter().any(|r| r == rule)
    {
        setting.ignored.push(rule.to_owned());
    }
    if let Some(rule) = unignore {
        let had = setting.ignored.len();
        setting.ignored.retain(|r| r != rule);
        if setting.ignored.len() == had {
            return Err(format!("{rule} is not ignored"));
        }
    }
    let path = match setting != before {
        true => Some(setting.save().map_err(|e| e.to_string())?),
        false => Setting::path(),
    };
    if check_server {
        let choice = setting
            .languagetool
            .as_ref()
            .ok_or("no LanguageTool server is chosen: `grind grammar <url|public|premium>`")?;
        let server = choice.server()?;
        let version = LanguageTool::new(server.clone(), false).probe()?;
        return Ok(vec![format!(
            "{version}\t{}\t{}",
            server.base(),
            server.describe()
        )]);
    }
    let mut lines: Vec<String> = setting.to_text().lines().map(str::to_owned).collect();
    if let Some(path) = path {
        lines.push(format!("# {}", path.display()));
    }
    Ok(lines)
}
