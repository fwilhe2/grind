// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The spreadsheet's grid. [`geom`] is where every cell is and [`paint`] is what a frame of it
//! draws, both portable and tested on any host; only turning [`paint`]'s list into pixels needs
//! a Mac (`render.rs`).

pub mod assist;
pub mod format;
pub mod geom;
pub mod paint;
pub mod search;
pub mod select;
pub mod state;
