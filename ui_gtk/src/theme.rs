// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Every colour the grid draws, resolved from the running theme in one place.
//!
//! A custom-drawn widget has to pick its own colours, and the failure mode is a grid that
//! is white in a dark theme. So nothing here is a literal: the foreground comes from the
//! widget, the rest from the theme's named colours, and the whole palette is rebuilt
//! whenever the style changes (see `grid.rs`'s `css_changed`).
//!
//! `lookup_color` is deprecated in GTK 4.10 and has no replacement — there is no other way
//! to read a named theme colour, and the alternative is hardcoding one. It is called
//! exactly here, with the fallbacks that make a missing name harmless.

use libadwaita::gtk;
use libadwaita::prelude::*;

use gtk::gdk;

/// The one stylesheet this shell installs: the in-cell editor.
///
/// A bare `gtk::Text` is transparent and carries the theme's entry padding, so over a cell
/// it shows the value underneath and puts the caret a few pixels off from where the grid
/// draws text. Both are fixed with named theme colours rather than literals, so it follows
/// light, dark and high-contrast like everything else here.
const EDITOR_CSS: &str = "
.sheet-editor {
  background-color: @view_bg_color;
  color: @view_fg_color;
  caret-color: @view_fg_color;
  padding: 0 3px;
  margin: 0;
  min-height: 0;
  border: none;
  border-radius: 0;
  box-shadow: none;
  outline: none;
}
";

/// Install the stylesheet, once, for the whole display.
pub fn install() {
    let Some(display) = gdk::Display::default() else {
        return;
    };
    let provider = gtk::CssProvider::new();
    provider.load_from_string(EDITOR_CSS);
    gtk::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    /// The sheet behind the cells.
    pub background: gdk::RGBA,
    /// Cell text.
    pub foreground: gdk::RGBA,
    /// The lines between cells.
    pub lines: gdk::RGBA,
    /// The header band behind the row numbers and column letters.
    pub header: gdk::RGBA,
    /// Row numbers and column letters.
    pub header_text: gdk::RGBA,
    /// The accent, for the selection and the active cell.
    pub accent: gdk::RGBA,
}

impl Palette {
    /// Read the palette out of `widget`'s current style.
    pub fn of(widget: &impl IsA<gtk::Widget>) -> Self {
        let widget = widget.as_ref();
        // `GtkWidget:color` is a 4.10 getter this build's gtk4 feature set does not
        // expose, and the named colour is what it returns anyway.
        let foreground = named(widget, "view_fg_color")
            .or_else(|| named(widget, "theme_fg_color"))
            .unwrap_or(BLACK);
        let background = named(widget, "view_bg_color").unwrap_or(WHITE);
        Self {
            background,
            foreground,
            // Borders are the one colour a theme reliably gets right for hairlines; a
            // faded foreground is the fallback, which works in both light and dark
            // because it is *the* foreground rather than a guess at one.
            lines: named(widget, "borders").unwrap_or(with_alpha(foreground, 0.15)),
            header: named(widget, "headerbar_bg_color")
                .or_else(|| named(widget, "window_bg_color"))
                .unwrap_or(with_alpha(foreground, 0.05)),
            header_text: with_alpha(foreground, 0.7),
            // libadwaita ≥ 1.6 has an accent API; on 1.5 the named colour is the way, and
            // it is what the API returns anyway.
            accent: named(widget, "accent_bg_color").unwrap_or(BLUE),
        }
    }
}

const WHITE: gdk::RGBA = gdk::RGBA::WHITE;
const BLACK: gdk::RGBA = gdk::RGBA::BLACK;
const BLUE: gdk::RGBA = gdk::RGBA::new(0.21, 0.52, 0.89, 1.0);

#[allow(deprecated)]
fn named(widget: &gtk::Widget, name: &str) -> Option<gdk::RGBA> {
    widget.style_context().lookup_color(name)
}

pub fn with_alpha(color: gdk::RGBA, alpha: f32) -> gdk::RGBA {
    gdk::RGBA::new(color.red(), color.green(), color.blue(), alpha)
}
