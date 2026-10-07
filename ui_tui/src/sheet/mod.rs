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
pub mod chartview;
pub mod geom;
pub mod keymap;

/// The spreadsheet's own keys and commands — `--help` prints it, `:help` shows it.
pub const HELP: &str = "\
Spreadsheet:
  i, a  edit the cell     c  edit from empty
  x, d  clear the cell, or everything selected
  n, N  the next / previous match of the last :find
  w b } {  jump to the next edge of the data (Ctrl+arrows too)
  V  select the row   Ctrl+V  the column   Ctrl+A  everything the sheet uses
  While typing a formula: Tab accepts a completion, Up/Down pick one, Esc dismisses
  After `=`, an operator or `(` the arrows point at a cell and write its address (point mode);
  they keep moving it until you type something else
  :bold  :italic  :underline  :strike  :wrap  :border  :plain
  :indent  :outdent       — one indent level in or out (> and < in Visual mode)
  :align l|c|r            :color <name|#rrggbb>   :fill <name|#rrggbb>
  :format general|int|number [n]|percent|currency|date|time|datetime
  :general                :recalc
  :group                  — thousands grouped, or not, over the selection
  :find <text>            — every cell whose text or formula holds it; n / N step
  :s/old/new/             — replace it in every cell, one undo step
  :down  :right           — fill the selection from its first cell (references shift)
  :across                 — the cell the selection grew from, into all of it; one undo step
  :value                  — formulas in the selection become the values they show
  :yank-values            — the selection as shown (results, not formulas) into the register
  :chart  :chart!         — a chart of the table here (drawn by the other windows); drop the last
  :chart <words>          — change the last chart: line|bar|pie  title=…|no-title  legend=top|…|none
                            width=12cm  height=8cm
  :chart here             — move the last chart so it starts at the cursor's cell
  :calc [text]            — every formula in the document, searchable; Enter jumps to its cell
  :functions [text]       — the functions, with plain names; Enter starts a formula with one
  :locale [tag|none]      — the document's own locale (de-DE): how it spells numbers
  :explain                — the active cell's formula unfolded in plain words, in a pane
  :friendly               — the formula line in plain words at rest, or as stored
  :filter                 — an autofilter over the selection (or the table around one cell); again to drop
  :eval <formula>         — what it would come to, storing nothing
  :charts   — the sheet's charts, drawn in characters
  :rule <look> <condition> — draw the selection differently while a formula is true:
                            :rule red =B2>100; looks are red, yellow, green, blue,
                            red-text, green-text, bold and strike
  :rules  :rule! [n]      — the sheet's rules (Enter jumps), or drop rule n / every
                            rule touching the selection
  :width [n|auto]  :fit  :height [n]   :hide  :show   — the columns the selection covers
  :fit rows  :fit all     — the selected rows back to their content, or every column and row
  :merge  :unmerge        — the selection into one cell, or every merge in it undone
  :name <name>  :name!    — define a name over the selection, or drop the one on it
  :rename <old> <new>     — rename a name, every formula that uses it following
  :inline <name>          — write a name's definition into every use and drop it
  :format-table [--no-header] [--totals [FUNC]] [--name NAME]
                           — autofilter, banding, an optional totals row and a name;
                             FUNC is sum (the default), average, count, count-numbers,
                             min, max, stdev or var
  :csv-in <file> [with delimiter=semicolon locale=de-DE text formulas trim no-dates]
  :csv-out <file>
  :formulas               — show each formula instead of its result (a reading; the same word undoes it)
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
