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

/// The kind `command` makes a block that is `current` — a list item keeps its depth when it is
/// asked to be a list item again.
pub fn kind(command: Command, current: &BlockKind) -> Option<BlockKind> {
    let Command::Block(block) = command else {
        return None;
    };
    Some(match block {
        Block::Body => BlockKind::Paragraph,
        Block::Heading(level) => BlockKind::Heading {
            level: u32::from(level),
        },
        Block::ListItem => match current {
            BlockKind::ListItem { depth } => BlockKind::ListItem { depth: *depth },
            _ => BlockKind::ListItem { depth: 1 },
        },
    })
}

/// Whether the command's menu item is ticked: an emphasis every selected character has, the
/// colour they agree on, the kind the caret's block is.
pub fn checked(command: Command, here: &CharStyle, block: &BlockKind) -> bool {
    match command {
        Command::Mark(emphasis) => has(here, emphasis),
        Command::TextColor(index) => here.color == color(index),
        Command::Background(index) => here.background == color(index),
        Command::Block(_) => kind(command, block).as_ref() == Some(block),
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
        assert!(checked(bold, &style, &BlockKind::Paragraph));
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
            &BlockKind::Paragraph
        ));
    }

    #[test]
    fn a_block_command_is_a_kind_and_is_ticked_on_its_own_kind() {
        let h2 = Command::Block(Block::Heading(2));
        assert_eq!(
            kind(h2, &BlockKind::Paragraph),
            Some(BlockKind::Heading { level: 2 })
        );
        assert!(checked(
            h2,
            &CharStyle::default(),
            &BlockKind::Heading { level: 2 }
        ));
        assert!(!checked(h2, &CharStyle::default(), &BlockKind::Paragraph));
        let deep = BlockKind::ListItem { depth: 3 };
        assert_eq!(
            kind(Command::Block(Block::ListItem), &deep),
            Some(deep.clone())
        );
        assert_eq!(kind(Command::Wrap, &deep), None);
        assert_eq!(change(Command::Wrap, &CharStyle::default()), None);
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
