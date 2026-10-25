// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later
//! `grind sheet …`: one arm per subcommand, each a few lines driving [`App`].

use super::*;

/// What kind of document a file holds, or a diagnostic naming what it is instead.
///
/// The whole point of doing this *before* opening: §8's reader is tolerant by construction, so
/// handing it a document of the wrong kind produces an empty one rather than an error. A user
/// who typed the wrong subcommand deserves to be told which one is right, not handed a
/// spreadsheet with no cells in it.
/// [`load`], with spelling checked the way the person has said spreadsheets should be
/// (`grind sheet spelling`) — what `lint` opens with, so its `misspelt` rule has something to ask,
/// and stays silent once spelling is turned off.
fn load_checked(file: &Path, cli: &Cli) -> Result<App, String> {
    let app = load(file, cli)?;
    check_spelling(&app);
    Ok(app)
}

pub(super) fn run_sheet(command: &Command, cli: &Cli) -> Result<Report, String> {
    match command {
        #[cfg(feature = "xlsx")]
        Command::Import {
            input,
            output,
            force,
            strict,
        } => {
            if output.exists() && !force {
                return Err(format!("{} exists; pass --force", output.display()));
            }
            let (document, report) = grind_xlsx::import_file(input).map_err(|e| e.to_string())?;
            // A strict import that lost something writes nothing: a pipeline told to fail on a
            // loss should not find a lossy file where it asked for one.
            let refused = *strict && !report.lossless();
            if !cli.dry_run && !refused {
                // Written by *our* writer, so R2 applies to the result unchanged: an imported
                // document validates against the ODF schema like any other. There is no `App`
                // in this path — an import is a file-to-file translation, not an edit — which
                // is also why `cli/tests/parity.rs` is undisturbed by it.
                grind_sheet::write_file(&document, output).map_err(|e| e.to_string())?;
            }
            Ok(Report::Import(Box::new(report::ImportReport::new(
                &show_path(input),
                &show_path(output),
                &document,
                &report,
                !cli.dry_run && !refused,
                *strict,
            ))))
        }

        Command::New {
            file,
            force,
            locale,
        } => {
            if file.exists() && !force {
                return Err(format!("{} exists; pass --force", file.display()));
            }
            let app = App::new();
            // Born with its locale rather than given one, so a new document opens with nothing
            // to undo.
            if locale.is_some() {
                let doc = grind_sheet::Document {
                    locale: locale.clone(),
                    ..Default::default()
                };
                let bytes = grind_sheet::write_bytes(&doc, grind_sheet::Form::Flat).say()?;
                app.open_bytes("new.fods", &bytes).say()?;
            }
            finish(&app, cli, file, true)
        }

        Command::Locale { file, tag, clear } => {
            let app = load(file, cli)?;
            if tag.is_none() && !clear {
                // The tag, then a number spelled the document's way, so `1.234,5` says at once
                // which sign is the decimal one.
                let locale = app.locale();
                let shown = locale
                    .as_ref()
                    .map_or_else(|| "none".to_owned(), |l| l.tag());
                let sample = grind_sheet::locale::sample(locale.as_ref());
                return Ok(Report::Text(TextReport {
                    lines: vec![format!("{shown}\t{sample}")],
                }));
            }
            let before = app.locale();
            app.set_locale(tag.clone()).say()?;
            finish(&app, cli, file, app.locale() != before)
        }

        Command::Get {
            file,
            address,
            formula,
            input,
            value,
            raw,
        } => {
            let app = load(file, cli)?;
            let reference = a1::parse(address).say()?;
            if !a1::is_single(&reference) {
                return Err(format!("{address}: get takes one cell, not a range"));
            }
            let (sheet, pos, _) = a1::resolve(&app, &reference).say()?;
            if *formula {
                let source = app.formula(sheet, pos).say()?.unwrap_or_default();
                return Ok(Report::Text(TextReport {
                    lines: vec![source],
                }));
            }
            if *input {
                // What a GUI puts in its formula bar, and what `sheet set` takes back
                // unchanged — the same text, because it is the same rule.
                return Ok(Report::Text(TextReport {
                    lines: vec![app.input_text(sheet, pos).say()?],
                }));
            }
            if *value {
                return Ok(Report::Text(TextReport {
                    lines: vec![app.value_text(sheet, pos).say()?],
                }));
            }
            Ok(Report::Cells(cells(
                &app,
                file,
                sheet,
                pos,
                pos,
                u32::MAX,
                match *raw {
                    true => Shown::Value,
                    false => Shown::Text,
                },
                Overlays::NONE,
            )?))
        }

        Command::View {
            file,
            address,
            max_rows,
            raw,
            roles,
            names,
            formulas,
        } => {
            let app = load(file, cli)?;
            let (sheet, start, end) = match address {
                Some(address) => a1::resolve(&app, &a1::parse(address).say()?).say()?,
                // No range given: everything the sheet uses.
                None => {
                    let (rows, cols) = app.used_extent(0).say()?;
                    let last = Pos::new(rows.saturating_sub(1), cols.saturating_sub(1));
                    (0, Pos::new(0, 0), last)
                }
            };
            // `--formulas --names` is the one combination that means something together:
            // the formula column, read through the names. The rest are one column each.
            let shown = match (*raw, *roles, *names, *formulas) {
                (true, ..) => Shown::Value,
                (_, true, ..) => Shown::Role,
                (.., true) => Shown::Formula,
                (_, _, true, _) => Shown::Name,
                _ => Shown::Text,
            };
            let overlays = Overlays {
                roles: *roles,
                names: *names,
            };
            Ok(Report::Cells(cells(
                &app, file, sheet, start, end, *max_rows, shown, overlays,
            )?))
        }

        Command::Project {
            file,
            tokens,
            anchors,
            as_read,
        } => {
            let app = load(file, cli)?;
            match as_read {
                true => as_read_report(file, app.source_anchors()),
                false => project_report(&app.project(), *tokens, *anchors),
            }
        }

        Command::Lint {
            file,
            hints,
            off,
            rules,
        } => match rules {
            true => Ok(rule_list(&grind_sheet::lint::RULES)),
            false => Ok(lint_report(
                file,
                load_checked(file, cli)?.lint(&lint_options(*hints, off)),
            )),
        },

        Command::Set {
            file,
            address,
            value,
            text,
            recalc,
        } => {
            let app = load(file, cli)?;
            let (sheet, pos, _) = single(&app, address)?;
            let value = read_stdin_if_dash(value)?;
            // `App::enter` is the typing rule, shared with every GUI: a leading `=` is a
            // formula stored verbatim with its cached value, a leading `'` is text, and
            // `--text` is that same rule spelled as a flag.
            let outcome = app
                .enter(sheet, pos, &forced_text(&value, *text), mode(*recalc))
                .say()?;
            // `1.5` in a German document is read as one and a half; say so, as every window does.
            if let Some(note) = outcome.note(app.locale().as_ref()) {
                eprintln!("grind: {note}");
            }
            finish(&app, cli, file, true)
        }

        Command::Paste {
            file,
            anchor,
            rows,
            recalc,
        } => {
            let app = load(file, cli)?;
            let (sheet, pos, _) = single(&app, anchor)?;
            let text = read_stdin_if_dash(rows)?;
            let rows: Vec<Vec<String>> = text
                .lines()
                .map(|line| line.split('\t').map(str::to_owned).collect())
                .collect();
            let outcome = app.enter_range(sheet, pos, &rows, mode(*recalc)).say()?;
            if let Some(note) = outcome.note(app.locale().as_ref()) {
                eprintln!("grind: {note}");
            }
            finish(&app, cli, file, outcome.cells > 0)
        }

        Command::ImportCsv {
            file,
            source,
            at,
            delimiter,
            quote,
            locale,
            text,
            formulas,
            dates,
            trim,
            recalc,
        } => {
            let app = load(file, cli)?;
            let (sheet, pos, _) = single(&app, at)?;
            let csv = read_text_file_or_stdin(source)?;
            // Sniffing reads the file rather than its name, for the reason `grind_core::kind`
            // does: a `.csv` written by a German Excel is semicolon-separated and a `.txt`
            // written by anything is whatever it is.
            let dialect = grind_sheet::csv::Dialect {
                delimiter: delimiter
                    .unwrap_or_else(|| grind_sheet::csv::Dialect::sniff(&csv).delimiter),
                quote: *quote,
            };
            let options = grind_sheet::csv::Import {
                dialect,
                locale: locale.clone(),
                text: *text,
                formulas: *formulas,
                dates: *dates,
                trim: *trim,
            };
            let outcome = app
                .import_csv(sheet, pos, &csv, &options, mode(*recalc))
                .say()?;
            finish(&app, cli, file, outcome.cells > 0)
        }

        Command::ExportCsv {
            file,
            address,
            out,
            delimiter,
            quote,
            quote_all,
            crlf,
            bom,
            formulas,
        } => {
            let app = load(file, cli)?;
            // A sheet with nothing in it has no rectangle at all, and its export is an empty
            // file rather than one blank line — `used_extent` reports 0×0 and the arithmetic
            // below would otherwise clamp that to the single cell A1.
            let (sheet, start, end, empty) = match address {
                Some(address) => {
                    let (sheet, start, end) =
                        a1::resolve(&app, &a1::parse(address).say()?).say()?;
                    (sheet, start, end, false)
                }
                None => {
                    let (rows, cols) = app.used_extent(0).say()?;
                    let last = Pos::new(rows.saturating_sub(1), cols.saturating_sub(1));
                    (0, Pos::new(0, 0), last, rows == 0 || cols == 0)
                }
            };
            let options = grind_sheet::csv::Export {
                dialect: grind_sheet::csv::Dialect {
                    delimiter: *delimiter,
                    quote: *quote,
                },
                quote_all: *quote_all,
                crlf: *crlf,
                bom: *bom,
                formulas: *formulas,
            };
            let csv = match empty {
                true => String::new(),
                false => app.export_csv(sheet, start, end, &options).say()?,
            };
            match out {
                Some(path) => {
                    grind_core::atomic::write(path, &csv)
                        .map_err(|e| format!("{}: {e}", path.display()))?;
                    text_lines(vec![show_path(path)])
                }
                // Split rather than `lines()`, which would eat the `\r` of a CRLF file: the
                // printer puts one `\n` back after each piece, so what reaches stdout is what
                // `--out` would have written, byte for byte — an empty export included, which
                // is no output rather than one blank line.
                None if csv.is_empty() => text_lines(Vec::new()),
                None => text_lines(
                    csv.strip_suffix('\n')
                        .unwrap_or(&csv)
                        .split('\n')
                        .map(str::to_owned)
                        .collect(),
                ),
            }
        }

        Command::Fill {
            file,
            source,
            address,
            recalc,
        } => {
            let app = load(file, cli)?;
            let (sheet, from, _) = single(&app, source)?;
            let (_, start, end) = a1::resolve(&app, &a1::parse(address).say()?).say()?;
            let outcome = app.fill(sheet, from, start, end, mode(*recalc)).say()?;
            finish(&app, cli, file, outcome.cells > 0)
        }

        Command::Eval {
            file,
            address,
            formula,
            trace,
            with,
            pretty,
            width,
        } => {
            let app = load(file, cli)?;
            let (sheet, pos, _) = single(&app, address)?;
            // A read, not an edit: nothing is stored and nothing is written.
            let mut entries = Vec::new();
            for entry in with {
                let (cell, value) = entry
                    .split_once('=')
                    .ok_or_else(|| format!("--with {entry}: expected ADDRESS=VALUE"))?;
                let (sheet, pos, _) = single(&app, cell)?;
                entries.push((sheet, pos, value));
            }
            let overlay = app.overlay(&entries).say()?;
            if *pretty {
                let shown = grind_sheet::formula::display::to_display(formula).say()?;
                let laid = grind_sheet::formula::display::pretty(&shown, *width).say()?;
                let lines = match trace {
                    true => laid_out(&app, sheet, pos, &laid, &overlay)?,
                    false => laid.lines().map(str::to_owned).collect(),
                };
                return Ok(Report::Text(TextReport { lines }));
            }
            if *trace {
                return Ok(Report::Text(TextReport {
                    lines: traced(&app, sheet, pos, formula, &overlay)?,
                }));
            }
            if !overlay.is_empty() {
                // `preview` has no overlay; the trace's root is the same answer with one.
                let shown = grind_sheet::formula::display::to_display(formula).say()?;
                let value = app.trace(sheet, pos, &shown, &overlay).say()?.value;
                return Ok(Report::Text(TextReport {
                    lines: vec![report::show(&grind_sheet::formula::eval::to_cell(value))],
                }));
            }
            let value = app.preview(sheet, pos, formula).say()?;
            Ok(Report::Text(TextReport {
                lines: vec![report::show(&value)],
            }))
        }

        Command::Clear {
            file,
            address,
            formula_only,
        } => {
            let app = load(file, cli)?;
            if *formula_only {
                // The one form that is a cell rather than a rectangle: it keeps the value.
                let (sheet, pos, _) = single(&app, address)?;
                app.clear_formula(sheet, pos).say()?;
                return finish(&app, cli, file, true);
            }
            let (sheet, start, end) = a1::resolve(&app, &a1::parse(address).say()?).say()?;
            let changed = app.clear_range(sheet, start, end).say()?;
            finish(&app, cli, file, changed > 0)
        }

        Command::Format {
            file,
            address,
            style,
            decimals,
            grouping,
            symbol,
            locale,
            show,
        } => {
            let app = load(file, cli)?;
            if *show {
                return shown(&app, file, address, false, true);
            }
            let (sheet, start, end) = a1::resolve(&app, &a1::parse(address).say()?).say()?;
            let format = match style.expect("clap requires a style unless --show") {
                Style::General => None,
                Style::Datetime => Some(numfmt::datetime_preset()),
                style => Some(numfmt::preset(kind(style), *decimals, *grouping, symbol)),
            }
            // Explicit, then the document's own, then the environment's — and written onto the
            // format whichever it is, so LibreOffice shows it the same way on any machine
            // (`doc/ods-format.md` §5.2).
            .map(|format| {
                format.in_locale(
                    locale
                        .clone()
                        .or_else(|| app.locale())
                        .or_else(grind_sheet::locale::from_environment),
                )
            });
            let changed = app.set_format(sheet, start, end, format).say()?;
            finish(&app, cli, file, changed > 0)
        }

        Command::Style {
            file,
            address,
            show,
            bold,
            italic,
            underline,
            strike,
            color,
            background,
            align,
            valign,
            wrap,
            size,
            border,
            indent,
            rotate,
        } => {
            let app = load(file, cli)?;
            if *show {
                return shown(&app, file, address, true, false);
            }
            let (sheet, start, end) = a1::resolve(&app, &a1::parse(address).say()?).say()?;
            let mut want = CellStyle {
                font_weight: bold.then(|| "bold".to_owned()),
                font_style: italic.then(|| "italic".to_owned()),
                font_size: size.clone(),
                color: color.clone(),
                background: background.clone(),
                align: align.map(|a| {
                    match a {
                        // §16.5: the values are relative to the writing direction.
                        Align::Left => "start",
                        Align::Center => "center",
                        Align::Right => "end",
                        Align::Justify => "justify",
                    }
                    .to_owned()
                }),
                vertical_align: valign.map(|v| {
                    match v {
                        VAlign::Top => "top",
                        VAlign::Middle => "middle",
                        VAlign::Bottom => "bottom",
                    }
                    .to_owned()
                }),
                wrap: wrap.then(|| "wrap".to_owned()),
                underline: underline.then(|| "solid".to_owned()),
                line_through: strike.then(|| "solid".to_owned()),
                indent: indent.clone(),
                rotation: rotate.clone(),
                borders: Default::default(),
            };
            want.set_border(border.clone());
            let changed = app.set_style(sheet, start, end, Some(want)).say()?;
            finish(&app, cli, file, changed > 0)
        }

        Command::Width {
            file,
            columns,
            length,
            clear,
        } => {
            let app = load(file, cli)?;
            let (sheet, start, end) = tracks(&app, columns)?;
            let range = start.col..end.col + 1;
            let Some(length) = length.clone().or(clear.then(String::new)) else {
                let widths = app.col_widths(sheet).say()?;
                return Ok(lines(widths.into_iter().filter_map(|(col, w)| {
                    range.contains(&col).then(|| (column_name(col), w))
                })));
            };
            let changed = app
                .set_col_width(sheet, range, (!length.is_empty()).then_some(length))
                .say()?;
            finish(&app, cli, file, changed > 0)
        }

        Command::Height {
            file,
            rows,
            length,
            clear,
        } => {
            let app = load(file, cli)?;
            let (sheet, start, end) = tracks(&app, rows)?;
            let range = start.row..end.row + 1;
            let Some(length) = length.clone().or(clear.then(String::new)) else {
                let heights = app.row_heights(sheet).say()?;
                return Ok(lines(heights.into_iter().filter_map(|(row, h)| {
                    range.contains(&row).then(|| ((row + 1).to_string(), h))
                })));
            };
            let changed = app
                .set_row_height(sheet, range, (!length.is_empty()).then_some(length))
                .say()?;
            finish(&app, cli, file, changed > 0)
        }

        Command::Fit { file, tracks } => {
            let app = load(file, cli)?;
            let metrics = fit::measure();
            let mut changed = 0;
            match tracks {
                Some(spec) => {
                    let (sheet, range, is_cols) = hide_tracks(&app, spec)?;
                    changed += match is_cols {
                        true => {
                            let cols = grind_sheet::fit::columns_in_use(&app, sheet, range);
                            let widths = fit::widths(&app, sheet, cols, &metrics);
                            app.fit(sheet, widths, 0..0).say()?
                        }
                        false => app.fit(sheet, Vec::new(), range).say()?,
                    };
                }
                None => {
                    for sheet in 0..app.sheet_count() {
                        let cols = grind_sheet::fit::all_columns(&app, sheet);
                        let widths = fit::widths(&app, sheet, cols, &metrics);
                        changed += app.fit(sheet, widths, 0..u32::MAX).say()?;
                    }
                }
            }
            finish(&app, cli, file, changed > 0)
        }

        Command::Hide {
            file,
            tracks,
            unhide,
        } => {
            let app = load(file, cli)?;
            let Some(spec) = tracks else {
                let mut hidden = Vec::new();
                for i in 0..app.sheet_count() {
                    let name = app.sheet_name(i).unwrap_or_default();
                    for col in app.hidden_cols(i).unwrap_or_default() {
                        hidden.push((format!("{name}.{}", column_name(col)), "column".to_owned()));
                    }
                    for row in app.manually_hidden_rows(i).unwrap_or_default() {
                        hidden.push((format!("{name}.{}", row + 1), "row".to_owned()));
                    }
                }
                return Ok(lines(hidden.into_iter()));
            };
            let (sheet, range, is_cols) = hide_tracks(&app, spec)?;
            let changed = match is_cols {
                true => app.set_col_hidden(sheet, range, !unhide).say()?,
                false => app.set_row_hidden(sheet, range, !unhide).say()?,
            };
            finish(&app, cli, file, changed > 0)
        }

        Command::Merge {
            file,
            range,
            unmerge,
        } => {
            let app = load(file, cli)?;
            let Some(range) = range else {
                let mut merged = Vec::new();
                for i in 0..app.sheet_count() {
                    let name = app.sheet_name(i).unwrap_or_default();
                    for (start, end) in app.merges(i).unwrap_or_default() {
                        merged.push((
                            format!("{name}.{}", a1::format(None, start)),
                            a1::format(None, end),
                        ));
                    }
                }
                return Ok(lines(merged.into_iter()));
            };
            let (sheet, start, end) = a1::resolve(&app, &a1::parse(range).say()?).say()?;
            let changed = match unmerge {
                true => app.unmerge(sheet, start, end).say()? > 0,
                false => app.merge(sheet, start, end).say()?,
            };
            finish(&app, cli, file, changed)
        }

        Command::Checkbox {
            file,
            cell,
            link,
            label,
            toggle,
            remove,
        } => {
            let app = load(file, cli)?;
            let Some(cell) = cell else {
                let mut boxes = Vec::new();
                for i in 0..app.sheet_count() {
                    let name = app.sheet_name(i).unwrap_or_default();
                    for (pos, checkbox) in app.checkboxes(i).unwrap_or_default() {
                        let state = match &checkbox.link {
                            Some(link) => {
                                format!("linked to {}", a1::format(link.sheet.as_deref(), link.pos))
                            }
                            None if checkbox.checked => "ticked".to_owned(),
                            None => "not ticked".to_owned(),
                        };
                        boxes.push((format!("{name}.{}", a1::format(None, pos)), state));
                    }
                }
                return Ok(lines(boxes.into_iter()));
            };
            let (sheet, pos, _) = single(&app, cell)?;
            if *remove {
                app.set_checkbox(sheet, pos, None).say()?;
                return finish(&app, cli, file, true);
            }
            if *toggle {
                let ticked = app.toggle_checkbox(sheet, pos).say()?;
                eprintln!(
                    "grind: {}",
                    match ticked {
                        true => "ticked",
                        false => "not ticked",
                    }
                );
                return finish(&app, cli, file, true);
            }
            let link = match link {
                Some(address) => {
                    let (to, at, _) = single(&app, address)?;
                    Some(grind_sheet::Link {
                        sheet: (to != sheet).then(|| app.sheet_name(to).unwrap_or_default()),
                        pos: at,
                    })
                }
                None => None,
            };
            let checkbox = grind_sheet::Checkbox {
                link,
                label: label.clone(),
                ..Default::default()
            };
            app.set_checkbox(sheet, pos, Some(checkbox)).say()?;
            finish(&app, cli, file, true)
        }

        Command::Rule {
            file,
            range,
            condition,
            remove,
            clear,
            bold,
            italic,
            underline,
            strike,
            color,
            background,
            size,
            border,
        } => {
            let app = load(file, cli)?;
            if let Some(number) = remove {
                let sheet = match range {
                    Some(spec) => a1::sheet(&app, spec)
                        .or_else(|_| a1::resolve(&app, &a1::parse(spec)?).map(|(s, _, _)| s))
                        .say()?,
                    None => 0,
                };
                if *number == 0 {
                    return Err("rules are numbered from 1".to_owned());
                }
                app.remove_rule(sheet, number - 1).say()?;
                return finish(&app, cli, file, true);
            }
            let (Some(range), Some(condition)) = (range, condition) else {
                if *clear && let Some(range) = range {
                    let (sheet, start, end) = a1::resolve(&app, &a1::parse(range).say()?).say()?;
                    let rules = app.rules(sheet).say()?;
                    let kept: Vec<_> = rules
                        .iter()
                        .filter(|r| !r.touches(start, end))
                        .cloned()
                        .collect();
                    let changed = kept.len() != rules.len();
                    app.set_rules(sheet, kept).say()?;
                    return finish(&app, cli, file, changed);
                }
                let mut listed = Vec::new();
                for i in 0..app.sheet_count() {
                    let name = app.sheet_name(i).unwrap_or_default();
                    for (n, rule) in app.rules(i).unwrap_or_default().iter().enumerate() {
                        let ranges: Vec<String> = rule
                            .ranges
                            .iter()
                            .map(|(a, b)| match a == b {
                                true => a1::format(None, *a),
                                false => {
                                    format!("{}:{}", a1::format(None, *a), a1::format(None, *b))
                                }
                            })
                            .collect();
                        listed.push((
                            format!("{name} {}", n + 1),
                            format!(
                                "{}\t{}\t{}",
                                ranges.join(" "),
                                rule.condition,
                                rule_look(&rule.style)
                            ),
                        ));
                    }
                }
                return Ok(lines(listed.into_iter()));
            };
            let (sheet, start, end) = a1::resolve(&app, &a1::parse(range).say()?).say()?;
            let mut look = CellStyle {
                font_weight: bold.then(|| "bold".to_owned()),
                font_style: italic.then(|| "italic".to_owned()),
                font_size: size.clone(),
                color: color.clone(),
                background: background.clone(),
                underline: underline.then(|| "solid".to_owned()),
                line_through: strike.then(|| "solid".to_owned()),
                ..CellStyle::default()
            };
            look.set_border(border.clone());
            if look.is_plain() {
                return Err(
                    "a rule has to draw something: give it --background, --color, \
                            --bold, --italic, --underline, --strike, --size or --border"
                        .to_owned(),
                );
            }
            let condition = condition
                .trim()
                .strip_prefix('=')
                .unwrap_or(condition.trim());
            app.add_rule(
                sheet,
                grind_sheet::rule::Rule::over(start, end, condition, look),
            )
            .say()?;
            finish(&app, cli, file, true)
        }

        Command::Name {
            file,
            name,
            target,
            delete,
            rename,
            inline,
        } => {
            let app = load(file, cli)?;
            if *inline {
                let rewritten = app.inline_name(name).say()?;
                eprintln!(
                    "grind: {name:?} written into {rewritten} use(s) and deleted; \
                     `grind sheet undo` undoes the whole inline"
                );
                return finish(&app, cli, file, true);
            }
            if let Some(to) = rename {
                // The count goes to stderr, as `sheet rename`'s does: stdout is the report.
                let rewritten = app.rename_name(name, to).say()?;
                if rewritten > 0 {
                    eprintln!(
                        "grind: {rewritten} use(s) rewritten to name {to:?}; \
                         `grind sheet undo` undoes the whole rename"
                    );
                }
                return finish(&app, cli, file, true);
            }
            if *delete {
                let removed = app.clear_name(name);
                if !removed {
                    return Err(format!("no such name: {name}"));
                }
                return finish(&app, cli, file, true);
            }
            let Some(target) = target else {
                // Reading one, which is a `get` for names and writes nothing.
                let (_, expression) = app
                    .names()
                    .into_iter()
                    .find(|(n, _)| n.eq_ignore_ascii_case(name))
                    .ok_or_else(|| format!("no such name: {name}"))?;
                return Ok(Report::Text(TextReport {
                    lines: vec![expression],
                }));
            };

            // Same rule as `set`: a leading `=` means a formula, anything else is an
            // address — `a1::definition`, shared with the shells so the two cannot differ.
            let expression = a1::definition(&app, target).say()?;
            app.set_name(name, &expression).say()?;
            finish(&app, cli, file, true)
        }

        Command::Filter {
            file,
            range,
            keep,
            clear,
            no_header,
        } => {
            let app = load(file, cli)?;
            let Some(range) = range else {
                if *clear {
                    app.set_filter(0, None).say()?;
                    return finish(&app, cli, file, true);
                }
                // Reading: what each sheet filters, and what that hides. 1-based rows,
                // since this is a person reading.
                return Ok(lines((0..app.sheet_count()).filter_map(|i| {
                    let filter = app.filter(i).ok()??;
                    let hidden = app.hidden_rows(i).unwrap_or_default();
                    let name = app.sheet_name(i).unwrap_or_default();
                    let rows: Vec<String> = hidden.iter().map(|r| (r + 1).to_string()).collect();
                    Some((
                        format!(
                            "{}:{}",
                            a1::format(Some(&name), filter.start),
                            a1::format(None, filter.end)
                        ),
                        format!("hides {}", rows.join(",")),
                    ))
                })));
            };
            let (sheet, start, end) = a1::resolve(&app, &a1::parse(range).say()?).say()?;
            if *clear {
                app.set_filter(sheet, None).say()?;
                return finish(&app, cli, file, true);
            }
            let mut filter = grind_sheet::Filter::new(
                // The name LibreOffice gives an autofilter nobody named.
                "__Anonymous_Sheet_DB__0",
                start,
                end,
            );
            filter.contains_header = !no_header;
            for item in keep {
                let (column, value) = item
                    .split_once('=')
                    .ok_or_else(|| format!("{item}: expected COLUMN=VALUE"))?;
                let (_, at, _) = single(&app, &format!("{column}1"))?;
                if at.col < start.col || at.col > end.col {
                    return Err(format!("{column} is outside {range}"));
                }
                filter
                    .keep
                    .entry(at.col - start.col)
                    .or_default()
                    .insert(value.to_owned());
            }
            app.set_filter(sheet, Some(filter)).say()?;
            finish(&app, cli, file, true)
        }

        Command::FormatTable {
            file,
            range,
            no_header,
            totals,
            name,
        } => {
            let app = load(file, cli)?;
            let (sheet, start, end) = a1::resolve(&app, &a1::parse(range).say()?).say()?;
            // `--totals` with no value is a sum, `--totals average` names another aggregate, and
            // no flag is no totals row (clap's `default_missing_value`).
            let options = grind_sheet::TableOptions {
                header: !no_header,
                totals: *totals,
                name: name.clone(),
            };
            let settled = app.format_table(sheet, start, end, options).say()?;
            eprintln!("grind: named {settled:?}");
            finish(&app, cli, file, true)
        }

        Command::Add { file, name } => {
            let app = load(file, cli)?;
            let name = name.clone().unwrap_or_else(|| app.fresh_sheet_name());
            app.add_sheet(&name).say()?;
            finish(&app, cli, file, true)
        }

        Command::Rename { file, sheet, name } => {
            let app = load(file, cli)?;
            let index = a1::sheet(&app, sheet).say()?;
            // A rename carries every formula, named expression and chart range that named the old
            // sheet, in one undo step. Said on stderr, like the staleness warning: stdout is the
            // report a script parses.
            let rewritten = app.rename_sheet(index, name).say()?;
            if rewritten > 0 {
                eprintln!(
                    "grind: {rewritten} reference(s) rewritten to name {name:?}; \
                     `grind sheet undo` undoes the whole rename"
                );
            }
            finish(&app, cli, file, true)
        }

        Command::Remove { file, sheet } => {
            let app = load(file, cli)?;
            let index = a1::sheet(&app, sheet).say()?;
            app.remove_sheet(index).say()?;
            finish(&app, cli, file, true)
        }

        Command::Recalc { file } => {
            let app = load(file, cli)?;
            let recalc = app.recalc().say()?;
            // To stderr, so the warning cannot corrupt parseable output. A document using a
            // function grind does not implement loses a good cached value to #NAME? here, and
            // saying so is the difference between a recalculation and silent data loss. `sheet
            // undo` is the way back.
            if recalc.spoiled > 0 {
                eprintln!(
                    "sheet: {} cell(s) became errors — a function this build does not \
                     implement; undo to restore, or see `grind sheet functions`",
                    recalc.spoiled
                );
            }
            finish(&app, cli, file, recalc.changed > 0)
        }

        Command::Undo { file } => {
            let app = load(file, cli)?;
            require_session(cli, "undo")?;
            let changed = app.undo();
            finish(&app, cli, file, changed)
        }

        Command::Redo { file } => {
            let app = load(file, cli)?;
            require_session(cli, "redo")?;
            let changed = app.redo();
            finish(&app, cli, file, changed)
        }

        Command::Fmt {
            formula,
            display,
            from_display,
            friendly,
            inline,
        } => {
            use grind_sheet::formula::display;
            let line = match (display, from_display, friendly) {
                (true, _, _) => display::to_display(formula).say()?,
                (_, true, _) => display::from_display(formula).say()?,
                (_, _, true) if *inline => {
                    grind_sheet::formula::friendly::explain_inline(formula).say()?
                }
                (_, _, true) => grind_sheet::formula::friendly::explain(formula).say()?,
                _ => format!("={}", grind_sheet::formula::parse::parse(formula).say()?),
            };
            Ok(Report::Text(TextReport { lines: vec![line] }))
        }

        #[cfg(feature = "spell")]
        Command::Spell {
            file,
            range,
            sheet,
            language,
            suggest,
            add,
        } => {
            if let Some(word) = add {
                let path = grind_spell::personal::add(word).map_err(|e| e.to_string())?;
                return text_lines(vec![format!("{word}\t{}", path.display())]);
            }
            let file = file.as_deref().expect("clap requires a file without --add");
            let app = load(file, cli)?;
            let named = match language.as_deref() {
                Some(tag) => Some(spell_language(tag)?),
                None => match grind_spell::preference::load(DocumentKind::Spreadsheet) {
                    grind_spell::Setting::In(language) => Some(language),
                    _ => None,
                },
            };
            let (choice, _) = grind_spell::attach(&app, named).map_err(|e| e.to_string())?;
            eprintln!("grind: checking in {}", choice.describe());
            let mut only = match sheet {
                Some(name) => Some(a1::sheet(&app, name).say()?),
                None => None,
            };
            let mut within = None;
            if let Some(range) = range {
                let reference = a1::parse(range).say()?;
                let (on, start, end) = a1::resolve(&app, &reference).say()?;
                // A qualified range names its own sheet; an unqualified one is on each checked.
                if reference.start.sheet.is_some() {
                    if only.is_some_and(|only| only != on) {
                        return Err(format!("{range} is not on the sheet --sheet names"));
                    }
                    only = Some(on);
                }
                within = Some((start, end));
            }
            let mut lines = Vec::new();
            for wrong in app.misspellings(only, within).say()? {
                let name = app.sheet_name(wrong.sheet).say()?;
                let mut line =
                    format!("{}\t{}\t{}", wrong.address(&name), wrong.offset, wrong.word);
                if *suggest {
                    line.push('\t');
                    line.push_str(&app.suggest(&wrong.word).join(", "));
                }
                lines.push(line);
            }
            text_lines(lines)
        }

        Command::Correct {
            file,
            address,
            word,
            with,
            at,
        } => {
            let app = load(file, cli)?;
            let reference = a1::parse(address).say()?;
            if !a1::is_single(&reference) {
                return Err(format!("{address}: correct takes one cell, not a range"));
            }
            let (sheet, pos, _) = a1::resolve(&app, &reference).say()?;
            let offset = match at {
                Some(offset) => *offset,
                // The first time the word occurs *as a word*: `recieve` in `recieved` is not it.
                None => {
                    let text = app.input_text(sheet, pos).say()?;
                    let chars: Vec<char> = text.chars().collect();
                    grind_core::spell::words(&text)
                        .into_iter()
                        .find(|range| chars[range.clone()].iter().collect::<String>() == *word)
                        .map(|range| range.start)
                        .ok_or_else(|| format!("{address} has no word {word:?}"))?
                }
            };
            app.correct(sheet, pos, offset, word, with).say()?;
            finish(&app, cli, file, word != with)
        }

        #[cfg(feature = "spell")]
        Command::Spelling { setting } => spelling_setting(DocumentKind::Spreadsheet, setting),

        Command::Find {
            file,
            needle,
            search,
        } => {
            let app = load(file, cli)?;
            let search = search.resolve(&app, needle)?;
            let hits = app.find(&search).say()?;
            Ok(Report::Text(TextReport {
                lines: hits
                    .iter()
                    .map(|hit| format!("{}\t{}", hit.address(), hit.text))
                    .collect(),
            }))
        }

        Command::Replace {
            file,
            needle,
            replacement,
            search,
            recalc,
        } => {
            let app = load(file, cli)?;
            let search = search.resolve(&app, needle)?;
            let done = app.replace(&search, replacement, mode(*recalc)).say()?;
            for (hit, reason) in &done.refused {
                eprintln!(
                    "grind: {} left as it was — the replaced formula would not parse: {reason}",
                    hit.address()
                );
            }
            finish(&app, cli, file, done.cells > 0)
        }

        Command::Calculations { file, filter } => {
            let app = load(file, cli)?;
            let needle = filter.as_deref().unwrap_or_default();
            let found: Vec<_> = app
                .calculations()
                .into_iter()
                .filter(|calc| calc.matches(needle))
                .collect();
            let mut lines: Vec<String> = found
                .iter()
                .map(|calc| {
                    format!(
                        "{}\t{}\t{}\t{}",
                        calc.address(),
                        calc.formula,
                        calc.value,
                        calc.functions.join(" ")
                    )
                })
                .collect();
            let tally = grind_sheet::function_tally(&found);
            let counted = match found.len() {
                1 => "1 calculation".to_owned(),
                n => format!("{n} calculations"),
            };
            let summary = match tally.is_empty() {
                true => counted,
                false => format!(
                    "{counted} — {}",
                    tally
                        .iter()
                        .map(|(name, count)| format!("{name} ×{count}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            };
            lines.push(summary);
            Ok(Report::Text(TextReport { lines }))
        }

        Command::Functions { long, filter } => {
            let names = grind_sheet::formula::funcs::implemented();
            let matches = |name: &str| {
                filter
                    .as_ref()
                    .is_none_or(|f| name.to_uppercase().contains(&f.to_uppercase()))
            };
            // The catalog is the same list with the spec's own summary and syntax beside
            // each name — what a GUI's autocomplete offers, so the two cannot disagree.
            let mut lines: Vec<String> = match long {
                true => grind_sheet::formula::funcs::catalog()
                    .iter()
                    .filter(|info| matches(info.name))
                    .map(|info| {
                        // The friendly signature carries the alias in its head, so it is one
                        // column rather than two: `Present Value(Rate; Number Of Periods; …)`.
                        let friendly = grind_sheet::formula::friendly::signature(info.name)
                            .map(|(head, params)| format!("{head}({})", params.join("; ")))
                            .unwrap_or_else(|| info.name.to_owned());
                        let category = grind_sheet::formula::funcs::category(info);
                        format!(
                            "{}\t{friendly}\t{category}\t{}\t§{}",
                            info.signature, info.brief, info.section
                        )
                    })
                    .collect(),
                false => names
                    .iter()
                    .filter(|n| matches(n))
                    .map(|n| (*n).to_owned())
                    .collect(),
            };
            lines.sort();
            let beyond = grind_sheet::formula::funcs::beyond_small_group();
            // Not "138 of 110": the Small Group is the conformance claim and the functions
            // moved in beside it are counted apart, or the line reads as broken arithmetic.
            let mut summary = format!(
                "{} of the Small Group's {}",
                names.len() - beyond.len(),
                grind_sheet::formula::funcs::SMALL_GROUP
            );
            if !beyond.is_empty() {
                summary.push_str(&format!(
                    " — plus {} beyond it: {}",
                    beyond.len(),
                    beyond.join(", ")
                ));
            }
            lines.push(summary);
            Ok(Report::Text(TextReport { lines }))
        }

        Command::ChartAdd {
            file,
            from,
            r#type,
            categories,
            series,
            clockwise,
            title,
            legend,
            sheet,
            x,
            y,
            width,
            height,
            x_axis_label,
            y_axis_label,
            x_tick_labels,
            y_tick_labels,
            x_gridlines,
            y_gridlines,
        } => {
            let app = load(file, cli)?;
            let sheet_index = chart_sheet(&app, sheet.as_deref())?;
            let mut spec = match from {
                Some(range) => {
                    let qualified =
                        grind_sheet::chart::parse_range(&app, sheet_index, range).say()?;
                    let (on, start, end) =
                        grind_sheet::chart::resolve_range(&app, &qualified).say()?;
                    let guessed = app.suggest_chart(on, start, end, None).say()?;
                    if guessed.spec.series.is_empty() && series.is_empty() {
                        return Err(format!("nothing in {range} is a number to chart"));
                    }
                    guessed.spec
                }
                None => {
                    if series.is_empty() {
                        return Err("give the chart a --series, or a table --from".to_owned());
                    }
                    grind_sheet::ChartSpec::new(
                        r#type
                            .map(Into::into)
                            .unwrap_or(grind_sheet::ChartKind::Bar),
                    )
                }
            };
            if let Some(kind) = r#type {
                spec.kind = (*kind).into();
            }
            if let Some(range) = categories {
                spec.categories = (!range.is_empty()).then(|| range.clone());
            }
            if !series.is_empty() {
                spec.series = split_series(series);
            }
            if let Some(clockwise) = clockwise {
                spec.clockwise = *clockwise;
            }
            if let Some(title) = title {
                spec.title = Some(title.clone());
            }
            // Asked of the spec as it finally stands, since `--series` may have changed how
            // many things there are to tell apart.
            spec.legend = match legend {
                Some(legend) => (*legend).into(),
                None => spec.default_legend(),
            };
            spec.x_axis = axis(
                Some(&spec.x_axis),
                x_axis_label,
                *x_tick_labels,
                *x_gridlines,
            );
            spec.y_axis = axis(
                Some(&spec.y_axis),
                y_axis_label,
                *y_tick_labels,
                *y_gridlines,
            );
            app.add_chart(sheet_index, &spec, x, y, width, height)
                .say()?;
            finish(&app, cli, file, true)
        }

        Command::ChartEdit {
            file,
            index,
            sheet,
            r#type,
            categories,
            series,
            clockwise,
            title,
            legend,
            x_axis_label,
            y_axis_label,
            x_tick_labels,
            y_tick_labels,
            x_gridlines,
            y_gridlines,
        } => {
            let app = load(file, cli)?;
            let sheet_index = chart_sheet(&app, sheet.as_deref())?;
            let charts = app.charts(sheet_index).say()?;
            let chart = charts
                .get(*index)
                .ok_or_else(|| format!("sheet {sheet_index} has no chart {index}"))?;
            // Nothing given keeps what is there — including the series, which are handed back
            // as the resolved addresses they already are and re-resolve to themselves.
            let mut spec = grind_sheet::ChartSpec::of(chart);
            if let Some(kind) = r#type {
                spec.kind = (*kind).into();
            }
            if !series.is_empty() {
                spec.series = split_series(series);
            }
            match categories {
                Some(range) if range.is_empty() => spec.categories = None,
                Some(range) => spec.categories = Some(range.clone()),
                None => {}
            }
            if let Some(clockwise) = clockwise {
                spec.clockwise = *clockwise;
            }
            if let Some(title) = title {
                // `App::edit_chart` reads an empty title as none.
                spec.title = Some(title.clone());
            }
            if let Some(legend) = legend {
                spec.legend = (*legend).into();
            }
            spec.x_axis = axis(
                Some(&chart.x_axis),
                x_axis_label,
                *x_tick_labels,
                *x_gridlines,
            );
            spec.y_axis = axis(
                Some(&chart.y_axis),
                y_axis_label,
                *y_tick_labels,
                *y_gridlines,
            );
            app.edit_chart(sheet_index, *index, &spec).say()?;
            finish(&app, cli, file, true)
        }

        Command::ChartList { file, sheet } => {
            let app = load(file, cli)?;
            let sheet_index = chart_sheet(&app, sheet.as_deref())?;
            let charts = app.charts(sheet_index).say()?;
            let lines = charts
                .iter()
                .enumerate()
                .map(|(i, chart)| {
                    let series = chart
                        .series
                        .iter()
                        .map(|s| s.values.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!(
                        "{i}\t{:?}\t{}\t{series}\t{}\t{}",
                        chart.kind,
                        chart.categories.as_deref().unwrap_or("-"),
                        axes(chart),
                        dress(chart)
                    )
                })
                .collect();
            Ok(Report::Text(TextReport { lines }))
        }

        Command::ChartRemove { file, index, sheet } => {
            let app = load(file, cli)?;
            let sheet_index = chart_sheet(&app, sheet.as_deref())?;
            app.remove_chart(sheet_index, *index).say()?;
            finish(&app, cli, file, true)
        }

        Command::ChartReshape {
            file,
            index,
            x,
            y,
            width,
            height,
            sheet,
        } => {
            let app = load(file, cli)?;
            let sheet_index = chart_sheet(&app, sheet.as_deref())?;
            app.reshape_chart(sheet_index, *index, x, y, width, height)
                .say()?;
            finish(&app, cli, file, true)
        }

        Command::ChartStyle {
            file,
            index,
            sheet,
            x_axis_label,
            y_axis_label,
            x_tick_labels,
            y_tick_labels,
            x_gridlines,
            y_gridlines,
            series_color,
            point_color,
        } => {
            let app = load(file, cli)?;
            let sheet_index = chart_sheet(&app, sheet.as_deref())?;
            let charts = app.charts(sheet_index).say()?;
            let chart = charts
                .get(*index)
                .ok_or_else(|| format!("sheet {sheet_index} has no chart {index}"))?;
            let mut series = chart.series.clone();
            for entry in series_color {
                let (n, hex) = entry
                    .split_once('=')
                    .ok_or_else(|| format!("{entry}: expected SERIES=COLOR"))?;
                let n: usize = n
                    .parse()
                    .map_err(|_| format!("{entry}: expected a series index"))?;
                let s = series
                    .get_mut(n)
                    .ok_or_else(|| format!("{entry}: chart has no series {n}"))?;
                s.color = if hex.is_empty() {
                    None
                } else {
                    Some(style::color(hex)?)
                };
            }
            for entry in point_color {
                let (pos, hex) = entry
                    .split_once('=')
                    .ok_or_else(|| format!("{entry}: expected SERIES.POINT=COLOR"))?;
                let (n, p) = pos
                    .split_once('.')
                    .ok_or_else(|| format!("{entry}: expected SERIES.POINT=COLOR"))?;
                let n: usize = n
                    .parse()
                    .map_err(|_| format!("{entry}: expected a series index"))?;
                let p: usize = p
                    .parse()
                    .map_err(|_| format!("{entry}: expected a point index"))?;
                let s = series
                    .get_mut(n)
                    .ok_or_else(|| format!("{entry}: chart has no series {n}"))?;
                if hex.is_empty() {
                    if p < s.point_colors.len() {
                        s.point_colors[p] = None;
                    }
                } else {
                    if s.point_colors.len() <= p {
                        s.point_colors.resize(p + 1, None);
                    }
                    s.point_colors[p] = Some(style::color(hex)?);
                }
            }
            app.set_chart_style(
                sheet_index,
                *index,
                axis(
                    Some(&chart.x_axis),
                    x_axis_label,
                    *x_tick_labels,
                    *x_gridlines,
                ),
                axis(
                    Some(&chart.y_axis),
                    y_axis_label,
                    *y_tick_labels,
                    *y_gridlines,
                ),
                series,
            )
            .say()?;
            finish(&app, cli, file, true)
        }
    }
}

fn chart_sheet(app: &App, name: Option<&str>) -> Result<usize, String> {
    match name {
        Some(name) => a1::sheet(app, name).say(),
        None => Ok(0),
    }
}

/// One axis' worth of flags, resolved against what the chart already carries — `current` is
/// `None` for `chart-add`, where there is nothing to keep and an omitted flag means the
/// default. A title given as the empty string clears it, the same convention every other
/// "unset it" flag in this CLI uses.
fn axis(
    current: Option<&grind_sheet::ChartAxis>,
    label: &Option<String>,
    tick_labels: Option<bool>,
    gridlines: Option<bool>,
) -> grind_sheet::ChartAxis {
    let mut axis = current.cloned().unwrap_or_default();
    if let Some(label) = label {
        axis.label = (!label.is_empty()).then(|| label.clone());
    }
    if let Some(show) = tick_labels {
        axis.tick_labels = show;
    }
    if let Some(show) = gridlines {
        axis.gridlines = show;
    }
    axis
}

/// A chart's two axes, for one column of `chart-list` — the title if it has one, then a letter
/// per thing switched on, so a chart that carries nothing prints `-` rather than a blank.
/// `chart-list`'s last column: the chart's own title, its legend's position and a pie's
/// direction — `title:Q1 legend:end`, or `-` for a chart with none of them.
fn dress(chart: &grind_sheet::Chart) -> String {
    let mut parts = Vec::new();
    if let Some(title) = &chart.title {
        parts.push(format!("title:{title}"));
    }
    if let Some(legend) = chart.legend {
        parts.push(format!("legend:{}", legend.token()));
    }
    if chart.kind == grind_sheet::ChartKind::Pie {
        parts.push(
            match chart.clockwise {
                true => "clockwise",
                false => "counter-clockwise",
            }
            .to_owned(),
        );
    }
    match parts.is_empty() {
        true => "-".to_owned(),
        false => parts.join(" "),
    }
}

fn axes(chart: &grind_sheet::Chart) -> String {
    let one = |name: &str, axis: &grind_sheet::ChartAxis| {
        let mut flags = String::new();
        if axis.tick_labels {
            flags.push('t');
        }
        if axis.gridlines {
            flags.push('g');
        }
        match (&axis.label, flags.is_empty()) {
            (None, true) => String::new(),
            (None, false) => format!("{name}:{flags}"),
            (Some(label), true) => format!("{name}:{label}"),
            (Some(label), false) => format!("{name}:{label}:{flags}"),
        }
    };
    let both = [one("x", &chart.x_axis), one("y", &chart.y_axis)]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    match both.is_empty() {
        true => "-".to_owned(),
        false => both,
    }
}

/// `--series RANGE[=LABEL]`, split into the pair `App` takes — `chart-add` and `chart-edit`
/// spell a series the same way, so they split one the same way too.
fn split_series(series: &[String]) -> Vec<(String, Option<String>)> {
    series
        .iter()
        .map(|s| match s.split_once('=') {
            Some((values, label)) => (values.to_owned(), Some(label.to_owned())),
            None => (s.clone(), None),
        })
        .collect()
}

// --- helpers ---

/// A core error reaching the user as text.
///
/// The CLI's own failures are already strings — a bad flag, a file that exists — so this is
/// the one adapter between the two, rather than a closure at every call site.
/// `eval --pretty --trace`: the laid-out formula with each line's answer in a column — the
/// outermost node starting on that line (`Trace::by_line`), and nothing for a closing bracket.
fn laid_out(
    app: &App,
    sheet: usize,
    pos: Pos,
    laid: &str,
    overlay: &grind_sheet::formula::trace::Overlay,
) -> Result<Vec<String>, String> {
    let trace = app.trace(sheet, pos, laid, overlay).say()?;
    let sheets: Vec<String> = (0..).map_while(|i| app.sheet_name(i).ok()).collect();
    let width = laid.lines().map(|l| l.chars().count()).max().unwrap_or(0);
    Ok(laid
        .lines()
        .zip(trace.by_line(laid))
        .map(|(line, node)| match node {
            Some(node) => {
                let pad = width - line.chars().count();
                format!(
                    "{line}{}  {}",
                    " ".repeat(pad),
                    node.came_to.spelled(&sheets, sheet)
                )
            }
            None => line.to_owned(),
        })
        .collect())
}

/// `eval --trace`: the formula in display syntax, one sub-expression a line indented by its
/// depth, and each one's answer in a column — `App::trace`, read top to bottom.
fn traced(
    app: &App,
    sheet: usize,
    pos: Pos,
    formula: &str,
    overlay: &grind_sheet::formula::trace::Overlay,
) -> Result<Vec<String>, String> {
    use grind_sheet::formula::display;
    // The CLI takes a formula as the file stores it; a trace's spans are in display syntax.
    let shown = display::to_display(formula).map_err(|e| e.to_string())?;
    let trace = app.trace(sheet, pos, &shown, overlay).say()?;
    let sheets: Vec<String> = (0..).map_while(|i| app.sheet_name(i).ok()).collect();
    let rows: Vec<(String, String)> = trace
        .rows()
        .into_iter()
        .map(|(depth, node)| {
            let indent = "  ".repeat(depth);
            // An omitted parameter (§5.6) has no text to show and nothing to come to.
            match node.range.is_empty() {
                true => (format!("{indent}(omitted)"), String::new()),
                false => (
                    format!("{indent}{}", &shown[node.range.clone()]),
                    node.came_to.spelled(&sheets, sheet),
                ),
            }
        })
        .collect();
    let width = rows
        .iter()
        .map(|(left, _)| left.chars().count())
        .max()
        .unwrap_or(0);
    Ok(rows
        .into_iter()
        .map(|(left, answer)| {
            let pad = width - left.chars().count();
            match answer.is_empty() {
                true => left,
                false => format!("{left}{}  {answer}", " ".repeat(pad)),
            }
        })
        .collect())
}

/// A locale tag as a person types one — `de-DE`, or a bare `de`.
pub(super) fn locale(value: &str) -> Result<grind_sheet::locale::Locale, String> {
    grind_sheet::locale::Locale::parse(value)
        .ok_or_else(|| format!("{value}: expected a language tag like de-DE"))
}

/// Undo history lives in the session file, never in the document.
fn require_session(cli: &Cli, what: &str) -> Result<(), String> {
    if cli.session.is_none() {
        return Err(format!(
            "{what} needs --session: history is not stored in the document"
        ));
    }
    Ok(())
}

/// A rectangle of cells, read the only way a shell may read them.
///
/// `shown` decides the text column *and* whether the read asks for `doc/view-modes.md`'s
/// overlays: deriving a role needs a document-wide analysis, so a plain `view` does not pay
/// for one and a `--roles` view does.
#[allow(clippy::too_many_arguments)]
fn cells(
    app: &App,
    file: &Path,
    sheet: usize,
    start: Pos,
    end: Pos,
    max_rows: u32,
    shown: Shown,
    overlays: Overlays,
) -> Result<CellsReport, String> {
    let last_row = end
        .row
        .min(start.row.saturating_add(max_rows.saturating_sub(1)));
    let rows = start.row..last_row.saturating_add(1);
    let cols = start.col..end.col.saturating_add(1);
    let name = app.sheet_name(sheet).say()?;
    let viewport = app
        .get_viewport_with(sheet, rows.clone(), cols.clone(), overlays)
        .say()?;

    let mut out = Vec::new();
    for row in rows.clone() {
        for col in cols.clone() {
            let pos = Pos::new(row, col);
            let value = viewport.get(row, col).cloned().unwrap_or(CellValue::Empty);
            out.push(Cell {
                address: a1::format(None, pos),
                value: report::show(&value),
                text: viewport.text(row, col).unwrap_or_default().to_owned(),
                kind: report::kind(&value),
                formula: app.formula(sheet, pos).say()?,
                named_formula: match overlays.names {
                    true => app.named_formula(sheet, pos).say()?,
                    false => None,
                },
                role: viewport.role(row, col).map(|r| r.name()),
                name: viewport.name_at(row, col).map(str::to_owned),
            });
        }
    }
    Ok(CellsReport {
        path: show_path(file),
        sheet: name,
        shown,
        cells: out,
        rows: rows.end - rows.start,
        cols: cols.end - cols.start,
    })
}

/// A run of columns (`B`, `B:D`, `Data.B:D`) or of rows (`3`, `3:7`) as the sheet and the
/// corners `core::a1` resolves it to.
///
/// A single track is spelled as a range of one and goes down the same path, so this stays
/// two lines and does no index arithmetic — §5.8's whole-column and whole-row forms are
/// already what a lone `B` or `3` means, and the only `+ 1` in the workspace stays in `a1`.
fn tracks(app: &App, spec: &str) -> Result<(usize, Pos, Pos), String> {
    let spec = match spec.contains(':') {
        true => spec.to_owned(),
        false => format!("{spec}:{spec}"),
    };
    a1::resolve(app, &a1::parse(&spec).say()?).say()
}

/// [`tracks`], plus which axis the spec named — a whole-column reference (`B`, `B:D`) has
/// no row of its own to be a whole one, and a whole-row reference is the same the other way
/// round, which is what `hide` needs to know and `width`/`height` do not, since each of
/// those already commits to one axis by its own name.
fn hide_tracks(app: &App, spec: &str) -> Result<(usize, std::ops::Range<u32>, bool), String> {
    let full = match spec.contains(':') {
        true => spec.to_owned(),
        false => format!("{spec}:{spec}"),
    };
    let reference = a1::parse(&full).say()?;
    let is_cols = reference.start.col.is_some() && reference.start.row.is_none();
    let (sheet, start, end) = a1::resolve(app, &reference).say()?;
    let range = match is_cols {
        true => start.col..end.col + 1,
        false => start.row..end.row + 1,
    };
    Ok((sheet, range, is_cols))
}

/// What a rule draws, as the `sheet rule` flags that would draw it.
fn rule_look(style: &CellStyle) -> String {
    let mut out = Vec::new();
    if let Some(w) = &style.font_weight {
        out.push(if w == "bold" {
            "--bold".to_owned()
        } else {
            format!("weight={w}")
        });
    }
    if let Some(s) = &style.font_style {
        out.push(if s == "italic" {
            "--italic".to_owned()
        } else {
            format!("slant={s}")
        });
    }
    if style.is_underlined() {
        out.push("--underline".to_owned());
    }
    if style.is_struck() {
        out.push("--strike".to_owned());
    }
    for (flag, value) in [
        ("--size", &style.font_size),
        ("--color", &style.color),
        ("--background", &style.background),
    ] {
        if let Some(value) = value {
            out.push(format!("{flag} {value}"));
        }
    }
    if let Some(border) = style.uniform_border() {
        out.push(format!("--border {border:?}"));
    }
    out.join(" ")
}

fn single(app: &App, address: &str) -> Result<(usize, Pos, Pos), String> {
    let reference = a1::parse(address).say()?;
    if !a1::is_single(&reference) {
        return Err(format!("{address}: expected one cell, not a range"));
    }
    a1::resolve(app, &reference).say()
}

fn shown(
    app: &App,
    file: &Path,
    address: &str,
    style: bool,
    format: bool,
) -> Result<Report, String> {
    let (sheet, pos, _) = single(app, address)?;
    Ok(Report::CellStyle(Box::new(report::CellStyleReport {
        path: file.display().to_string(),
        sheet: app.sheet_name(sheet).say()?,
        address: a1::format(None, pos),
        style: match style {
            true => app.style_at(sheet, pos).say()?,
            false => None,
        },
        format: match format {
            true => app.format_at(sheet, pos).say()?,
            false => None,
        },
    })))
}

/// `--text`, as the core's typing rule spells it. The rule lives in `App::enter`, so a
/// document edited from a shell and one edited from the CLI cannot disagree about what
/// `123` in a cell means.
fn forced_text(value: &str, force: bool) -> String {
    match force {
        true => format!("'{value}"),
        false => value.to_owned(),
    }
}

fn mode(recalc: bool) -> RecalcMode {
    match recalc {
        true => RecalcMode::Document,
        false => RecalcMode::No,
    }
}

/// A delimiter as a person names one.
///
/// The four that have names have them, because a shell is a poor place to type a tab and a
/// literal `;` needs quoting in most of them. Anything else is the character itself — one
/// character, so a typo becomes a message rather than a silently unmatched separator.
pub(super) fn delimiter(value: &str) -> Result<char, String> {
    let named = match value {
        "comma" => Some(','),
        "semicolon" => Some(';'),
        "tab" | "\\t" => Some('\t'),
        "pipe" => Some('|'),
        "space" => Some(' '),
        _ => None,
    };
    let mut chars = value.chars();
    let delimiter = match (named, chars.next(), chars.next()) {
        (Some(named), ..) => named,
        (None, Some(one), None) => one,
        _ => {
            return Err(format!(
                "{value}: expected one character, or one of comma, semicolon, tab, pipe, space"
            ));
        }
    };
    match delimiter {
        // A quote is how a field escapes the delimiter; the two cannot be the same character
        // and still describe a readable file.
        '"' | '\'' | '\n' | '\r' => Err(format!("{value}: not usable as a field separator")),
        _ => Ok(delimiter),
    }
}
