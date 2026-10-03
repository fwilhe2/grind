// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Every verb this shell has, as data — and the menu bar it hangs in.
//!
//! **Portable, and tested on any host.** `doc/windows-shell.md`'s decision 4 makes the menu bar
//! this platform's growable surface, where the GNOME window needed a Ctrl+K palette: Windows
//! draws a menu itself, so it scales with DPI, follows the theme and gets Alt-key navigation for
//! nothing. The price of a surface that is *meant* to grow is a rule about what may go in it, and
//! it is the GTK window's rule unchanged — **a verb goes in a menu, a property of the selection
//! goes on the format strip** (which is W5's, since `CharStyle` and `CellStyle` bound it).
//!
//! Two things follow from the menus being a table rather than a sequence of `AppendMenuW` calls:
//!
//! * The **id** of a command is derived from its position in [`Command::ALL`] and never written
//!   down twice, so the classic Win32 bug of two menu items sharing a `WM_COMMAND` id cannot
//!   happen here.
//! * The **check that every command is reachable** runs on Linux with no window at all. Its other
//!   half — that every command has a handler — is the Rust compiler's: `win.rs`'s dispatcher
//!   matches on [`Command`] exhaustively, so a command with nowhere to go fails the build.
//!
//! The accelerators live here too rather than in `sheet/keymap.rs`, because they are *verbs*
//! and that file is navigation. `sheet/state.rs` consults this first and the keymap second,
//! which is what makes Ctrl+S mean Save whatever the grid would otherwise have done with `S`.
//!
//! ponytail: the accelerators are matched by [`accelerator`] rather than by a real `HACCEL` and
//! `TranslateAccelerator`. The table is the same either way; what a real accelerator table would
//! add is Windows drawing the key next to the menu item automatically instead of this file
//! spelling it — a cosmetic difference `win.rs`'s own `context_menu` and [`shortcuts`] (W7) do
//! not depend on, since both read the same `\t`-separated spelling this file already carries.

use crate::sheet::keymap::{Key, Mods};

