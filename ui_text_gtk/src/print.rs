// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Export PDF…, Print… and Print Preview (`doc/pdf-export.md` §3–§4).
//!
//! **The core makes the PDF, and this window only hands it on.** Export writes
//! `grind_print::export`'s bytes; Print gives the same bytes to `gtk::PrintDialog`, which prints
//! a file as it is rather than asking this window to draw pages a second time through cairo; and
//! the preview shows `grind_print::raster`'s pixels of the very display list the PDF is written
//! from. So what the preview shows, what the file holds and what the printer puts down are one
//! typesetting, not three.
//!
//! The arithmetic is portable and tested here with no display: the name a PDF is offered under,
//! a page's label, and the zoom steps.

use std::cell::Cell;
use std::rc::Rc;

use libadwaita as adw;
use libadwaita::gtk;
use libadwaita::prelude::*;

use gtk::{gdk, gio, glib};

use crate::Ui;

/// The zoom levels the preview steps through, as fractions of the page's real size on a screen
/// at 96 dpi. 100% is the page at its printed size.
pub const ZOOMS: [f64; 7] = [0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 2.0];

/// Where the preview opens: the whole width of a page in a laptop-sized window.
pub const START_ZOOM: usize = 2;

/// The name a PDF of the document at `path` is offered under.
pub fn pdf_name(path: Option<&std::path::Path>) -> String {
    let stem = path.and_then(std::path::Path::file_stem).map_or_else(
        || "Untitled".to_owned(),
        |s| s.to_string_lossy().into_owned(),
    );
    format!("{stem}.pdf")
}

/// The preview's subtitle: how many pages, and on what paper.
pub fn subtitle(pages: usize, paper: &grind_core::page::PageGeometry) -> String {
    let count = format!("{pages} page{}", if pages == 1 { "" } else { "s" });
    let name = match paper.iso_name() {
        Some(name) if paper.width > paper.height => format!("{name} landscape"),
        Some(name) => name,
        None => {
            let mm = |v: f64| (v * 10.0).round() / 10.0;
            format!("{} × {} mm", mm(paper.width), mm(paper.height))
        }
    };
    format!("{count} · {name}")
}

/// One step in or out from zoom `at`, staying inside [`ZOOMS`].
pub fn zoom_step(at: usize, inward: bool) -> usize {
    match inward {
        true => (at + 1).min(ZOOMS.len() - 1),
        false => at.saturating_sub(1),
    }
}

/// Pixels per point at a zoom: 96 dpi on a screen at 100%, times the screen's own scale.
pub fn pixels_per_point(zoom: f64, screen_scale: f64) -> f32 {
    (zoom * screen_scale * 96.0 / 72.0) as f32
}

/// What every export from this window asks for: the document's own page, titled after its file.
fn options(ui: &Ui) -> grind_print::Options {
    let title = ui
        .path
        .borrow()
        .as_deref()
        .and_then(std::path::Path::file_stem)
        .map(|stem| stem.to_string_lossy().into_owned());
    grind_print::Options { paper: None, title }
}

fn pdf_filters() -> gio::ListStore {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("PDF"));
    filter.add_pattern("*.pdf");
    filter.add_mime_type("application/pdf");
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    filters
}

/// File ▸ Export PDF… — the document typeset and written as a PDF; the toast is the export's
/// own report, which names anything it had to substitute.
pub fn export_pdf(ui: &Rc<Ui>) {
    let dialog = gtk::FileDialog::builder()
        .title("Export PDF")
        .filters(&pdf_filters())
        .initial_name(pdf_name(ui.path.borrow().as_deref()))
        .build();
    dialog.save(
        Some(&ui.window),
        gio::Cancellable::NONE,
        glib::clone!(
            #[strong]
            ui,
            move |result| {
                let Some(path) = result.ok().and_then(|file| file.path()) else {
                    return;
                };
                let written =
                    grind_print::export(&ui.app, grind_print::fonts_for(&ui.app), &options(&ui))
                        .and_then(|(bytes, report)| {
                            grind_core::atomic::write(&path, bytes)
                                .map(|()| report)
                                .map_err(|e| e.to_string())
                        });
                match written {
                    Ok(report) => ui.toast(&format!(
                        "Exported {} — {}",
                        path.display(),
                        report.summary()
                    )),
                    Err(error) => ui.toast(&format!("Could not export: {error}")),
                }
            }
        ),
    );
}

