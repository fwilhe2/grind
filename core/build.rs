// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Stamps the commit, tree cleanliness and build date into env vars every shell's
//! `--version`/about dialog reads through [`sheet_core::build_info`]. One build script for
//! the whole workspace, since every binary already depends on `sheet-core`.

use std::process::Command;

fn run(args: &[&str]) -> Option<String> {
    let output = Command::new(args[0]).args(&args[1..]).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8(output.stdout).ok())
        .flatten()
}

fn main() {
    let commit = run(&["git", "rev-parse", "--short", "HEAD"])
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let dirty = run(&["git", "status", "--porcelain"])
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    let date = run(&["date", "-u", "+%Y-%m-%dT%H:%M:%SZ"])
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    println!("cargo:rustc-env=SHEET_BUILD_COMMIT={commit}");
    println!(
        "cargo:rustc-env=SHEET_BUILD_TREE={}",
        if dirty { "dirty" } else { "clean" }
    );
    println!("cargo:rustc-env=SHEET_BUILD_DATE={date}");
}