/// One verb. Everything the shell can be *asked* to do that is not a movement.
///
/// Deliberately not a home for anything that reads or writes a property of the selection —
/// alignment, a number format, bold — which is the format strip's admission test and W5's work.
/// Two exceptions, and each of them is also somewhere else: the text pane's toggles also sit on
/// its strip, and the three currencies are in the grid's Format menu because that pane has no
/// strip of its own yet. A cell's other properties wait for one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// A new, empty spreadsheet — **in this window**, whichever kind it was showing.
    ///
    /// One of a pair where there used to be a single kind-locked `New`, which could only ever
    /// make another document of the kind already open. That was a stop-gap with its own comment
    /// saying so, and the welcome screen is what made it untenable: a window that offers the two
    /// choices when it opens cannot then refuse to offer them from the File menu.
    NewSheet,
    NewText,
    /// Back to the welcome screen — [`crate::welcome`], the pane a window with no document shows.
    ///
    /// Reachable rather than only initial, because a welcome screen you can never return to is a
    /// splash screen. It asks the close question first, like every other verb that replaces what
    /// the window is showing.
    Welcome,
    Open,
    Save,
    SaveAs,
    Exit,
    Undo,
    Redo,
    /// Copy the selection to the clipboard as `CF_UNICODETEXT`, then clear it —
    /// `App::clear_range` under a `crate::clipboard::set_text`.
    Cut,
    /// Copy the selection to the clipboard, formulas and all, as tab-separated
    /// `App::input_text` — `doc/windows-shell.md` decision 6.
    Copy,
    /// Fill from the clipboard at the selection's corner — `App::enter_range` under a
    /// `crate::clipboard::get_text`.
    Paste,
    /// Empty the selected cells, keeping their formatting — `App::clear_range`.
    ClearCells,
    /// Fill Down (Ctrl+D) and Fill Right (Ctrl+R): each column's top cell, or each row's left
    /// one, copied across the rest of the selection with its relative references shifted —
    /// `App::fill` over `grind_sheet::nav::fills`, the lines the Mac and the browser fill too.
    FillDown,
    FillRight,
    /// The cell the selection grew from, copied across the whole selection, references shifted —
    /// one `App::fill`.
    FillAcross,
    /// A chart of the table the selection means, placed beside it (`grind_sheet::verbs::
    /// insert_chart`), and the last chart taken away again. Drawn by `sheet/chart.rs`.
    InsertChart,
    DeleteChart,
    /// The last chart's kind, title and legend, asked for in words (`verbs::restyle_chart`).
    RestyleChart,
    /// A hairline round every selected cell, or every edge taken away — `grind_sheet::format::
    /// bordered`, the call every other shell's Borders control makes. The grid draws them
    /// (`look::border_strokes`).
    /// Wrap the selected cells' text at their column's width (`Toggle::Wrap`); on when the active
    /// cell is already wrapped, off. The grid breaks it with `grind_core::layout::wrap` and grows
    /// the row to hold it.
    WrapText,
    BordersAll,
    BordersNone,
    /// Copy the selection as it is *shown* — a formula's formatted result rather than its source
    /// (`App::value_text`), for pasting into something that is not a spreadsheet.
    CopyValue,
    /// Replace every formula in the selection with the value it last computed
    /// (`grind_sheet::verbs::formulas_to_values`).
    FormulaToValue,
    /// Hide or show the rows (or columns) the selection spans — `App::set_row_hidden` and
    /// `set_col_hidden` over `grind_sheet::verbs::rows`/`cols`, the call `grind sheet hide` makes.
    HideRows,
    ShowRows,
    HideColumns,
    ShowColumns,
    /// A length asked for in a prompt — `2.5cm`, `1in`, `64pt` — set on every selected row or
    /// column in one undo step: `App::set_row_height` / `set_col_width`, which check the length.
    RowHeight,
    /// Each selected column set to its widest text — measured in the font it is drawn in.
    FitColumns,
    ColumnWidth,
    /// A name for the selection, to use in formulas instead of its address (`App::set_name`
    /// through `grind_sheet::a1::definition`, as `grind sheet name` reads one).
    DefineName,
    /// The document's own locale, asked for as a tag (`de-DE`) — how it spells its numbers
    /// (`App::set_locale`, `grind sheet locale`'s call).
    DocumentLocale,
    /// Every formula in the document in a list, filtered by a word, each a jump to its cell
    /// (`App::calculations`, the GNOME window's *Find a Calculation*).
    FindCalculation,
    /// A formula worked out at the active cell and said in the notice bar, storing nothing and
    /// making no undo step (`grind_sheet::verbs::evaluated`, `grind sheet eval`'s call).
    Evaluate,
    /// Put the caret in the name box. A menu item as well as F5, because a verb nobody can
    /// find is a verb this shell does not have.
    GoTo,
    /// Ask for a word, then select the first cell holding it at or after the cursor, across
    /// every sheet — `App::find`, whose matching rule (`grind_sheet::find`) is the one every
    /// other shell's find uses. The grid's alone: a text document has no cells.
    Find,
    /// The next or previous cell holding the word [`Command::Find`] last asked for, wrapping
    /// at either end — `grind_sheet::find::step`, the GNOME bar's and the browser's F3.
    FindNext,
    FindPrevious,
    /// Ask what and with what, then `App::replace` in every cell of every sheet — one undo
    /// step, and a formula it would break left alone and named on the notice bar.
    Replace,
    /// Read a delimited file in at the cursor — a file dialog, then `App::import_csv` with
    /// `csv::Import::sniffed`, one undo step. The grid's alone: a text document has no cells
    /// for fields to land in.
    ImportCsv,
    /// [`Command::ImportCsv`], having first asked for options in words — `delimiter=semicolon
    /// locale=de-DE text trim` (`csv::Import::amended`), the flags `grind sheet import-csv` has.
    ImportCsvWith,
    /// Write the selection out as one — a save dialog whose **name** says which delimiter
    /// (`csv::Dialect::for_name`, the two filters below it), then `App::export_csv`. Nothing is
    /// stored, so the document is untouched by it.
    ExportCsv,
    Recalculate,
    /// Every function this build implements, as a list to pick from — `sheet/assist.rs`'s
    /// `function_lines`, which is `grind sheet functions --long`'s four columns in a dialog.
    /// Picking one puts `=NAME(` in the cell, so the list is a way of *writing* a formula and
    /// not only of reading about one.
    FunctionList,
    /// The active cell's formula in full — `formula::friendly::explain`: names spelled out,
    /// arguments labelled with the parameter they fill, one per line once a call stops fitting
    /// on one. Read-only, and it never parses back (R1: the document's formula is ODF's).
    ExplainFormula,
    /// Whether the formula bar shows that same reading in place of the text that would be typed
    /// back in — `friendly::explain_inline`, and the `fx` badge says which of the two is up.
    ToggleFriendly,
    /// An autofilter over the selection (§9.4), or clear the one the sheet already has —
    /// `App::set_filter`. The on/off switch its name implies, the same shape `sheet.filter`
    /// has in the web shell and `Grid::toggle_filter` in the GTK one.
    ToggleFilter,
    /// An autofilter, alternating row shading and a name over the selection, one undo step —
    /// `App::format_table`. No dialog: fixed defaults (a header, no totals row), the same
    /// zero-prompt shape `sheet.filter` already has in the web shell.
    FormatTable,
    /// The same composite with a totals row, having asked which aggregate it carries —
    /// `dialog::choose` over `TotalsFunction::ALL`. A second verb rather than a prompt on the
    /// first, so the plain one stays zero-prompt, which is the shape the web shell's palette
    /// gives the pair too.
    FormatTableTotals,
    /// Format the selection as one of `numfmt::CURRENCIES` — the euro, the dollar, the pound,
    /// in that order ([`Command::currency`]), one click each, as the GTK picker's buttons are.
    /// The grid's alone; `sheet/currency.rs` is what each one writes, and the item whose
    /// currency the active cell already has carries a check.
    CurrencyEuro,
    CurrencyDollar,
    CurrencyPound,
    /// Align the selection's text to one side of its cells, or centre it — `fo:text-align`,
    /// written `start`/`center`/`end` (§16.5, relative to the writing direction). The grid's
    /// alone, and the format strip's three alignment toggles (`sheet/format.rs`): pressing the
    /// alignment a cell already has takes it off.
    AlignLeft,
    AlignCenter,
    AlignRight,
    /// *Cell Background* — the grid's fill, `fo:background-color` on a cell, over the same palette
    /// as *Text Colour*. A verb of its own rather than [`Command::PickHighlight`] under another
    /// label, because a run's highlight and a cell's fill are two properties that merely share an
    /// attribute name, and one command id with two labels is one the menu cannot spell.
    PickBackground,
    /// *Number Format* — the nine kinds `grind sheet format` takes, one click each
    /// (`sheet/format.rs`'s `KINDS`).
    NumberFormat,
    /// One decimal fewer, or more, over a number, percentage or currency — Excel's *Decrease* and
    /// *Increase Decimal* (`sheet/format.rs`'s `stepped`).
    FewerDecimals,
    MoreDecimals,
    SheetAdd,
    SheetRename,
    SheetDelete,
    SheetNext,
    SheetPrevious,
    /// Toggle bold or italic across the selection — **both panes'**: a run's `CharStyle` in the
    /// text pane, a cell's `CellStyle` on the grid, and Ctrl+B means bold in either. Underline and
    /// the two after it are the text pane's alone, since a cell style has no underline to toggle.
    Bold,
    Italic,
    Underline,
    /// Toggle strikethrough — the fourth of `markdown::Emphasis`'s five, drawn on the strip now
    /// alongside Bold/Italic/Underline rather than left menu-and-key-only the way
    /// `grind-text-gtk`'s own bar still has it.
    Strike,
    /// Toggle the monospace family — what the `` `code` `` notation writes, and the strip's
    /// fifth toggle.
    Code,
    /// *Font* — `dialog::choose` over a curated list, `fo:font-family` verbatim.
    PickFamily,
    /// *Font Size* — `dialog::choose` over `grind_text::format::sizes`.
    PickSize,
    /// *Text Colour* — `dialog::choose` over `grind_core::style::PALETTE`.
    PickColor,
    /// *Highlight* — the same picker, writing `fo:background-color` instead.
    PickHighlight,
    /// Every property of the selection's formatting, off at once — the one-shot the toggles can
    /// only approximate, and the strip's own *Clear* button.
    ClearFormatting,
    /// The caret's block becomes a paragraph wearing the `Title` named style —
    /// `ui_text_gtk`'s own `win.title`, mirrored so the two windows both know how to apply and
    /// draw the same two names (`grind_text::NAMED_STYLES`).
    Title,
    Subtitle,
    /// The caret's block becomes a plain paragraph — `App::set_kind`, `BlockKind::Paragraph`.
    /// Left out of the grid's menus, the same way the three toggles above are.
    Paragraph,
    /// The caret's block becomes a heading at this level — `ui_text_gtk`'s own Ctrl+1/2/3, kept
    /// to the same three quick levels here so a key reaches the common case the way it does in
    /// that window. [`Command::BlockKindDialog`] is where the rest of the schema's uncapped
    /// `positiveInteger` (`doc/text-core.md`) and every list depth live instead of a key each.
    Heading1,
    Heading2,
    Heading3,
    /// The text pane's outline dialog — every heading, jump to any of them. Left out of the
    /// grid's menus.
    Outline,
    /// Every block kind this build authors, heading levels past 3 and list items included, as one
    /// dialog rather than a key per depth — the gap `doc/text-shell.md` names for every shell's
    /// own window ("no lists UI") and the first one to close it.
    BlockKindDialog,
    /// Insert a picture at the caret — a file dialog, then `App::insert_image`, into a paragraph
    /// of its own below the caret's block (an empty block is used as it stands). Decoded and
    /// drawn for real (`image.rs`, WIC), matching `ui_text_gtk`'s own Ctrl+Shift+I.
    InsertPicture,
    /// Insert a table below the caret's block — a size asked for in one prompt (`3x4`), then
    /// `grind_text::table::insert_below`, which also decides where it goes and that a document
    /// never ends with one. Matches `ui_text_gtk`'s Ctrl+Shift+T.
    InsertTable,
    /// Anchor a bookmark at the caret's block — a name, then `App::set_bookmark`. Naming one
    /// already there moves it. Switches the name overlay on, since a bookmark contributes no
    /// characters and would otherwise leave nothing on the page to say it was made.
    Bookmark,
    /// Name the style of the paragraph the selection touches (`App::set_style`); an empty answer
    /// takes the name away. A named style is kept and never interpreted (`doc/text-core.md`).
    ParagraphStyle,
    /// Swap the paragraphs the selection touches past their neighbour, or delete them —
    /// `grind_text::blocks::shift` / `remove`, one undo step each, the Mac's Format ▸ Paragraph.
    ParagraphUp,
    ParagraphDown,
    ParagraphDelete,
    /// The document as its own projection (D9) — a modal list of its lines, `App::project`'s
    /// text-form, opened on whichever line the pane's own selection or caret projects to. Applies
    /// to both document types, the same as `App::project` reaching both.
    ShowSource,
    /// "Check Document" (D6) — every `App::lint` finding, worst first, each row a jump. Applies
    /// to both document types, the same as `App::lint` reaching both.
    CheckDocument,
    /// `doc/view-modes.md`'s role overlay, on or off. The grid's alone: the text pane has no
    /// `CellRole`.
    ToggleRoles,
    /// Show each formula cell's formula instead of what it came to — `grind sheet view --formulas`,
    /// a reading like the two overlays beside it: nothing is written.
    ToggleFormulas,
    /// The grid's zoom — a factor on every cell, 25% to 400%, never stored.
    ZoomIn,
    ZoomOut,
    ZoomReset,
    /// `doc/view-modes.md`'s name overlay, on or off — a defined name's range on the grid,
    /// `BlockView::marks`' bookmarks on the text pane (`:names`' equivalent there, since a
    /// bookmark contributes no characters of its own). Both document types.
    ToggleNames,
    /// W7's "key list" — every accelerator this shell answers, read straight off [`MENUS`]'
    /// own labels rather than a second table that could drift from them.
    Shortcuts,
    About,
}

