// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The word processor half of the terminal shell. **\[ODT\]**
//!
//! Phase 10's S8, and it comes *before* the GTK text shell on purpose (`doc/suite.md`): it is
//! the cheapest complete editor, and it is the sharpest test of `doc/text-layout.md`'s
//! decision. A terminal cannot fake font metrics — a cell is a cell — so if the layout engine
//! were secretly assuming pixels, this is where it would fall over.
//!
//! It does not. [`Cells`] below is the whole of what this shell contributes to layout: about
//! twenty lines answering "how wide is this text, in terminal columns". Line breaking, and
//! every caret motion defined in terms of a line, come from `grind_core::layout` through
//! `grind_text::App` — the same code the GTK shell will drive through Pango.

pub mod app;
pub mod keymap;

/// The word processor's own keys and commands — `--help` prints it, `:help` shows it.
pub const HELP: &str = "\
Word processor:
  i  type here    a  type after    o  new paragraph below
  x  erase a character    X  delete the block    J  join with the next
  Visual mode also: _  underline    ~  strikethrough    `  monospace

  Markdown as you type:
    **bold**   *italic*   __underline__   ~~struck~~   `code`
    \"# \" a heading (to \"###### \")     \"- \" a list item
    ``` on its own — a code block; ``` again ends it

  :color <name>  :highlight <name>  :plain
  :h <level>     :li [depth]        :style [name]
  :find <text>   :s/old/new/        :outline   :words
  :names         — show where each bookmark anchors, which is otherwise invisible
  :<address>     — p12, p12+40, #bookmark or \u{a7}2.1.3
";

/// What `:help` shows: what this shell shares with the other, then its own.
pub fn help() -> String {
    format!("{}\n{HELP}", crate::help::COMMON)
}

use grind_core::style::TextStyle;
use grind_text::Metrics;
use unicode_width::UnicodeWidthChar;

/// Text measured in **terminal cells**.
///
/// The proof that injecting metrics works, and the reason `doc/text-layout.md` chose Path C
/// over putting a line breaker in each shell: the unit here is a character cell, GTK's will be
/// a Pango unit, and the engine above neither knows nor cares — it does arithmetic against a
/// width supplied in the same unit and never invents one.
///
/// `unicode-width` rather than counting `char`s, because a terminal is a grid of cells and a
/// CJK ideograph occupies two of them while a combining mark occupies none. Getting that wrong
/// does not merely look untidy: every caret x this shell draws would be off by the difference.
///
/// Font family, size, weight and style are all ignored, because a terminal has one font at one
/// size. That is not a shortcut — it is what makes the terminal the honest test.
#[derive(Clone, Copy, Debug, Default)]
pub struct Cells;

impl Metrics for Cells {
    fn advances(&self, text: &str, _style: &TextStyle, out: &mut Vec<f32>) {
        let mut x = 0.0;
        for c in text.chars() {
            // A control character has no width here; `\t` and `\n` reach us as themselves and
            // are one cell each so that a caret can sit either side of one.
            x += match c {
                '\t' | '\n' => 1.0,
                _ => c.width().unwrap_or(0) as f32,
            };
            out.push(x);
        }
    }

    fn line_height(&self, _style: &TextStyle) -> f32 {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn widths(text: &str) -> Vec<f32> {
        let mut out = Vec::new();
        Cells.advances(text, &TextStyle::default(), &mut out);
        out
    }

    #[test]
    fn an_ascii_character_is_one_cell() {
        assert_eq!(widths("abc"), vec![1.0, 2.0, 3.0]);
    }

    /// The reason this is not `chars().count()`. A terminal that believed an ideograph were one
    /// cell wide would wrap a line short and draw every caret after it in the wrong column.
    #[test]
    fn a_wide_character_is_two_cells_and_a_combining_mark_is_none() {
        assert_eq!(widths("\u{4e16}\u{754c}"), vec![2.0, 4.0], "CJK");
        // "e" then a combining acute: one cell between them, not two.
        assert_eq!(widths("e\u{301}"), vec![1.0, 1.0]);
    }

    #[test]
    fn a_tab_or_a_break_is_one_cell_so_a_caret_can_sit_beside_it() {
        assert_eq!(widths("a\tb"), vec![1.0, 2.0, 3.0]);
        assert_eq!(widths("a\nb"), vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn every_character_contributes_exactly_one_advance() {
        // The trait's contract, and the one thing the layout engine relies on.
        for text in ["", "plain", "\u{4e16}x\u{301}\t"] {
            assert_eq!(widths(text).len(), text.chars().count(), "{text:?}");
        }
    }
}
