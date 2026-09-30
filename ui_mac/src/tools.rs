// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The toolbar, as data (M7) — portable, and held to the menu bar by its tests.
//!
//! Decision 3 gives the toolbar one kind of control: **a property of the selection**, which is
//! the format bar's admission test, bounded by `CellStyle`, `numfmt::Format` and `CharStyle`.
//! And because a Mac user may hide the toolbar, **every item has a menu twin**: each segment and
//! each pop-up row *is* a [`Command`] that a Format menu item also sends, and a colour well's
//! twin is the colour submenu beside it. `toolbar.rs` builds the AppKit controls from this and
//! nothing else, so a control cannot be added without its twin failing a test first.

use grind_core::DocumentKind;
use grind_sheet::format::Preset;
use grind_text::markdown::Emphasis;

use crate::menu::{Align, Block, Command};

/// One segment of a segmented control: an SF Symbol, a label for when the symbol is missing
/// or for VoiceOver, and the command it sends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Segment {
    pub symbol: &'static str,
    pub label: &'static str,
    pub command: Command,
}

/// One toolbar item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    /// Toggles side by side, each on or off by itself — the emphases, the alignments.
    Segments {
        id: &'static str,
        label: &'static str,
        segments: &'static [Segment],
        /// Whether a segment stays pressed while what it sends is true of the selection.
        toggles: bool,
    },
    /// A pop-up whose rows are commands, showing the one the selection already is.
    PopUp {
        id: &'static str,
        label: &'static str,
        rows: &'static [(&'static str, Command)],
    },
    /// A colour well opening the system colour panel: the text's colour, or the fill's.
    Well {
        id: &'static str,
        label: &'static str,
        background: bool,
    },
}

const fn segment(symbol: &'static str, label: &'static str, command: Command) -> Segment {
    Segment {
        symbol,
        label,
        command,
    }
}

const EMPHASIS: Tool = Tool::Segments {
    id: "emphasis",
    label: "Style",
    segments: &[
        segment("bold", "Bold", Command::Mark(Emphasis::Bold)),
        segment("italic", "Italic", Command::Mark(Emphasis::Italic)),
        segment("underline", "Underline", Command::Mark(Emphasis::Underline)),
        segment(
            "strikethrough",
            "Strikethrough",
            Command::Mark(Emphasis::Strike),
        ),
        segment(
            "chevron.left.forwardslash.chevron.right",
            "Code",
            Command::Mark(Emphasis::Code),
        ),
    ],
    toggles: true,
};

const WEIGHT: Tool = Tool::Segments {
    id: "weight",
    label: "Style",
    segments: &[
        segment("bold", "Bold", Command::Mark(Emphasis::Bold)),
        segment("italic", "Italic", Command::Mark(Emphasis::Italic)),
    ],
    toggles: true,
};

const ALIGN: Tool = Tool::Segments {
    id: "align",
    label: "Alignment",
    segments: &[
        segment("text.alignleft", "Align Left", Command::Align(Align::Left)),
        segment("text.aligncenter", "Center", Command::Align(Align::Center)),
        segment(
            "text.alignright",
            "Align Right",
            Command::Align(Align::Right),
        ),
    ],
    toggles: true,
};

const NUMBER: Tool = Tool::PopUp {
    id: "number",
    label: "Number",
    rows: &[
        ("General", Command::Number(Preset::General)),
        ("Number", Command::Number(Preset::Number)),
        ("Percent", Command::Number(Preset::Percent)),
        ("Currency", Command::Number(Preset::Currency)),
        ("Date", Command::Number(Preset::Date)),
        ("Date and Time", Command::Number(Preset::DateTime)),
        ("Time", Command::Number(Preset::Time)),
        ("Boolean", Command::Number(Preset::Boolean)),
        ("Text", Command::Number(Preset::Text)),
    ],
};

const DECIMALS: Tool = Tool::Segments {
    id: "decimals",
    label: "Decimals",
    segments: &[
        segment("minus", "Decrease Decimals", Command::Decimals(-1)),
        segment("plus", "Increase Decimals", Command::Decimals(1)),
    ],
    toggles: false,
};

const PARAGRAPH: Tool = Tool::PopUp {
    id: "paragraph",
    label: "Paragraph",
    rows: &[
        ("Body", Command::Block(Block::Body)),
        ("Heading 1", Command::Block(Block::Heading(1))),
        ("Heading 2", Command::Block(Block::Heading(2))),
        ("Heading 3", Command::Block(Block::Heading(3))),
        ("List Item", Command::Block(Block::ListItem)),
    ],
};

const TEXT_COLOR: Tool = Tool::Well {
    id: "text-color",
    label: "Text Color",
    background: false,
};

const BACKGROUND: Tool = Tool::Well {
    id: "background",
    label: "Background",
    background: true,
};