impl Command {
    /// Every command, in the order that decides their `WM_COMMAND` ids.
    ///
    /// Adding one here and nowhere else fails two checks at once: the test below says it is in no
    /// menu, and `win.rs`'s exhaustive match says it has no handler.
    pub const ALL: &'static [Command] = &[
        Command::NewSheet,
        Command::NewText,
        Command::Welcome,
        Command::Open,
        Command::Save,
        Command::SaveAs,
        Command::Exit,
        Command::Undo,
        Command::Redo,
        Command::Cut,
        Command::Copy,
        Command::Paste,
        Command::ClearCells,
        Command::FillDown,
        Command::FillRight,
        Command::FillAcross,
        Command::InsertChart,
        Command::DeleteChart,
        Command::RestyleChart,
        Command::WrapText,
        Command::BordersAll,
        Command::BordersNone,
        Command::CopyValue,
        Command::FormulaToValue,
        Command::HideRows,
        Command::ShowRows,
        Command::HideColumns,
        Command::ShowColumns,
        Command::RowHeight,
        Command::FitColumns,
        Command::ColumnWidth,
        Command::DefineName,
        Command::DocumentLocale,
        Command::FindCalculation,
        Command::Evaluate,
        Command::GoTo,
        Command::Find,
        Command::FindNext,
        Command::FindPrevious,
        Command::Replace,
        Command::ImportCsv,
        Command::ImportCsvWith,
        Command::ExportCsv,
        Command::Recalculate,
        Command::FunctionList,
        Command::ExplainFormula,
        Command::ToggleFriendly,
        Command::ToggleFilter,
        Command::FormatTable,
        Command::FormatTableTotals,
        Command::CurrencyEuro,
        Command::CurrencyDollar,
        Command::CurrencyPound,
        Command::AlignLeft,
        Command::AlignCenter,
        Command::AlignRight,
        Command::PickBackground,
        Command::NumberFormat,
        Command::FewerDecimals,
        Command::MoreDecimals,
        Command::SheetAdd,
        Command::SheetRename,
        Command::SheetDelete,
        Command::SheetNext,
        Command::SheetPrevious,
        Command::Bold,
        Command::Italic,
        Command::Underline,
        Command::Strike,
        Command::Code,
        Command::PickFamily,
        Command::PickSize,
        Command::PickColor,
        Command::PickHighlight,
        Command::ClearFormatting,
        Command::Title,
        Command::Subtitle,
        Command::Paragraph,
        Command::Heading1,
        Command::Heading2,
        Command::Heading3,
        Command::Outline,
        Command::BlockKindDialog,
        Command::InsertPicture,
        Command::InsertTable,
        Command::Bookmark,
        Command::ParagraphStyle,
        Command::ParagraphUp,
        Command::ParagraphDown,
        Command::ParagraphDelete,
        Command::ShowSource,
        Command::CheckDocument,
        Command::ToggleRoles,
        Command::ToggleFormulas,
        Command::ZoomIn,
        Command::ZoomOut,
        Command::ZoomReset,
        Command::ToggleNames,
        Command::Shortcuts,
        Command::About,
    ];

    /// The `WM_COMMAND` id this verb arrives as.
    ///
    /// Offset by 100 so that a control notification's id — the name box is 1, the editor 2 —
    /// can never collide with a menu command's. Win32 puts both through the same message and
    /// tells them apart by the high word, and this makes a mistake there visible rather than
    /// silent.
    pub fn id(self) -> u16 {
        let at = Self::ALL
            .iter()
            .position(|c| *c == self)
            .expect("every command is in ALL");
        FIRST_ID + at as u16
    }

    /// The three currency verbs, in `numfmt::CURRENCIES`' order.
    pub const CURRENCIES: [Command; 3] = [
        Command::CurrencyEuro,
        Command::CurrencyDollar,
        Command::CurrencyPound,
    ];

    /// Which of `numfmt::CURRENCIES` this verb writes, by index — `None` for every other verb.
    pub fn currency(self) -> Option<usize> {
        Self::CURRENCIES.iter().position(|c| *c == self)
    }
}

/// The first id a menu command may take. Below it are the child controls' ids.
pub const FIRST_ID: u16 = 100;

/// Which command a `WM_COMMAND` id names, or `None` for one that is not a menu command.
pub fn command_for(id: u16) -> Option<Command> {
    id.checked_sub(FIRST_ID)
        .and_then(|at| Command::ALL.get(usize::from(at)))
        .copied()
}

/// One entry in a menu: a verb, or the line between two groups of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    /// The label carries its own `&` mnemonic, which is what Windows draws underlined, and the
    /// accelerator's *name* after a tab — the convention every Win32 menu follows.
    Verb {
        command: Command,
        label: &'static str,
    },
    Separator,
}

/// One top-level menu.
#[derive(Clone, Copy, Debug)]
pub struct Menu {
    pub title: &'static str,
    pub items: &'static [Item],
}

