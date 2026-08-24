// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Every colour the document view draws, resolved from the running theme in one place.
//!
//! `ui_gtk/src/theme.rs`'s rule, unchanged and worth repeating: **nothing here is a
//! literal.** A custom-drawn widget has to pick its own colours, and the failure mode is a
//! page that stays white in a dark theme. The whole palette is rebuilt whenever the style
//! changes.
//!
//! `lookup_color` is deprecated in GTK 4.10 and has no replacement — there is no other way to
//! read a named theme colour, and the alternative is hardcoding one. It is called exactly
//! here, with fallbacks that make a missing name harmless.
//!
//! Smaller than the spreadsheet's palette, because a page of prose has fewer parts than a
//! grid: paper, ink, a dimmed ink for the furniture a document did not ask for, and the
//! accent the caret is drawn in.

use libadwaita::gtk;
use libadwaita::prelude::*;

use gtk::gdk;

const BLACK: gdk::RGBA = gdk::RGBA::BLACK;
const WHITE: gdk::RGBA = gdk::RGBA::WHITE;

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    /// The page behind the text.
    pub background: gdk::RGBA,
    /// The text itself.
    pub foreground: gdk::RGBA,
    /// Anything the shell draws that the document does not contain — a list's bullet.
    pub dim: gdk::RGBA,
    /// The caret.
    pub accent: gdk::RGBA,
}

impl Palette {
    /// Read the palette out of `widget`'s current style.
    pub fn of(widget: &impl IsA<gtk::Widget>) -> Self {
        let widget = widget.as_ref();
        let foreground = named(widget, "view_fg_color")
            .or_else(|| named(widget, "theme_fg_color"))
            .unwrap_or(BLACK);
        Palette {
            background: named(widget, "view_bg_color")
                .or_else(|| named(widget, "theme_base_color"))
                .unwrap_or(WHITE),
            foreground,
            dim: with_alpha(foreground, 0.55),
            accent: named(widget, "accent_color")
                .or_else(|| named(widget, "theme_selected_bg_color"))
                .unwrap_or(foreground),
        }
    }
}

#[allow(deprecated)]
fn named(widget: &gtk::Widget, name: &str) -> Option<gdk::RGBA> {
    widget.style_context().lookup_color(name)
}

fn with_alpha(color: gdk::RGBA, alpha: f32) -> gdk::RGBA {
    gdk::RGBA::new(color.red(), color.green(), color.blue(), alpha)
}
