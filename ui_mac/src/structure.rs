// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The structural verbs — Fill Down and Right, hiding, showing and sizing tracks, a name for the
//! selection, Export as CSV, and on a page a table and a bookmark. Each is one core call over
//! what `sheet/verbs.rs` reads off the selection, with a question asked through `prompt.rs`
//! where the verb needs an answer first.

use grind_sheet::{Pos, RecalcMode};
use objc2::MainThreadMarker;
use objc2_app_kit::{NSModalResponseOK, NSOpenPanel, NSSavePanel};
use objc2_foundation::NSString;

use crate::grid_view::Pane;
use crate::menu::{Command, Track};
use crate::notice;
use crate::page_view::TextPane;
use crate::places::NameVerb;
use crate::prompt;
use crate::sheet::chart;
use crate::sheet::filter;
use crate::sheet::geom::{HEADER_H, HEADER_W, PT_PER_MM};
use crate::sheet::verbs;
use crate::text::picture;
use crate::text::state::table_size;

impl Pane {
    /// One of the grid's structural verbs.
    pub fn structure(&self, command: Command, mtm: MainThreadMarker) {
        let (sheet, selection) = (self.sheet.get(), self.selection.get());
        let done = match command {
            // ponytail: a fill over several columns is one `App::fill` each, so as many ⌘Z as
            // columns. `fill` takes one source; the trigger is a user filling a wide block.
            Command::Fill(down) => grind_sheet::nav::fills(selection, down)
                .into_iter()
                .try_for_each(|(source, start, end)| {
                    self.app
                        .fill(sheet, source, start, end, RecalcMode::Document)
                        .map(|_| ())
                }),
            // The cell the selection grew from into the whole selection — one `App::fill`, one undo step.
            Command::FillAcross => {
                let (start, end) = selection.rect();
                self.app
                    .fill(sheet, selection.anchor, start, end, RecalcMode::Document)
                    .map(|_| ())
            }
            // A whole row or column is cut to the part in use first (`nav::target`).
            Command::Merge(merge) => {
                let used = self.app.used_extent(sheet).unwrap_or((0, 0));
                let (start, end) = grind_sheet::nav::target(selection, used);
                match merge {
                    true => self.app.merge(sheet, start, end).map(|_| ()),
                    false => self.app.unmerge(sheet, start, end).map(|_| ()),
                }
            }
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
            // A row given back to its content is a row without a height of its own.
            Command::Rows(Track::Fit) => self
                .app
                .set_row_height(sheet, verbs::rows(selection), None)
                .map(|_| ()),
            // Only the columns in use: a whole row selected is sixteen thousand of them, and
            // past the used extent there is no text to fit to (`fit::columns_in_use`).
            Command::Columns(Track::Fit) => {
                let cols =
                    grind_sheet::fit::columns_in_use(&self.app, sheet, verbs::cols(selection));
                self.fit(cols, 0..0)
            }
            Command::FitAll => {
                self.fit(grind_sheet::fit::all_columns(&self.app, sheet), 0..u32::MAX)
            }
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
            Command::AddRule => {
                use grind_sheet::rule::{self, Look};
                let (start, end) = selection.rect();
                let range = rule::range_hint(start, end);
                let Some(condition) = prompt::ask(
                    mtm,
                    "Add Conditional Format",
                    &format!(
                        "Draw {range} differently where this formula is true, written for its \
                         first cell."
                    ),
                    "Next",
                    &rule::condition_hint(start),
                ) else {
                    return;
                };
                let looks: Vec<String> = Look::ALL
                    .iter()
                    .map(|look| look.label().to_owned())
                    .collect();
                let Some(look) = prompt::pick(
                    mtm,
                    "Draw It With",
                    "Drawn over the cell's own formatting while the condition holds.",
                    "Add",
                    &looks,
                    0,
                )
                .and_then(|picked| Look::ALL.get(picked).copied()) else {
                    return;
                };
                if let Err(why) = rule::add_from_input(&self.app, sheet, &range, &condition, look) {
                    prompt::tell(mtm, "That rule cannot be added.", &why);
                    return;
                }
                Ok(())
            }
            Command::RemoveRule => {
                let rules = self.app.rules(sheet).unwrap_or_default();
                if rules.is_empty() {
                    prompt::tell(
                        mtm,
                        "This sheet has no conditional formats.",
                        "Format ▸ Conditional Formatting ▸ Add Rule… makes one.",
                    );
                    return;
                }
                let (start, end) = selection.rect();
                let rows: Vec<String> = rules
                    .iter()
                    .enumerate()
                    .map(|(index, r)| {
                        format!(
                            "{}. {}",
                            index + 1,
                            grind_sheet::rule::summary(r).replace('\t', "  ")
                        )
                    })
                    .collect();
                let initial = rules
                    .iter()
                    .position(|r| r.touches(start, end))
                    .unwrap_or(0);
                let Some(picked) = prompt::pick(
                    mtm,
                    "Remove Conditional Format",
                    "Rules are tried in this order; the first that holds at a cell is drawn.",
                    "Remove",
                    &rows,
                    initial,
                ) else {
                    return;
                };
                self.app.remove_rule(sheet, picked).map(|_| ())
            }
            Command::ExportCsv => {
                self.export_csv(mtm);
                return;
            }
            Command::ImportCsv => {
                self.import_csv(mtm, false);
                return;
            }
            Command::ImportCsvWith => {
                self.import_csv(mtm, true);
                return;
            }
            Command::InsertChart(kind) => self.insert_chart(kind),
            Command::PreviewChart => {
                let (start, end) = selection.rect();
                let preview =
                    |kind| verbs::preview_insert_chart(&self.app, sheet, start, end, kind);
                let pictures = preview(None).and_then(|(guessed, _)| {
                    let pictures = grind_sheet::ChartKind::ALL
                        .into_iter()
                        .map(|kind| preview(Some(kind)).map(|(chart, data)| (kind, chart, data)))
                        .collect::<grind_sheet::Result<Vec<_>>>()?;
                    let shown = grind_sheet::ChartKind::ALL
                        .iter()
                        .position(|kind| *kind == guessed.kind)
                        .unwrap_or(0);
                    Ok((pictures, shown))
                });
                match pictures {
                    Ok((pictures, shown)) => {
                        match crate::chart_preview::choose(mtm, &pictures, shown) {
                            Some(kind) => self.insert_chart(Some(kind)),
                            None => Ok(()),
                        }
                    }
                    Err(error) => Err(error),
                }
            }
            Command::Evaluate => {
                let at = selection.active;
                let Some(typed) = prompt::ask(
                    mtm,
                    "Evaluate",
                    &format!(
                        "A formula, worked out at {} without storing it.",
                        grind_sheet::a1::format(None, at)
                    ),
                    "Evaluate",
                    "=",
                ) else {
                    return;
                };
                match verbs::evaluated(&self.app, sheet, at, &typed) {
                    Ok(value) => prompt::tell(mtm, &value, typed.trim()),
                    Err(why) => prompt::tell(mtm, "That cannot be worked out.", &why),
                }
                return;
            }
            Command::DocumentLocale => {
                let now = self
                    .app
                    .locale()
                    .map(|locale| locale.tag())
                    .unwrap_or_default();
                let Some(typed) = prompt::ask(
                    mtm,
                    "Document Locale",
                    "How this document writes numbers and reads them typed: a tag such as \
                         en-US, de-DE or fr-FR, or nothing for this Mac's own.",
                    "Set",
                    &now,
                ) else {
                    return;
                };
                match verbs::locale(&typed) {
                    Ok(locale) => self.app.set_locale(locale),
                    Err(_) => {
                        prompt::tell(mtm, "That is not a locale.", "A tag such as de-DE.");
                        return;
                    }
                }
            }
            // On: asked what to search for, listed in the sidebar. On already: off.
            Command::Calculations => {
                if self.calculations.borrow().is_some() {
                    self.show_calculations(None);
                    return;
                }
                let Some(needle) = prompt::ask(
                    mtm,
                    "Calculations",
                    "Every formula in the document, listed in the sidebar. Search for a \
                         function, an address or a piece of a formula — or leave it empty for \
                         all of them.",
                    "Show",
                    "",
                ) else {
                    return;
                };
                self.show_calculations(Some(needle.trim().to_owned()));
                return;
            }
            Command::CopyValue => {
                self.copy_value();
                return;
            }
            Command::FormulaToValue => {
                let used = self.app.used_extent(sheet).unwrap_or((0, 0));
                let (start, end) = grind_sheet::nav::target(selection, used);
                verbs::formulas_to_values(&self.app, sheet, start, end).map(|_| ())
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
    /// sight — as `kind` when one was chosen (Insert ▸ Bar/Line/Pie Chart, or the preview's
    /// control). One undo step; a chart's own menu changes it afterwards.
    fn insert_chart(&self, kind: Option<grind_sheet::ChartKind>) -> grind_sheet::Result<()> {
        let sheet = self.sheet.get();
        let (start, end) = self.selection.get().rect();
        verbs::insert_chart(&self.app, sheet, start, end, kind, |col, row| {
            let grid = self.grid.borrow();
            (
                grid.cols.offset_of(col) / PT_PER_MM,
                grid.rows.offset_of(row) / PT_PER_MM,
            )
        })?;
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

    /// File ▸ Import CSV…: a delimited file read into this sheet at the active cell, its
    /// delimiter sniffed and each field read as if typed (`csv::Import::sniffed`, the one answer
    /// every window gives), in one undo step. Not UTF-8 is refused with `csv::NOT_UTF8`'s words.
    fn import_csv(&self, mtm: MainThreadMarker, with_options: bool) {
        let panel = NSOpenPanel::openPanel(mtm);
        panel.setCanChooseDirectories(false);
        panel.setAllowsMultipleSelection(false);
        panel.setPrompt(Some(&NSString::from_str("Import")));
        if panel.runModal() != NSModalResponseOK {
            return;
        }
        let Some(path) = panel.URL().and_then(|url| url.to_file_path()) else {
            return;
        };
        let text = std::fs::read(&path)
            .map_err(|error| error.to_string())
            .and_then(|bytes| grind_sheet::csv::decode(bytes).map_err(str::to_owned));
        let text = match text {
            Ok(text) => text,
            Err(why) => {
                return prompt::tell(
                    mtm,
                    "That file could not be imported.",
                    &format!("{}: {why}", path.display()),
                );
            }
        };
        // The options in words (`csv::Import::amended`), asked after the file so the panel that
        // chose it is not left open under a question.
        let words = match with_options {
            true => {
                let Some(words) = prompt::ask(
                    mtm,
                    "Import Options",
                    "delimiter=semicolon locale=de-DE text formulas trim no-dates — or leave it empty to let the file decide.",
                    "Import",
                    "",
                ) else {
                    return;
                };
                words
            }
            false => String::new(),
        };
        let at = self.selection.get().active;
        let options = match grind_sheet::csv::Import::sniffed(&text).amended(&words) {
            Ok(options) => options,
            Err(why) => return prompt::tell(mtm, "Those options cannot be used.", &why),
        };
        match self
            .app
            .import_csv(self.sheet.get(), at, &text, &options, RecalcMode::Document)
        {
            Ok(outcome) => self.say(Some((&notice::imported(outcome.cells, at), None))),
            Err(error) => prompt::tell(mtm, "That file could not be imported.", &error.to_string()),
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
            // The name picks the delimiter — `.tsv` is tabs — as every other window's save does.
            false => self.app.export_csv(
                sheet,
                Pos::new(0, 0),
                Pos::new(rows - 1, cols - 1),
                &grind_sheet::csv::Export {
                    dialect: grind_sheet::csv::Dialect::for_name(&path.display().to_string()),
                    ..Default::default()
                },
            ),
        };
        let written = text.map_err(|error| error.to_string()).and_then(|text| {
            grind_core::atomic::write(&path, text)
                .map_err(|error| format!("{}: {error}", path.display()))
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
            Command::ImportMarkdown => return self.import_markdown(mtm),
            Command::ExportMarkdown => return self.export_markdown(mtm),
            Command::ExportPdf => return self.export_pdf(mtm),
            Command::Print => return self.print(mtm),
            Command::MoveParagraph(up) => {
                return self.act_on(|page, app, _| page.move_paragraphs(app, up));
            }
            Command::DeleteParagraph => {
                return self.act_on(|page, app, _| page.delete_paragraphs(app));
            }
            // A name this build keeps and does not interpret, unless it is Title or Subtitle
            // (`doc/text-core.md`'s Styles section) — `grind text style`'s verb.
            Command::ParagraphStyle => {
                let blocks = self.state.borrow().blocks();
                let now = self
                    .app
                    .get_viewport(*blocks.start()..*blocks.start() + 1)
                    .get(*blocks.start())
                    .and_then(|view| view.style.clone())
                    .unwrap_or_default();
                let Some(name) = prompt::ask(
                    mtm,
                    "Paragraph Style",
                    "A named style for the selected paragraphs, as LibreOffice names them — \
                     Quotations, Text Body — or nothing to take it off.",
                    "Set",
                    &now,
                ) else {
                    return;
                };
                let name = Some(name.trim().to_owned()).filter(|name| !name.is_empty());
                self.app
                    .set_style(*blocks.start()..*blocks.end() + 1, name)
                    .map(|_| ())
            }
            _ => return,
        };
        if let Err(error) = done {
            prompt::tell(mtm, "That could not be done.", &error.to_string());
        }
    }
}

impl TextPane {
    /// File ▸ Import Markdown…: a markdown file read in before the caret's block, one undo step
    /// (`App::import_markdown`) — where Open would make it a document of its own.
    fn import_markdown(&self, mtm: MainThreadMarker) {
        let panel = NSOpenPanel::openPanel(mtm);
        panel.setCanChooseDirectories(false);
        panel.setAllowsMultipleSelection(false);
        panel.setPrompt(Some(&NSString::from_str("Import")));
        if panel.runModal() != NSModalResponseOK {
            return;
        }
        let Some(path) = panel.URL().and_then(|url| url.to_file_path()) else {
            return;
        };
        let text = std::fs::read(&path)
            .map_err(|error| error.to_string())
            .and_then(|bytes| String::from_utf8(bytes).map_err(|_| "not UTF-8".to_owned()));
        let block = self.state.borrow().caret.block;
        let resolve = grind_text::commonmark::beside(&path);
        let done = text
            .and_then(|text| {
                self.app
                    .import_markdown(block, &text, &resolve)
                    .map_err(|error| error.to_string())
            })
            .map_err(|why| format!("{}: {why}", path.display()));
        match done {
            Ok(_) => self.go_to(grind_text::Caret { block, offset: 0 }),
            Err(why) => prompt::tell(mtm, "That file could not be imported.", &why),
        }
    }

    /// File ▸ Export as Markdown…: the selected paragraphs, or the whole page with no selection,
    /// as CommonMark in a file the save panel names. Writes a file and changes nothing.
    fn export_markdown(&self, mtm: MainThreadMarker) {
        let panel = NSSavePanel::savePanel(mtm);
        panel.setNameFieldStringValue(&NSString::from_str("Untitled.md"));
        if panel.runModal() != NSModalResponseOK {
            return;
        }
        let Some(path) = panel.URL().and_then(|url| url.to_file_path()) else {
            return;
        };
        let blocks = {
            let state = self.state.borrow();
            match state.selection().is_some() {
                true => {
                    let blocks = state.blocks();
                    *blocks.start()..*blocks.end() + 1
                }
                false => 0..self.app.block_count(),
            }
        };
        // The pictures go in a directory beside it (`notes.images/`), named for their bytes.
        let pictures = grind_text::commonmark::Pictures::beside(&path.display().to_string());
        let done = self
            .app
            .export_markdown(blocks, &pictures)
            .map_err(|error| error.to_string())
            .and_then(|exported| exported.save(&path));
        if let Err(why) = done {
            prompt::tell(mtm, "That could not be exported.", &why);
        }
    }

    /// The page typeset as a PDF, in the bundled faces and the Mac's own (`doc/pdf-export.md`).
    fn pdf(&self) -> Result<(Vec<u8>, grind_print::Report), String> {
        let options = grind_print::Options::default();
        grind_print::export(&self.app, grind_print::fonts_for(&self.app), &options)
    }

    /// File ▸ Export as PDF…: the page typeset and written where the save panel says. A family
    /// set in another face is said afterwards, since that is the one thing worth knowing about a
    /// PDF that came out.
    fn export_pdf(&self, mtm: MainThreadMarker) {
        let panel = NSSavePanel::savePanel(mtm);
        panel.setNameFieldStringValue(&NSString::from_str("Untitled.pdf"));
        if panel.runModal() != NSModalResponseOK {
            return;
        }
        let Some(path) = panel.URL().and_then(|url| url.to_file_path()) else {
            return;
        };
        let done = self.pdf().and_then(|(bytes, report)| {
            grind_core::atomic::write(&path, bytes)
                .map(|()| report)
                .map_err(|error| format!("{}: {error}", path.display()))
        });
        match done {
            Ok(report) if !report.substitutions.is_empty() || report.missing_glyphs > 0 => {
                prompt::tell(mtm, "The PDF was exported.", &report.summary());
            }
            Ok(_) => {}
            Err(why) => prompt::tell(mtm, "That could not be exported.", &why),
        }
    }

    /// File ▸ Print…: the same PDF handed to the system's print panel through PDFKit — so the
    /// panel's own preview is the PDF, page for page, rather than a second drawing of the page.
    fn print(&self, mtm: MainThreadMarker) {
        use objc2::AllocAnyThread;
        use objc2_foundation::NSData;
        use objc2_pdf_kit::{PDFDocument, PDFPrintScalingMode};
        let bytes = match self.pdf() {
            Ok((bytes, _)) => bytes,
            Err(why) => return prompt::tell(mtm, "That could not be printed.", &why),
        };
        let data = NSData::with_bytes(&bytes);
        // SAFETY: `data` is a complete PDF that outlives the document made from it.
        let Some(document) = (unsafe { PDFDocument::initWithData(PDFDocument::alloc(), &data) })
        else {
            return prompt::tell(mtm, "That could not be printed.", "PDFKit refused the PDF.");
        };
        // SAFETY: on the main thread, as the marker proves; the operation runs modally and is
        // done with the document when it returns.
        let operation = unsafe {
            document.printOperationForPrintInfo_scalingMode_autoRotate(
                None,
                PDFPrintScalingMode::PageScaleNone,
                true,
                mtm,
            )
        };
        if let Some(operation) = operation {
            operation.runOperation();
        }
    }

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
        let done = grind_text::picture::insert_below(&self.app, block, mime, data);
        match done {
            Ok(at) => self.go_to(grind_text::Caret {
                block: at,
                offset: 1,
            }),
            Err(error) => prompt::tell(mtm, "That could not be done.", &error.to_string()),
        }
    }
}

impl Pane {
    /// A defined name's context menu in the sidebar: rename it with every use following, point
    /// it somewhere else, write it out into every formula that uses it, or delete it — each one
    /// core call and one undo step, the first and third the first window to have them
    /// (`grind sheet name --rename`/`--inline` until now).
    pub fn act_on_name(&self, verb: NameVerb, name: &str, mtm: MainThreadMarker) {
        let said = match verb {
            NameVerb::Rename => {
                let Some(to) = prompt::ask(
                    mtm,
                    "Rename Name",
                    &format!("A new name for {name}. Every formula using it follows."),
                    "Rename",
                    name,
                ) else {
                    return;
                };
                self.app
                    .rename_name(name, to.trim())
                    .map(|count| Some(notice::name_renamed(count)))
                    .map_err(|error| error.to_string())
            }
            NameVerb::Redefine => {
                let now = self
                    .app
                    .names()
                    .into_iter()
                    .find(|(defined, _)| defined.eq_ignore_ascii_case(name))
                    .map(|(_, expression)| verbs::shown_definition(&expression))
                    .unwrap_or_default();
                let Some(typed) = prompt::ask(
                    mtm,
                    "Redefine Name",
                    &format!("What {name} stands for: a range such as Sheet1.A1:B9, or a formula."),
                    "Redefine",
                    &now,
                ) else {
                    return;
                };
                verbs::definition(&self.app, &typed)
                    .and_then(|expression| {
                        self.app
                            .set_name(name, &expression)
                            .map_err(|error| error.to_string())
                    })
                    .map(|()| None)
            }
            NameVerb::Inline => self
                .app
                .inline_name(name)
                .map(|count| Some(notice::name_inlined(name, count)))
                .map_err(|error| error.to_string()),
            NameVerb::Delete => {
                self.app.clear_name(name);
                Ok(None)
            }
        };
        match said {
            Ok(sentence) => self.say(sentence.as_deref().map(|sentence| (sentence, None))),
            Err(why) => prompt::tell(mtm, "That could not be done.", &why),
        }
    }
}
