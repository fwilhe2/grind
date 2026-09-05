// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The spreadsheet pane.
//!
//! Four files, and not one of them owns any state: [`geom`] answers where a cell is, [`draw`]
//! what it looks like, [`keymap`] what a keystroke means, and [`status`] what the name box and
//! the status bar say — all of them from arguments. The document itself lives in
//! `grind_sheet::App` and the window in `win.rs`, which is rule 1 of the architecture unchanged
//! — every paint reads a viewport and throws it away.
//!
//! Only [`draw`] has a Windows half at all, and only the part of it that puts pixels down. The
//! other three compile and run their tests on any host, which is what lets this shell be
//! developed on the Linux machine this repository lives on.

pub mod draw;
pub mod geom;
pub mod keymap;
pub mod state;
pub mod status;
