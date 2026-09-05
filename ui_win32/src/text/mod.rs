// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The word processor's pane — **W5**, and the milestone `doc/windows-shell.md` says to be
//! nervous about.
//!
//! The grid was arithmetic this project had done three times. This is the first time
//! `grind_core::layout::Metrics` meets a proportional font with no shaping engine behind it, and
//! `crate::metrics` is the bet decision 3 makes about that.
//!
//! Four files, and not one of them owns any state: [`geom`] answers where a block is, [`draw`]
//! what it looks like, [`keymap`] what a keystroke means to a caret, and [`status`] what the
//! status bar says — all of them from arguments. The document itself lives in `grind_text::App`
//! and the window in `win.rs`, which is architecture rule 1 unchanged: every paint reads a
//! viewport and throws it away, and every paint reads a *layout* and throws that away too.
//!
//! Only [`draw`] has a Windows half at all, and only the part of it that puts pixels down.

pub mod draw;
pub mod geom;
pub mod keymap;
pub mod status;
