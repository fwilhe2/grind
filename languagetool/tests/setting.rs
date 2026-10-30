// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The remembered grammar setting (`doc/grammar.md` decision 5): which checkers, which server,
//! style or not, and the rules ignored — the person's, in `$XDG_CONFIG_HOME/grind/grammar`.

use grind_languagetool::setting::{Choice, Setting};

#[test]
fn nothing_chosen_is_the_built_in_checker_alone() {
    let setting = Setting::parse("");
    assert!(setting.builtin);
    assert_eq!(setting.languagetool, None);
    assert!(!setting.style);
    assert!(setting.ignored.is_empty());
}

#[test]
fn a_file_is_read_line_by_line_and_written_back_the_same() {
    let text = "# mine\nbuiltin off\nlanguagetool http://localhost:8010\nstyle on\nignore lt:EN_A_VS_AN\nignore builtin:double-space\n";
    let setting = Setting::parse(text);
    assert!(!setting.builtin);
    assert_eq!(
        setting.languagetool,
        Some(Choice::SelfHosted("http://localhost:8010".into()))
    );
    assert!(setting.style);
    assert_eq!(setting.ignored, ["lt:EN_A_VS_AN", "builtin:double-space"]);
    assert_eq!(Setting::parse(&setting.to_text()), setting);
    assert_eq!(
        Setting::parse("languagetool public").languagetool,
        Some(Choice::Public)
    );
    assert_eq!(
        Setting::parse("languagetool premium").languagetool,
        Some(Choice::Premium)
    );
    assert_eq!(Setting::parse("languagetool off").languagetool, None);
    assert!(
        Setting::parse("nonsense here\nbuiltin maybe").builtin,
        "unread lines ignored"
    );
}

#[test]
fn premium_needs_its_key_from_the_environment() {
    let server = Choice::Premium.server_from(|name| match name {
        "GRIND_LANGUAGETOOL_USER" => Some("me@example.org".into()),
        "GRIND_LANGUAGETOOL_KEY" => Some("k".into()),
        _ => None,
    });
    assert_eq!(
        server,
        Ok(grind_languagetool::Server::Premium {
            username: "me@example.org".into(),
            key: "k".into()
        })
    );
    let missing = Choice::Premium.server_from(|_| None);
    assert!(missing.unwrap_err().contains("GRIND_LANGUAGETOOL_KEY"));
    assert_eq!(
        Choice::Public.server_from(|_| None),
        Ok(grind_languagetool::Server::Public)
    );
}
