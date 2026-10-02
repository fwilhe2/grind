// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The structural verbs — Fill Down and Right, hiding, showing and sizing tracks, a name for the
//! selection, Export as CSV, and on a page a table and a bookmark. Each is one core call over
//! what `sheet/verbs.rs` reads off the selection, with a question asked through `prompt.rs`
//! where the verb needs an answer first.

use grind_sheet::style::mm_length;
use grind_sheet::{Pos, RecalcMode};
use objc2::MainThreadMarker;
use objc2_app_kit::{NSModalResponseOK, NSOpenPanel, NSSavePanel};
use objc2_foundation::NSString;

use crate::grid_view::Pane;
use crate::menu::{Command, Track};
use crate::page_view::TextPane;
use crate::prompt;
use crate::sheet::chart;
use crate::sheet::filter;
use crate::sheet::geom::{HEADER_H, HEADER_W, PT_PER_MM};
use crate::sheet::verbs;
use crate::text::picture;
use crate::text::state::table_size;

/// A new chart's size — the GNOME window's — and its gap from the table it charts.
const CHART_WIDTH: &str = "12cm";
const CHART_HEIGHT: &str = "7.5cm";
const CHART_MARGIN_MM: f64 = 6.0;

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
                Command::InsertChart => self.insert_chart(),
                Command::CopyValue => {
                    self.copy_value();
                    return;
                }
                Command::FormulaToValue => {
                    // ponytail: one `clear_formula` per formula, so as many ⌘Z as formulas — the
                    // core has no range form, and the trigger is a user converting a large block.
                    let used = self.app.used_extent(sheet).unwrap_or((0, 0));
                    let (start, end) = grind_sheet::nav::target(selection, used);
                    (start.row..=end.row)
                        .flat_map(|row| (start.col..=end.col).map(move |col| Pos::new(row, col)))
                        .filter(|pos| self.app.formula(sheet, *pos).is_ok_and(|f| f.is_some()))
                        .try_for_each(|pos| self.app.clear_formula(sheet, pos))
                }
                Command::Filter => match self.app.filter(sheet).ok().flatten() {
                    Some(_) => self.app.set_filter(sheet, None),
                    None => {
                        let used = self.app.used_extent(sheet).unwrap_or((0, 0));
                        match filter::range(selection.rect(), used) {
                            Ok((start, end)) => self.app.set_filter(
                                sheet,
                                Some(grind_sheet::Filter::new(filter::NAME, start, end)),
                            ),
                            Err(why) => {
                                self.say(Some((why, None)));
                                return;
                            }
                        }
                    }
                },
                _ => return,
            };
        match done {
            Ok(()) => self.say(None),
            Err(error) => self.say(Some((&error.to_string(), None))),
        }
    }

    /// Insert ▸ Chart: the table the selection is in, read the way `chart-add --from` reads one
    /// (`App::suggest_chart` — which way the series run, what names them, and what kind of chart
    /// the cells want), placed beside that table at the GNOME window's size, and brought into
    /// sight. One undo step; there is no dialog, and changing the chart is the CLI's.
    fn insert_chart(&self) -> grind_sheet::Result<()> {
        let sheet = self.sheet.get();
        let (start, end) = self.selection.get().rect();
        let guessed = self.app.suggest_chart(sheet, start, end, None)?;
        let spec = grind_sheet::ChartSpec {
            legend: guessed.spec.default_legend(),
            ..guessed.spec
        };
        let (x, y) = {
            let grid = self.grid.borrow();
            let x = grid.cols.offset_of(guessed.end.col + 1) / PT_PER_MM + CHART_MARGIN_MM;
            let y = grid.rows.offset_of(guessed.start.row) / PT_PER_MM;
            (mm_length(x), mm_length(y))
        };
        self.app
            .add_chart(sheet, &spec, &x, &y, CHART_WIDTH, CHART_HEIGHT)?;
        let added = self
            .app
            .charts(sheet)
            .ok()
            .and_then(|charts| charts.last().and_then(chart::frame_of));
        if let (Some(frame), Some(view)) = (added, self.grid_view()) {
            view.scrollRectToVisible(crate::grid_view::ns_rect(frame.offset(HEADER_W, HEADER_H)));
        }
        Ok(())
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
    /// One of the page's structural verbs: a table after the caret's block, a bookmark on it, or
    /// a picture.
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
            Command::InsertPicture => return self.insert_picture(mtm),
            _ => return,
        };
        if let Err(error) = done {
            prompt::tell(mtm, "That could not be done.", &error.to_string());
        }
    }
}

impl TextPane {
    /// Insert ▸ Picture…: a file the open panel names, embedded in the document — a picture is
    /// its bytes, never a link to them — in a paragraph of its own: the caret's, when that one is
    /// empty, and otherwise a new one after it. The caret lands just past the picture, so what is
    /// typed next is its caption, which is `ui_text_gtk`'s rule.
    fn insert_picture(&self, mtm: MainThreadMarker) {
        let panel = NSOpenPanel::openPanel(mtm);
        panel.setCanChooseDirectories(false);
        panel.setAllowsMultipleSelection(false);
        panel.setPrompt(Some(&NSString::from_str("Insert")));
        if panel.runModal() != NSModalResponseOK {
            return;
        }
        let Some(path) = panel.URL().and_then(|url| url.to_file_path()) else {
            return;
        };
        let data = match std::fs::read(&path) {
            Ok(data) => data,
            Err(error) => {
                return prompt::tell(
                    mtm,
                    "That file could not be read.",
                    &format!("{}: {error}", path.display()),
                );
            }
        };
        let Some(mime) = picture::mime(&data) else {
            return prompt::tell(
                mtm,
                "That file is not a picture.",
                "A PNG, JPEG, GIF, TIFF, WebP, HEIC, BMP or SVG file can be inserted.",
            );
        };
        let block = self.state.borrow().caret.block;
        let empty = self.app.input_text(block).is_ok_and(|text| text.is_empty());
        let at = match empty {
            true => Ok(block),
            false => self
                .app
                .insert(block + 1, grind_text::BlockKind::Paragraph, "")
                .map(|_| block + 1),
        };
        let done = at.and_then(|at| {
            let caret = grind_text::Caret {
                block: at,
                offset: 0,
            };
            self.app
                .insert_image(caret, mime.to_owned(), data, None, None)
                .map(|()| at)
        });
        match done {
            Ok(at) => self.go_to(grind_text::Caret {
                block: at,
                offset: 1,
            }),
            Err(error) => prompt::tell(mtm, "That could not be done.", &error.to_string()),
        }
    }
}
