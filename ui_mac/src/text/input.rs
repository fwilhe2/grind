// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What an input method is told about the page, and what its ranges mean — `NSTextInputClient`'s
//! arithmetic, portable (M6).
//!
//! An input method sees a **string** and speaks in `NSRange`s over it, counted in UTF-16 units.
//! The page is not one string: it is blocks, and the core counts characters. So the string an
//! input method is shown is **the caret's own block**, with any marked text spliced in where the
//! caret is — which is everything an input method asks about (the characters before the caret
//! for a reconversion, the ones under the press-and-hold accent menu, where the marked text is).
//! A range is converted here, once, through `grind_core::utf16`, so `page_view.rs` never counts
//! a unit itself.
//!
//! **A selection across blocks** is shown as the caret alone: the one block an input method sees
//! cannot hold it. Typing still replaces the whole selection, because that is `Page::type_text`'s
//! rule and not the input method's.

use std::ops::Range;

use grind_core::utf16;
use grind_text::{App, Caret};

use super::state::{Page, Refused};

/// `NSNotFound`, which is `NSIntegerMax` — how AppKit says "no range" in a location.
pub const NOT_FOUND: usize = isize::MAX as usize;

/// The UTF-16 units before character `offset` of `text`.
pub fn units(text: &str, offset: usize) -> usize {
    let byte = text
        .char_indices()
        .nth(offset)
        .map_or(text.len(), |(b, _)| b);
    utf16::units_before(text, byte)
}

/// The character `units` UTF-16 units into `text` — the one a unit inside a surrogate pair
/// belongs to rounded down, never a place between two halves of one character.
pub fn offset(text: &str, units: usize) -> usize {
    text[..utf16::byte_of(text, units)].chars().count()
}

/// The string the input method is shown: the caret's block, with the marked text in it.
pub fn seen(app: &App, page: &Page) -> String {
    let mut text = app.input_text(page.caret.block).unwrap_or_default();
    if let Some(composing) = &page.composing {
        let byte = text
            .char_indices()
            .nth(page.caret.offset)
            .map_or(text.len(), |(b, _)| b);
        text.insert_str(byte, &composing.text);
    }
    text
}

/// `selectedRange`: the selection inside the caret's block, or the caret where the selection
/// leaves it; inside the marked text, the input method's own selection there.
pub fn selected_range(app: &App, page: &Page) -> Range<usize> {
    let text = seen(app, page);
    let (from, to) = match (&page.composing, page.selection()) {
        (Some(composing), _) => (
            page.caret.offset + composing.selected.start,
            page.caret.offset + composing.selected.end,
        ),
        (None, Some((from, to))) if from.block == to.block => (from.offset, to.offset),
        _ => (page.caret.offset, page.caret.offset),
    };
    units(&text, from)..units(&text, to)
}

/// `markedRange`: where the marked text is in [`seen`], or `None` when nothing is marked.
pub fn marked_range(app: &App, page: &Page) -> Option<Range<usize>> {
    let composing = page.composing.as_ref()?;
    let text = seen(app, page);
    let at = page.caret.offset;
    Some(units(&text, at)..units(&text, at + composing.text.chars().count()))
}

/// `setMarkedText:selectedRange:`'s range, which counts UTF-16 units of the marked text itself,
/// as the characters [`Page::mark`] takes.
pub fn marked_selection(marked: &str, range: Range<usize>) -> Range<usize> {
    offset(marked, range.start)..offset(marked, range.end.max(range.start))
}

/// A `replacementRange` an input method named, over [`seen`] with no marked text in it, made
/// the selection — so the insert that follows replaces exactly it. The press-and-hold accent
/// menu is the everyday case: the `e` already typed is the range, and `é` replaces it.
///
/// A location of [`NOT_FOUND`] names nothing and changes nothing.
pub fn replace(app: &App, page: &mut Page, range: Range<usize>) {
    if range.start == NOT_FOUND {
        return;
    }
    let text = app.input_text(page.caret.block).unwrap_or_default();
    let block = page.caret.block;
    let at = |units| Caret {
        block,
        offset: offset(&text, units),
    };
    page.composing = None;
    page.place(at(range.start), false);
    page.place(at(range.end.max(range.start)), true);
}

/// `insertText:replacementRange:` — the input method's commit, over what it asked to replace.
pub fn insert(app: &App, page: &mut Page, text: &str, range: Range<usize>) -> Result<(), Refused> {
    if page.composing.is_none() {
        replace(app, page, range);
    }
    page.type_text(app, text)
}

/// `setMarkedText:selectedRange:replacementRange:`, the whole of it: what is marked, and which
/// of it the input method has selected.
pub fn mark(
    app: &App,
    page: &mut Page,
    text: &str,
    selected: Range<usize>,
    range: Range<usize>,
) -> Result<(), Refused> {
    if page.composing.is_none() {
        replace(app, page, range);
    }
    page.mark(app, text, marked_selection(text, selected))
}

