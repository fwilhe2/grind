// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The menu bar, as data — `ui_win32/src/menu.rs`'s idea in the Mac's vocabulary.
//!
//! **Portable, and tested on any host.** The menu bar is this platform's surface that grows
//! (decision 3), so it is also where "follow the platform's conventions" would rot first in a
//! shell nobody here can run. `doc/macos-shell.md`'s *Conventions made mechanical* is therefore a
//! list of tests at the foot of this file: standard items use the standard selectors, no two items
//! share a key equivalent, nothing takes a shortcut the system reserves, the platform's own
//! shortcuts are the platform's, an ellipsis means the item asks for more, and the application,
//! Window and Help menus are the ones AppKit is told about.
//!
//! An item's action is one of two things. A **standard selector** goes down the responder chain,
//! where AppKit, `NSDocument` and `NSDocumentController` answer most of them with no code here —
//! Open, Save, Close, Revert, Duplicate, Undo, the window list, Hide and Quit — and where an item
//! nobody answers is greyed by the system. A [`Command`] is a verb of this shell's own, sent as one
//! selector with the command's index as the item's tag, so the dispatcher in `app.rs` matches on
//! [`Command`] exhaustively and a command with no handler fails the build.

use grind_core::DocumentKind;
use grind_core::style::PALETTE;
use grind_sheet::format::Preset;
use grind_text::markdown::Emphasis;

/// The application's name, as the menu bar, the Dock and About say it.
pub const APP_NAME: &str = "Grind";

/// The selector every [`Command`] item sends, with [`Command::tag`] as the item's tag.
pub const COMMAND_SELECTOR: &str = "performGrindCommand:";

/// A verb of this shell's own — one no standard selector already means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// A new, empty spreadsheet. Not `newDocument:`, which makes a document of the *first* type
    /// only: one application holding two kinds of document needs a New for each (decision 1).
    NewSheet,
    /// A new, empty text document.
    NewText,
    /// The keyboard into the name box, to type a place — `g20`, `Data.B2:C9`, a defined name —
    /// and go there (M3). ⌘L, the precedent `doc/macos-shell.md` names. On a page, a prompt for
    /// an address — `p12`, `#intro`, `§2.1` — since a page has no box of its own.
    GoTo,
    /// A sheet, after the last, under `App::fresh_sheet_name` (M4) — Insert ▸ Sheet, where Excel
    /// for Mac has it.
    AddSheet,
    /// The sheet showing, renamed, with every reference that named it following
    /// (`App::rename_sheet`) — Format ▸ Rename Sheet….
    RenameSheet,
    /// The sheet showing, deleted — one ⌘Z brings it back, so there is no confirmation to click
    /// through. Edit ▸ Delete Sheet.
    DeleteSheet,
    /// One of the five character emphases (M7): on over the selection, or off where every
    /// character already has it. Bold and Italic reach a cell as well; the other three are the
    /// page's, since a `CellStyle` has no underline, strike or family of its own to set.
    Mark(Emphasis),
    /// A cell's alignment — one field with three answers (`grind_sheet::format::Toggle`).
    Align(Align),
    /// `fo:wrap-option` on the selected cells.
    Wrap,
    /// A hairline round every selected cell (`true`), or no border at all (`false`).
    Borders(bool),
    /// One of the number picker's nine formats over the selected cells.
    Number(Preset),
    /// One decimal fewer or more (`grind_sheet::format::stepped`).
    Decimals(i8),
    /// The text's own colour — a [`PALETTE`] entry by index, or `None` for *Automatic*.
    TextColor(Option<u8>),
    /// A cell's fill, or a run's highlight — a [`PALETTE`] entry, or `None` for none.
    Background(Option<u8>),
    /// What the blocks the selection touches are — a paragraph, a heading, a list item.
    Block(Block),
    /// Every character or cell property off at once, and a cell's number format with them.
    ClearFormatting,
    /// The formula read-out and the signature band in plain English — `Present Value(Rate: …)`
    /// for `=PV(…)` — or in the spec's own spelling (M8). On by default, as in the GNOME and
    /// Windows windows; a reading, never written back.
    FriendlyFormulas,
    /// The active cell's formula unfolded, a call at a time, in plain English (M8).
    ExplainFormula,
    /// All 110 functions, with their plain-English names, and the chosen one's call written
    /// into the cell — Excel for Mac's Insert ▸ Function… (M8).
    InsertFunction,
    /// `doc/view-modes.md`'s role overlay: a marker at each cell's leading edge saying what the
    /// cell *is* — an input, a formula, a label, a number nobody named (M8).
    CellRoles,
    /// The name overlay: an outline round each defined name's range, or each bookmark's name
    /// beside its line on a page (M8).
    Names,
    /// The document as its projection, in a pane beside it (D9, M8) — ⌥⌘U, Safari's key for
    /// showing a page's source.
    ShowSource,
    /// Every formula recalculated now — Excel for Mac's Calculate Now, on its key, ⌘=.
    Recalculate,
    /// Fill Down (`true`, ⌘D) or Fill Right (⌘R): the top row or the left column of the
    /// selection copied over the rest, references shifted (`App::fill`).
    Fill(bool),
    /// Something done to the selection's rows.
    Rows(Track),
    /// Something done to the selection's columns.
    Columns(Track),
    /// A name for the selection — Insert ▸ Name….
    DefineName,
    /// The standard About panel, told the build's stamp (`grind_core::build_info`) — Grind ▸
    /// About Grind.
    About,
    /// A delimited file read into the sheet at the active cell — File ▸ Import CSV….
    ImportCsv,
    /// [`Command::ImportCsv`], having first asked for options in words — `delimiter=semicolon
    /// locale=de-DE text trim` (`csv::Import::amended`) — File ▸ Import CSV with Options….
    ImportCsvWith,
    /// The sheet showing, as comma-separated values — File ▸ Export as CSV….
    ExportCsv,
    /// A markdown file read into the page before the caret's block — File ▸ Import Markdown….
    ImportMarkdown,
    /// The page as CommonMark, the selected paragraphs or all of it — File ▸ Export as
    /// Markdown….
    ExportMarkdown,
    /// The page typeset and written as a PDF (`grind_print::export`, `doc/pdf-export.md`) —
    /// File ▸ Export as PDF….
    ExportPdf,
    /// The same PDF handed to the system's print panel through PDFKit, whose preview is then the
    /// PDF itself — File ▸ Print….
    Print,
    /// A formula worked out at the active cell without storing it — Edit ▸ Evaluate….
    Evaluate,
    /// The locale the document speaks in, asked for — Format ▸ Document Locale….
    DocumentLocale,
    /// A conditional-format rule over the selection — Format ▸ Conditional Formatting ▸ Add
    /// Rule…: the condition asked for, then the look, and `grind_sheet::rule::add_from_input`,
    /// the call every shell's rule editor makes (`doc/conditional-format.md` §4).
    AddRule,
    /// One of the sheet's rules chosen and taken away — Format ▸ Conditional Formatting ▸
    /// Remove Rule….
    RemoveRule,
    /// The active cell copied into every selected cell, references shifted — Edit ▸ Fill ▸
    /// Across Selection.
    FillAcross,
    /// The selection into one cell (`true`), or every merge in it taken away — Edit ▸ Merge
    /// Cells and Unmerge Cells, `App::merge`/`App::unmerge`.
    Merge(bool),
    /// Every formula in the document listed in the sidebar, searched — View ▸ Calculations….
    Calculations,
    /// Every formula shown in its cell rather than its value — View ▸ Formulas.
    ShowFormulas,
    /// The selection's shown values on the pasteboard, not their formulas — Edit ▸ Copy Value.
    CopyValue,
    /// Every formula in the selection dropped, its last value kept — Edit ▸ Formula to Value,
    /// LibreOffice's name for it.
    FormulaToValue,
    /// An autofilter over the selection, or off again — Edit ▸ Filter (`sheet/filter.rs`).
    Filter,
    /// A chart of the table the selection is in, beside it — Insert ▸ Chart.
    InsertChart,
    /// The chart Insert ▸ Chart would make, drawn in an alert with Insert and Cancel — nothing
    /// is written unless Insert is chosen (`verbs::preview_insert_chart`).
    PreviewChart,
    /// The selected paragraphs past their neighbour, up (`true`) or down — Format ▸ Paragraph ▸
    /// Move Up and Move Down.
    MoveParagraph(bool),
    /// The selected paragraphs gone — Format ▸ Paragraph ▸ Delete Paragraph.
    DeleteParagraph,
    /// A named paragraph style on the selected paragraphs, asked for — Format ▸ Paragraph ▸
    /// Style….
    ParagraphStyle,
    /// A table at the caret — Insert ▸ Table….
    InsertTable,
    /// A bookmark on the caret's block — Insert ▸ Bookmark….
    InsertBookmark,
    /// A picture from a file, at the caret — Insert ▸ Picture….
    InsertPicture,
    /// Zoom In (1), Zoom Out (-1) or Actual Size (0) — `zoom.rs`.
    Zoom(i8),
    /// The welcome window back (decision 6, M9) — Window ▸ Welcome to Grind, ⇧⌘1, where Xcode
    /// keeps its own.
    Welcome,
}

