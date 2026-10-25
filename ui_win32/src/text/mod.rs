// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The word processor's pane: the first place `grind_core::layout::Metrics` meets a proportional
//! font with no shaping engine behind it, which is `crate::metrics`' bet (decision 3).
//!
//! None of its files holds state: [`geom`] answers where a block is, [`draw`] what it looks like,
//! [`keymap`] what a keystroke means to a caret, [`status`] what the status bar says — all from
//! arguments. The document lives in `grind_text::App` and the window in `win.rs`; every paint
//! reads a viewport and a layout and throws both away. Only [`draw`] has a Windows half.

pub mod draw;
pub mod geom;
pub mod keymap;
pub mod paper;
pub mod status;