/// File ▸ Print… — the same PDF, handed to the system's print dialog as a file, so the printer
/// gets what Export would have written rather than a second drawing of it.
pub fn print(ui: &Rc<Ui>) {
    let bytes = match grind_print::export(&ui.app, grind_print::fonts_for(&ui.app), &options(ui)) {
        Ok((bytes, _)) => bytes,
        Err(error) => return ui.toast(&format!("Could not print: {error}")),
    };
    let file = glib::tmp_dir().join(format!(
        "grind-print-{}-{}.pdf",
        std::process::id(),
        glib::monotonic_time()
    ));
    if let Err(error) = std::fs::write(&file, bytes) {
        return ui.toast(&format!("Could not print: {error}"));
    }
    gtk::PrintDialog::new().print_file(
        Some(&ui.window),
        None::<&gtk::PrintSetup>,
        &gio::File::for_path(&file),
        gio::Cancellable::NONE,
        glib::clone!(
            #[strong]
            ui,
            move |result| {
                let _ = std::fs::remove_file(&file);
                if let Err(error) = result
                    && !error.matches(gtk::DialogError::Dismissed)
                    && !error.matches(gtk::DialogError::Cancelled)
                {
                    ui.toast(&format!("Could not print: {error}"));
                }
            }
        ),
    );
}

/// File ▸ Print Preview — every page as it prints, in a window of its own.
///
/// Typeset once when it opens; a zoom only rasterises again. The pages are a snapshot: an edit
/// in the document window does not reach a preview already open, which says so in its title by
/// being a separate window rather than a mode of this one.
///
/// ponytail: every page is rasterised at each zoom, eagerly. Fine for the documents in hand (a
/// page is a few milliseconds); a hundred-page document wants the visible pages first.
pub fn preview(ui: &Rc<Ui>) -> adw::Window {
    let (doc, setter) =
        grind_print::typeset(&ui.app, grind_print::fonts_for(&ui.app), &options(ui));
    let paper = ui.app.page().unwrap_or_default();
    let window = adw::Window::builder()
        .title("Print Preview")
        .transient_for(&ui.window)
        .default_width(760)
        .default_height(920)
        .build();
    let title = adw::WindowTitle::new("Print Preview", &subtitle(doc.pages.len(), &paper));
    let header = adw::HeaderBar::builder().title_widget(&title).build();
    let zoom_out = gtk::Button::from_icon_name("zoom-out-symbolic");
    zoom_out.set_tooltip_text(Some("Zoom Out"));
    let zoom_in = gtk::Button::from_icon_name("zoom-in-symbolic");
    zoom_in.set_tooltip_text(Some("Zoom In"));
    let zoom = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    zoom.add_css_class("linked");
    zoom.append(&zoom_out);
    zoom.append(&zoom_in);
    header.pack_start(&zoom);
    let print_button = gtk::Button::with_label("Print…");
    print_button.add_css_class("suggested-action");
    let export_button = gtk::Button::with_label("Export PDF…");
    header.pack_end(&print_button);
    header.pack_end(&export_button);

    let pages = gtk::Box::new(gtk::Orientation::Vertical, 24);
    pages.set_halign(gtk::Align::Center);
    pages.set_margin_top(24);
    pages.set_margin_bottom(24);
    pages.set_margin_start(24);
    pages.set_margin_end(24);
    let pictures: Vec<gtk::Picture> = doc
        .pages
        .iter()
        .map(|_| {
            let picture = gtk::Picture::new();
            picture.set_can_shrink(false);
            picture.add_css_class("card");
            pages.append(&picture);
            picture
        })
        .collect();
    let scroller = gtk::ScrolledWindow::builder()
        .child(&pages)
        .vexpand(true)
        .build();
    let body = adw::ToolbarView::new();
    body.add_top_bar(&header);
    body.set_content(Some(&scroller));
    window.set_content(Some(&body));

    let state = Rc::new((doc, setter, pictures, Cell::new(START_ZOOM)));
    let paint = {
        let state = state.clone();
        let window = window.clone();
        move || {
            let (doc, setter, pictures, zoom) = &*state;
            let screen = f64::from(window.scale_factor().max(1));
            let scale = pixels_per_point(ZOOMS[zoom.get()], screen);
            for (page, picture) in doc.pages.iter().zip(pictures) {
                let raster = grind_print::raster::render(page, setter.fonts(), scale);
                let texture = gdk::MemoryTexture::new(
                    raster.width as i32,
                    raster.height as i32,
                    gdk::MemoryFormat::R8g8b8a8Premultiplied,
                    &glib::Bytes::from_owned(raster.rgba),
                    raster.width as usize * 4,
                );
                picture.set_paintable(Some(&texture));
                // Logical pixels, so a HiDPI screen gets its extra pixels as sharpness.
                let logical = |points: f32| (f64::from(points) * f64::from(scale) / screen) as i32;
                picture.set_size_request(logical(page.width), logical(page.height));
            }
        }
    };
    paint();
    let paint = Rc::new(paint);
    for (button, inward) in [(&zoom_in, true), (&zoom_out, false)] {
        let state = state.clone();
        let paint = paint.clone();
        button.connect_clicked(move |_| {
            let zoom = &state.3;
            zoom.set(zoom_step(zoom.get(), inward));
            paint();
        });
    }
    export_button.connect_clicked(glib::clone!(
        #[strong]
        ui,
        move |_| export_pdf(&ui)
    ));
    print_button.connect_clicked(glib::clone!(
        #[strong]
        ui,
        move |_| print(&ui)
    ));
    window.present();
    window
}