/// A cell alignment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// What Format ▸ Row or Column does to the selection's tracks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Track {
    /// Asks for a height or a width.
    Size,
    Hide,
    Show,
    /// A column as wide as its widest text, a row given back to its content — what a
    /// double-click on a header edge does, from the menu.
    Fit,
}

/// A block kind Format ▸ Paragraph offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Block {
    /// A paragraph wearing LibreOffice's `Title` or `Subtitle` style — the two names every page
    /// in the suite draws in a face of its own (`grind_text::look`).
    Title,
    Subtitle,
    Body,
    /// Levels 1 to 6, ODF's outline levels that every page gives a face of its own.
    Heading(u8),
    ListItem,
}

/// Every block kind Format ▸ Paragraph sets, in its order.
pub const BLOCKS: [Block; 10] = [
    Block::Title,
    Block::Subtitle,
    Block::Body,
    Block::Heading(1),
    Block::Heading(2),
    Block::Heading(3),
    Block::Heading(4),
    Block::Heading(5),
    Block::Heading(6),
    Block::ListItem,
];

/// The five emphases, in the order the menu and the toolbar give them.
pub const EMPHASES: [Emphasis; 5] = [
    Emphasis::Bold,
    Emphasis::Italic,
    Emphasis::Underline,
    Emphasis::Strike,
    Emphasis::Code,
];

impl Command {
    /// Every command, in the order the menus give them — tags are positions in this.
    pub fn all() -> Vec<Command> {
        let mut all = vec![
            Command::NewSheet,
            Command::NewText,
            Command::GoTo,
            Command::AddSheet,
            Command::RenameSheet,
            Command::DeleteSheet,
        ];
        all.extend(EMPHASES.map(Command::Mark));
        all.extend([Align::Left, Align::Center, Align::Right].map(Command::Align));
        all.push(Command::Wrap);
        all.extend([Command::Borders(true), Command::Borders(false)]);
        all.extend(Preset::ALL.map(Command::Number));
        all.extend([Command::Decimals(-1), Command::Decimals(1)]);
        let palette = || std::iter::once(None).chain((0..PALETTE.len() as u8).map(Some));
        all.extend(palette().map(Command::TextColor));
        all.extend(palette().map(Command::Background));
        all.extend(BLOCKS.map(Command::Block));
        all.push(Command::ClearFormatting);
        all.extend([
            Command::FriendlyFormulas,
            Command::ExplainFormula,
            Command::InsertFunction,
            Command::CellRoles,
            Command::Names,
            Command::ShowSource,
            Command::Welcome,
            Command::Recalculate,
            Command::Fill(true),
            Command::Fill(false),
        ]);
        for track in [Track::Size, Track::Hide, Track::Show, Track::Fit] {
            all.extend([Command::Rows(track), Command::Columns(track)]);
        }
        all.extend([
            Command::DefineName,
            Command::ExportCsv,
            Command::ImportCsv,
            Command::ImportCsvWith,
            Command::ImportMarkdown,
            Command::ExportMarkdown,
            Command::ExportPdf,
            Command::Print,
            Command::About,
            Command::InsertChart,
            Command::PreviewChart,
            Command::Filter,
            Command::CopyValue,
            Command::FormulaToValue,
            Command::Evaluate,
            Command::DocumentLocale,
            Command::AddRule,
            Command::RemoveRule,
            Command::ShowFormulas,
            Command::FillAcross,
            Command::Merge(true),
            Command::Merge(false),
            Command::Calculations,
            Command::InsertTable,
            Command::InsertBookmark,
            Command::InsertPicture,
            Command::MoveParagraph(true),
            Command::MoveParagraph(false),
            Command::DeleteParagraph,
            Command::ParagraphStyle,
            Command::Zoom(1),
            Command::Zoom(-1),
            Command::Zoom(0),
        ]);
        all
    }

