// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `sheet-gtk` — the GNOME shell, and phase 9's first one.
//!
//! A renderer and an event forwarder that owns nothing (doc/plan.md rule 1). All state is
//! in `sheet-core`'s [`App`]; this crate holds a window, a grid that draws whatever
//! [`App::get_viewport`] returns, and eventually the keys and the editing. If a field
//! shows up here that is not a presentation concern, the core is missing something.
//!
//! `doc/gtk-shell.md` is the plan and the running record of what is built. Milestones 1 and
//! 3 are here: a document opens, draws, and can be navigated and selected with the keyboard
//! and the mouse. Nothing edits it yet.

mod geom;
mod grid;
mod keymap;
mod theme;

use std::cell::Cell;
use std::path::PathBuf;
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::Arc;

use libadwaita as adw;
use libadwaita::gtk;
use libadwaita::prelude::*;

use sheet_core::{App, CellValue, Pos, a1};

use keymap::Selection;

/// The reverse-DNS identity GNOME keys a window, its icon and its settings on.
const APP_ID: &str = "io.github.fwilhe2.Sheet";

fn main() -> ExitCode {
    let app = Arc::new(App::new());

    // The document is opened before the toolkit starts, so a bad path is a shell error
    // with an exit code rather than an empty window with a dialog in front of it. Like
    // every other shell here, a missing file is an error and not a new document.
    let mut args = std::env::args_os().skip(1);
    let path: Option<PathBuf> = args.next().map(PathBuf::from);
    // `--render-to <png>` draws one frame, writes it and exits. Not a user feature: it is
    // how a machine checks that the grid still draws, since a custom-drawn widget has no
    // other assertable output and `editor`'s §5 rule is to exercise every boundary with a
    // program that runs where the UI cannot.
    let render_to: Option<PathBuf> = match args.next() {
        Some(flag) if flag == "--render-to" => Some(PathBuf::from(
            args.next().unwrap_or_else(|| "grid.png".into()),
        )),
        _ => None,
    };
    if let Some(path) = &path
        && let Err(error) = app.open_file(path)
    {
        eprintln!("sheet-gtk: {}: {error}", path.display());
        return ExitCode::FAILURE;
    }

    let application = adw::Application::builder().application_id(APP_ID).build();
    application.connect_activate(move |application| {
        let window = window(application, &app, path.as_deref());
        window.present();
        if let Some(target) = render_to.clone() {
            render_once(&window, target);
        }
    });

    // The file argument was consumed above; GApplication must not try to parse it.
    match application.run_with_args::<&str>(&[]) {
        code if code == glib_success() => ExitCode::SUCCESS,
        _ => ExitCode::FAILURE,
    }
}

fn glib_success() -> gtk::glib::ExitCode {
    gtk::glib::ExitCode::SUCCESS
}

fn window(
    application: &adw::Application,
    app: &Arc<App>,
    path: Option<&std::path::Path>,
) -> adw::ApplicationWindow {
    let grid = grid::Grid::new(app.clone());

    let scroller = gtk::ScrolledWindow::builder()
        .hexpand(true)
        .vexpand(true)
        .child(&grid)
        .build();

    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&adw::WindowTitle::new(
        &title(path),
        &app.sheet_name(0).unwrap_or_default(),
    )));

    let view = adw::ToolbarView::builder().content(&scroller).build();
    view.add_top_bar(&header);
    view.add_bottom_bar(&status_bar(&grid, app));

    let window = adw::ApplicationWindow::builder()
        .application(application)
        .title(title(path))
        .default_width(1100)
        .default_height(700)
        .content(&view)
        .build();
    // The grid is what the keys are for, so it starts focused rather than waiting for a
    // click to make the arrow keys work.
    // Two traits spell `set_focus`; `GtkWindowExt`'s is the one that means "focus this".
    gtk::prelude::GtkWindowExt::set_focus(&window, Some(&grid));
    window
}