#[cfg(test)]
mod tests {
    use super::*;
    use grind_core::page::PageGeometry;
    use std::path::Path;

    #[test]
    fn a_pdf_is_offered_under_the_documents_own_name() {
        assert_eq!(pdf_name(Some(Path::new("/tmp/report.fodt"))), "report.pdf");
        assert_eq!(pdf_name(Some(Path::new("notes.md"))), "notes.pdf");
        assert_eq!(pdf_name(None), "Untitled.pdf");
    }

    #[test]
    fn the_subtitle_counts_pages_and_names_the_paper() {
        assert_eq!(subtitle(1, &PageGeometry::a4()), "1 page · A4");
        assert_eq!(
            subtitle(12, &PageGeometry::a4().landscape()),
            "12 pages · A4 landscape"
        );
        let letter = PageGeometry {
            width: 215.9,
            height: 279.4,
            ..PageGeometry::a4()
        };
        assert_eq!(subtitle(2, &letter), "2 pages · 215.9 × 279.4 mm");
    }

    #[test]
    fn zoom_steps_stop_at_either_end() {
        assert_eq!(zoom_step(START_ZOOM, true), START_ZOOM + 1);
        assert_eq!(zoom_step(START_ZOOM, false), START_ZOOM - 1);
        assert_eq!(zoom_step(0, false), 0);
        assert_eq!(zoom_step(ZOOMS.len() - 1, true), ZOOMS.len() - 1);
        assert_eq!(ZOOMS[3], 1.0, "100% is a step");
    }

    #[test]
    fn a_hundred_percent_is_the_printed_size_on_the_screen() {
        assert!((pixels_per_point(1.0, 1.0) - 96.0 / 72.0).abs() < 1e-6);
        assert!((pixels_per_point(0.5, 2.0) - 96.0 / 72.0).abs() < 1e-6);
    }
}