    /// Whether the command asks for more before it acts — which is what an ellipsis in its title
    /// promises, and what the tests hold the titles to.
    #[cfg(test)]
    pub fn asks(self) -> bool {
        matches!(
            self,
            Command::GoTo
                | Command::RenameSheet
                | Command::InsertFunction
                | Command::Rows(Track::Size)
                | Command::Columns(Track::Size)
                | Command::DefineName
                | Command::ExportCsv
                | Command::ImportCsv
                | Command::ImportCsvWith
                | Command::ImportMarkdown
                | Command::ExportMarkdown
                | Command::ExportPdf
                | Command::Print
                | Command::InsertTable
                | Command::InsertBookmark
                | Command::InsertPicture
                | Command::PreviewChart
                | Command::Evaluate
                | Command::DocumentLocale
                | Command::AddRule
                | Command::RemoveRule
                | Command::Calculations
                | Command::ParagraphStyle
        )
    }

    /// The item's tag: the command's place in [`Command::all`], so no two share one.
    pub fn tag(self) -> isize {
        Command::all()
            .iter()
            .position(|command| *command == self)
            .expect("every command is in all()") as isize
    }

    /// The command an item's tag names.
    pub fn from_tag(tag: isize) -> Option<Command> {
        usize::try_from(tag)
            .ok()
            .and_then(|at| Command::all().get(at).copied())
    }

    /// Whether the command means anything in a document of `kind` — what greys it otherwise.
    /// The two News mean something everywhere, and every other verb belongs to one pane or,
    /// for Bold, Italic, the colours and Clear, to both.
    pub fn applies(self, kind: DocumentKind) -> bool {
        let sheet = kind == DocumentKind::Spreadsheet;
        let text = kind == DocumentKind::Text;
        match self {
            Command::NewSheet | Command::NewText | Command::Welcome | Command::About => true,
            Command::Names | Command::ShowSource | Command::GoTo | Command::Zoom(_) => {
                sheet || text
            }
            Command::Mark(Emphasis::Bold | Emphasis::Italic)
            | Command::TextColor(_)
            | Command::Background(_)
            | Command::ClearFormatting => sheet || text,
            Command::Mark(_)
            | Command::Block(_)
            | Command::InsertTable
            | Command::InsertBookmark
            | Command::InsertPicture
            | Command::ImportMarkdown
            | Command::ExportMarkdown
            | Command::ExportPdf
            | Command::Print
            | Command::MoveParagraph(_)
            | Command::DeleteParagraph
            | Command::ParagraphStyle => text,
            Command::AddSheet
            | Command::RenameSheet
            | Command::DeleteSheet
            | Command::Align(_)
            | Command::Wrap
            | Command::Borders(_)
            | Command::Number(_)
            | Command::Decimals(_)
            | Command::FriendlyFormulas
            | Command::ExplainFormula
            | Command::InsertFunction
            | Command::CellRoles
            | Command::Recalculate
            | Command::Fill(_)
            | Command::Rows(_)
            | Command::Columns(_)
            | Command::DefineName
            | Command::InsertChart
            | Command::PreviewChart
            | Command::Filter
            | Command::CopyValue
            | Command::FormulaToValue
            | Command::Evaluate
            | Command::DocumentLocale
            | Command::AddRule
            | Command::RemoveRule
            | Command::ShowFormulas
            | Command::FillAcross
            | Command::Merge(_)
            | Command::Calculations
            | Command::ExportCsv
            | Command::ImportCsvWith
            | Command::ImportCsv => sheet,
        }
    }
}

/// What choosing an item does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// A standard selector, sent to the first responder that answers it.
    Standard(&'static str),
    /// A standard selector whose sender's tag says which of its jobs it is — Find's
    /// `performFindPanelAction:`, whose tags are `NSFindPanelAction`'s.
    Tagged(&'static str, isize),
    Command(Command),
}

/// `NSFindPanelAction`'s values, which Edit ▸ Find's items carry as their tags — what makes
/// ⌘F, ⌘G, ⇧⌘G and ⌘E the platform's own Find in any view that answers
/// `performFindPanelAction:`.
pub mod find {
    pub const SHOW: isize = 1;
    pub const NEXT: isize = 2;
    pub const PREVIOUS: isize = 3;
    pub const USE_SELECTION: isize = 7;
}

/// The modifier keys of a key equivalent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Mods {
    pub command: bool,
    pub shift: bool,
    pub option: bool,
    pub control: bool,
}

/// ⌘ alone, which is most of them.
pub const CMD: Mods = Mods {
    command: true,
    shift: false,
    option: false,
    control: false,
};
pub const SHIFT_CMD: Mods = Mods { shift: true, ..CMD };
pub const OPT_CMD: Mods = Mods {
    option: true,
    ..CMD
};
pub const CTRL_CMD: Mods = Mods {
    control: true,
    ..CMD
};

/// A key equivalent: the key, as `NSMenuItem.keyEquivalent` takes it (lower case — Shift is a
/// modifier here, never an upper-case letter), and its modifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Key {
    pub key: &'static str,
    pub mods: Mods,
}

impl Key {
    // Tested now, and a notice's from M4 (every notice spells a key as ⌘, never Ctrl).
    #[cfg(test)]
    /// How the menu draws it — `⇧⌘S` — which is also how a notice spells a key (⌘, never Ctrl).
    pub fn spelled(&self) -> String {
        let mut out = String::new();
        for (on, glyph) in [
            (self.mods.control, '⌃'),
            (self.mods.option, '⌥'),
            (self.mods.shift, '⇧'),
            (self.mods.command, '⌘'),
        ] {
            if on {
                out.push(glyph);
            }
        }
        out.push_str(&self.key.to_uppercase());
        out
    }
}

