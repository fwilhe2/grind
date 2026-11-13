// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The remembered grammar setting (`doc/grammar.md` decision 5), in
//! `$XDG_CONFIG_HOME/grind/grammar`: `builtin on|off`, `style on|off`, and an `ignore <rule>`
//! line per rule ignored. A line this build cannot read is ignored rather than read as something
//! else.

use std::path::PathBuf;

/// Whether the checker runs, with which rules, and what is left alone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Setting {
    pub builtin: bool,
    /// The style rules too.
    pub style: bool,
    /// Rule ids the person ignores, as findings carry them (`builtin:…`).
    pub ignored: Vec<String>,
}

impl Default for Setting {
    fn default() -> Self {
        Setting {
            builtin: true,
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
                "ignore" => setting.ignored.push(value.to_owned()),
                _ => {}
            }
        }
        setting
    }

    #[must_use]
    pub fn to_text(&self) -> String {
        let flag = |b: bool| if b { "on" } else { "off" };
        let mut out = format!("builtin {}\n", flag(self.builtin));
        out += &format!("style {}\n", flag(self.style));
        for rule in &self.ignored {
            out += &format!("ignore {rule}\n");
        }
        out
    }

    /// Where it lives, or `None` with no home directory to put it in.
    #[must_use]
    pub fn path() -> Option<PathBuf> {
        Some(grind_spell::config_dir()?.join("grammar"))
    }

    /// The remembered setting — the built-in checker without style rules when nothing was ever
    /// chosen.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_setting_round_trips_and_a_line_it_cannot_read_is_left_alone() {
        let setting = Setting {
            builtin: false,
            style: true,
            ignored: vec!["builtin:double-space".into()],
        };
        assert_eq!(Setting::parse(&setting.to_text()), setting);
        // A file written when a grammar server could be named still reads.
        let older = "builtin on\nlanguagetool http://localhost:8010\nstyle on\n";
        assert_eq!(
            Setting::parse(older),
            Setting {
                style: true,
                ..Setting::default()
            }
        );
    }
}
