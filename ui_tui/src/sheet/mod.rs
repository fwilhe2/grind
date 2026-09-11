// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The spreadsheet half of the terminal shell. **\[ODS\]**
//!
//! Its sibling is [`crate::text`], and `main.rs` picks between them by asking
//! `grind_core::kind` what the file is — R10's rule that every document type reaches every
//! shell, arriving in the cheapest shell first.

pub mod app;
pub mod assist;
pub mod geom;
pub mod keymap;

/// The spreadsheet's own keys and commands — `--help` prints it, `:help` shows it.
pub const HELP: &str = "\
Spreadsheet:
  i, a  edit the cell     c  edit from empty
  x, d  clear the cell, or everything selected
  n, N  the next / previous match of the last :find
  While typing a formula: Tab accepts a completion, Up/Down pick one, Esc dismisses
  :bold  :italic  :wrap  :border  :plain
  :align l|c|r            :color <name|#rrggbb>   :fill <name|#rrggbb>
  :format general|int|number [n]|percent|currency|date|time|datetime
  :general                :recalc
  :find <text>            — every cell whose text or formula holds it; n / N step
  :down  :right           — fill the selection from its first cell (references shift)
  :eval <formula>         — what it would come to, storing nothing
  :width [n|auto]  :height [n]   :hide  :show   — the columns the selection covers
  :name <name>  :name!    — define a name over the selection, or drop the one on it
  :format-table [--no-header] [--totals] [--name NAME]
                           — autofilter, banding, an optional totals row and a name
  :csv-in <file>  :csv-out <file>
  :roles  :names          — what each cell is, and what it is called (a reading;
                            nothing is written, and the same word turns it off)
  :source                 — the document as its projection, read-only; j/k moves and
                            selects the cell that line is
  :lint  :lint hints      — what the document says about itself; Enter goes to a finding
  :sheet <name>  :sheet-new  :sheet-rename <name>  :sheet-delete
  :<address>              — a cell, a range or a defined name, e.g. B12, Data.A1, tax_rate
";

/// What `:help` shows: what this shell shares with the other, then its own.
pub fn help() -> String {
    format!("{}\n{HELP}", crate::help::COMMON)
}

/// The ODF sheet limits, and the only bound [`keymap::moved`] clamps a plain move to.
pub const MAX_ROWS: u32 = 1_048_576;
pub const MAX_COLS: u32 = 16_384;
