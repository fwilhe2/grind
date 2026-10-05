// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `grind licences` — the third-party list every window's About shows (`doc/third-party.md`),
//! from the command line too (rule 4).

use std::process::Command;

fn ok(args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_grind"))
        .args(args)
        .output()
        .expect("the binary runs");
    assert!(
        output.status.success(),
        "{args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("utf-8")
}

#[test]
fn licences_prints_every_notice_and_the_list_names_every_component() {
    let notices = ok(&["licences"]);
    assert!(notices.contains("GNU Affero General Public License"));
    assert!(
        notices.contains("SIL OPEN FONT LICENSE Version 1.1"),
        "the fonts"
    );
    assert!(
        notices.contains("Permission is hereby granted"),
        "an MIT text"
    );

    let list = ok(&["licenses", "--list"]);
    let rows: Vec<Vec<&str>> = list.lines().map(|l| l.split('\t').collect()).collect();
    assert_eq!(rows.len(), grind_core::third_party::components().len());
    assert!(rows.iter().all(|r| r.len() == 4));
    for name in [
        "quick-xml",
        "zip",
        "krilla",
        "Liberation Fonts",
        "Rust standard library",
    ] {
        assert!(rows.iter().any(|r| r[0] == name), "{name} is on the list");
    }

    let json = ok(&["--format", "json", "licences", "--list"]);
    assert!(json.starts_with('['), "{json}");
    assert!(json.contains("\"kind\":\"font\""), "{json}");
}
