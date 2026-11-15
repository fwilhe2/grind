// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The word processor's pane: the document's printed pages, set by `grind_print` and drawn from its
//! raster (`doc/page-model.md` PM7).
//!
//! None of its files holds state: [`geom`] answers where the window's bands are, [`pages`] where
//! each page and each caret is, [`draw`] what it looks like, [`keymap`] what a keystroke means to a
//! caret, [`status`] what the status bar says — all from arguments. The document lives in
//! `grind_text::App` and the window in `win.rs`. Only [`draw`] has a Windows half.

pub mod draw;
pub mod geom;
pub mod keymap;
pub mod pages;
pub mod paper;
pub mod status;
