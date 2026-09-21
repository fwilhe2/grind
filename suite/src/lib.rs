// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The suite's meta-package, which is not code. `Cargo.toml` is the whole of it: a `.deb` and an
//! `.rpm` that install nothing and depend on the four applications (`doc/suite.md`, S11). This
//! file exists because a crate must have a target.
