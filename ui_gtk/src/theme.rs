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

/// Eight colours for the references in a formula, in the order they appear.
///
/// These are the one place a colour is **not** taken from the theme, and deliberately: they
/// are data colours, like a chart's series, and a theme has no opinion about the fourth
/// reference in a formula. What the theme decides is which of the two sets to use — the
/// darker one reads on a light sheet and the lighter one on a dark sheet, which is the part
/// that would otherwise be unreadable.
pub fn reference_palette(dark: bool) -> [gdk::RGBA; 8] {
    let hex = match dark {
        false => [
            (0x1c, 0x71, 0xd8), // blue
            (0x2e, 0xc2, 0x7e), // green
            (0xc6, 0x46, 0x00), // orange
            (0x81, 0x3d, 0x9c), // purple
            (0xc0, 0x1c, 0x28), // red
            (0x18, 0x65, 0x6a), // teal
            (0xa5, 0x1d, 0x2d), // maroon
            (0x86, 0x5e, 0x3c), // brown
        ],
        true => [
            (0x78, 0xae, 0xed),
            (0x8f, 0xf0, 0xa4),
            (0xff, 0xbe, 0x6f),
            (0xdc, 0x8a, 0xdd),
            (0xff, 0x7b, 0x63),
            (0x5b, 0xc8, 0xaf),
            (0xf6, 0x61, 0x51),
            (0xcd, 0xab, 0x8f),
        ],
    };
    hex.map(|(r, g, b): (u8, u8, u8)| {
        gdk::RGBA::new(
            f32::from(r) / 255.0,
            f32::from(g) / 255.0,
            f32::from(b) / 255.0,
            1.0,
        )
    })
}

/// Which colour each reference in a formula gets: one per **distinct** reference, so the
/// same range mentioned twice is the same colour in both places and in the grid.
///
/// The spans come from `display::spans` — the scanner that also decides what a reference
/// *is* when the formula is committed — so what is coloured and what is stored cannot
/// disagree.
pub fn reference_colors(text: &str, dark: bool) -> Vec<(std::ops::Range<usize>, gdk::RGBA)> {
    let palette = reference_palette(dark);
    let mut seen: Vec<&str> = Vec::new();
    grind_sheet::formula::display::spans(text)
        .into_iter()
        .filter(|span| span.kind == grind_sheet::formula::display::TokenKind::Ref)
        .map(|span| {
            let source = &text[span.range.clone()];
            let index = seen.iter().position(|s| *s == source).unwrap_or_else(|| {
                seen.push(source);
                seen.len() - 1
            });
            (span.range, palette[index % palette.len()])
        })
        .collect()
}

/// The same colours as Pango attributes, for a `GtkEditable` showing the formula.
///
/// Byte indices line up by construction: `display::spans` reports bytes because Pango
/// counts in bytes.
pub fn reference_attributes(text: &str, dark: bool) -> gtk::pango::AttrList {
    let attributes = gtk::pango::AttrList::new();
    for (range, color) in reference_colors(text, dark) {
        let mut attribute = gtk::pango::AttrColor::new_foreground(
            (color.red() * 65535.0) as u16,
            (color.green() * 65535.0) as u16,
            (color.blue() * 65535.0) as u16,
        );
        attribute.set_start_index(range.start as u32);
        attribute.set_end_index(range.end as u32);
        attributes.insert(attribute);
    }
    attributes
}

/// Whether the running theme is a dark one, which is all the reference palette needs to
/// know. Read from the background rather than from a setting, so a high-contrast or
/// hand-rolled theme is classified by what it actually looks like.
pub fn is_dark(palette: &Palette) -> bool {
    let bg = palette.background;
    0.299 * bg.red() + 0.587 * bg.green() + 0.114 * bg.blue() < 0.5
}

#[allow(deprecated)]
fn named(widget: &gtk::Widget, name: &str) -> Option<gdk::RGBA> {
    widget.style_context().lookup_color(name)
}

/// A colour **the document** chose — `fo:color`, `fo:background-color`, a border's third
/// field — as a colour to paint with.
///
/// The one place a colour does not come from the theme, and it has to be: a cell that says
/// `#ffff00` is yellow in every session. `None` for anything not to be painted, which is
/// where the two cases that are not colours live: ODF's `"transparent"` (GDK parses it as
/// opaque black, so leaving it to the parser would be a very visible bug) and a value this
/// build does not recognise — a document's attribute is whatever the document said
/// (`core/src/style.rs`), so it is dropped at the point of painting rather than at the point
/// of reading, where dropping it would lose the cell.
pub fn color(value: &str) -> Option<gdk::RGBA> {
    match value.trim() {
        "transparent" | "none" => None,
        value => value.parse().ok(),
    }
}

pub fn with_alpha(color: gdk::RGBA, alpha: f32) -> gdk::RGBA {
    gdk::RGBA::new(color.red(), color.green(), color.blue(), alpha)
}
