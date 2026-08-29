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

/// The spreadsheet's own keys and commands — `--help` prints it, `:help` shows it.
pub const HELP: &str = "\
Spreadsheet:
  i, a  edit the cell     c  edit from empty
  x, d  clear the cell, or everything selected
  :bold  :italic  :wrap  :border  :plain
  :align l|c|r            :color <name|#rrggbb>   :fill <name|#rrggbb>
  :format general|int|number [n]|percent|currency|date|time|datetime
  :general                :recalc
  :roles  :names          — what each cell is, and what it is called (a reading;
                            nothing is written, and the same word turns it off)
  :sheet <name>  :sheet-new  :sheet-rename <name>  :sheet-delete
  :<address>              — a cell or a range, e.g. B12 or Data.A1
";

/// What `:help` shows: what this shell shares with the other, then its own.
pub fn help() -> String {
    format!("{}\n{HELP}", crate::help::COMMON)
}

/// The ODF sheet limits, and the only bound [`keymap::moved`] clamps a plain move to.
pub const MAX_ROWS: u32 = 1_048_576;
pub const MAX_COLS: u32 = 16_384;
