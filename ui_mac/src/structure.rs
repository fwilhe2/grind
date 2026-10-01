// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The structural verbs — Fill Down and Right, hiding, showing and sizing tracks, a name for the
//! selection, Export as CSV, and on a page a table and a bookmark. Each is one core call over
//! what `sheet/verbs.rs` reads off the selection, with a question asked through `prompt.rs`
//! where the verb needs an answer first.

use grind_sheet::{Pos, RecalcMode};
use objc2::MainThreadMarker;
use objc2_app_kit::{NSModalResponseOK, NSSavePanel};
use objc2_foundation::NSString;

use crate::grid_view::Pane;
use crate::menu::{Command, Track};
use crate::page_view::TextPane;
use crate::prompt;
use crate::sheet::verbs;
use crate::text::state::table_size;

impl Pane {
    /// One of the grid's structural verbs.
    pub fn structure(&self, command: Command, mtm: MainThreadMarker) {
        let (sheet, selection) = (self.sheet.get(), self.selection.get());
        let done =
            match command {
                // ponytail: a fill over several columns is one `App::fill` each, so as many ⌘Z as
                // columns. `fill` takes one source; the trigger is a user filling a wide block.
                Command::Fill(down) => verbs::fills(selection, down).into_iter().try_for_each(
                    |(source, start, end)| {
                        self.app
                            .fill(sheet, source, start, end, RecalcMode::Document)
                            .map(|_| ())
                    },
                ),
                Command::Rows(Track::Hide) => self
                    .app
                    .set_row_hidden(sheet, verbs::rows(selection), true)
                    .map(|_| ()),
                Command::Rows(Track::Show) => self
                    .app
                    .set_row_hidden(sheet, verbs::rows(selection), false)
                    .map(|_| ()),
                Command::Columns(Track::Hide) => self
                    .app
                    .set_col_hidden(sheet, verbs::cols(selection), true)
                    .map(|_| ()),
                Command::Columns(Track::Show) => self
                    .app
                    .set_col_hidden(sheet, verbs::cols(selection), false)
                    .map(|_| ()),
                Command::Rows(Track::Size) | Command::Columns(Track::Size) => {
                    let rows = matches!(command, Command::Rows(_));
                    let Some(size) = prompt::ask(
                        mtm,
                        if rows { "Row Height" } else { "Column Width" },
                        "A length, such as 2.5cm, 1in or 64pt.",
                        "Set",
                        "",
                    ) else {
                        return;
                    };
                    match rows {
                        true => self
                            .app
                            .set_row_height(sheet, verbs::rows(selection), Some(size)),
                        false => self
                            .app
                            .set_col_width(sheet, verbs::cols(selection), Some(size)),
                    }
                    .map(|_| ())
                }
                Command::DefineName => {
                    let target = verbs::name_target(&self.sheet_name(), selection);
                    let Some(name) = prompt::ask(
                        mtm,
                        "Define Name",
                        &format!("A name for {target}, to use in formulas instead of its address."),
                        "Define",
                        "",
                    ) else {
                        return;
                    };
                    grind_sheet::a1::definition(&self.app, &target)
                        .and_then(|expression| self.app.set_name(&name, &expression))
                }
                Command::ExportCsv => {
                    self.export_csv(mtm);
                    return;
                }
                _ => return,
            };
        match done {
            Ok(()) => self.say(None),
            Err(error) => self.say(Some((&error.to_string(), None))),
        }
    }

    /// File ▸ Export as CSV…: the sheet showing, everything it uses, as comma-separated values
    /// in a file the save panel names — the cells' shown values, as `grind sheet export-csv`
    /// writes them by default.
    fn export_csv(&self, mtm: MainThreadMarker) {
        let sheet = self.sheet.get();
        let panel = NSSavePanel::savePanel(mtm);
        panel.setNameFieldStringValue(&NSString::from_str(&format!("{}.csv", self.sheet_name())));
        if panel.runModal() != NSModalResponseOK {
            return;
        }
        let Some(path) = panel.URL().and_then(|url| url.to_file_path()) else {
            return;
        };
        // One past the last used row and column — and an empty sheet is an empty file, as the
        // CLI writes one, rather than one blank field.
        let (rows, cols) = self.app.used_extent(sheet).unwrap_or((0, 0));
        let text = match rows == 0 || cols == 0 {
            true => Ok(String::new()),
            false => self.app.export_csv(
                sheet,
                Pos::new(0, 0),
                Pos::new(rows - 1, cols - 1),
                &Default::default(),
            ),
        };
        let written = text.map_err(|error| error.to_string()).and_then(|text| {
            std::fs::write(&path, text).map_err(|error| format!("{}: {error}", path.display()))
        });
        if let Err(message) = written {
            self.say(Some((&message, None)));
        }
    }
}

impl TextPane {
    /// One of the page's structural verbs: a table after the caret's block, or a bookmark on it.
    pub fn structure(&self, command: Command, mtm: MainThreadMarker) {
        let block = self.state.borrow().caret.block;
        let done = match command {
            Command::InsertTable => {
                let Some(answer) = prompt::ask(
                    mtm,
                    "Insert Table",
                    "Columns by rows, such as 3x2.",
                    "Insert",
                    "3x2",
                ) else {
                    return;
                };
                let Some((columns, rows)) = table_size(&answer) else {
                    prompt::tell(
                        mtm,
                        "That is not a table's size.",
                        "Columns by rows, such as 3x2.",
                    );
                    return;
                };
                self.app.insert_table(block + 1, rows, columns, None)
            }
            Command::InsertBookmark => {
                let Some(name) = prompt::ask(
                    mtm,
                    "Insert Bookmark",
                    "A name for this paragraph, so #name reaches it from anywhere.",
                    "Insert",
                    "",
                ) else {
                    return;
                };
                self.app.set_bookmark(&name, Some(block)).map(|_| ())
            }
            _ => return,
        };
        if let Err(error) = done {
            prompt::tell(mtm, "That could not be done.", &error.to_string());
        }
    }
}