const CLEAR: Tool = Tool::Segments {
    id: "clear",
    label: "Clear",
    segments: &[segment(
        "eraser",
        "Clear Formatting",
        Command::ClearFormatting,
    )],
    toggles: false,
};

/// The spreadsheet's toolbar, left to right: the weight of the text, its alignment, its
/// colours, the number it shows, and Clear — `ui_win32`'s strip's five groups.
const SHEET: &[Tool] = &[
    WEIGHT, ALIGN, TEXT_COLOR, BACKGROUND, NUMBER, DECIMALS, CLEAR,
];

/// The page's: what a paragraph is, the five emphases, the two colours, and Clear.
const TEXT: &[Tool] = &[PARAGRAPH, EMPHASIS, TEXT_COLOR, BACKGROUND, CLEAR];

/// The toolbar a document of `kind` gets.
pub fn tools(kind: DocumentKind) -> &'static [Tool] {
    match kind {
        DocumentKind::Text => TEXT,
        _ => SHEET,
    }
}

impl Tool {
    pub fn id(&self) -> &'static str {
        match self {
            Tool::Segments { id, .. } | Tool::PopUp { id, .. } | Tool::Well { id, .. } => id,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Tool::Segments { label, .. } | Tool::PopUp { label, .. } | Tool::Well { label, .. } => {
                label
            }
        }
    }

    /// Every command the item sends.
    #[cfg(test)]
    pub fn commands(&self) -> Vec<Command> {
        match self {
            Tool::Segments { segments, .. } => segments.iter().map(|s| s.command).collect(),
            Tool::PopUp { rows, .. } => rows.iter().map(|(_, command)| *command).collect(),
            Tool::Well { .. } => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menu::{Action, Item, MENUS, every_item};
    use std::collections::HashSet;

    fn menu_commands() -> Vec<Command> {
        every_item(MENUS)
            .into_iter()
            .filter_map(|item| match item {
                Item::Entry {
                    action: Action::Command(command),
                    ..
                } => Some(*command),
                _ => None,
            })
            .collect()
    }

    /// Decision 3: a toolbar may be hidden, so nothing may be only on it.
    #[test]
    fn every_toolbar_item_has_a_menu_twin() {
        let menu = menu_commands();
        for kind in [DocumentKind::Spreadsheet, DocumentKind::Text] {
            for tool in tools(kind) {
                for command in tool.commands() {
                    assert!(menu.contains(&command), "{}: {command:?}", tool.id());
                    assert!(
                        command.applies(kind),
                        "{}: {command:?} on {kind:?}",
                        tool.id()
                    );
                }
                if let Tool::Well { background, .. } = tool {
                    let twin = match background {
                        true => Command::Background(Some(0)),
                        false => Command::TextColor(Some(0)),
                    };
                    assert!(menu.contains(&twin), "{}", tool.id());
                }
            }
        }
    }

    /// A pop-up's rows are called what their menu twins are called.
    #[test]
    fn a_rows_title_is_its_twins() {
        let titles: Vec<(&str, Command)> = every_item(MENUS)
            .into_iter()
            .filter_map(|item| match item {
                Item::Entry {
                    title,
                    action: Action::Command(command),
                    ..
                } => Some((*title, *command)),
                _ => None,
            })
            .collect();
        for tool in [NUMBER, PARAGRAPH] {
            let Tool::PopUp { rows, .. } = tool else {
                unreachable!()
            };
            for (row, command) in rows {
                assert!(titles.contains(&(*row, *command)), "{row}");
            }
        }
    }

    /// Within one toolbar no two items share an identifier, since the toolbar finds an item
    /// by it; across the two, the same item has the same one.
    #[test]
    fn ids_are_unique_within_a_toolbar() {
        for kind in [DocumentKind::Spreadsheet, DocumentKind::Text] {
            let mut seen = HashSet::new();
            for tool in tools(kind) {
                assert!(seen.insert(tool.id()), "{} twice", tool.id());
                assert!(!tool.label().is_empty());
            }
        }
        assert_eq!(TEXT_COLOR, TEXT[2]);
        assert_eq!(TEXT_COLOR, SHEET[2]);
    }

    /// Only properties of the selection: every toolbar command is a formatting one.
    #[test]
    fn nothing_on_the_toolbar_is_a_verb() {
        for tool in SHEET.iter().chain(TEXT) {
            for command in tool.commands() {
                assert!(
                    !matches!(
                        command,
                        Command::NewSheet
                            | Command::NewText
                            | Command::GoTo
                            | Command::AddSheet
                            | Command::RenameSheet
                            | Command::DeleteSheet
                    ),
                    "{command:?} is a verb, which belongs in a menu"
                );
            }
        }
    }
}
