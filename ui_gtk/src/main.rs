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
//! `doc/gtk-shell.md` is the plan and the running record of what is built. This is
//! milestone 1: a document opens and draws, and nothing edits it yet.

mod geom;
mod grid;
mod theme;

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use libadwaita as adw;
use libadwaita::gtk;
use libadwaita::prelude::*;

use sheet_core::App;

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

    adw::ApplicationWindow::builder()
        .application(application)
        .title(title(path))
        .default_width(1100)
        .default_height(700)
        .content(&view)
        .build()
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
