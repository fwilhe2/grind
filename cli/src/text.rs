// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later
//! `grind text …`: one arm per subcommand, each a few lines driving [`TextApp`].

use super::*;

/// `--paper`: the ISO 216 A series and nothing else (`doc/pdf-export.md`, decision 7).
#[cfg(feature = "pdf")]
pub(super) fn paper(name: &str) -> Result<grind_core::page::PageGeometry, String> {
    grind_core::page::PageGeometry::paper(name).ok_or_else(|| {
        format!("{name:?} is not a paper this build prints on: a0 to a10, e.g. a4 or a5-landscape")
    })
}

/// Resolve a range of blocks. A **heading** address alone means its whole section, which is
/// what makes `grind text move report.fodt §3.2 §1` mean what a person expects — the extent is
/// computed from outline levels, because the document stores no such container.
fn span(app: &TextApp, address: &str) -> Result<std::ops::Range<usize>, String> {
    if !address.contains(':')
        && let Ok(index) = at(app, address)
        && let Some(section) = app.section(index)
    {
        return Ok(section);
    }
    let range = grind_text::loc::parse_range(address).map_err(|e| e.to_string())?;
    app.resolve_range(&range).map_err(|e| e.to_string())
}

/// Resolve one address to a caret — a block *and* an offset within it, which is what `p3+12`
/// names and what the caret-level verbs take.
fn caret_at(app: &TextApp, address: &str) -> Result<grind_text::Caret, String> {
    let loc = grind_text::loc::parse(address).map_err(|e| e.to_string())?;
    app.resolve_caret(&loc).map_err(|e| e.to_string())
}

/// Resolve a range of **characters**, as `erase` takes it.
///
/// Deliberately not [`span`]: there, a bare heading address means the heading's whole *section*,
/// because the verbs that take one relocate and delete structure. `erase` removes characters,
/// so a bare address there means that one block's text and a heading is not special. Two verbs
/// spelled the same and meaning different things would be worse than two functions.
fn caret_span(
    app: &TextApp,
    address: &str,
) -> Result<(grind_text::Caret, grind_text::Caret), String> {
    let range = grind_text::loc::parse_range(address).map_err(|e| e.to_string())?;
    app.resolve_caret_range(&range).map_err(|e| e.to_string())
}

/// One block's text with its bookmark anchors written in — what `text view --names` prints,
/// and `doc/view-modes.md` §3.6's whole point: a bookmark contributes no characters, so
/// nothing a reader sees says it is there.
///
/// `‹name›` at the offset it anchors to, inserted from the end backwards so that inserting
/// one does not move the next one's offset. The brackets are the marker rather than a
/// colour, because this has to work down a pipe.
fn with_marks(text: &str, marks: &[(usize, String)]) -> String {
    let mut chars: Vec<char> = text.chars().collect();
    for (at, name) in marks.iter().rev() {
        let at = (*at).min(chars.len());
        let mark: Vec<char> = format!("\u{2039}{name}\u{203a}").chars().collect();
        chars.splice(at..at, mark);
    }
    chars.into_iter().collect()
}

/// One block's text, broken into lines at `width` — what `view --width` prints.
///
/// The CLI measures one unit per character (`grind_text::Fixed`), so a width of 72 is 72
/// characters. Good enough to be useful and honest about what it is: a terminal shell wants
/// `unicode-width` for CJK and combining marks, and implements `Metrics` itself.
fn wrapped(app: &TextApp, index: usize, width: u32) -> Result<Vec<String>, String> {
    let text: Vec<char> = app
        .input_text(index)
        .map_err(|e| e.to_string())?
        .chars()
        .collect();
    let layout = app
        .layout_block(index, width as f32, &grind_text::Fixed)
        .map_err(|e| e.to_string())?;
    Ok(layout
        .lines()
        .iter()
        // A trailing space or newline is *on* the line it ended, which is right for a caret and
        // wrong for printing, so the printer trims what the model deliberately keeps.
        .map(|line| text[line.start..line.end].iter().collect::<String>())
        .map(|line| line.trim_end().to_owned())
        .collect())
}

