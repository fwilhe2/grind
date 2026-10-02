// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What a Format command does to the page (M7), and whether its menu item is ticked — portable.
//!
//! Every character change is a `grind_text::format::Change`, applied where `format::apply`
//! applies it — over the selection, or held for the next character typed when nothing is
//! selected — so a Bold from this menu, from the toolbar, from `**` typed and from `grind text
//! format --bold` are the same run. A paragraph kind is `App::set_kind` over every block the
//! selection touches.

use grind_text::BlockKind;
use grind_text::format::{Change, has};
use grind_text::style::CharStyle;

use crate::menu::{Block, Command};
use crate::sheet::format::color;

/// The character change `command` asks for, over what the selection agrees about (`here`).
pub fn change(command: Command, here: &CharStyle) -> Option<Change> {
    match command {
        Command::Mark(emphasis) => Some(Change::toggle(emphasis, here)),
        Command::TextColor(index) => Some(Change::Color(color(index))),
        Command::Background(index) => Some(Change::Highlight(color(index))),
        Command::ClearFormatting => Some(Change::Clear),
        _ => None,
    }
}

/// The two style names this page applies and takes away — the ones `grind_text::look` draws in a
/// face of their own. Any other name on a block is the document's, and is left alone.
const OURS: [&str; 2] = ["Title", "Subtitle"];

/// What `command` makes a block that is `current`, wearing `style`: its kind, and the paragraph
/// style it should wear — the GNOME window's rule. Title and Subtitle are paragraphs wearing those
/// names; any other kind takes one of those two names off, and leaves a document's own name (a
/// `Quotations`, say) exactly where it was. A list item keeps its depth when asked to be a list
/// item again.
pub fn block(
    command: Command,
    current: &BlockKind,
    style: Option<&str>,
) -> Option<(BlockKind, Option<String>)> {
    let Command::Block(block) = command else {
        return None;
    };
    let kept = style.filter(|name| !OURS.contains(name)).map(str::to_owned);
    Some(match block {
        Block::Title => (BlockKind::Paragraph, Some("Title".to_owned())),
        Block::Subtitle => (BlockKind::Paragraph, Some("Subtitle".to_owned())),
        Block::Body => (BlockKind::Paragraph, kept),
        Block::Heading(level) => (
            BlockKind::Heading {
                level: u32::from(level),
            },
            kept,
        ),
        Block::ListItem => match current {
            BlockKind::ListItem { depth } => (BlockKind::ListItem { depth: *depth }, kept),
            _ => (BlockKind::ListItem { depth: 1 }, kept),
        },
    })
}

/// Whether the command's menu item is ticked: an emphasis every selected character has, the
/// colour they agree on, the kind the caret's block is — and for a block, what it wears, since a
/// heading wearing `Title` is drawn as a title.
pub fn checked(command: Command, here: &CharStyle, kind: &BlockKind, style: Option<&str>) -> bool {
    match command {
        Command::Mark(emphasis) => has(here, emphasis),
        Command::TextColor(index) => here.color == color(index),
        Command::Background(index) => here.background == color(index),
        Command::Block(_) => block(command, kind, style)
            .is_some_and(|(wanted, wears)| wanted == *kind && wears.as_deref() == style),
        _ => false,
    }
}

/// What the font panel says about a font — the four things it can change that a document can
/// hold. `NSFontManager` hands `changeFont:` a whole font; the difference between the one it was
/// shown and the one it answers is the change.
#[derive(Clone, Debug, PartialEq)]
pub struct Facts {
    pub family: String,
    /// In points.
    pub size: f64,
    pub bold: bool,
    pub italic: bool,
}

/// A size as a document spells it: `14pt`, `10.5pt` — the font panel's own points, rounded to
/// a tenth, since a size the panel's slider happens to land on is not a size anybody chose.
pub fn points(size: f64) -> String {
    let tenths = (size * 10.0).round() / 10.0;
    match tenths.fract() == 0.0 {
        true => format!("{}pt", tenths as i64),
        false => format!("{tenths}pt"),
    }
}