/// The menu bar.
///
/// Seven menus and nothing that is not a verb. Format holds every control of either pane's
/// format strip even though the strips reach them too — those items *read and write* a property of
/// the selection, exactly what a strip is for, but a menu they can also start from costs nothing,
/// carries the keys, and is how the strip's verbs are reached from the keyboard. Each pane sees its
/// own: bold, italic, a text colour and Clear are both panes', and `applies_to` keeps the rest to
/// the one pane that has the property. View holds W6's three shared panes: the
/// source, the check, and the two overlays only the grid can draw. What is deliberately absent:
/// anything resembling a ribbon — `doc/sheet-shell.md`'s tab strip was removed for being one,
/// and the argument carries.
pub const MENUS: &[Menu] = &[
    Menu {
        title: "&File",
        items: &[
            Item::Verb {
                command: Command::NewSheet,
                label: "&New Spreadsheet\tCtrl+N",
            },
            Item::Verb {
                command: Command::NewText,
                label: "New &Text Document\tCtrl+Shift+N",
            },
            Item::Verb {
                command: Command::Open,
                label: "&Open…\tCtrl+O",
            },
            Item::Verb {
                command: Command::Welcome,
                label: "&Welcome Screen",
            },
            Item::Separator,
            Item::Verb {
                command: Command::Save,
                label: "&Save\tCtrl+S",
            },
            Item::Verb {
                command: Command::SaveAs,
                label: "Save &As…\tCtrl+Shift+S",
            },
            Item::Separator,
            // The one non-ODF format (`doc/not-doing.md` §2), and File is where Windows puts
            // Import and Export. Its own section, because importing is not another way of
            // saving the document — and over a text document [`applies_to`] leaves both out,
            // so this menu is the four file verbs and Exit there.
            Item::Verb {
                command: Command::ImportCsv,
                label: "&Import CSV…",
            },
            Item::Verb {
                command: Command::ImportCsvWith,
                label: "Import CSV wit&h Options…",
            },
            Item::Verb {
                command: Command::ExportCsv,
                label: "&Export CSV…",
            },
            Item::Separator,
            Item::Verb {
                command: Command::Exit,
                label: "E&xit\tAlt+F4",
            },
        ],
    },
    Menu {
        title: "&Edit",
        items: &[
            Item::Verb {
                command: Command::Undo,
                label: "&Undo\tCtrl+Z",
            },
            Item::Verb {
                command: Command::Redo,
                label: "&Redo\tCtrl+Y",
            },
            Item::Separator,
            Item::Verb {
                command: Command::Cut,
                label: "Cu&t\tCtrl+X",
            },
            Item::Verb {
                command: Command::Copy,
                label: "&Copy\tCtrl+C",
            },
            Item::Verb {
                command: Command::Paste,
                label: "&Paste\tCtrl+V",
            },
            Item::Separator,
            Item::Verb {
                command: Command::ClearCells,
                label: "&Delete\tDel",
            },
            Item::Verb {
                command: Command::FillDown,
                label: "Fill Do&wn\tCtrl+D",
            },
            Item::Verb {
                command: Command::FillRight,
                label: "Fill Rig&ht\tCtrl+R",
            },
            Item::Verb {
                command: Command::FillAcross,
                label: "Fill Acro&ss",
            },
            Item::Separator,
            Item::Verb {
                command: Command::CopyValue,
                label: "Copy Va&lue",
            },
            Item::Verb {
                command: Command::FormulaToValue,
                label: "For&mula to Value",
            },
            Item::Separator,
            Item::Verb {
                command: Command::Find,
                label: "&Find…\tCtrl+F",
            },
            Item::Verb {
                command: Command::FindNext,
                label: "Find &Next\tF3",
            },
            Item::Verb {
                command: Command::FindPrevious,
                label: "Find Pre&vious\tShift+F3",
            },
            Item::Verb {
                command: Command::Replace,
                label: "R&eplace…\tCtrl+H",
            },
            Item::Separator,
            Item::Verb {
                command: Command::GoTo,
                label: "&Go To…\tF5",
            },
            Item::Verb {
                command: Command::Outline,
                label: "&Outline…\tCtrl+Shift+O",
            },
        ],
    },
    Menu {
        title: "&Sheet",
        items: &[
            Item::Verb {
                command: Command::SheetAdd,
                label: "&Add…",
            },
            Item::Verb {
                command: Command::SheetRename,
                label: "&Rename…",
            },
            Item::Verb {
                command: Command::SheetDelete,
                label: "&Delete",
            },
            Item::Separator,
            Item::Verb {
                command: Command::SheetNext,
                label: "&Next\tCtrl+PgDn",
            },
            Item::Verb {
                command: Command::SheetPrevious,
                label: "&Previous\tCtrl+PgUp",
            },
            Item::Separator,
            Item::Verb {
                command: Command::HideRows,
                label: "Hide Row&s",
            },
            Item::Verb {
                command: Command::ShowRows,
                label: "Show R&ows",
            },
            Item::Verb {
                command: Command::HideColumns,
                label: "Hide &Columns",
            },
            Item::Verb {
                command: Command::ShowColumns,
                label: "Show Col&umns",
            },
            Item::Verb {
                command: Command::RowHeight,
                label: "Row &Height…",
            },
            Item::Verb {
                command: Command::ColumnWidth,
                label: "Column &Width…",
            },
            Item::Verb {
                command: Command::FitColumns,
                label: "&Fit Column Width",
            },
        ],
    },
    Menu {
        title: "&Data",
        items: &[
            Item::Verb {
                command: Command::Recalculate,
                label: "&Recalculate\tF9",
            },
            Item::Separator,
            Item::Verb {
                command: Command::FunctionList,
                label: "&Function List…\tCtrl+Shift+F",
            },
            Item::Verb {
                command: Command::ExplainFormula,
                label: "&Explain Formula…\tCtrl+Shift+E",
            },
            Item::Verb {
                command: Command::InsertChart,
                label: "Insert &Chart",
            },
            Item::Verb {
                command: Command::RestyleChart,
                label: "Change Last C&hart…",
            },
            Item::Verb {
                command: Command::DeleteChart,
                label: "Delete &Last Chart",
            },
            Item::Verb {
                command: Command::Evaluate,
                label: "E&valuate…",
            },
            Item::Verb {
                command: Command::FindCalculation,
                label: "Find a Calc&ulation…",
            },
            Item::Verb {
                command: Command::DocumentLocale,
                label: "Docu&ment Locale…",
            },
            Item::Verb {
                command: Command::DefineName,
                label: "Define &Name…",
            },
            Item::Separator,
            Item::Verb {
                command: Command::ToggleFilter,
                label: "&Autofilter\tCtrl+Shift+L",
            },
            Item::Verb {
                command: Command::FormatTable,
                label: "F&ormat as Table",
            },
            Item::Verb {
                command: Command::FormatTableTotals,
                label: "Format as Table with &Totals…",
            },
        ],
    },
    Menu {
        title: "F&ormat",
        items: &[
            // One menu over both panes, and `items_for` drops what the pane showing has no
            // answer for — collapsing the separators it leaves, so each pane sees its own groups
            // in the order its strip draws them: the weight of the text, its alignment, its
            // colours, the number it shows, and Clear on its own.
            Item::Verb {
                command: Command::Bold,
                label: "&Bold\tCtrl+B",
            },
            Item::Verb {
                command: Command::Italic,
                label: "&Italic\tCtrl+I",
            },
            Item::Verb {
                command: Command::Underline,
                label: "&Underline\tCtrl+U",
            },
            Item::Verb {
                command: Command::Strike,
                label: "S&trikethrough\tCtrl+Shift+X",
            },
            Item::Verb {
                command: Command::Code,
                label: "&Monospace\tCtrl+Shift+M",
            },
            Item::Separator,
            Item::Verb {
                command: Command::AlignLeft,
                label: "&Align Left",
            },
            Item::Verb {
                command: Command::AlignCenter,
                label: "Ce&nter",
            },
            Item::Verb {
                command: Command::AlignRight,
                label: "Align Righ&t",
            },
            Item::Separator,
            Item::Verb {
                command: Command::PickFamily,
                label: "&Font…",
            },
            Item::Verb {
                command: Command::PickSize,
                label: "Si&ze…",
            },
            Item::Verb {
                command: Command::PickColor,
                label: "Text &Colour…",
            },
            Item::Verb {
                command: Command::PickHighlight,
                label: "&Highlight…",
            },
            Item::Verb {
                command: Command::WrapText,
                label: "&Wrap Text",
            },
            Item::Verb {
                command: Command::BordersAll,
                label: "All Bord&ers",
            },
            Item::Verb {
                command: Command::BordersNone,
                label: "Remo&ve Borders",
            },
            Item::Verb {
                command: Command::PickBackground,
                label: "Cell Backgr&ound…",
            },
            Item::Separator,
            Item::Verb {
                command: Command::NumberFormat,
                label: "Number &Format…",
            },
            Item::Verb {
                command: Command::FewerDecimals,
                label: "Decrease Decimal&s",
            },
            Item::Verb {
                command: Command::MoreDecimals,
                label: "Increase Deci&mals",
            },
            // The currency verbs, one click each, the same three the other windows offer
            // (`numfmt::CURRENCIES`) — a number format, so they sit with the number format.
            Item::Verb {
                command: Command::CurrencyEuro,
                label: "Currency: Eu&ro (€)",
            },
            Item::Verb {
                command: Command::CurrencyDollar,
                label: "Currency: US &Dollar ($)",
            },
            Item::Verb {
                command: Command::CurrencyPound,
                label: "Currency: Pound Sterlin&g (£)",
            },
            Item::Separator,
            Item::Verb {
                command: Command::ClearFormatting,
                label: "C&lear Formatting",
            },
            Item::Separator,
            Item::Verb {
                command: Command::Title,
                label: "Titl&e",
            },
            Item::Verb {
                command: Command::Subtitle,
                label: "&Subtitle",
            },
            Item::Separator,
            Item::Verb {
                command: Command::Paragraph,
                label: "&Paragraph\tCtrl+0",
            },
            Item::Verb {
                command: Command::Heading1,
                label: "Heading &1\tCtrl+1",
            },
            Item::Verb {
                command: Command::Heading2,
                label: "Heading &2\tCtrl+2",
            },
            Item::Verb {
                command: Command::Heading3,
                label: "Heading &3\tCtrl+3",
            },
            Item::Separator,
            Item::Verb {
                command: Command::BlockKindDialog,
                label: "Block &Kind…\tCtrl+Shift+K",
            },
            Item::Verb {
                command: Command::InsertPicture,
                label: "I&nsert Picture…\tCtrl+Shift+I",
            },
            Item::Verb {
                command: Command::InsertTable,
                label: "Insert T&able…\tCtrl+Shift+T",
            },
            Item::Separator,
            Item::Verb {
                command: Command::Bookmark,
                label: "B&ookmark Here…\tCtrl+Shift+B",
            },
            Item::Verb {
                command: Command::ParagraphStyle,
                label: "Paragraph St&yle Name…",
            },
            Item::Verb {
                command: Command::ParagraphUp,
                label: "Mo&ve Paragraph Up",
            },
            Item::Verb {
                command: Command::ParagraphDown,
                label: "Move Paragraph Do&wn",
            },
            Item::Verb {
                command: Command::ParagraphDelete,
                label: "Delete Para&graph",
            },
        ],
    },
    Menu {
        title: "&View",
        items: &[
            Item::Verb {
                command: Command::ShowSource,
                label: "Show &Source\tCtrl+Shift+U",
            },
            Item::Verb {
                command: Command::CheckDocument,
                label: "&Check Document\tF8",
            },
            Item::Separator,
            Item::Verb {
                command: Command::ToggleRoles,
                label: "Cell R&oles",
            },
            Item::Verb {
                command: Command::ToggleNames,
                label: "&Names",
            },
            Item::Verb {
                command: Command::ToggleFormulas,
                label: "Show Fo&rmulas",
            },
            Item::Separator,
            Item::Verb {
                command: Command::ZoomIn,
                label: "Zoom &In\tCtrl+=",
            },
            Item::Verb {
                command: Command::ZoomOut,
                label: "Zoom O&ut\tCtrl+-",
            },
            Item::Verb {
                command: Command::ZoomReset,
                label: "Reset &Zoom",
            },
            Item::Verb {
                command: Command::ToggleFriendly,
                label: "&Friendly Formulas",
            },
        ],
    },
    Menu {
        title: "&Help",
        items: &[
            Item::Verb {
                command: Command::Shortcuts,
                label: "&Keyboard Shortcuts",
            },
            Item::Verb {
                command: Command::About,
                label: "&About Grind",
            },
        ],
    },
];

/// The label a command shows in whichever menu names it — the one copy of that string, so a
/// context menu (W7) can put a verb next to its own spelling rather than a second one written
/// down beside it. Carries its `\t`-separated accelerator too, since a context menu benefits
/// from the reminder exactly as the menu bar's own does.
pub fn label_for(command: Command) -> Option<&'static str> {
    MENUS
        .iter()
        .flat_map(|menu| menu.items)
        .find_map(|item| match item {
            Item::Verb { command: c, label } if *c == command => Some(*label),
            _ => None,
        })
}

/// Every command that carries a `\t`-separated accelerator, as one line each — W7's "key list",
/// the answer to `gtk::ShortcutsWindow` this shell can build with no resources and no dialog
/// template: a plain read of the labels every menu item already has, so a shortcut shown here and
/// a shortcut shown on the bar can never say two different things about the same command.
///
/// Read out of [`MENUS`] in the bar's own order rather than [`Command::ALL`]'s, which is the
/// order a person tabbing through the menus meets them in — the more useful one for a reference
/// list — and the mnemonic's `&` is stripped, since a keyboard shortcuts window is not itself
/// navigated by one.
pub fn shortcuts() -> Vec<String> {
    MENUS
        .iter()
        .flat_map(|menu| menu.items)
        .filter_map(|item| match item {
            Item::Verb { label, .. } => label.split_once('\t'),
            Item::Separator => None,
        })
        .map(|(name, key)| format!("{} — {key}", name.replace('&', "")))
        .collect()
}