/// The block kind two flags describe. Neither means a paragraph.
fn kind_of(heading: Option<u32>, list: Option<u32>) -> Result<grind_text::BlockKind, String> {
    match (heading, list) {
        (Some(0), _) => Err("headings start at level 1".to_owned()),
        (Some(level), _) => Ok(grind_text::BlockKind::Heading { level }),
        (_, Some(0)) => Err("list items start at depth 1".to_owned()),
        (_, Some(depth)) => Ok(grind_text::BlockKind::ListItem { depth }),
        (None, None) => Ok(grind_text::BlockKind::Paragraph),
    }
}

pub(super) fn run_text(command: &TextCommand, cli: &Cli) -> Result<Report, String> {
    match command {
        #[cfg(feature = "docx")]
        TextCommand::Import {
            input,
            output,
            force,
            strict,
        } => {
            if output.exists() && !force {
                return Err(format!("{} exists; pass --force", output.display()));
            }
            let (document, report) = grind_docx::import_file(input).map_err(|e| e.to_string())?;
            let refused = *strict && !report.lossless();
            if !cli.dry_run && !refused {
                // The import is already an ODF document — `grind_docx::convert` wrote it flat,
                // and this is that document read back — so writing it in the form the name asks
                // for is an ordinary save of an untouched document: the flat bytes as they are,
                // a package moved across by `odf::forms`, or a projection.
                grind_text::write_file(&document, output).map_err(|e| e.to_string())?;
            }
            Ok(Report::TextImport(Box::new(report::TextImportReport::new(
                &show_path(input),
                &show_path(output),
                &document,
                &report,
                !cli.dry_run && !refused,
                *strict,
            ))))
        }

        TextCommand::New { file, force } => {
            if file.exists() && !force {
                return Err(format!("{} exists; pass --force", file.display()));
            }
            finish_text(&TextApp::new(), cli, file, true)
        }

        TextCommand::View {
            file,
            range,
            marks,
            names,
            width,
        } => {
            let app = open_text(file)?;
            let blocks = match range {
                Some(range) => span(&app, range)?,
                None => 0..app.block_count(),
            };
            let view = app.get_viewport(blocks);
            let mut lines = Vec::new();
            for block in view.iter() {
                // Without --width a block is one line, which is what `view` has always printed.
                // With it, the core breaks the block and the CLI prints what any shell would
                // draw at that width — the same engine, measured one unit per character.
                let pieces = match width {
                    None if *names => vec![with_marks(&block.text, &block.marks)],
                    None => vec![block.text.clone()],
                    Some(width) => wrapped(&app, block.index, *width)?,
                };
                for (n, piece) in pieces.into_iter().enumerate() {
                    lines.push(match marks {
                        false => piece,
                        // Only the first line of a block carries its address, so the marks stay
                        // one-per-block and a wrapped block reads as a block.
                        true if n == 0 => format!(
                            "{}\t{}\t{}",
                            grind_text::loc::format(block.index),
                            describe_block(&block.kind, block.cell.as_ref()),
                            piece
                        ),
                        true => format!("\t\t{piece}"),
                    });
                }
            }
            text_lines(lines)
        }

        TextCommand::Caret {
            file,
            at: address,
            down,
            home,
            end,
            width,
        } => {
            let app = open_text(file)?;
            let caret = caret_at(&app, address)?;
            // Every block set alike, at one unit per character: a terminal has one font, and
            // so a document printed into one has one face. A GUI answers the same questions
            // through a `Faces` that returns a different font per block, and the core does not
            // know which of the two it is talking to.
            let faces = grind_text::Uniform::new(*width as f32, &grind_text::Fixed);
            let moved = if *home || *end {
                let (start, finish) = app
                    .caret_line_bounds(caret, &faces)
                    .map_err(|e| e.to_string())?;
                match home {
                    true => start,
                    false => finish,
                }
            } else {
                // The goal column is the caret's own x, because a single move from a script has
                // no run of keystrokes to remember one from.
                let goal = app.caret_x(caret, &faces).map_err(|e| e.to_string())?;
                app.caret_line(caret, down.unwrap_or(0) as isize, goal, &faces)
                    .map_err(|e| e.to_string())?
            };
            text_lines(vec![grind_text::loc::format_offset(
                moved.block,
                moved.offset,
            )])
        }

        TextCommand::Get { file, at: address } => {
            let app = open_text(file)?;
            let index = at(&app, address)?;
            text_lines(vec![app.input_text(index).map_err(|e| e.to_string())?])
        }

        TextCommand::Set {
            file,
            at: address,
            text,
        } => {
            let app = open_text(file)?;
            let index = at(&app, address)?;
            app.set_text(index, &read_stdin_if_dash(text)?)
                .map_err(|e| e.to_string())?;
            finish_text(&app, cli, file, true)
        }

        TextCommand::Type {
            file,
            at: address,
            text,
            markdown,
        } => {
            let app = open_text(file)?;
            let caret = caret_at(&app, address)?;
            let text = read_stdin_if_dash(text)?;
            match markdown {
                // One call rather than one per character: the notation is resolved between
                // them inside the core, so a whole line is one action and one undo step.
                true => {
                    app.type_markdown(caret, &text, None)
                        .map_err(|e| e.to_string())?;
                }
                false => app.insert_text(caret, &text).map_err(|e| e.to_string())?,
            }
            finish_text(&app, cli, file, !text.is_empty())
        }

        TextCommand::Image {
            file,
            at: address,
            from,
            mime,
            width,
            height,
        } => {
            let app = open_text(file)?;
            let caret = caret_at(&app, address)?;
            let data = std::fs::read(from).map_err(|e| format!("{}: {e}", from.display()))?;
            let mime = mime.clone().unwrap_or_else(|| sniff_image_mime(from));
            app.insert_image(caret, mime, data, width.clone(), height.clone())
                .map_err(|e| e.to_string())?;
            finish_text(&app, cli, file, true)
        }

        TextCommand::Erase { file, range } => {
            let app = open_text(file)?;
            let (from, to) = caret_span(&app, range)?;
            let removed = app.erase(from, to).map_err(|e| e.to_string())?;
            finish_text(&app, cli, file, removed > 0)
        }

        TextCommand::Split { file, at: address } => {
            let app = open_text(file)?;
            let caret = caret_at(&app, address)?;
            app.split_block(caret).map_err(|e| e.to_string())?;
            finish_text(&app, cli, file, true)
        }

        TextCommand::Join { file, at: address } => {
            let app = open_text(file)?;
            let index = at(&app, address)?;
            app.join_block(index).map_err(|e| e.to_string())?;
            finish_text(&app, cli, file, true)
        }

        TextCommand::Insert {
            file,
            at: address,
            text,
            after,
            heading,
            list,
        } => {
            let app = open_text(file)?;
            let index = match address {
                Some(address) => at(&app, address)? + usize::from(*after),
                // No address appends, which is what building a document from a script does
                // most of the time.
                None => app.block_count(),
            };
            app.insert(index, kind_of(*heading, *list)?, &read_stdin_if_dash(text)?)
                .map_err(|e| e.to_string())?;
            finish_text(&app, cli, file, true)
        }

        TextCommand::Alt {
            file,
            at: address,
            title,
            description,
        } => {
            let app = open_text(file)?;
            let caret = caret_at(&app, address)?;
            app.set_image_alt(caret, Some(title.clone()), description.clone())
                .map_err(|e| e.to_string())?;
            finish_text(&app, cli, file, true)
        }

        TextCommand::ImportMd {
            file,
            source,
            at: address,
            after,
        } => {
            let app = open_text(file)?;
            let index = match address {
                Some(address) => at(&app, address)? + usize::from(*after),
                None => app.block_count(),
            };
            let markdown = read_text_file_or_stdin(source)?;
            let resolve = grind_text::commonmark::beside(Path::new(source));
            let resolve: &grind_text::commonmark::Resolve = match source.as_str() {
                "-" => &grind_text::commonmark::nowhere,
                _ => &resolve,
            };
            app.import_markdown(index, &markdown, resolve)
                .map_err(|e| e.to_string())?;
            finish_text(&app, cli, file, true)
        }

        #[cfg(feature = "pdf")]
        TextCommand::ExportPdf {
            file,
            out,
            paper,
            title,
            bundled_fonts,
        } => {
            let app = open_text(file)?;
            let title = title.clone().or_else(|| {
                file.file_stem()
                    .map(|stem| stem.to_string_lossy().into_owned())
            });
            let options = grind_print::Options {
                paper: *paper,
                title,
            };
            let (bytes, report) = grind_print::export(&app, fonts(&app, *bundled_fonts), &options)?;
            grind_core::atomic::write(out, &bytes)
                .map_err(|e| format!("{}: {e}", out.display()))?;
            Ok(Report::Pdf(report::PdfReport::new(show_path(out), &report)))
        }

        #[cfg(feature = "pdf")]
        TextCommand::Pages {
            file,
            paper,
            bundled_fonts,
        } => {
            let app = open_text(file)?;
            let options = grind_print::Options {
                paper: *paper,
                title: None,
            };
            let at = |caret: Option<grind_text::Caret>| {
                caret.map_or_else(
                    || "-".to_owned(),
                    |c| grind_text::loc::format_offset(c.block, c.offset),
                )
            };
            let pages = grind_print::pages(&app, fonts(&app, *bundled_fonts), &options);
            text_lines(
                pages
                    .into_iter()
                    .enumerate()
                    .map(|(number, (start, end))| {
                        format!("{}\t{}\t{}", number + 1, at(start), at(end))
                    })
                    .collect(),
            )
        }

        #[cfg(feature = "pdf")]
        TextCommand::Preview {
            file,
            out,
            page,
            dpi,
            paper,
            bundled_fonts,
        } => {
            let app = open_text(file)?;
            let options = grind_print::Options {
                paper: *paper,
                title: None,
            };
            if !(1.0..=1200.0).contains(dpi) {
                return Err(format!("--dpi {dpi}: between 1 and 1200"));
            }
            let index = page.checked_sub(1).ok_or("pages are counted from 1")?;
            let raster = grind_print::preview(
                &app,
                fonts(&app, *bundled_fonts),
                &options,
                index,
                dpi / 72.0,
            )?;
            grind_core::atomic::write(out, raster.png())
                .map_err(|e| format!("{}: {e}", out.display()))?;
            text_lines(vec![show_path(out)])
        }

        TextCommand::ExportMd {
            file,
            range,
            out,
            images_dir,
            inline_images,
        } => {
            use grind_text::commonmark::Pictures;
            let app = open_text(file)?;
            let blocks = match range {
                Some(range) => span(&app, range)?,
                None => 0..app.block_count(),
            };
            let pictures = match (images_dir, out) {
                _ if *inline_images => Pictures::Inline,
                (Some(dir), _) => Pictures::Files { dir: dir.clone() },
                (None, Some(path)) => Pictures::beside(&path.display().to_string()),
                (None, None) => Pictures::Inline,
            };
            let exported = app
                .export_markdown(blocks, &pictures)
                .map_err(|e| e.to_string())?;
            match out {
                Some(path) => {
                    exported.save(path)?;
                    let mut written = vec![show_path(path)];
                    let base = path.parent().unwrap_or(Path::new(""));
                    written.extend(
                        exported
                            .pictures
                            .iter()
                            .map(|p| show_path(&base.join(&p.path))),
                    );
                    text_lines(written)
                }
                None => {
                    // To stdout, its pictures — when it has any — relative to here.
                    for picture in &exported.pictures {
                        let path = Path::new(&picture.path);
                        if let Some(dir) = path.parent() {
                            std::fs::create_dir_all(dir)
                                .map_err(|e| format!("{}: {e}", dir.display()))?;
                        }
                        grind_core::atomic::write(path, &picture.data)
                            .map_err(|e| format!("{}: {e}", path.display()))?;
                    }
                    let markdown = exported.markdown;
                    text_lines(
                        markdown
                            .strip_suffix('\n')
                            .unwrap_or(&markdown)
                            .split('\n')
                            .map(str::to_owned)
                            .collect(),
                    )
                }
            }
        }

        TextCommand::Table {
            file,
            at: address,
            show,
            after,
            rows,
            columns,
            name,
        } => {
            let app = open_text(file)?;
            if *show {
                let index = at(&app, address.as_deref().unwrap_or("p1"))?;
                let table = app.table(index).ok_or_else(|| {
                    format!("{} is not in a table", grind_text::loc::format(index))
                })?;
                return text_lines(vec![format!(
                    "{}\t{} rows\t{} columns\t{}:{}",
                    table.name,
                    table.rows,
                    table.columns,
                    grind_text::loc::format(table.blocks.start),
                    grind_text::loc::format(table.blocks.end - 1),
                )]);
            }
            let index = match address {
                Some(address) => at(&app, address)? + usize::from(*after),
                None => app.block_count(),
            };
            app.insert_table(index, *rows, *columns, name.clone())
                .map_err(|e| e.to_string())?;
            finish_text(&app, cli, file, true)
        }

        TextCommand::Project {
            file,
            tokens,
            anchors,
            as_read,
        } => {
            let app = open_text(file)?;
            match as_read {
                true => as_read_report(file, app.source_anchors()),
                false => project_report(&app.project(), *tokens, *anchors),
            }
        }

        TextCommand::Lint {
            file,
            hints,
            off,
            rules,
        } => match rules {
            true => Ok(rule_list(&grind_text::lint::RULES)),
            false => Ok(lint_report(
                file,
                open_text_checked(file)?.lint(&lint_options(*hints, off)),
            )),
        },

        TextCommand::Delete { file, range } => {
            let app = open_text(file)?;
            let blocks = span(&app, range)?;
            let removed = app.delete(blocks).map_err(|e| e.to_string())?;
            finish_text(&app, cli, file, removed > 0)
        }

        TextCommand::Move { file, range, to } => {
            let app = open_text(file)?;
            let blocks = span(&app, range)?;
            let landing = at(&app, to)?;
            let moved = app
                .move_blocks(blocks, landing)
                .map_err(|e| e.to_string())?;
            finish_text(&app, cli, file, moved > 0)
        }

        TextCommand::Style { file, range, style } => {
            let app = open_text(file)?;
            let blocks = span(&app, range)?;
            let changed = app
                .set_style(blocks, style.clone())
                .map_err(|e| e.to_string())?;
            finish_text(&app, cli, file, changed > 0)
        }

        TextCommand::Kind {
            file,
            at: address,
            heading,
            list,
        } => {
            let app = open_text(file)?;
            let index = at(&app, address)?;
            app.set_kind(index, kind_of(*heading, *list)?)
                .map_err(|e| e.to_string())?;
            finish_text(&app, cli, file, true)
        }

        TextCommand::Outline { file, filter } => {
            let app = open_text(file)?;
            let needle = filter.as_deref().unwrap_or("").to_lowercase();
            text_lines(
                app.outline()
                    .into_iter()
                    .filter(|h| {
                        needle.is_empty()
                            || h.text.to_lowercase().contains(&needle)
                            || h.address().contains(&needle)
                    })
                    .map(|h| format!("{}\t{}\t{}", h.address(), h.level, h.text))
                    .collect(),
            )
        }

        TextCommand::Format {
            file,
            range,
            show,
            bold,
            italic,
            underline,
            strike,
            font,
            size,
            color,
            background,
        } => {
            let app = open_text(file)?;
            let (from, to) = caret_span(&app, range)?;
            if *show {
                let style = app.char_style(from, to).map_err(|e| e.to_string())?;
                return text_lines(describe_char_style(&style));
            }
            let mut style = grind_text::CharStyle {
                font_family: font.clone(),
                font_size: size.clone(),
                color: color.clone(),
                background: background.clone(),
                ..grind_text::CharStyle::default()
            };
            style.set_bold(*bold);
            style.set_italic(*italic);
            style.set_underlined(*underline);
            style.set_struck(*strike);
            let changed = app
                .set_char_style(from, to, &style)
                .map_err(|e| e.to_string())?;
            finish_text(&app, cli, file, changed > 0)
        }

        TextCommand::Link {
            file,
            range,
            target,
            remove,
        } => {
            let app = open_text(file)?;
            let (from, to) = caret_span(&app, range)?;
            let link_at = |at| app.link_at(at).map_err(|e| e.to_string());
            if *remove {
                let (from, to) = match (from == to, link_at(from)?) {
                    (true, Some(link)) => (link.from, link.to),
                    (true, None) => return Err(format!("no link at {range}")),
                    (false, _) => (from, to),
                };
                let changed = app.set_link(from, to, None).map_err(|e| e.to_string())?;
                return finish_text(&app, cli, file, changed > 0);
            }
            let Some(target) = target else {
                let shown = match from == to {
                    true => link_at(from)?.map(|link| (link.from, link.to, link.href)),
                    false => app
                        .link(from, to)
                        .map_err(|e| e.to_string())?
                        .map(|href| (from, to, href)),
                };
                return text_lines(
                    shown
                        .map(|(from, to, href)| {
                            format!(
                                "{}:{}\t{href}",
                                grind_text::loc::format_offset(from.block, from.offset),
                                grind_text::loc::format_offset(to.block, to.offset),
                            )
                        })
                        .into_iter()
                        .collect(),
                );
            };
            if from == to {
                return Err(format!("{range} is a caret; link a range of characters"));
            }
            let changed = app
                .set_link(from, to, Some(target))
                .map_err(|e| e.to_string())?;
            finish_text(&app, cli, file, changed > 0)
        }

        TextCommand::Formatting { file } => {
            let app = open_text(file)?;
            text_lines(
                app.formatting()
                    .into_iter()
                    .map(|b| {
                        format!(
                            "{}\t{}\t{}",
                            grind_text::loc::format(b.index),
                            b.style.as_deref().unwrap_or("(direct)"),
                            b.text
                        )
                    })
                    .collect(),
            )
        }

        TextCommand::Find {
            file,
            needle,
            ignore_case,
        } => {
            let app = open_text(file)?;
            let hits = match ignore_case {
                true => app.find_ignoring_case(needle),
                false => app.find(needle),
            };
            text_lines(
                hits.into_iter()
                    .map(|m| format!("{}\t{}", m.address(), m.text))
                    .collect(),
            )
        }

        TextCommand::Replace {
            file,
            needle,
            replacement,
        } => {
            let app = open_text(file)?;
            let changed = app
                .replace(needle, replacement)
                .map_err(|e| e.to_string())?;
            finish_text(&app, cli, file, changed > 0)
        }

        #[cfg(feature = "spell")]
        TextCommand::Spelling { setting } => spelling_setting(DocumentKind::Text, setting),
        #[cfg(feature = "spell")]
        TextCommand::Spell {
            file,
            range,
            language,
            suggest,
            add,
        } => {
            if let Some(word) = add {
                let path = grind_spell::personal::add(word).map_err(|e| e.to_string())?;
                return text_lines(vec![format!("{word}\t{}", path.display())]);
            }
            let file = file.as_deref().expect("clap requires a file without --add");
            let app = open_text(file)?;
            // Without --language, the language the person remembered for text documents, if
            // they chose one; `off` is for the windows and `lint`, and an explicit `spell`
            // is asked for, so it still checks.
            let named = match language.as_deref() {
                Some(tag) => Some(spell_language(tag)?),
                None => match grind_spell::preference::load(DocumentKind::Text) {
                    grind_spell::Setting::In(language) => Some(language),
                    _ => None,
                },
            };
            let (choice, _) = grind_spell::attach(&app, named).map_err(|e| e.to_string())?;
            eprintln!("grind: checking in {}", choice.describe());
            let blocks = match range {
                Some(range) => span(&app, range)?,
                None => 0..app.block_count(),
            };
            text_lines(
                app.misspellings(blocks)
                    .into_iter()
                    .map(|wrong| match suggest {
                        true => format!(
                            "{}\t{}\t{}",
                            wrong.address(),
                            wrong.word,
                            app.suggest(&wrong.word).join(", ")
                        ),
                        false => format!("{}\t{}", wrong.address(), wrong.word),
                    })
                    .collect(),
            )
        }

        TextCommand::Correct {
            file,
            at: address,
            word,
            with,
        } => {
            let app = open_text(file)?;
            let caret = caret_at(&app, address)?;
            app.correct(caret, word, with).map_err(|e| e.to_string())?;
            finish_text(&app, cli, file, word != with)
        }

        TextCommand::Name {
            file,
            name,
            at: address,
            delete,
        } => {
            let app = open_text(file)?;
            let Some(name) = name else {
                return text_lines(
                    app.bookmarks()
                        .into_iter()
                        .map(|(name, index)| format!("#{name}\t{}", grind_text::loc::format(index)))
                        .collect(),
                );
            };
            let target = match (delete, address) {
                (true, _) => None,
                (false, Some(address)) => Some(at(&app, address)?),
                (false, None) => {
                    // Reading one back, rather than setting it — the same shape `grind sheet
                    // name <name>` has.
                    let index = app
                        .bookmarks()
                        .into_iter()
                        .find(|(n, _)| n == name)
                        .map(|(_, index)| index)
                        .ok_or_else(|| format!("no bookmark named {name}"))?;
                    return text_lines(vec![format!(
                        "#{name}\t{}",
                        grind_text::loc::format(index)
                    )]);
                }
            };
            let changed = app.set_bookmark(name, target).map_err(|e| e.to_string())?;
            finish_text(&app, cli, file, changed)
        }

        TextCommand::Words { file } => {
            let app = open_text(file)?;
            Ok(Report::TextDocument(text_document(
                &app, file, false, false,
            )))
        }
    }
}

