// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The commit, tree cleanliness and build date `core/build.rs` stamps in, formatted once so
//! every shell's `--version`/about dialog reads the same fact rather than each computing its
//! own.

/// The commit `HEAD` pointed at when this binary was built, `"unknown"` outside a git
/// checkout (a source tarball, say).
pub const COMMIT: &str = env!("GRIND_BUILD_COMMIT");

/// `"clean"` or `"dirty"` — whether `git status --porcelain` had anything to say.
pub const TREE: &str = env!("GRIND_BUILD_TREE");

/// UTC build timestamp, `YYYY-MM-DDTHH:MM:SSZ`.
pub const DATE: &str = env!("GRIND_BUILD_DATE");

/// `version` is the caller's own (`CARGO_PKG_VERSION`), since every shell is its own crate
/// and this one built `grind-core`, not them. For a caller (clap) that already prints its
/// own `name version` header; see [`describe`] for a standalone one.
pub fn describe_version(version: &str) -> String {
    let profile = if cfg!(debug_assertions) {
        "debug build"
    } else {
        "release build"
    };
    format!("{version} ({profile})\ncommit: {COMMIT} ({TREE} git tree)\nbuilt:  {DATE}")
}

/// Like [`describe_version`], with a `name v` header of its own — for a standalone display
/// (an about dialog) with no other caller printing a header.
pub fn describe(name: &str, version: &str) -> String {
    format!("{name} v{}", describe_version(version))
}
