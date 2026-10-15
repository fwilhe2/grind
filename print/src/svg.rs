// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! An SVG picture, parsed once for both backends: the PDF writes it as vectors (`krilla-svg`),
//! the preview rasterises it (`resvg`), and both from the same SVG bytes and the same fonts so they agree.
//!
//! An SVG's text is set in the export's own faces — the bundled ones and whatever the machine
//! added — so a chart's labels come out in the fonts the rest of the page uses, and a browser
//! with no fonts installed still draws them.

use std::sync::Arc;

use crate::fonts::Fonts;

/// One parse, spelled once for the two `usvg` versions in the build: `krilla-svg` is built against
/// one and `resvg` against the next, and a tree of either is no tree of the other.
macro_rules! parse {
    ($usvg:path, $data:expr, $fonts:expr) => {{
        use $usvg as usvg;
        let mut db = usvg::fontdb::Database::new();
        for face in $fonts.faces() {
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
        usvg::Tree::from_data($data, &options).ok()
    }};
}

/// The picture's tree for the PDF, or `None` for bytes that are no SVG this can read.
pub fn tree(data: &[u8], fonts: &Fonts) -> Option<usvg::Tree> {
    parse!(usvg, data, fonts)
}

/// The same picture's tree for the preview, in the `usvg` that `resvg` draws.
pub fn preview_tree(data: &[u8], fonts: &Fonts) -> Option<resvg::usvg::Tree> {
    parse!(resvg::usvg, data, fonts)
}
