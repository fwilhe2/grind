// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! W8's icon and version resource (`doc/windows-shell.md`), compiled and linked in by
//! `embed_resource` — never attempted off Windows.
//!
//! `CARGO_CFG_TARGET_OS` rather than `cfg!(windows)`: a build script runs on the **host**, so
//! `cfg!(windows)` would answer for the machine doing the build rather than the one the binary
//! is for, and this crate is routinely *checked* for `x86_64-pc-windows-msvc` from Linux
//! (`doc/windows-shell.md`'s whole point). `embed_resource::compile` already answers "nothing to
//! do" on its own when it finds no resource compiler for the target rather than failing the
//! build — the check here just skips the attempt entirely on a target that was never Windows,
//! so `cargo check -p grind-win32` on this crate's *own* host target build (used nowhere in this
//! workspace, but not forbidden either) stays silent instead of printing a compiler search that
//! could never succeed.
//!
//! How far this can be verified off Windows is a property of the machine, not of this crate.
//! With no resource compiler present, `embed_resource` degrades to a no-op rather than an error
//! (`CompilationResult::NotAttempted`, logged but not propagated) and nothing here is checked
//! beyond "did not panic". With `llvm-windres` installed it compiles the resource for real, and
//! `cargo xwin build` then links an executable whose `.rsrc` directory can be read back — which
//! is how W8's version-block bug was found and fixed from Linux (`doc/windows-shell.md`). The
//! shipped claim is still only ever asserted on `windows-latest`, where `win32.yml` reads
//! `FileVersionInfo` off the linked `.exe` — the same rule every other Windows-only claim in
//! this crate is held to.
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        // `manifest_optional` is the crate's own answer to "no compiler found is fine, a
        // compiler that tried and choked is not" — the distinction this build script cares
        // about, since the former is every host but `windows-latest` and the latter would mean
        // `grind.rc` itself is broken.
        let result = embed_resource::compile("data/grind.rc", embed_resource::NONE);
        // Printed unconditionally, not only on failure: `manifest_optional()` swallows
        // `Ok` and `NotAttempted` alike, so a run where the resource silently failed to embed
        // left no warning anywhere to explain why the version check after it failed.
        println!("cargo:warning=grind.rc: {result}");
        if let Err(failed) = result.manifest_optional() {
            println!("cargo:warning=grind.rc failed to compile: {failed}");
        }
    }
}