const fn key(key: &'static str, mods: Mods) -> Option<Key> {
    Some(Key { key, mods })
}

/// One row of a menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    Entry {
        title: &'static str,
        key: Option<Key>,
        action: Action,
    },
    Separator,
    /// A submenu of items this table lists.
    Submenu {
        title: &'static str,
        menu: &'static Menu,
    },
}

/// Which of AppKit's own menus a menu is, when it is one.
///
/// Telling AppKit is what makes them work: the Window menu gets the window list, the Help menu
/// gets Help ▸ Search over every item, Services fills itself, and `NSDocumentController` finds
/// Open Recent by its `clearRecentDocuments:` item and keeps it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Application,
    Window,
    Help,
    Services,
    OpenRecent,
    Plain,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Menu {
    pub title: &'static str,
    pub role: Role,
    pub items: &'static [Item],
}

const fn standard(title: &'static str, key: Option<Key>, selector: &'static str) -> Item {
    Item::Entry {
        title,
        key,
        action: Action::Standard(selector),
    }
}

const fn tagged(title: &'static str, key: Option<Key>, selector: &'static str, tag: isize) -> Item {
    Item::Entry {
        title,
        key,
        action: Action::Tagged(selector, tag),
    }
}

const fn command(title: &'static str, key: Option<Key>, command: Command) -> Item {
    Item::Entry {
        title,
        key,
        action: Action::Command(command),
    }
}

static SERVICES: Menu = Menu {
    title: "Services",
    role: Role::Services,
    items: &[],
};

static OPEN_RECENT: Menu = Menu {
    title: "Open Recent",
    role: Role::OpenRecent,
    items: &[standard("Clear Menu", None, "clearRecentDocuments:")],
};

static FIND: Menu = Menu {
    title: "Find",
    role: Role::Plain,
    items: &[
        tagged(
            "Find…",
            key("f", CMD),
            "performFindPanelAction:",
            find::SHOW,
        ),
        tagged(
            "Find Next",
            key("g", CMD),
            "performFindPanelAction:",
            find::NEXT,
        ),
        tagged(
            "Find Previous",
            key("g", SHIFT_CMD),
            "performFindPanelAction:",
            find::PREVIOUS,
        ),
        tagged(
            "Use Selection for Find",
            key("e", CMD),
            "performFindPanelAction:",
            find::USE_SELECTION,
        ),
    ],
};

static REVERT_TO: Menu = Menu {
    title: "Revert To",
    role: Role::Plain,
    items: &[
        standard("Last Saved Version", None, "revertDocumentToSaved:"),
        standard("Browse All Versions…", None, "browseDocumentVersions:"),
    ],
};

static FONT: Menu = Menu {
    title: "Font",
    role: Role::Plain,
    items: &[
        standard("Show Fonts", key("t", CMD), "orderFrontFontPanel:"),
        command("Bold", key("b", CMD), Command::Mark(Emphasis::Bold)),
        command("Italic", key("i", CMD), Command::Mark(Emphasis::Italic)),
        command(
            "Underline",
            key("u", CMD),
            Command::Mark(Emphasis::Underline),
        ),
        command(
            "Strikethrough",
            key("x", SHIFT_CMD),
            Command::Mark(Emphasis::Strike),
        ),
        command("Code", None, Command::Mark(Emphasis::Code)),
        Item::Separator,
        standard("Show Colors", key("c", SHIFT_CMD), "orderFrontColorPanel:"),
    ],
};

static TEXT: Menu = Menu {
    title: "Text",
    role: Role::Plain,
    items: &[
        command("Align Left", key("{", CMD), Command::Align(Align::Left)),
        command("Center", key("|", CMD), Command::Align(Align::Center)),
        command("Align Right", key("}", CMD), Command::Align(Align::Right)),
        Item::Separator,
        command("Wrap Text", None, Command::Wrap),
    ],
};

static BORDERS: Menu = Menu {
    title: "Borders",
    role: Role::Plain,
    items: &[
        command("All Borders", None, Command::Borders(true)),
        command("No Borders", None, Command::Borders(false)),
    ],
};

static TEXT_COLOR: Menu = Menu {
    title: "Text Color",
    role: Role::Plain,
    items: &[
        command("Automatic", None, Command::TextColor(None)),
        Item::Separator,
        command("Navy", None, Command::TextColor(Some(0))),
        command("Blue", None, Command::TextColor(Some(1))),
        command("Aqua", None, Command::TextColor(Some(2))),
        command("Teal", None, Command::TextColor(Some(3))),
        command("Purple", None, Command::TextColor(Some(4))),
        command("Fuchsia", None, Command::TextColor(Some(5))),
        command("Maroon", None, Command::TextColor(Some(6))),
        command("Red", None, Command::TextColor(Some(7))),
        command("Orange", None, Command::TextColor(Some(8))),
        command("Yellow", None, Command::TextColor(Some(9))),
        command("Olive", None, Command::TextColor(Some(10))),
        command("Green", None, Command::TextColor(Some(11))),
        command("Lime", None, Command::TextColor(Some(12))),
        command("Black", None, Command::TextColor(Some(13))),
        command("Gray", None, Command::TextColor(Some(14))),
        command("Silver", None, Command::TextColor(Some(15))),
        command("White", None, Command::TextColor(Some(16))),
    ],
};

static BACKGROUND: Menu = Menu {
    title: "Background Color",
    role: Role::Plain,
    items: &[
        command("None", None, Command::Background(None)),
        Item::Separator,
        command("Navy", None, Command::Background(Some(0))),
        command("Blue", None, Command::Background(Some(1))),
        command("Aqua", None, Command::Background(Some(2))),
        command("Teal", None, Command::Background(Some(3))),
        command("Purple", None, Command::Background(Some(4))),
        command("Fuchsia", None, Command::Background(Some(5))),
        command("Maroon", None, Command::Background(Some(6))),
        command("Red", None, Command::Background(Some(7))),
        command("Orange", None, Command::Background(Some(8))),
        command("Yellow", None, Command::Background(Some(9))),
        command("Olive", None, Command::Background(Some(10))),
        command("Green", None, Command::Background(Some(11))),
        command("Lime", None, Command::Background(Some(12))),
        command("Black", None, Command::Background(Some(13))),
        command("Gray", None, Command::Background(Some(14))),
        command("Silver", None, Command::Background(Some(15))),
        command("White", None, Command::Background(Some(16))),
    ],
};

