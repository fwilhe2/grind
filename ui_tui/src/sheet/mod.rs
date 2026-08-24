// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The spreadsheet half of the terminal shell. **\[ODS\]**
//!
//! Its sibling is [`crate::text`], and `main.rs` picks between them by asking
//! `grind_core::kind` what the file is — R10's rule that every document type reaches every
//! shell, arriving in the cheapest shell first.

pub mod app;
pub mod keymap;

/// The ODF sheet limits, and the only bound [`keymap::moved`] clamps a plain move to.
pub const MAX_ROWS: u32 = 1_048_576;
pub const MAX_COLS: u32 = 16_384;