/// The status bar: where the selection is, and what it adds up to.
///
/// The aggregates are `App::preview` over generated formulas rather than a second summing
/// loop here — `SUM`, `COUNTA` (a status bar's Count is non-empty, not numeric) and
/// `AVERAGE` — so what the bar says and what a cell would say cannot differ.
///
/// Debounced, because a drag changes the selection on every motion event and each change
/// costs a walk of the range. ponytail: the walk runs on the main thread, so a selection
/// spanning a very large used extent stutters the drag; the plan's answer is a worker and a
/// generation counter, which is the threading milestone's machinery rather than this one's.
fn status_bar(grid: &grid::Grid, app: &Arc<App>) -> gtk::Box {
    let label = gtk::Label::builder()
        .xalign(0.0)
        .margin_start(10)
        .margin_end(10)
        .margin_top(4)
        .margin_bottom(4)
        .build();
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    // libadwaita paints a toolbar; without the class the bar is transparent and the grid
    // scrolls underneath the text.
    bar.add_css_class("toolbar");
    bar.append(&label);

    let pending: Rc<Cell<Option<gtk::glib::SourceId>>> = Rc::new(Cell::new(None));
    let app = app.clone();
    let sheet = grid.sheet();
    grid.connect_selection_changed(move |selection| {
        if let Some(source) = pending.take() {
            source.remove();
        }
        let (app, label, pending2) = (app.clone(), label.clone(), pending.clone());
        pending.set(Some(gtk::glib::timeout_add_local_once(
            std::time::Duration::from_millis(100),
            move || {
                pending2.set(None);
                label.set_text(&status_text(&app, sheet, selection));
            },
        )));
    });
    // Nothing has moved yet, so the first paint would otherwise show an empty bar.
    grid.set_selection(grid.selection());
    bar
}

/// `B2:B4 — Sum 1,234.5 · Count 3 · Average 411.5`, or just the address when the selection
/// holds nothing.
fn status_text(app: &App, sheet: usize, selection: Selection) -> String {
    let (start, end) = selection.rect();
    if selection.is_single() {
        // One cell has nothing to add up, and every other spreadsheet stays quiet about it.
        return a1::format(None, start);
    }
    let address = format!("{}:{}", a1::format(None, start), a1::format(None, end));
    // Clamped to the used extent first: a whole-column selection must not ask the evaluator
    // to walk a million empty rows.
    let Ok((rows, cols)) = app.used_extent(sheet) else {
        return address;
    };
    let end = Pos::new(end.row.min(rows.saturating_sub(1)), end.col.min(cols.saturating_sub(1)));
    if rows == 0 || cols == 0 || end.row < start.row || end.col < start.col {
        return address;
    }

    let range = format!("[.{}:.{}]", a1::format(None, start), a1::format(None, end));
    // Evaluated at a cell one past the used extent: a formula is evaluated *as if* it sat
    // somewhere, and somewhere inside the range would be a circular reference.
    let at = Pos::new(rows, 0);
    let of = |formula: String| match app.preview(sheet, at, &formula) {
        Ok(CellValue::Number(n)) => Some(n),
        _ => None,
    };
    let count = of(format!("=COUNTA({range})")).unwrap_or(0.0);
    if count == 0.0 {
        return address;
    }
    let mut parts = vec![address, format!("Count {}", show(count))];
    // Sum and Average of no numbers are not zero, they are nothing — AVERAGE says so with
    // #DIV/0!, which is why both are read back as an optional number.
    if let Some(sum) = of(format!("=SUM({range})"))
        && let Some(average) = of(format!("=AVERAGE({range})"))
    {
        parts.insert(1, format!("Sum {}", show(sum)));
        parts.push(format!("Average {}", show(average)));
    }
    parts.join("  ·  ")
}

fn show(n: f64) -> String {
    sheet_core::formula::value::format_number(n)
}

/// The document's name, or what an unsaved one is called until it has one.
fn title(path: Option<&std::path::Path>) -> String {
    path.and_then(|p| p.file_name())
        .map_or_else(|| "Untitled".to_owned(), |name| name.to_string_lossy().into_owned())
}

/// Draw one frame to a PNG and quit — the smoke path behind `--render-to`.
///
/// A frame late enough to be real: the window has to be mapped and allocated before the
/// grid knows how many cells fit, so this waits for a beat rather than rendering an
/// unallocated widget and writing a blank image.
fn render_once(window: &adw::ApplicationWindow, target: PathBuf) {
    let window = window.clone();
    gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(600), move || {
        let width = window.width();
        let height = window.height();
        let paintable = gtk::WidgetPaintable::new(Some(&window));
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(&snapshot, f64::from(width), f64::from(height));

        let result = window
            .native()
            .and_then(|native| native.renderer())
            .zip(snapshot.to_node())
            .map(|(renderer, node)| renderer.render_texture(&node, None))
            .ok_or_else(|| "the window has no renderer yet".to_owned())
            .and_then(|texture| texture.save_to_png(&target).map_err(|e| e.to_string()));

        match result {
            Ok(()) => println!("{}", target.display()),
            Err(error) => eprintln!("sheet-gtk: --render-to: {error}"),
        }
        window.close();
    });
}