static NUMBER: Menu = Menu {
    title: "Number",
    role: Role::Plain,
    items: &[
        command("General", None, Command::Number(Preset::General)),
        command("Number", None, Command::Number(Preset::Number)),
        command("Percent", None, Command::Number(Preset::Percent)),
        command("Currency", None, Command::Number(Preset::Currency)),
        command("Date", None, Command::Number(Preset::Date)),
        command("Date and Time", None, Command::Number(Preset::DateTime)),
        command("Time", None, Command::Number(Preset::Time)),
        command("Boolean", None, Command::Number(Preset::Boolean)),
        command("Text", None, Command::Number(Preset::Text)),
        Item::Separator,
        command("Increase Decimals", None, Command::Decimals(1)),
        command("Decrease Decimals", None, Command::Decimals(-1)),
        Item::Separator,
        command("Document Locale…", None, Command::DocumentLocale),
    ],
};

static CONDITIONAL: Menu = Menu {
    title: "Conditional Formatting",
    role: Role::Plain,
    items: &[
        command("Add Rule…", None, Command::AddRule),
        command("Remove Rule…", None, Command::RemoveRule),
    ],
};

static FILL: Menu = Menu {
    title: "Fill",
    role: Role::Plain,
    items: &[
        command("Down", key("d", CMD), Command::Fill(true)),
        command("Right", key("r", CMD), Command::Fill(false)),
        command("Across Selection", None, Command::FillAcross),
    ],
};

static ROW: Menu = Menu {
    title: "Row",
    role: Role::Plain,
    items: &[
        command("Height…", None, Command::Rows(Track::Size)),
        command("Fit Height to Content", None, Command::Rows(Track::Fit)),
        command("Hide", None, Command::Rows(Track::Hide)),
        command("Show", None, Command::Rows(Track::Show)),
    ],
};

static COLUMN: Menu = Menu {
    title: "Column",
    role: Role::Plain,
    items: &[
        command("Width…", None, Command::Columns(Track::Size)),
        command("Fit Width to Content", None, Command::Columns(Track::Fit)),
        command("Hide", None, Command::Columns(Track::Hide)),
        command("Show", None, Command::Columns(Track::Show)),
    ],
};

static PARAGRAPH: Menu = Menu {
    title: "Paragraph",
    role: Role::Plain,
    items: &[
        command("Title", None, Command::Block(Block::Title)),
        command("Subtitle", None, Command::Block(Block::Subtitle)),
        Item::Separator,
        command("Body", key("0", OPT_CMD), Command::Block(Block::Body)),
        command(
            "Heading 1",
            key("1", OPT_CMD),
            Command::Block(Block::Heading(1)),
        ),
        command(
            "Heading 2",
            key("2", OPT_CMD),
            Command::Block(Block::Heading(2)),
        ),
        command(
            "Heading 3",
            key("3", OPT_CMD),
            Command::Block(Block::Heading(3)),
        ),
        command(
            "Heading 4",
            key("4", OPT_CMD),
            Command::Block(Block::Heading(4)),
        ),
        command(
            "Heading 5",
            key("5", OPT_CMD),
            Command::Block(Block::Heading(5)),
        ),
        command(
            "Heading 6",
            key("6", OPT_CMD),
            Command::Block(Block::Heading(6)),
        ),
        Item::Separator,
        command("List Item", None, Command::Block(Block::ListItem)),
        Item::Separator,
        command("Move Up", None, Command::MoveParagraph(true)),
        command("Move Down", None, Command::MoveParagraph(false)),
        command("Delete Paragraph", None, Command::DeleteParagraph),
        Item::Separator,
        command("Style…", None, Command::ParagraphStyle),
    ],
};

