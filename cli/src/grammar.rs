// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The remembered grammar setting (`doc/grammar.md`), as `grind grammar` changes it and
//! `grind text grammar` follows it.

use grind_grammar::setting::Setting;

/// `grind grammar`: change the remembered setting as asked, then print it.
pub fn setting(
    choice: Option<&str>,
    style: Option<&str>,
    (ignore, unignore): (Option<&str>, Option<&str>),
) -> Result<Vec<String>, String> {
    let mut setting = Setting::load();
    let before = setting.clone();
    match choice {
        Some("off") => setting.builtin = false,
        Some("builtin" | "on") => setting.builtin = true,
        Some(word) => return Err(format!("{word:?} is neither builtin nor off")),
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
    let path = if setting == before {
        Setting::path()
    } else {
        Some(setting.save().map_err(|e| e.to_string())?)
    };
    let mut lines: Vec<String> = setting.to_text().lines().map(str::to_owned).collect();
    if let Some(path) = path {
        lines.push(format!("# {}", path.display()));
    }
    Ok(lines)
}