/// Which verb a keystroke asks for, if any.
///
/// Consulted **before** the navigation keymap, which is the whole reason it exists as a separate
/// question: Ctrl+S has to be Save even though `S` is a perfectly good key to type into a cell,
/// and Ctrl+PageDown has to be "next sheet" even though PageDown is a motion.
///
/// Deliberately silent about Delete: clearing the selection is a verb, but the *key* only means
/// it in Ready mode, and deciding that is `sheet/state.rs`'s job rather than this table's.
pub fn accelerator(key: Key, mods: Mods) -> Option<Command> {
    if mods.alt {
        return None;
    }
    match (key, mods.ctrl, mods.shift) {
        (Key::Char('N'), true, false) => Some(Command::NewSheet),
        (Key::Char('N'), true, true) => Some(Command::NewText),
        (Key::Char('O'), true, false) => Some(Command::Open),
        (Key::Char('S'), true, false) => Some(Command::Save),
        (Key::Char('S'), true, true) => Some(Command::SaveAs),
        (Key::Char('Z'), true, false) => Some(Command::Undo),
        (Key::Char('Y'), true, false) => Some(Command::Redo),
        (Key::Char('X'), true, false) => Some(Command::Cut),
        (Key::Char('C'), true, false) => Some(Command::Copy),
        (Key::Char('V'), true, false) => Some(Command::Paste),
        (Key::PageDown, true, _) => Some(Command::SheetNext),
        (Key::PageUp, true, _) => Some(Command::SheetPrevious),
        (Key::F9, false, false) => Some(Command::Recalculate),
        (Key::Char('='), true, _) => Some(Command::ZoomIn),
        (Key::Char('-'), true, false) => Some(Command::ZoomOut),
        (Key::Char('D'), true, false) => Some(Command::FillDown),
        (Key::Char('R'), true, false) => Some(Command::FillRight),
        (Key::Char('B'), true, false) => Some(Command::Bold),
        (Key::Char('I'), true, false) => Some(Command::Italic),
        (Key::Char('U'), true, false) => Some(Command::Underline),
        (Key::Char('X'), true, true) => Some(Command::Strike),
        (Key::Char('M'), true, true) => Some(Command::Code),
        (Key::Char('0'), true, false) => Some(Command::Paragraph),
        (Key::Char('1'), true, false) => Some(Command::Heading1),
        (Key::Char('2'), true, false) => Some(Command::Heading2),
        (Key::Char('3'), true, false) => Some(Command::Heading3),
        (Key::Char('O'), true, true) => Some(Command::Outline),
        (Key::Char('K'), true, true) => Some(Command::BlockKindDialog),
        (Key::Char('I'), true, true) => Some(Command::InsertPicture),
        (Key::Char('T'), true, true) => Some(Command::InsertTable),
        (Key::Char('B'), true, true) => Some(Command::Bookmark),
        (Key::Char('U'), true, true) => Some(Command::ShowSource),
        (Key::Char('F'), true, true) => Some(Command::FunctionList),
        (Key::Char('E'), true, true) => Some(Command::ExplainFormula),
        (Key::Char('L'), true, true) => Some(Command::ToggleFilter),
        (Key::F8, false, false) => Some(Command::CheckDocument),
        (Key::Char('F'), true, false) => Some(Command::Find),
        (Key::Char('H'), true, false) => Some(Command::Replace),
        (Key::F3, false, false) => Some(Command::FindNext),
        (Key::F3, false, true) => Some(Command::FindPrevious),
        _ => None,
    }
}

/// What the window is showing, for the one question the menu bar asks of it.
///
/// A document *kind* was enough until the welcome screen arrived, and then it was not: a window
/// showing [`crate::welcome`] holds no document at all, and answering "which kind" with a guess
/// is exactly the guess that pane exists to stop making. So the menus ask this instead, and
/// `DocumentKind` is one of its two arms rather than the whole question.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    /// The welcome screen — no document, and so only the verbs that make or find one.
    Welcome,
    Document(grind_core::DocumentKind),
}

/// Whether this verb means anything on this surface. [`items_for`] and [`menu_has_items`] are
/// the callers, and `win.rs`'s `build_menu` is theirs.
pub fn applies(command: Command, surface: Surface) -> bool {
    match surface {
        Surface::Welcome => on_welcome(command),
        Surface::Document(kind) => applies_to(command, kind),
    }
}

/// Which verbs mean anything with **no document open**.
///
/// A short list, and short on purpose: everything else in this shell acts on a document, and a
/// File menu offering Save over nothing would be the greying problem back again in its original
/// form. `Command::Welcome` is absent because you are already on it — a menu item that does nothing
/// but confirm where you are is noise.
fn on_welcome(command: Command) -> bool {
    matches!(
        command,
        Command::NewSheet
            | Command::NewText
            | Command::Open
            | Command::Exit
            | Command::Shortcuts
            | Command::About
    )
}

/// Whether this verb means anything for a document of this kind — the half of "the menus are
/// finished" that can be answered with no window at all. `win.rs`'s `build_menu` is the caller,
/// through [`items_for`]/[`menu_has_items`]: a `false` here is why `Recalculate` is missing from
/// the bar over a text document rather than shown and unclickable. State *within* a kind — Undo
/// with nothing to undo, Paste with an empty clipboard — is a separate question this does not
/// answer, and W7 still owes it. A command silent about both kinds would be a command with
/// nowhere to act, which is a different bug from the one this answers — that one is
/// `Command::ALL` matching `MENUS`, checked below.
///
/// `Presentation` answers `false` to everything: this shell opens neither kind of pane for one
/// (`opened` refuses the document before a `Pane` exists), so there is nothing it could mean.
pub fn applies_to(command: Command, kind: grind_core::DocumentKind) -> bool {
    use grind_core::DocumentKind::{Presentation, Spreadsheet, Text};
    match command {
        Command::Recalculate
        | Command::FillDown
        | Command::FillRight
        | Command::CopyValue
        | Command::FormulaToValue
        | Command::FillAcross
        | Command::InsertChart
        | Command::DeleteChart
        | Command::RestyleChart
        | Command::WrapText
        | Command::BordersAll
        | Command::BordersNone
        | Command::HideRows
        | Command::ShowRows
        | Command::HideColumns
        | Command::ShowColumns
        | Command::RowHeight
        | Command::FitColumns
        | Command::ColumnWidth
        | Command::DefineName
        | Command::DocumentLocale
        | Command::FindCalculation
        | Command::Evaluate
        | Command::SheetAdd
        | Command::SheetRename
        | Command::SheetDelete
        | Command::SheetNext
        | Command::SheetPrevious
        // The three formula verbs are the grid's for the plainest of reasons: a text document
        // has no formulas, so a function list in one would offer things it cannot hold.
        | Command::FunctionList
        | Command::ExplainFormula
        | Command::ToggleFriendly
        | Command::ToggleFilter
        | Command::FormatTable
        | Command::FormatTableTotals
        | Command::CurrencyEuro
        | Command::CurrencyDollar
        | Command::CurrencyPound
        // A cell's alignment, fill and number format are `CellStyle` and `numfmt::Format` — the
        // grid's format strip (`sheet/format.rs`); a run of text has none of the three.
        | Command::AlignLeft
        | Command::AlignCenter
        | Command::AlignRight
        | Command::PickBackground
        | Command::NumberFormat
        | Command::FewerDecimals
        | Command::MoreDecimals
        // CSV is cells: fields land in a grid and a range comes out of one, so both are the
        // spreadsheet's even though they sit in the File menu with the universal verbs.
        | Command::ImportCsv
        | Command::ImportCsvWith
        | Command::ExportCsv
        // `doc/view-modes.md`'s role overlay is `CellRole`, the grid's own vocabulary; the text
        // pane has no per-character role.
        | Command::ToggleFormulas
        | Command::ZoomIn
        | Command::ZoomOut
        | Command::ZoomReset
        | Command::ToggleRoles => matches!(kind, Spreadsheet),
        // Bold, italic, a text colour and Clear mean the same thing to a run and to a cell, and each
        // pane answers them over its own style — `CharStyle` there, `CellStyle` here — so they are
        // both panes' verbs, and Ctrl+B is bold wherever the window is.
        Command::Bold | Command::Italic | Command::PickColor | Command::ClearFormatting => {
            matches!(kind, Spreadsheet | Text)
        }
        // Find and replace reach both: `App::find` over cells on the grid, `grind_text::find`
        // over the document's text on the page.
        Command::Find | Command::FindNext | Command::FindPrevious | Command::Replace => {
            matches!(kind, Spreadsheet | Text)
        }
        Command::Underline
        | Command::Strike
        | Command::Code
        | Command::PickFamily
        | Command::PickSize
        | Command::PickHighlight
        | Command::Title
        | Command::Subtitle
        | Command::Paragraph
        | Command::Heading1
        | Command::Heading2
        | Command::Heading3
        | Command::Outline
        | Command::BlockKindDialog
        | Command::InsertPicture
        | Command::InsertTable
        | Command::Bookmark
        | Command::ParagraphStyle
        | Command::ParagraphUp
        | Command::ParagraphDown
        | Command::ParagraphDelete => matches!(kind, Text),
        // The two New verbs and the way back to the welcome screen mean the same thing over either
        // document: they replace what the window is showing, and what it is showing now does not
        // change what they do.
        Command::NewSheet
        | Command::NewText
        | Command::Welcome
        | Command::Open
        | Command::Save
        | Command::SaveAs
        | Command::Exit
        | Command::Undo
        | Command::Redo
        | Command::Cut
        | Command::Copy
        | Command::Paste
        | Command::ClearCells
        | Command::GoTo
        // `App::project` and `App::lint` both reach either document type, so the two W6 verbs
        // that are not overlays follow the universal group rather than either specific one — and
        // so does the name overlay, since a bookmark is the text pane's own name anchor.
        | Command::ShowSource
        | Command::CheckDocument
        | Command::ToggleNames
        | Command::Shortcuts
        | Command::About => !matches!(kind, Presentation),
    }
}

