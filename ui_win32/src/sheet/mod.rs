// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The spreadsheet pane. None of its files owns the document: [`geom`] answers where a cell is,
//! [`draw`] what it looks like, [`keymap`] what a keystroke means, [`state`] what one means while
//! editing, [`status`] what the name box and status bar say, [`assist`] what to offer somebody
//! typing a formula, [`format`](mod@format) what the format strip reads and writes, [`tabs`] where
//! the sheet tabs sit, and [`currency`] what the three currencies write — all from arguments. The
//! document lives in `grind_sheet::App` and the window in `win.rs`; every paint reads a viewport
//! and throws it away. Only [`draw`] has a Windows half.

pub mod assist;
pub mod chart;
pub mod currency;
pub mod draw;
pub mod format;
pub mod geom;
pub mod keymap;
pub mod measure;
pub mod state;
pub mod status;
pub mod tabs;