/// `attributedSubstringForProposedRange:actualRange:`'s text and the range it really covers — the
/// proposal cut to what [`seen`] holds, and never between the two halves of a character.
pub fn substring(app: &App, page: &Page, range: Range<usize>) -> Option<(String, Range<usize>)> {
    let text = seen(app, page);
    let total = units(&text, text.chars().count());
    if range.start == NOT_FOUND || range.start > total {
        return None;
    }
    let (from, to) = (
        offset(&text, range.start),
        offset(&text, range.end.min(total).max(range.start)),
    );
    let piece: String = text.chars().skip(from).take(to - from).collect();
    Some((piece, units(&text, from)..units(&text, to)))
}

/// The caret a unit of [`seen`] is at, for `firstRectForCharacterRange:` — which asks where a
/// range is drawn so the candidate window can sit under it. Inside the marked text the answer is
/// in the composition, which `text/paint.rs::caret_rect` places.
pub fn caret_at(app: &App, page: &Page, units: usize) -> Caret {
    let text = seen(app, page);
    Caret {
        block: page.caret.block,
        offset: offset(&text, units),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grind_text::BlockKind;

    fn app(blocks: &[&str]) -> App {
        let app = App::new();
        for (index, text) in blocks.iter().enumerate() {
            match index {
                0 => app.set_text(0, text).unwrap(),
                _ => app.insert(index, BlockKind::Paragraph, text).unwrap(),
            }
        }
        app
    }

    fn at(block: usize, offset: usize) -> Caret {
        Caret { block, offset }
    }

    /// A character outside the Basic Multilingual Plane is two units and one caret stop.
    #[test]
    fn units_and_characters_convert_both_ways() {
        let text = "a😀b";
        assert_eq!(units(text, 1), 1);
        assert_eq!(units(text, 2), 3);
        assert_eq!(units(text, 9), 4, "past the end is the end");
        assert_eq!(offset(text, 3), 2);
        assert_eq!(offset(text, 2), 1, "inside the pair rounds down");
    }

    #[test]
    fn the_input_method_sees_the_carets_block_and_its_selection() {
        let app = app(&["first", "héllo world"]);
        let mut page = Page::default();
        page.place(at(1, 6), false);
        page.place(at(1, 11), true);
        assert_eq!(seen(&app, &page), "héllo world");
        assert_eq!(selected_range(&app, &page), 6..11);
        assert_eq!(marked_range(&app, &page), None);
        page.place(at(0, 2), true);
        assert_eq!(
            selected_range(&app, &page),
            2..2,
            "across blocks, only the caret"
        );
    }

    #[test]
    fn marked_text_is_part_of_what_the_input_method_sees() {
        let app = app(&["caf"]);
        let mut page = Page::default();
        page.place(at(0, 3), false);
        mark(&app, &mut page, "\u{b4}", 1..1, NOT_FOUND..NOT_FOUND).unwrap();
        assert_eq!(seen(&app, &page), "caf\u{b4}");
        assert_eq!(marked_range(&app, &page), Some(3..4));
        assert_eq!(selected_range(&app, &page), 4..4);
        insert(&app, &mut page, "é", NOT_FOUND..NOT_FOUND).unwrap();
        assert_eq!(app.input_text(0).unwrap(), "café");
        assert_eq!(marked_range(&app, &page), None);
    }

    /// Press and hold `e`, choose `é`: the input method replaces the `e` it can see.
    #[test]
    fn a_replacement_range_replaces_what_it_names() {
        let app = app(&["cafe"]);
        let mut page = Page::default();
        page.place(at(0, 4), false);
        insert(&app, &mut page, "é", 3..4).unwrap();
        assert_eq!(app.input_text(0).unwrap(), "café");
        assert_eq!(page.caret, at(0, 4));
    }

    #[test]
    fn a_substring_is_cut_to_what_there_is() {
        let app = app(&["a😀b"]);
        let page = Page::default();
        assert_eq!(substring(&app, &page, 1..3), Some(("😀".to_owned(), 1..3)));
        assert_eq!(
            substring(&app, &page, 2..99),
            Some(("😀b".to_owned(), 1..4))
        );
        assert_eq!(substring(&app, &page, 9..10), None);
        assert_eq!(substring(&app, &page, NOT_FOUND..NOT_FOUND), None);
        assert_eq!(caret_at(&app, &page, 3), at(0, 2));
    }

    /// The marked text's own selection is counted in its own units.
    #[test]
    fn a_marked_selection_is_in_the_marked_texts_units() {
        assert_eq!(marked_selection("😀x", 2..3), 1..2);
        assert_eq!(marked_selection("ab", 2..0), 2..2, "backwards is empty");
    }
}