/// One menu's items for this surface — a verb it has no answer for is **omitted**, not greyed,
/// and a separator left with nothing either side of it (because everything around it was dropped)
/// goes with it.
///
/// This replaced greying: a menu bar with every verb visible and half of them unclickable read
/// as a text pane that still thought it was a grid, since the sheet's own six verbs (`Sheet`'s
/// whole menu, `Recalculate`, `Cell Roles`) so outnumbered the universal ones that the &View
/// and &Sheet menus looked identical open on either pane. `Sheet`/`Data` on the text pane can
/// end up with nothing in them at all, which is
/// [`menu_has_items`]'s question, asked before a menu is put in the bar at all. The welcome screen
/// is the extreme case of the same rule: everything but `File` and `Help` empties out, and the
/// bar over it is those two.
pub fn items_for(menu: &Menu, surface: Surface) -> Vec<Item> {
    let mut items: Vec<Item> = Vec::with_capacity(menu.items.len());
    for item in menu.items {
        match item {
            Item::Verb { command, .. } if !applies(*command, surface) => continue,
            Item::Separator if matches!(items.last(), None | Some(Item::Separator)) => continue,
            other => items.push(*other),
        }
    }
    if matches!(items.last(), Some(Item::Separator)) {
        items.pop();
    }
    items
}