/// The menu bar, left to right.
pub static MENUS: &[Menu] = &[
    Menu {
        title: APP_NAME,
        role: Role::Application,
        items: &[
            command("About Grind", None, Command::About),
            Item::Separator,
            Item::Submenu {
                title: "Services",
                menu: &SERVICES,
            },
            Item::Separator,
            standard("Hide Grind", key("h", CMD), "hide:"),
            standard("Hide Others", key("h", OPT_CMD), "hideOtherApplications:"),
            standard("Show All", None, "unhideAllApplications:"),
            Item::Separator,
            standard("Quit Grind", key("q", CMD), "terminate:"),
        ],
    },
    Menu {
        title: "File",
        role: Role::Plain,
        items: &[
            command("New Spreadsheet", key("n", CMD), Command::NewSheet),
            command("New Text Document", key("n", OPT_CMD), Command::NewText),
            standard("Open…", key("o", CMD), "openDocument:"),
            Item::Submenu {
                title: "Open Recent",
                menu: &OPEN_RECENT,
            },
            Item::Separator,
            standard("Close", key("w", CMD), "performClose:"),
            standard("Save…", key("s", CMD), "saveDocument:"),
            standard("Duplicate", key("s", SHIFT_CMD), "duplicateDocument:"),
            standard("Rename…", None, "renameDocument:"),
            standard("Move To…", None, "moveDocument:"),
            command("Import CSV…", None, Command::ImportCsv),
            command("Import CSV with Options…", None, Command::ImportCsvWith),
            command("Export as CSV…", None, Command::ExportCsv),
            command("Import Markdown…", None, Command::ImportMarkdown),
            command("Export as Markdown…", None, Command::ExportMarkdown),
            command("Export as PDF…", None, Command::ExportPdf),
            Item::Submenu {
                title: "Revert To",
                menu: &REVERT_TO,
            },
            Item::Separator,
            // Where every Mac application keeps it, at the key every one uses.
            command("Print…", key("p", CMD), Command::Print),
        ],
    },
    Menu {
        title: "Edit",
        role: Role::Plain,
        items: &[
            standard("Undo", key("z", CMD), "undo:"),
            standard("Redo", key("z", SHIFT_CMD), "redo:"),
            Item::Separator,
            standard("Cut", key("x", CMD), "cut:"),
            standard("Copy", key("c", CMD), "copy:"),
            command("Copy Value", None, Command::CopyValue),
            standard("Paste", key("v", CMD), "paste:"),
            standard("Delete", None, "delete:"),
            standard("Select All", key("a", CMD), "selectAll:"),
            Item::Separator,
            Item::Submenu {
                title: "Find",
                menu: &FIND,
            },
            command("Go To…", key("l", CMD), Command::GoTo),
            command("Recalculate", key("=", CMD), Command::Recalculate),
            command("Filter", key("f", SHIFT_CMD), Command::Filter),
            command("Formula to Value", None, Command::FormulaToValue),
            command("Evaluate…", None, Command::Evaluate),
            Item::Submenu {
                title: "Fill",
                menu: &FILL,
            },
            command("Merge Cells", None, Command::Merge(true)),
            command("Unmerge Cells", None, Command::Merge(false)),
            Item::Separator,
            command("Delete Sheet", None, Command::DeleteSheet),
        ],
    },
    Menu {
        title: "Insert",
        role: Role::Plain,
        items: &[
            command("Sheet", None, Command::AddSheet),
            command("Function…", None, Command::InsertFunction),
            command("Name…", None, Command::DefineName),
            command("Chart", None, Command::InsertChart),
            command("Chart Preview…", None, Command::PreviewChart),
            Item::Separator,
            command("Table…", None, Command::InsertTable),
            command("Bookmark…", None, Command::InsertBookmark),
            command("Picture…", None, Command::InsertPicture),
        ],
    },
    Menu {
        title: "Format",
        role: Role::Plain,
        items: &[
            Item::Submenu {
                title: "Font",
                menu: &FONT,
            },
            Item::Submenu {
                title: "Text",
                menu: &TEXT,
            },
            Item::Submenu {
                title: "Text Color",
                menu: &TEXT_COLOR,
            },
            Item::Submenu {
                title: "Borders",
                menu: &BORDERS,
            },
            Item::Submenu {
                title: "Background Color",
                menu: &BACKGROUND,
            },
            Item::Submenu {
                title: "Number",
                menu: &NUMBER,
            },
            Item::Submenu {
                title: "Conditional Formatting",
                menu: &CONDITIONAL,
            },
            Item::Submenu {
                title: "Paragraph",
                menu: &PARAGRAPH,
            },
            Item::Submenu {
                title: "Row",
                menu: &ROW,
            },
            Item::Submenu {
                title: "Column",
                menu: &COLUMN,
            },
            Item::Separator,
            command("Clear Formatting", None, Command::ClearFormatting),
            Item::Separator,
            command("Rename Sheet…", None, Command::RenameSheet),
        ],
    },
    Menu {
        title: "View",
        role: Role::Plain,
        items: &[
            standard("Show Toolbar", None, "toggleToolbarShown:"),
            standard("Show Sidebar", key("s", CTRL_CMD), "toggleSidebar:"),
            Item::Separator,
            command("Friendly Formulas", None, Command::FriendlyFormulas),
            command("Explain Formula", None, Command::ExplainFormula),
            Item::Separator,
            command("Formulas", key("`", CTRL_CMD), Command::ShowFormulas),
            command("Calculations…", None, Command::Calculations),
            command("Cell Roles", None, Command::CellRoles),
            command("Names", None, Command::Names),
            command("Show Source", key("u", OPT_CMD), Command::ShowSource),
            Item::Separator,
            command("Zoom In", key("+", CMD), Command::Zoom(1)),
            command("Zoom Out", key("-", CMD), Command::Zoom(-1)),
            command("Actual Size", key("0", CMD), Command::Zoom(0)),
            Item::Separator,
            standard("Enter Full Screen", key("f", CTRL_CMD), "toggleFullScreen:"),
        ],
    },
    Menu {
        title: "Window",
        role: Role::Window,
        items: &[
            standard("Minimize", key("m", CMD), "performMiniaturize:"),
            standard("Zoom", None, "performZoom:"),
            Item::Separator,
            standard("Bring All to Front", None, "arrangeInFront:"),
            Item::Separator,
            command("Welcome to Grind", key("1", SHIFT_CMD), Command::Welcome),
        ],
    },
    Menu {
        title: "Help",
        role: Role::Help,
        items: &[standard("Grind Help", key("?", CMD), "showHelp:")],
    },
];

/// The grid's context menu (M9): what a right-click on the cells offers — the clipboard, the
/// two toggles a cell has, the colours, and Clear. Every row is a menu-bar item with the same
/// title and action, which a test holds, so the two can never disagree about what a verb is
/// called.
pub static GRID_CONTEXT: Menu = Menu {
    title: "Cells",
    role: Role::Plain,
    items: &[
        standard("Cut", key("x", CMD), "cut:"),
        standard("Copy", key("c", CMD), "copy:"),
        standard("Paste", key("v", CMD), "paste:"),
        standard("Delete", None, "delete:"),
        Item::Separator,
        command("Bold", key("b", CMD), Command::Mark(Emphasis::Bold)),
        command("Italic", key("i", CMD), Command::Mark(Emphasis::Italic)),
        Item::Submenu {
            title: "Background Color",
            menu: &BACKGROUND,
        },
        command("Clear Formatting", None, Command::ClearFormatting),
        Item::Separator,
        command("Function…", None, Command::InsertFunction),
    ],
};

/// The page's context menu: the clipboard, the three emphases a person reaches for, what the
/// paragraph is, and Clear.
pub static PAGE_CONTEXT: Menu = Menu {
    title: "Text",
    role: Role::Plain,
    items: &[
        standard("Cut", key("x", CMD), "cut:"),
        standard("Copy", key("c", CMD), "copy:"),
        standard("Paste", key("v", CMD), "paste:"),
        Item::Separator,
        command("Bold", key("b", CMD), Command::Mark(Emphasis::Bold)),
        command("Italic", key("i", CMD), Command::Mark(Emphasis::Italic)),
        command(
            "Underline",
            key("u", CMD),
            Command::Mark(Emphasis::Underline),
        ),
        Item::Submenu {
            title: "Paragraph",
            menu: &PARAGRAPH,
        },
        command("Clear Formatting", None, Command::ClearFormatting),
    ],
};