/// Guess a MIME type from a file's extension — good enough for `grind text image`'s default,
/// and always overridable with `--mime` for anything this misses.
fn sniff_image_mime(path: &Path) -> String {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "tif" | "tiff" => "image/tiff",
        _ => "application/octet-stream",
    }
    .to_owned()
}

/// What a block is, for `--marks`: its kind, and the cell it is in when it is in one.
///
/// A cell reads as `Prices!r1c2` — the table's name and the coordinate — because a paragraph
/// inside a table is still `p12` and nothing else in this line would say it is in one.
fn describe_block(kind: &grind_text::BlockKind, cell: Option<&grind_text::Cell>) -> String {
    let kind = describe_kind(kind);
    match cell {
        Some(cell) => format!("{kind} {}!r{}c{}", cell.table, cell.row, cell.column),
        None => kind,
    }
}

fn describe_kind(kind: &grind_text::BlockKind) -> String {
    match kind {
        grind_text::BlockKind::Paragraph => "p".to_owned(),
        grind_text::BlockKind::Heading { level } => format!("h{level}"),
        grind_text::BlockKind::ListItem { depth } => format!("li{depth}"),
    }
}

/// `--show` for both `style` and `format`: one cell's styling, its format, or both.
///
/// One cell rather than a rectangle, because the answer for a rectangle is either "they
/// agree" or a list, and a caller that wants the list already has `sheet view`. It reads and
/// writes nothing, so no `finish` and no `stale` warning.
/// `grind text format --show`, as lines: one `property<TAB>value` per property that is set,
/// and nothing at all for a plain span.
///
/// The values are ODF's own, unchanged — `bold`, `12pt`, `#001f3f` — because that is what the
/// model carries and re-spelling them here would invent a second vocabulary for a shell to
/// parse. Properties the span does not agree about are absent, which is the same answer a
/// toolbar shows over a mixed selection (`grind_text::App::char_style`).
fn describe_char_style(style: &grind_text::CharStyle) -> Vec<String> {
    [
        ("font", &style.font_family),
        ("size", &style.font_size),
        ("weight", &style.font_weight),
        ("style", &style.font_style),
        ("underline", &style.underline),
        ("strike", &style.line_through),
        ("color", &style.color),
        ("background", &style.background),
    ]
    .into_iter()
    .filter_map(|(name, value)| value.as_ref().map(|value| format!("{name}\t{value}")))
    .collect()
}
