// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Byte offsets ⇄ the protocol's positions.
//!
//! Everything in the suite counts **bytes** — a span in the projection, an anchor, a parse
//! error's offset — and the protocol counts **lines and UTF-16 code units**, because that is
//! what VS Code counts and the protocol's default (`positionEncoding` was added later and VS
//! Code still asks for nothing else). The conversion within a line is `grind_core::utf16`'s, the
//! one the Windows and Mac shells already use for the same reason, so an `é` or an emoji before
//! a cell lands the mark where it is in all three.

use std::ops::Range;

use grind_core::utf16;
use lsp_types::{Position, Range as LspRange};

/// A text and where each of its lines starts — built once per version of a document, since
/// every request asks for several positions.
pub struct Lines<'a> {
    text: &'a str,
    starts: Vec<usize>,
}

impl<'a> Lines<'a> {
    pub fn new(text: &'a str) -> Self {
        let mut starts = vec![0];
        starts.extend(text.match_indices('\n').map(|(at, _)| at + 1));
        Lines { text, starts }
    }

    /// The line a byte is on, 0-based.
    fn line_of(&self, byte: usize) -> usize {
        self.starts.partition_point(|&start| start <= byte) - 1
    }

    /// The text of one line, without its newline.
    fn line_text(&self, line: usize) -> &'a str {
        let start = self.starts[line];
        let end = self
            .starts
            .get(line + 1)
            .map_or(self.text.len(), |&next| next - 1);
        &self.text[start..end.max(start)]
    }

    /// Where a byte offset is, as the protocol counts. Past the end is the end.
    pub fn position(&self, byte: usize) -> Position {
        let byte = byte.min(self.text.len());
        let line = self.line_of(byte);
        let start = self.starts[line];
        Position {
            line: line as u32,
            character: utf16::units_before(self.line_text(line), byte - start) as u32,
        }
    }

    /// The byte offset of a protocol position. A line past the end is the end of the text; a
    /// character past the end of its line is the end of that line.
    pub fn offset(&self, position: Position) -> usize {
        let line = position.line as usize;
        if line >= self.starts.len() {
            return self.text.len();
        }
        self.starts[line] + utf16::byte_of(self.line_text(line), position.character as usize)
    }

    pub fn range(&self, span: Range<usize>) -> LspRange {
        LspRange {
            start: self.position(span.start),
            end: self.position(span.end),
        }
    }

    /// A position given as a 1-based line and a 1-based column counted in characters — what
    /// a parse error or `grind build` reports — as the protocol's.
    pub fn at_line_column(&self, line: usize, column: usize) -> Position {
        let line = line.saturating_sub(1).min(self.starts.len() - 1);
        let text = self.line_text(line);
        let byte = text
            .char_indices()
            .nth(column.saturating_sub(1))
            .map_or(text.len(), |(at, _)| at);
        Position {
            line: line as u32,
            character: utf16::units_before(text, byte) as u32,
        }
    }

    /// The word around a byte offset — letters, digits, `_` and `.` — and its span. What a
    /// hover or a completion is about.
    pub fn word_at(&self, byte: usize) -> (Range<usize>, &'a str) {
        let byte = byte.min(self.text.len());
        let is_word = |c: char| c.is_alphanumeric() || c == '_';
        let start = self.text[..byte]
            .char_indices()
            .rev()
            .take_while(|&(_, c)| is_word(c))
            .last()
            .map_or(byte, |(at, _)| at);
        let end = self.text[byte..]
            .char_indices()
            .find(|&(_, c)| !is_word(c))
            .map_or(self.text.len(), |(at, _)| byte + at);
        (start..end, &self.text[start..end])
    }

    pub fn text(&self) -> &'a str {
        self.text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_count_utf16_units_within_a_line() {
        let text = "ab\nwé😀x\n";
        let lines = Lines::new(text);
        // `x` is after `w` (1), `é` (1 unit, 2 bytes) and `😀` (2 units, 4 bytes).
        let x = text.find('x').expect("there");
        assert_eq!(lines.position(x), Position::new(1, 4));
        assert_eq!(lines.offset(Position::new(1, 4)), x);
        assert_eq!(lines.position(0), Position::new(0, 0));
        assert_eq!(lines.position(text.len()), Position::new(2, 0), "the end");
        assert_eq!(
            lines.offset(Position::new(9, 0)),
            text.len(),
            "past the end"
        );
    }

    #[test]
    fn a_compilers_line_and_column_is_a_position() {
        let lines = Lines::new("one\n  é two\n");
        // Column 5 is `t`: two spaces, `é`, a space, then `t`.
        assert_eq!(lines.at_line_column(2, 5), Position::new(1, 4));
        assert_eq!(lines.at_line_column(9, 1), Position::new(2, 0), "clamped");
    }

    #[test]
    fn a_word_is_found_from_inside_it() {
        let lines = Lines::new("s.push(row);");
        let at = "s.pu".len();
        assert_eq!(lines.word_at(at), (2..6, "push"));
        assert_eq!(lines.word_at(1).1, "s", "at the dot, the word before it");
    }
}
