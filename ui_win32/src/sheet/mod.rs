// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The spreadsheet pane.
//!
//! Two files in W1 and neither of them owns any state: [`geom`] answers where a cell is and
//! [`draw`] what it looks like, both from arguments. The document itself lives in
//! `grind_sheet::App` and the window in `win.rs`, which is rule 1 of the architecture unchanged
//! — every paint reads a viewport and throws it away.

pub mod draw;
pub mod geom;