/// The changes the font panel asked for: one per property that differs between the font it was
/// shown and the one it answered. A family is written by name; nothing that stayed the same is
/// touched, so choosing a size in the panel leaves a run's own family alone.
pub fn font_changes(before: &Facts, after: &Facts) -> Vec<Change> {
    let mut changes = Vec::new();
    if after.family != before.family {
        changes.push(Change::Family(Some(after.family.clone())));
    }
    if points(after.size) != points(before.size) {
        changes.push(Change::Size(Some(points(after.size))));
    }
    if after.bold != before.bold {
        changes.push(Change::Bold(after.bold));
    }
    if after.italic != before.italic {
        changes.push(Change::Italic(after.italic));
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::*;
    use grind_text::markdown::Emphasis;

    #[test]
    fn an_emphasis_toggles_over_what_the_selection_agrees_about() {
        let plain = CharStyle::default();
        let bold = Command::Mark(Emphasis::Bold);
        assert_eq!(change(bold, &plain), Some(Change::Bold(true)));
        let mut style = plain.clone();
        Change::Bold(true).apply(&mut style);
        assert_eq!(change(bold, &style), Some(Change::Bold(false)));
        assert!(checked(bold, &style, &BlockKind::Paragraph, None));
        assert_eq!(
            change(Command::Mark(Emphasis::Code), &plain),
            Some(Change::Code(true))
        );
    }

    #[test]
    fn a_colour_is_a_highlight_on_the_page() {
        assert_eq!(
            change(Command::Background(Some(9)), &CharStyle::default()),
            Some(Change::Highlight(Some("#ffdc00".into())))
        );
        assert_eq!(
            change(Command::TextColor(None), &CharStyle::default()),
            Some(Change::Color(None))
        );
        assert!(checked(
            Command::TextColor(None),
            &CharStyle::default(),
            &BlockKind::Paragraph,
            None
        ));
    }

    #[test]
    fn a_block_command_is_a_kind_and_is_ticked_on_its_own_kind() {
        let plain = CharStyle::default();
        let h2 = Command::Block(Block::Heading(2));
        let heading = BlockKind::Heading { level: 2 };
        assert_eq!(
            block(h2, &BlockKind::Paragraph, None),
            Some((heading.clone(), None))
        );
        assert!(checked(h2, &plain, &heading, None));
        assert!(!checked(h2, &plain, &BlockKind::Paragraph, None));
        let deep = BlockKind::ListItem { depth: 3 };
        assert_eq!(
            block(Command::Block(Block::ListItem), &deep, None),
            Some((deep.clone(), None))
        );
        assert_eq!(block(Command::Wrap, &deep, None), None);
        assert_eq!(change(Command::Wrap, &plain), None);
    }

    #[test]
    fn a_title_is_a_paragraph_wearing_its_name_and_only_ours_are_taken_off() {
        let plain = CharStyle::default();
        let title = Command::Block(Block::Title);
        let body = Command::Block(Block::Body);
        assert_eq!(
            block(title, &BlockKind::Heading { level: 1 }, None),
            Some((BlockKind::Paragraph, Some("Title".into())))
        );
        assert!(checked(title, &plain, &BlockKind::Paragraph, Some("Title")));
        assert!(!checked(body, &plain, &BlockKind::Paragraph, Some("Title")));
        assert_eq!(
            block(body, &BlockKind::Paragraph, Some("Title")),
            Some((BlockKind::Paragraph, None)),
            "our name, taken off"
        );
        assert_eq!(
            block(body, &BlockKind::Heading { level: 1 }, Some("Quotations")),
            Some((BlockKind::Paragraph, Some("Quotations".into()))),
            "the document's own name, left"
        );
        assert!(checked(
            body,
            &plain,
            &BlockKind::Paragraph,
            Some("Quotations")
        ));
    }

    #[test]
    fn the_font_panel_changes_only_what_it_changed() {
        let before = Facts {
            family: "Helvetica".into(),
            size: 12.0,
            bold: false,
            italic: false,
        };
        assert_eq!(font_changes(&before, &before), []);
        let bigger = Facts {
            size: 14.0,
            ..before.clone()
        };
        assert_eq!(
            font_changes(&before, &bigger),
            [Change::Size(Some("14pt".into()))]
        );
        let georgia = Facts {
            family: "Georgia".into(),
            bold: true,
            ..before.clone()
        };
        assert_eq!(
            font_changes(&before, &georgia),
            [Change::Family(Some("Georgia".into())), Change::Bold(true)]
        );
        assert_eq!(points(10.5), "10.5pt");
        assert_eq!(points(11.999), "12pt");
    }
}
