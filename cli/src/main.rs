// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `sheet` — the CLI. Phase 6 gives it subcommands; until the core has anything to
//! expose, it exists so the workspace builds and the parity rule has somewhere to live.

fn main() {
    eprintln!("sheet: nothing to drive yet — the core is at phase 0. See doc/plan.md.");
    std::process::exit(1);
}