/// Whether a menu has anything left to show on this surface — a menu whose every verb
/// [`items_for`] dropped contributes nothing to the bar rather than an empty popup with only its
/// title.
pub fn menu_has_items(menu: &Menu, surface: Surface) -> bool {
    items_for(menu, surface)
        .iter()
        .any(|item| matches!(item, Item::Verb { .. }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn verbs() -> Vec<Command> {
        MENUS
            .iter()
            .flat_map(|menu| menu.items)
            .filter_map(|item| match item {
                Item::Verb { command, .. } => Some(*command),
                Item::Separator => None,
            })
            .collect()
    }

    /// Decision 4's rule made mechanical, and the half of it a Linux machine can check: a verb
    /// that is in `ALL` and in no menu is a verb nobody can find. The other half — that every
    /// command has a handler — is `win.rs`'s exhaustive match, which is the compiler's job.
    #[test]
    fn every_command_is_reachable_from_exactly_one_menu_item() {
        let verbs = verbs();
        for command in Command::ALL {
            let found = verbs.iter().filter(|c| *c == command).count();
            assert_eq!(found, 1, "{command:?} appears in {found} menu items");
        }
        assert_eq!(
            verbs.len(),
            Command::ALL.len(),
            "a menu item names a command that is not in ALL"
        );
    }

    /// The ids are derived, so this is really a check that nothing has started writing them
    /// down: two items sharing a `WM_COMMAND` id is the classic Win32 menu bug, and it looks
    /// like one item quietly doing the other's work.
    #[test]
    fn every_command_has_an_id_of_its_own_and_it_round_trips() {
        let mut seen = HashSet::new();
        for command in Command::ALL {
            let id = command.id();
            assert!(
                id >= FIRST_ID,
                "{command:?} would collide with a control id"
            );
            assert!(seen.insert(id), "{command:?} shares an id");
            assert_eq!(command_for(id), Some(*command));
        }
        assert_eq!(command_for(0), None, "a control notification is not a verb");
        assert_eq!(command_for(FIRST_ID - 1), None);
        assert_eq!(command_for(u16::MAX), None);
    }

    /// A Win32 menu underlines the letter after `&`, and a menu with two items claiming the
    /// same one in the same menu makes Alt-navigation ambiguous.
    ///
    /// Checked **per surface** — over the items [`items_for`] actually shows on each pane — since
    /// that is the only place two letters can collide: the grid's *Number Format* and the text
    /// pane's *Font* are never on one menu, and a whole-table check would force one of them onto
    /// a letter its own words do not have. Every item still needs a mnemonic on every surface it
    /// appears on.
    #[test]
    fn every_menu_has_distinct_mnemonics() {
        let mnemonic = |label: &str| {
            label
                .split('&')
                .nth(1)
                .and_then(|rest| rest.chars().next())
                .map(|c| c.to_ascii_uppercase())
        };
        let mut titles = HashSet::new();
        for menu in MENUS {
            let title = mnemonic(menu.title).unwrap_or_else(|| panic!("{}", menu.title));
            assert!(titles.insert(title), "two menus answer Alt+{title}");
            for surface in [
                Surface::Welcome,
                Surface::Document(grind_core::DocumentKind::Spreadsheet),
                Surface::Document(grind_core::DocumentKind::Text),
            ] {
                let mut keys = HashSet::new();
                for item in items_for(menu, surface) {
                    let Item::Verb { label, .. } = item else {
                        continue;
                    };
                    let key = mnemonic(label).unwrap_or_else(|| panic!("{label} has no mnemonic"));
                    assert!(
                        keys.insert(key),
                        "{} on {surface:?}: two items answer {key}",
                        menu.title
                    );
                }
            }
        }
    }

    /// An accelerator has to name a verb that is really in a menu, or the menu shows a key that
    /// does something the menu cannot.
    #[test]
    fn every_accelerator_names_a_command_that_is_in_a_menu() {
        let verbs = verbs();
        let ctrl = Mods {
            ctrl: true,
            ..Default::default()
        };
        let ctrl_shift = Mods {
            ctrl: true,
            shift: true,
            ..Default::default()
        };
        for (key, mods, want) in [
            (Key::Char('N'), ctrl, Command::NewSheet),
            (Key::Char('N'), ctrl_shift, Command::NewText),
            (Key::Char('O'), ctrl, Command::Open),
            (Key::Char('S'), ctrl, Command::Save),
            (Key::Char('S'), ctrl_shift, Command::SaveAs),
            (Key::Char('Z'), ctrl, Command::Undo),
            (Key::Char('Y'), ctrl, Command::Redo),
            (Key::Char('X'), ctrl, Command::Cut),
            (Key::Char('C'), ctrl, Command::Copy),
            (Key::Char('V'), ctrl, Command::Paste),
            (Key::PageDown, ctrl, Command::SheetNext),
            (Key::PageUp, ctrl, Command::SheetPrevious),
            (Key::F9, Mods::default(), Command::Recalculate),
            (Key::Char('B'), ctrl, Command::Bold),
            (Key::Char('I'), ctrl, Command::Italic),
            (Key::Char('U'), ctrl, Command::Underline),
            (Key::Char('0'), ctrl, Command::Paragraph),
            (Key::Char('1'), ctrl, Command::Heading1),
            (Key::Char('2'), ctrl, Command::Heading2),
            (Key::Char('3'), ctrl, Command::Heading3),
            (Key::Char('O'), ctrl_shift, Command::Outline),
            (Key::Char('K'), ctrl_shift, Command::BlockKindDialog),
            (Key::Char('U'), ctrl_shift, Command::ShowSource),
            (Key::Char('F'), ctrl_shift, Command::FunctionList),
            (Key::Char('E'), ctrl_shift, Command::ExplainFormula),
            (Key::Char('L'), ctrl_shift, Command::ToggleFilter),
            (Key::F8, Mods::default(), Command::CheckDocument),
        ] {
            assert_eq!(accelerator(key, mods), Some(want), "{key:?}");
            assert!(verbs.contains(&want), "{want:?} is in no menu");
        }
    }

    /// Alt belongs to the menu bar itself, so no accelerator may claim it — otherwise Alt+F
    /// would do something before the File menu ever opened.
    #[test]
    fn alt_is_left_to_the_menu_bar() {
        let alt = Mods {
            alt: true,
            ctrl: true,
            shift: false,
        };
        for key in [Key::Char('S'), Key::Char('N'), Key::F9, Key::PageDown] {
            assert_eq!(accelerator(key, alt), None, "{key:?}");
        }
    }

    /// A plain letter is text to type, not a command — the failure mode of an accelerator
    /// table that forgets to test its modifiers.
    #[test]
    fn an_unmodified_key_is_not_a_verb() {
        for key in [Key::Char('S'), Key::Char('N'), Key::Left, Key::PageDown] {
            assert_eq!(accelerator(key, Mods::default()), None, "{key:?}");
        }
    }

    /// A verb that means nothing anywhere would be dead weight in the menu; every one must apply
    /// to at least the sheet or the text pane, since those are the only two kinds this shell ever
    /// shows a `Pane` for.
    #[test]
    fn every_command_applies_to_at_least_one_document_kind() {
        use grind_core::DocumentKind::{Spreadsheet, Text};
        for command in Command::ALL {
            assert!(
                applies_to(*command, Spreadsheet) || applies_to(*command, Text),
                "{command:?} applies to neither document type"
            );
        }
    }

    /// Nothing applies to a presentation: this shell never holds a `Pane` for one, so a menu
    /// item that thought otherwise would be untestable by construction.
    #[test]
    fn nothing_applies_to_a_presentation() {
        for command in Command::ALL {
            assert!(!applies_to(
                *command,
                grind_core::DocumentKind::Presentation
            ));
        }
    }

    /// The Format menu's toggles, pickers and Clear, plus the outline dialog, are the text
    /// pane's and only the text pane's — the sheet has no `char_style` and no headings to grey
    /// them into meaning.
    #[test]
    fn formatting_and_the_outline_are_the_text_panes_alone() {
        use grind_core::DocumentKind::{Spreadsheet, Text};
        for command in [
            Command::Underline,
            Command::Strike,
            Command::Code,
            Command::PickFamily,
            Command::PickSize,
            Command::PickHighlight,
            Command::Title,
            Command::Subtitle,
            Command::Paragraph,
            Command::Heading1,
            Command::Heading2,
            Command::Heading3,
            Command::Outline,
            Command::BlockKindDialog,
            Command::InsertPicture,
            Command::InsertTable,
            Command::Bookmark,
            Command::ParagraphStyle,
            Command::ParagraphUp,
            Command::ParagraphDown,
            Command::ParagraphDelete,
        ] {
            assert!(applies_to(command, Text), "{command:?}");
            assert!(!applies_to(command, Spreadsheet), "{command:?}");
        }
    }

    /// Recalculate, the four sheet verbs and the three formula verbs are the grid's alone — the
    /// text pane has no sheets and no formulas.
    #[test]
    fn sheet_verbs_are_the_grids_alone() {
        use grind_core::DocumentKind::{Spreadsheet, Text};
        for command in [
            Command::Recalculate,
            Command::FillDown,
            Command::FillRight,
            Command::CopyValue,
            Command::FormulaToValue,
            Command::FillAcross,
            Command::InsertChart,
            Command::DeleteChart,
            Command::RestyleChart,
            Command::BordersAll,
            Command::BordersNone,
            Command::HideRows,
            Command::ShowRows,
            Command::HideColumns,
            Command::ShowColumns,
            Command::RowHeight,
            Command::FitColumns,
            Command::ColumnWidth,
            Command::DefineName,
            Command::DocumentLocale,
            Command::FindCalculation,
            Command::Evaluate,
            Command::FunctionList,
            Command::ExplainFormula,
            Command::ToggleFriendly,
            Command::SheetAdd,
            Command::SheetRename,
            Command::SheetDelete,
            Command::SheetNext,
            Command::SheetPrevious,
            // In the File menu beside the universal four, and still the grid's: a text
            // document has no cells for fields to land in.
            Command::ImportCsv,
            Command::ImportCsvWith,
            Command::ExportCsv,
            // A cell's currency: the text pane's runs have no number format.
            Command::CurrencyEuro,
            Command::CurrencyDollar,
            Command::CurrencyPound,
        ] {
            assert!(applies_to(command, Spreadsheet), "{command:?}");
            assert!(!applies_to(command, Text), "{command:?}");
        }
    }

    /// The role overlay is the grid's alone — the text pane has no per-character `CellRole`.
    #[test]
    fn the_role_overlay_is_the_grids_alone() {
        use grind_core::DocumentKind::{Spreadsheet, Text};
        assert!(applies_to(Command::ToggleRoles, Spreadsheet));
        assert!(!applies_to(Command::ToggleRoles, Text));
    }

    /// The name overlay reaches both: a defined name on the grid, a bookmark on the text pane.
    #[test]
    fn the_name_overlay_reaches_both_document_types() {
        use grind_core::DocumentKind::{Spreadsheet, Text};
        assert!(applies_to(Command::ToggleNames, Spreadsheet));
        assert!(applies_to(Command::ToggleNames, Text));
    }

    /// The source and the check are W6's two verbs that are not overlays, and `App::project` /
    /// `App::lint` reach both document types, so both panes get them.
    #[test]
    fn the_source_and_the_check_reach_both_document_types() {
        use grind_core::DocumentKind::{Spreadsheet, Text};
        for command in [Command::ShowSource, Command::CheckDocument] {
            assert!(applies_to(command, Spreadsheet), "{command:?}");
            assert!(applies_to(command, Text), "{command:?}");
        }
    }

    /// Every command in a menu has a label a context menu (W7) can borrow, and it is the same
    /// string the bar itself shows — there is no second copy for `label_for` to disagree with.
    #[test]
    fn every_menu_command_has_a_label_and_it_is_the_bars_own() {
        for menu in MENUS {
            for item in menu.items {
                let Item::Verb { command, label } = item else {
                    continue;
                };
                assert_eq!(label_for(*command), Some(*label), "{command:?}");
            }
        }
    }

    /// A command in no menu — there are none, `every_command_is_reachable_from_exactly_one_menu_item`
    /// already says so — would have no label; this is the same answer from the other function.
    #[test]
    fn every_command_has_a_label() {
        for command in Command::ALL {
            assert!(label_for(*command).is_some(), "{command:?}");
        }
    }

    /// The shortcuts list is one line per command that carries an accelerator, name and key both
    /// present and the mnemonic's `&` gone — a keyboard shortcuts window is not itself navigated
    /// by one.
    #[test]
    fn the_shortcuts_list_names_every_accelerator_and_drops_the_mnemonic() {
        let rows = shortcuts();
        assert!(!rows.is_empty());
        for row in &rows {
            assert!(!row.contains('&'), "{row}");
            assert!(row.contains(" — "), "{row}");
        }
        assert!(
            rows.iter()
                .any(|row| row.contains("Save") && row.contains("Ctrl+S"))
        );
        // A command with no accelerator — `SheetAdd` has none — contributes no row at all,
        // rather than one with an empty key.
        assert!(
            !rows.iter().any(|row| row.starts_with("Add")),
            "a menu item with no accelerator is not a shortcut"
        );
    }

    fn menu(title: &str) -> &'static Menu {
        MENUS
            .iter()
            .find(|menu| menu.title == title)
            .unwrap_or_else(|| panic!("no menu titled {title}"))
    }

    /// `Sheet` and `Data` are the grid's alone — every one of their verbs answers only to
    /// `Spreadsheet` — so a text document leaves both out of the bar rather than showing them
    /// with nothing clickable in them.
    #[test]
    fn sheet_and_data_vanish_on_the_text_pane() {
        use grind_core::DocumentKind::Text;
        assert!(!menu_has_items(menu("&Sheet"), Surface::Document(Text)));
        assert!(!menu_has_items(menu("&Data"), Surface::Document(Text)));
        assert!(items_for(menu("&Sheet"), Surface::Document(Text)).is_empty());
        assert!(items_for(menu("&Data"), Surface::Document(Text)).is_empty());
    }

    /// `Format` over the grid is exactly its strip's verbs in the strip's order, plus the three
    /// currencies beside the number format — and over the text pane none of the grid's own.
    #[test]
    fn format_on_the_grid_is_its_strip() {
        use grind_core::DocumentKind::{Spreadsheet, Text};
        let over = |kind| {
            items_for(menu("F&ormat"), Surface::Document(kind))
                .into_iter()
                .filter_map(|item| match item {
                    Item::Verb { command, .. } => Some(command),
                    Item::Separator => None,
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            over(Spreadsheet),
            [
                Command::Bold,
                Command::Italic,
                Command::AlignLeft,
                Command::AlignCenter,
                Command::AlignRight,
                Command::PickColor,
                Command::WrapText,
                Command::BordersAll,
                Command::BordersNone,
                Command::PickBackground,
                Command::NumberFormat,
                Command::FewerDecimals,
                Command::MoreDecimals,
                Command::CurrencyEuro,
                Command::CurrencyDollar,
                Command::CurrencyPound,
                Command::ClearFormatting,
            ]
        );
        let text = over(Text);
        assert!(!text.iter().any(|c| c.currency().is_some()));
        for grid_only in [
            Command::AlignLeft,
            Command::PickBackground,
            Command::NumberFormat,
            Command::MoreDecimals,
        ] {
            assert!(!text.contains(&grid_only), "{grid_only:?}");
        }
    }

    /// Find, the two steps and Replace: cells on the grid, text on the page.
    #[test]
    fn find_and_replace_reach_both_document_types() {
        use grind_core::DocumentKind::{Spreadsheet, Text};
        for command in [
            Command::Find,
            Command::FindNext,
            Command::FindPrevious,
            Command::Replace,
        ] {
            assert!(applies_to(command, Text), "{command:?}");
            assert!(applies_to(command, Spreadsheet), "{command:?}");
        }
    }

    /// The four verbs both panes answer, each over its own style.
    #[test]
    fn bold_italic_colour_and_clear_are_both_panes() {
        use grind_core::DocumentKind::{Spreadsheet, Text};
        for command in [
            Command::Bold,
            Command::Italic,
            Command::PickColor,
            Command::ClearFormatting,
        ] {
            assert!(applies_to(command, Text), "{command:?}");
            assert!(applies_to(command, Spreadsheet), "{command:?}");
        }
    }

    #[test]
    fn every_currency_item_names_the_currency_it_writes() {
        for (index, command) in Command::CURRENCIES.iter().enumerate() {
            assert_eq!(command.currency(), Some(index));
            let (symbol, _) = grind_sheet::numfmt::CURRENCIES[index];
            let label = label_for(*command).unwrap();
            assert!(label.contains(&format!("({symbol})")), "{label}");
        }
        assert_eq!(Command::Bold.currency(), None);
        assert_eq!(
            grind_sheet::numfmt::CURRENCIES.len(),
            Command::CURRENCIES.len()
        );
    }

    /// `Edit` mixes universal verbs with `Outline`, which is the text pane's alone — so the
    /// menu survives on the grid, minus that one item, rather than vanishing or keeping a verb
    /// with nothing to do.
    #[test]
    fn edit_loses_only_outline_on_the_grid() {
        use grind_core::DocumentKind::Spreadsheet;
        assert!(menu_has_items(
            menu("&Edit"),
            Surface::Document(Spreadsheet)
        ));
        let items = items_for(menu("&Edit"), Surface::Document(Spreadsheet));
        assert!(!items.iter().any(|item| matches!(
            item,
            Item::Verb {
                command: Command::Outline,
                ..
            }
        )));
        assert!(items.iter().any(|item| matches!(
            item,
            Item::Verb {
                command: Command::GoTo,
                ..
            }
        )));
    }

    /// `View` mixes a grid-only toggle (`Cell Roles`) with three universal verbs, so it survives
    /// on the text pane minus that one item — the same shape `Edit` has on the grid.
    #[test]
    fn view_loses_only_cell_roles_on_the_text_pane() {
        use grind_core::DocumentKind::Text;
        assert!(menu_has_items(menu("&View"), Surface::Document(Text)));
        let items = items_for(menu("&View"), Surface::Document(Text));
        assert!(!items.iter().any(|item| matches!(
            item,
            Item::Verb {
                command: Command::ToggleRoles,
                ..
            }
        )));
        assert!(items.iter().any(|item| matches!(
            item,
            Item::Verb {
                command: Command::ToggleNames,
                ..
            }
        )));
    }

    /// Never a leading, trailing, or doubled separator — the visible cost of filtering a menu's
    /// items by hand, and the reason `items_for` cleans them up rather than leaving them for
    /// `AppendMenuW` to draw as dead space.
    #[test]
    fn filtering_never_leaves_a_stray_separator() {
        use grind_core::DocumentKind::{Spreadsheet, Text};
        for menu in MENUS {
            for kind in [Spreadsheet, Text] {
                let items = items_for(menu, Surface::Document(kind));
                assert_ne!(
                    items.first(),
                    Some(&Item::Separator),
                    "{}: {kind:?}",
                    menu.title
                );
                assert_ne!(
                    items.last(),
                    Some(&Item::Separator),
                    "{}: {kind:?}",
                    menu.title
                );
                assert!(
                    !items
                        .windows(2)
                        .any(|pair| pair == [Item::Separator, Item::Separator]),
                    "{}: {kind:?} has two separators in a row",
                    menu.title
                );
            }
        }
    }

    /// A menu that keeps every one of its items for a kind is unchanged, order included — the
    /// baseline `filtering_never_leaves_a_stray_separator` and the vanish/survive tests above
    /// all lean on.
    ///
    /// `Help` rather than `File`, which used to be the example here: File carries the two CSV
    /// verbs now, and they are the grid's, so it is no longer a menu with nothing to drop over
    /// a text document. Help is — both of its items are about the *window*.
    #[test]
    fn a_menu_with_nothing_to_drop_is_returned_whole() {
        use grind_core::DocumentKind::{Spreadsheet, Text};
        for kind in [Spreadsheet, Text] {
            assert_eq!(
                items_for(menu("&Help"), Surface::Document(kind)),
                menu("&Help").items.to_vec(),
                "{kind:?}"
            );
        }
    }

    /// And the one File loses: importing fields into cells and writing a range out are both the
    /// grid's, so a text document's File menu is the file verbs and Exit — with no stray
    /// separator left where the pair was (`filtering_never_leaves_a_stray_separator` is the
    /// general rule; this is the case that first exercised it in *this* menu).
    #[test]
    fn the_csv_pair_is_the_grids_and_leaves_the_file_menu_over_a_document_of_text() {
        use grind_core::DocumentKind::{Spreadsheet, Text};
        let over = |kind| {
            items_for(menu("&File"), Surface::Document(kind))
                .into_iter()
                .filter_map(|item| match item {
                    Item::Verb { command, .. } => Some(command),
                    Item::Separator => None,
                })
                .collect::<Vec<_>>()
        };
        assert!(over(Spreadsheet).contains(&Command::ImportCsv));
        assert!(over(Spreadsheet).contains(&Command::ExportCsv));
        assert!(!over(Text).contains(&Command::ImportCsv));
        assert!(!over(Text).contains(&Command::ExportCsv));
    }

    /// The welcome screen's bar is `File` and `Help` and nothing else — every other menu is made
    /// of verbs that act on a document, and there is none.
    #[test]
    fn the_start_screen_keeps_only_file_and_help() {
        let kept: Vec<&str> = MENUS
            .iter()
            .filter(|menu| menu_has_items(menu, Surface::Welcome))
            .map(|menu| menu.title)
            .collect();
        assert_eq!(kept, vec!["&File", "&Help"]);
    }

    /// What `File` offers with nothing open: make one of each, find one, leave. Save, Save As and
    /// the way back to the screen you are already on are gone — `items_for` drops them, and the
    /// separator that used to sit between Open and Save goes with them.
    #[test]
    fn file_offers_only_the_verbs_that_need_no_document() {
        let items = items_for(menu("&File"), Surface::Welcome);
        let verbs: Vec<Command> = items
            .iter()
            .filter_map(|item| match item {
                Item::Verb { command, .. } => Some(*command),
                Item::Separator => None,
            })
            .collect();
        assert_eq!(
            verbs,
            vec![
                Command::NewSheet,
                Command::NewText,
                Command::Open,
                Command::Exit
            ]
        );
        assert_ne!(items.first(), Some(&Item::Separator));
        assert_ne!(items.last(), Some(&Item::Separator));
    }

    /// Nothing that reads or writes a document is offered when there is none. The failure this
    /// guards is the greying problem in its first form: a Save item on a window with nothing to
    /// save, which W5b's own history says reads as a shell that has not noticed where it is.
    #[test]
    fn no_verb_that_needs_a_document_is_offered_without_one() {
        for command in [
            Command::Save,
            Command::SaveAs,
            Command::Undo,
            Command::Redo,
            Command::Cut,
            Command::Copy,
            Command::Paste,
            Command::GoTo,
            Command::Recalculate,
            Command::ShowSource,
            Command::CheckDocument,
            Command::Welcome,
        ] {
            assert!(!applies(command, Surface::Welcome), "{command:?}");
        }
    }

    /// And the four that do mean something are exactly the three the welcome screen draws as cards
    /// plus the two a window always has. `welcome.rs` names the cards; this is the menu bar
    /// agreeing with it.
    #[test]
    fn the_start_screens_own_verbs_are_offered() {
        for command in [
            Command::NewSheet,
            Command::NewText,
            Command::Open,
            Command::Exit,
            Command::Shortcuts,
            Command::About,
        ] {
            assert!(applies(command, Surface::Welcome), "{command:?}");
        }
    }

    /// The separator cleanup holds on the welcome screen too, where more items are dropped than on
    /// either document — which is where a leading or doubled separator would show up first.
    #[test]
    fn the_start_screen_leaves_no_stray_separator() {
        for menu in MENUS {
            let items = items_for(menu, Surface::Welcome);
            assert_ne!(items.first(), Some(&Item::Separator), "{}", menu.title);
            assert_ne!(items.last(), Some(&Item::Separator), "{}", menu.title);
            assert!(
                !items
                    .windows(2)
                    .any(|pair| pair == [Item::Separator, Item::Separator]),
                "{}",
                menu.title
            );
        }
    }
}
