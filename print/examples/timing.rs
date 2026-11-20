// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! How long a document takes to set and a page to draw — the numbers `doc/page-model.md` §4
//! holds: `cargo run --release -p grind-print --example timing -- report.odt [page]`.

use std::path::Path;
use std::time::Instant;

use grind_print::{Options, fonts_for, raster, typeset};

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: timing <document> [page]");
        std::process::exit(2);
    };
    let page: usize = args.next().and_then(|n| n.parse().ok()).unwrap_or(1);

    let started = Instant::now();
    let app = grind_text::App::new();
    if let Err(error) = app.open_file(Path::new(&path)) {
        eprintln!("{path}: {error}");
        std::process::exit(1);
    }
    println!("open      {:>9.1} ms", ms(started));

    let started = Instant::now();
    let fonts = fonts_for(&app);
    println!("fonts     {:>9.1} ms", ms(started));

    let started = Instant::now();
    let (doc, setter) = typeset(&app, fonts, &Options::default());
    let set = ms(started);
    let pages = doc.pages.len();
    println!(
        "typeset   {set:>9.1} ms  {pages} pages, {:.2} ms a page",
        set / pages.max(1) as f64
    );

    // What an edit costs: the same document set again by the same typesetter, which has
    // measured all of it before.
    let started = Instant::now();
    let again = grind_print::text::typeset(&app, &setter, &grind_print::text::Options::default());
    println!(
        "again     {:>9.1} ms  {} pages",
        ms(started),
        again.pages.len()
    );

    // What a keystroke costs: a word typed in the middle, the pages set again from the ones
    // before, which copies those it finds unchanged. The best of five, since one is noisy.
    let middle = app.block_count() / 2;
    let options = grind_print::text::Options::default();
    let mut previous = again;
    let mut best = f64::INFINITY;
    for _ in 0..5 {
        let _ = app.insert_text(
            grind_text::Caret {
                block: middle,
                offset: 0,
            },
            "word ",
        );
        let started = Instant::now();
        previous = grind_print::text::typeset_after(&app, &setter, &options, Some(&previous));
        best = best.min(ms(started));
    }
    println!("edit      {best:>9.1} ms  {} pages", previous.pages.len());

    let Some(shown) = doc.pages.get(page.saturating_sub(1)) else {
        eprintln!("there is no page {page}");
        std::process::exit(1);
    };
    for scale in [96.0 / 72.0, 192.0 / 72.0] {
        let started = Instant::now();
        let frame = raster::render(shown, setter.fonts(), scale);
        println!(
            "raster    {:>9.1} ms  page {page} at {:.0} dpi, {}x{}",
            ms(started),
            scale * 72.0,
            frame.width,
            frame.height
        );
    }
    // What a window pays to draw a page again after an edit: its glyphs are kept from the last.
    let mut kept = raster::Rasteriser::default();
    let _ = kept.render(shown, setter.fonts(), 96.0 / 72.0);
    let started = Instant::now();
    let _ = kept.render(shown, setter.fonts(), 96.0 / 72.0);
    println!("again     {:>9.1} ms  page {page} at 96 dpi", ms(started));
}

fn ms(since: Instant) -> f64 {
    since.elapsed().as_secs_f64() * 1e3
}