/// Every item in `menus` and their submenus, depth first.
#[cfg(test)]
pub fn every_item(menus: &'static [Menu]) -> Vec<&'static Item> {
    fn walk(menu: &'static Menu, out: &mut Vec<&'static Item>) {
        for item in menu.items {
            out.push(item);
            if let Item::Submenu { menu, .. } = item {
                walk(menu, out);
            }
        }
    }
    let mut out = Vec::new();
    for menu in menus {
        walk(menu, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// The AppKit selectors this table may name — the responder chain's own vocabulary, spelled
    /// as `NSResponder`, `NSApplication`, `NSWindow`, `NSDocument` and `NSDocumentController`
    /// declare them. A selector outside this list is a typo until proved otherwise: a misspelt one
    /// is a grey item and nothing else, which is why it is a test.
    const KNOWN: &[&str] = &[
        "orderFrontStandardAboutPanel:",
        "hide:",
        "hideOtherApplications:",
        "unhideAllApplications:",
        "terminate:",
        "openDocument:",
        "clearRecentDocuments:",
        "performClose:",
        "saveDocument:",
        "saveDocumentAs:",
        "duplicateDocument:",
        "renameDocument:",
        "moveDocument:",
        "revertDocumentToSaved:",
        "browseDocumentVersions:",
        "undo:",
        "redo:",
        "cut:",
        "copy:",
        "paste:",
        "delete:",
        "selectAll:",
        "performFindPanelAction:",
        "orderFrontFontPanel:",
        "orderFrontColorPanel:",
        "toggleToolbarShown:",
        "runToolbarCustomizationPalette:",
        "toggleSidebar:",
        "toggleFullScreen:",
        "performMiniaturize:",
        "performZoom:",
        "arrangeInFront:",
        "showHelp:",
    ];

    /// The shortcuts the system keeps for itself, and which standard selector may carry each —
    /// `None` when no application item may take it at all.
    fn reserved() -> Vec<(Key, Option<&'static str>)> {
        let k = |key, mods| Key { key, mods };
        vec![
            (k(" ", CMD), None),
            (k("\t", CMD), None),
            (k("`", CMD), None),
            (k("h", CMD), Some("hide:")),
            (k("h", OPT_CMD), Some("hideOtherApplications:")),
            (k("m", CMD), Some("performMiniaturize:")),
            (k("q", CMD), Some("terminate:")),
            (k("q", CTRL_CMD), None),
            (k("\u{1b}", OPT_CMD), None),
            (k("3", SHIFT_CMD), None),
            (k("4", SHIFT_CMD), None),
            (k("5", SHIFT_CMD), None),
            (k(" ", CTRL_CMD), None),
            (k("f", CTRL_CMD), Some("toggleFullScreen:")),
        ]
    }

    fn entries() -> Vec<(&'static str, Option<Key>, Action)> {
        every_item(MENUS)
            .into_iter()
            .filter_map(|item| match item {
                Item::Entry { title, key, action } => Some((*title, *key, *action)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn every_standard_item_names_a_standard_selector() {
        for (title, _, action) in entries() {
            if let Action::Standard(selector) | Action::Tagged(selector, _) = action {
                assert!(KNOWN.contains(&selector), "{title}: {selector}");
                assert!(selector.ends_with(':'), "{title}: an action takes a sender");
            }
        }
    }

    #[test]
    fn every_command_is_in_exactly_one_menu() {
        let items = entries();
        for command in Command::all() {
            let count = items
                .iter()
                .filter(|(_, _, action)| *action == Action::Command(command))
                .count();
            assert_eq!(count, 1, "{command:?}");
            assert_eq!(Command::from_tag(command.tag()), Some(command));
        }
        assert_eq!(Command::from_tag(-1), None);
    }

    #[test]
    fn no_two_items_share_a_key_equivalent() {
        let mut seen: HashMap<Key, &str> = HashMap::new();
        for (title, key, _) in entries() {
            if let Some(key) = key {
                assert!(key.mods.command, "{title}: a menu shortcut uses ⌘");
                assert_eq!(
                    key.key,
                    key.key.to_lowercase(),
                    "{title}: Shift is a modifier"
                );
                if let Some(other) = seen.insert(key, title) {
                    panic!("{title} and {other} both take {}", key.spelled());
                }
            }
        }
    }

    #[test]
    fn nothing_takes_a_shortcut_the_system_reserves() {
        let reserved = reserved();
        for (title, key, action) in entries() {
            let Some(key) = key else { continue };
            if let Some((_, owner)) = reserved.iter().find(|(r, _)| *r == key) {
                assert_eq!(
                    Some(action),
                    owner.map(Action::Standard),
                    "{title} takes {}, which the system keeps",
                    key.spelled()
                );
            }
        }
    }

    /// The shortcuts that are the platform's are the platform's.
    #[test]
    fn the_platforms_shortcuts_mean_what_they_mean_everywhere() {
        let key_of = |action: Action| {
            entries()
                .into_iter()
                .find(|(_, _, a)| *a == action)
                .and_then(|(_, key, _)| key)
                .map(|key| key.spelled())
        };
        for (action, spelled) in [
            (Action::Command(Command::NewSheet), "⌘N"),
            (Action::Standard("openDocument:"), "⌘O"),
            (Action::Standard("saveDocument:"), "⌘S"),
            (Action::Standard("duplicateDocument:"), "⇧⌘S"),
            (Action::Command(Command::Print), "⌘P"),
            (Action::Standard("performClose:"), "⌘W"),
            (Action::Standard("undo:"), "⌘Z"),
            (Action::Standard("redo:"), "⇧⌘Z"),
            (Action::Standard("copy:"), "⌘C"),
            (Action::Standard("toggleSidebar:"), "⌃⌘S"),
            (Action::Command(Command::Mark(Emphasis::Bold)), "⌘B"),
            (Action::Command(Command::Mark(Emphasis::Italic)), "⌘I"),
            (Action::Command(Command::Mark(Emphasis::Underline)), "⌘U"),
            (Action::Standard("orderFrontFontPanel:"), "⌘T"),
            (Action::Standard("orderFrontColorPanel:"), "⇧⌘C"),
            (Action::Command(Command::ShowSource), "⌥⌘U"),
            (Action::Tagged("performFindPanelAction:", find::SHOW), "⌘F"),
            (Action::Tagged("performFindPanelAction:", find::NEXT), "⌘G"),
            (
                Action::Tagged("performFindPanelAction:", find::PREVIOUS),
                "⇧⌘G",
            ),
            (
                Action::Tagged("performFindPanelAction:", find::USE_SELECTION),
                "⌘E",
            ),
        ] {
            assert_eq!(key_of(action).as_deref(), Some(spelled), "{action:?}");
        }
    }

    /// An ellipsis says the item asks for more before it acts — and nothing else does.
    #[test]
    fn an_ellipsis_means_the_item_asks_for_more() {
        const ASKS: &[&str] = &[
            "openDocument:",
            "saveDocument:",
            "renameDocument:",
            "moveDocument:",
            "browseDocumentVersions:",
        ];
        for (title, _, action) in entries() {
            let asks = match action {
                Action::Standard(selector) => ASKS.contains(&selector),
                // Find… opens the bar that asks; the others act on the word already there.
                Action::Tagged(_, tag) => tag == find::SHOW,
                Action::Command(command) => command.asks(),
            };
            assert_eq!(title.ends_with('…'), asks, "{title}");
            assert!(
                !title.ends_with("..."),
                "{title}: an ellipsis, not three dots"
            );
            assert!(title.starts_with(char::is_uppercase), "{title}");
        }
    }

    #[test]
    fn the_application_window_and_help_menus_are_where_appkit_expects_them() {
        let roles: Vec<Role> = MENUS.iter().map(|menu| menu.role).collect();
        assert_eq!(roles.first(), Some(&Role::Application));
        assert_eq!(roles.last(), Some(&Role::Help));
        assert_eq!(roles[roles.len() - 2], Role::Window);
        for role in [Role::Application, Role::Window, Role::Help] {
            assert_eq!(roles.iter().filter(|r| **r == role).count(), 1, "{role:?}");
        }
        assert_eq!(MENUS[0].title, APP_NAME);
    }

    /// `NSDocumentController` finds Open Recent by the one item it owns.
    #[test]
    fn open_recent_carries_the_item_the_document_controller_looks_for() {
        assert!(OPEN_RECENT.items.iter().any(|item| matches!(
            item,
            Item::Entry {
                action: Action::Standard("clearRecentDocuments:"),
                ..
            }
        )));
    }

    #[test]
    fn a_key_is_spelled_the_mac_way() {
        assert_eq!(
            Key {
                key: "s",
                mods: SHIFT_CMD
            }
            .spelled(),
            "⇧⌘S"
        );
        assert_eq!(
            Key {
                key: "f",
                mods: CTRL_CMD
            }
            .spelled(),
            "⌃⌘F"
        );
    }
    /// The colour menus are the palette, in its order, under its own names — what `grind sheet
    /// style --color navy` and every other shell's swatch call them — after the one that sets
    /// none.
    #[test]
    fn the_colour_menus_are_the_palette() {
        for (menu, ctor) in [
            (&TEXT_COLOR, Command::TextColor as fn(Option<u8>) -> Command),
            (&BACKGROUND, Command::Background),
        ] {
            let entries: Vec<(&str, Action)> = menu
                .items
                .iter()
                .filter_map(|item| match item {
                    Item::Entry { title, action, .. } => Some((*title, *action)),
                    _ => None,
                })
                .collect();
            assert_eq!(entries.len(), PALETTE.len() + 1, "{}", menu.title);
            assert_eq!(entries[0].1, Action::Command(ctor(None)));
            for (index, ((title, action), (name, hex))) in
                entries[1..].iter().zip(PALETTE).enumerate()
            {
                assert_eq!(title.to_lowercase(), name);
                assert_eq!(grind_core::style::palette(title), Some(hex));
                assert_eq!(*action, Action::Command(ctor(Some(index as u8))));
            }
        }
    }

    /// Every verb means something in some document, and the two kinds share only what both
    /// can hold: a cell has no underline and a paragraph no number format.
    #[test]
    fn every_command_applies_somewhere_and_only_where_it_can() {
        let (sheet, text) = (DocumentKind::Spreadsheet, DocumentKind::Text);
        for command in Command::all() {
            assert!(
                command.applies(sheet) || command.applies(text),
                "{command:?}"
            );
            assert!(
                !command.applies(DocumentKind::Presentation)
                    || matches!(
                        command,
                        Command::NewSheet | Command::NewText | Command::Welcome | Command::About
                    )
            );
        }
        assert!(Command::Mark(Emphasis::Bold).applies(sheet));
        assert!(!Command::Mark(Emphasis::Underline).applies(sheet));
        assert!(!Command::Number(Preset::Currency).applies(text));
        assert!(Command::Block(Block::Heading(2)).applies(text));
        for paper in [Command::ExportPdf, Command::Print] {
            assert!(paper.applies(text) && !paper.applies(sheet), "{paper:?}");
        }
    }

    /// A context menu row is a menu-bar row: the same title for the same action, so a verb has
    /// one name whichever surface it is chosen from — and the keys shown are the bar's own.
    #[test]
    fn every_context_row_is_a_menu_bar_row() {
        let bar: Vec<(&str, Option<Key>, Action)> = entries();
        for context in [&GRID_CONTEXT, &PAGE_CONTEXT] {
            for item in context.items {
                match item {
                    Item::Entry { title, key, action } => assert!(
                        bar.contains(&(*title, *key, *action)),
                        "{}: {title}",
                        context.title
                    ),
                    Item::Submenu { menu, .. } => assert!(
                        every_item(MENUS)
                            .iter()
                            .any(|i| matches!(i, Item::Submenu { menu: m, .. } if std::ptr::eq(*m, *menu))),
                        "{}: a submenu the bar has",
                        context.title
                    ),
                    Item::Separator => {}
                }
            }
        }
    }
}
