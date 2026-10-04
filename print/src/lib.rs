// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Pages, PDF and print preview for the word processor. `doc/pdf-export.md` is normative here.
//!
//! The font stack lives in this crate and nowhere else: `grind-core` stays font-free
//! (`doc/text-layout.md`, decision 2), and so does any build that leaves this crate out.

pub mod fonts;
