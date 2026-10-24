// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The **code view** — the document as its projection, on the other page of a `gtk::Stack`
//! (`doc/dsl.md` §6).
//!
//! **No `GtkSourceView`, and no highlighter**: the colours come from the writer, which knows what
//! every byte it emits is, applied as one `gtk::TextTag` per [`TokenKind`]. A highlighter would
//! re-derive that from regexes over text it did not write.
//!
//! **Read-only** (§6.4). Moving the cursor in it moves the caret in the document; typing into it
//! would need an error-tolerant parser, a model diff and an answer for the other view's caret.
//!
//! ponytail: `ui_sheet_gtk/src/code.rs` is this file with a different address vocabulary. There is
//! no crate a widget both GTK shells could share — `grind-core` may not hold GTK types and neither
//! application crate may depend on the other (R8). The upgrade is a `grind-gtk` crate when a third
//! GTK shell or a third copy appears.
//!
//! The widget half is tested through `view.rs`'s `the_widget` harness.

use grind_text::projection::{Projection, TokenKind};
use libadwaita::gtk;
use libadwaita::prelude::*;

/// The tag name a kind is painted with — [`TokenKind::name`], because the core already has one
/// word for each and a second one here would be a table to keep in sync.
fn tag(kind: TokenKind) -> &'static str {
    kind.name()
}

/// The name of the tag marking the line the document's own selection is on.
const CURRENT: &str = "current";

/// Build the view, its buffer and the tags. Monospace, since a projection's alignment means it,
/// and non-editable, which makes §6.4 structural.
pub fn build() -> gtk::TextView {
    let view = gtk::TextView::builder()
        .editable(false)
        .cursor_visible(true)
        .monospace(true)
        .left_margin(12)
        .right_margin(12)
        .top_margin(8)
        .bottom_margin(8)
        .build();
    let buffer = view.buffer();
    let palette = crate::theme::Palette::of(&view);
    for (kind, colour) in colours(&palette) {
        let tag = gtk::TextTag::builder().name(tag(kind)).build();
        tag.set_foreground_rgba(Some(&colour));
        if kind == TokenKind::Node {
            tag.set_weight(700);
        }
        if kind == TokenKind::Comment {
            tag.set_style(gtk::pango::Style::Italic);
        }
        buffer.tag_table().add(&tag);
    }
    let current = gtk::TextTag::builder().name(CURRENT).build();
    current.set_background_rgba(Some(&palette.selection));
    buffer.tag_table().add(&current);
    view
}

/// What each kind is painted, from the running theme's ink and accent — never a literal, or the
/// page stays white in a dark theme. A theme yields about five distinguishable colours, so kinds
/// share where they can and weight tells them apart.
fn colours(palette: &crate::theme::Palette) -> [(TokenKind, gtk::gdk::RGBA); 6] {
    [
        (TokenKind::Node, palette.accent),
        (TokenKind::Property, mix(palette.accent, palette.foreground)),
        (TokenKind::Text, palette.foreground),
        (TokenKind::Number, mix(palette.foreground, palette.accent)),
        (TokenKind::Keyword, palette.accent),
        (TokenKind::Comment, palette.dim),
    ]
}

fn mix(a: gtk::gdk::RGBA, b: gtk::gdk::RGBA) -> gtk::gdk::RGBA {
    gtk::gdk::RGBA::new(
        (a.red() + b.red()) / 2.0,
        (a.green() + b.green()) / 2.0,
        (a.blue() + b.blue()) / 2.0,
        1.0,
    )
}

/// Put a projection in the view, with `cursor` — the line the document's selection is on — marked
/// and scrolled to. Rebuilt whole rather than patched: the pane is read-only, so it is refilled
/// only when the document changed or the tab opened.
pub fn fill(view: &gtk::TextView, projection: &Projection, cursor: Option<usize>) {
    let buffer = view.buffer();
    buffer.set_text("");
    let mut end = buffer.end_iter();
    for line in 0..projection.line_count() {
        for piece in projection.line_pieces(line) {
            match piece.kind {
                Some(kind) => buffer.insert_with_tags_by_name(&mut end, piece.text, &[tag(kind)]),
                // The stretches the writer never named — indentation, braces, the spaces
                // between values — go in untagged. A view that dropped them would be showing a
                // projection that does not parse.
                None => buffer.insert(&mut end, piece.text),
            }
        }
        if line + 1 < projection.line_count() {
            buffer.insert(&mut end, "\n");
        }
    }
    if let Some(line) = cursor {
        go_to(view, line);
    }
}

/// Mark a line as the document's own — the tag, and nothing else. **It must not move the cursor**:
/// this runs from the handler that runs because the cursor moved, and placing it again loops until
/// the window stops answering. [`go_to`] is the half that drives the view.
pub fn mark(view: &gtk::TextView, line: usize) {
    let buffer = view.buffer();
    let (start, end) = (buffer.start_iter(), buffer.end_iter());
    buffer.remove_tag_by_name(CURRENT, &start, &end);
    let Some(from) = buffer.iter_at_line(line as i32) else {
        return;
    };
    let mut to = from;
    if !to.ends_line() {
        to.forward_to_line_end();
    }
    buffer.apply_tag_by_name(CURRENT, &from, &to);
}

/// Mark a line *and* put the view's own cursor on it, scrolled into sight — what the shell calls
/// when the document moved and the source has to follow.
pub fn go_to(view: &gtk::TextView, line: usize) {
    mark(view, line);
    let buffer = view.buffer();
    let Some(mut at) = buffer.iter_at_line(line as i32) else {
        return;
    };
    buffer.place_cursor(&at);
    view.scroll_to_iter(&mut at, 0.1, false, 0.0, 0.5);
}

/// Which line the view's own cursor is on — the question the shell asks after the cursor moved,
/// so it can select what that line projects.
pub fn line_at_cursor(view: &gtk::TextView) -> usize {
    let buffer = view.buffer();
    buffer
        .iter_at_offset(buffer.cursor_position())
        .line()
        .max(0) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tag names are the core's own words, so this file and `TokenKind` cannot drift into
    /// two vocabularies for one thing. No display needed — this is a table, not a widget.
    #[test]
    fn every_kind_has_a_tag_named_after_it() {
        for kind in [
            TokenKind::Node,
            TokenKind::Property,
            TokenKind::Text,
            TokenKind::Number,
            TokenKind::Keyword,
            TokenKind::Comment,
        ] {
            assert_eq!(tag(kind), kind.name());
            assert_ne!(
                tag(kind),
                CURRENT,
                "and none of them collides with the mark"
            );
        }
    }
}
