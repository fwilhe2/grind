// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! An SVG picture, parsed once for both backends: the PDF writes it as vectors (`krilla-svg`),
//! the preview rasterises it (`resvg`), and both from the same `usvg` tree so they agree.
//!
//! An SVG's text is set in the export's own faces — the bundled ones and whatever the machine
//! added — so a chart's labels come out in the fonts the rest of the page uses, and a browser
//! with no fonts installed still draws them.

use std::sync::Arc;

use crate::fonts::Fonts;

/// The picture's tree, or `None` for bytes that are no SVG this can read.
pub fn tree(data: &[u8], fonts: &Fonts) -> Option<usvg::Tree> {
    let mut db = usvg::fontdb::Database::new();
    for face in fonts.faces() {
        db.load_font_source(usvg::fontdb::Source::Binary(Arc::new(
            face.bytes().to_vec(),
        )));
    }
    db.set_sans_serif_family("Liberation Sans");
    db.set_serif_family("Liberation Serif");
    db.set_monospace_family("Liberation Mono");
    let options = usvg::Options {
        font_family: "Liberation Sans".to_owned(),
        fontdb: Arc::new(db),
        ..usvg::Options::default()
    };
    usvg::Tree::from_data(data, &options).ok()
}
