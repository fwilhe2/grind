// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Every colour the grid draws, resolved from the running theme in one place. **Nothing here is a
//! literal**: the foreground comes from the widget, the rest from the theme's named colours,
//! rebuilt on every style change (`grid.rs`'s style-manager notifies). `lookup_color` is
//! deprecated in GTK 4.10 with no replacement and is the only way to read a named theme colour; it
//! is called here alone, with fallbacks.

use libadwaita::gtk;
use libadwaita::prelude::*;

use gtk::gdk;

/// The one stylesheet this shell installs: the in-cell editor. A bare `gtk::Text` is transparent
/// and carries the theme's entry padding, so over a cell it shows the value underneath and
/// misplaces the caret; both are fixed with named theme colours, so it follows light, dark and
/// high-contrast.
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
    /// A misspelt word's wavy underline — the theme's own error colour, the red libadwaita
    /// gives every error, as the word processor's window draws it.
    pub misspelt: gdk::RGBA,
}

impl Palette {
    /// Read the palette out of `widget`'s current style.
    pub fn of(widget: &impl IsA<gtk::Widget>) -> Self {
        let widget = widget.as_ref();
        // `GtkWidget:color` is a 4.10 getter the gtk4 features here do not expose; the named
        // colour is what it returns anyway.
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
            misspelt: named(widget, "error_color").unwrap_or(RED),
        }
    }
}

const WHITE: gdk::RGBA = gdk::RGBA::WHITE;
const BLACK: gdk::RGBA = gdk::RGBA::BLACK;
const BLUE: gdk::RGBA = gdk::RGBA::new(0.21, 0.52, 0.89, 1.0);
const RED: gdk::RGBA = gdk::RGBA::new(0.88, 0.11, 0.14, 1.0);

/// Eight colours for the references in a formula, in order of appearance — **not** from the theme:
/// they are data colours, like a chart's series. The theme decides only which set: darker on a
/// light sheet, lighter on a dark one.
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

/// The colour role mode draws a cell's text in, or `None` for a role drawn as nothing. **Hues from
/// `style::PALETTE`, readability from the theme** (`doc/view-modes.md` §4.5): `readable` walks the
/// hue towards the theme's foreground until it separates from the background, so navy on a dark
/// sheet can be seen. A computed cell takes the foreground — the "black" of every hand-coloured
/// financial model — and a label the foreground muted.
pub fn role_color(role: grind_sheet::view::CellRole, palette: &Palette) -> Option<gdk::RGBA> {
    use grind_sheet::view::Hue;
    // Which colour each role is — the financial-modelling convention — is the core's
    // (`CellRole::hue`); what is this window's is resolving the ink and lifting a hue until it
    // is legible on this theme's ground.
    match role.hue() {
        Hue::None => None,
        Hue::Palette(name) => grind_sheet::style::palette(name)
            .and_then(|hex| hex.parse::<gdk::RGBA>().ok())
            .map(|hue| readable(hue, palette)),
        Hue::Ink => Some(palette.foreground),
        Hue::Quiet => Some(with_alpha(palette.foreground, 0.6)),
    }
}

/// A hue moved towards the theme's foreground until legible on its background — towards the
/// foreground rather than white or black, so a high-contrast theme lands where its own text does.
fn readable(hue: gdk::RGBA, palette: &Palette) -> gdk::RGBA {
    let luminance = |c: gdk::RGBA| 0.299 * c.red() + 0.587 * c.green() + 0.114 * c.blue();
    let background = luminance(palette.background);
    let mix = |t: f32| {
        let blend = |a: f32, b: f32| a + (b - a) * t;
        gdk::RGBA::new(
            blend(hue.red(), palette.foreground.red()),
            blend(hue.green(), palette.foreground.green()),
            blend(hue.blue(), palette.foreground.blue()),
            1.0,
        )
    };
    // Ten steps is enough resolution for a colour and terminates whatever the theme is; the
    // last one is the foreground itself, which is legible by definition.
    (0..=10)
        .map(|step| mix(step as f32 / 10.0))
        .find(|c| (luminance(*c) - background).abs() >= 0.35)
        .unwrap_or(palette.foreground)
}

/// Which colour each reference gets: one per **distinct** reference, so the same range twice is
/// one colour in both places and in the grid. The spans are `display::spans`', the scanner that
/// decides what a reference is when the formula commits.
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

/// The same colours as Pango attributes, for a `GtkEditable` showing the formula; `display::spans`
/// reports bytes, as Pango counts.
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

/// The ink a cell's text is drawn in — `color` and `fill` the document's own `fo:color` and
/// `fo:background-color`, if it said them. Nothing here is written.
///
/// - **No colour of its own** is ODF's automatic: the theme's ink, unless the document's fill
///   makes it unreadable, then black or white ([`grind_core::color::automatic_ink`]) — white theme
///   text on a silver heading row.
/// - **A colour of its own on the theme's ground in a dark theme** is lifted along its hue until
///   it reads ([`grind_core::color::legible`]): navy chosen on white is invisible on a dark sheet.
/// - **A colour on a fill of its own**, or in a light theme, is the document's decision, drawn as
///   it is.
pub fn ink(color: Option<&str>, fill: Option<&str>, palette: &Palette) -> gdk::RGBA {
    let own = color.and_then(self::color);
    let fill = fill.and_then(self::color);
    match (own, fill) {
        // The page's own ink and a document's own colour are handed back as they were, rather
        // than through three bytes and back — a theme's ink need not be 8-bit.
        (None, None) => palette.foreground,
        (Some(own), Some(_)) => own,
        _ => rgba(grind_core::color::document_ink(
            own.map(rgb8),
            fill.map(rgb8),
            rgb8(palette.background),
            rgb8(palette.foreground),
            is_dark(palette),
        )),
    }
}

/// A colour as the core's three bytes ([`grind_core::color::Rgb`]), alpha dropped: a cell's
/// colour is opaque in ODF, and a theme's is compared as it lands.
pub fn rgb8(c: gdk::RGBA) -> grind_core::color::Rgb {
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    (byte(c.red()), byte(c.green()), byte(c.blue()))
}

pub fn rgba((r, g, b): grind_core::color::Rgb) -> gdk::RGBA {
    gdk::RGBA::new(
        f32::from(r) / 255.0,
        f32::from(g) / 255.0,
        f32::from(b) / 255.0,
        1.0,
    )
}

/// Whether the running theme is a dark one, which is all the reference palette needs to
/// know. Read from the background rather than from a setting, so a high-contrast or
/// hand-rolled theme is classified by what it actually looks like.
pub fn is_dark(palette: &Palette) -> bool {
    let bg = palette.background;
    0.299 * bg.red() + 0.587 * bg.green() + 0.114 * bg.blue() < 0.5
}

/// Whether an announcement from `widget` reaches anybody. GTK's fallback accessibility context —
/// used with no AT-SPI bus, as in a container, a VM or a minimal session — has no `announce` hook
/// and `gtk_accessible_announce` calls it anyway, a jump to address zero (measured on GTK 4.18
/// under Xvfb). With `GTK_A11Y=none` there is no context at all, which the generated binding
/// asserts against. The raw call is the one spelling that can say "none".
pub fn heard(widget: &impl IsA<gtk::Accessible>) -> bool {
    use gtk::glib::translate::{ToGlibPtr, from_glib_full};
    // SAFETY: the call is transfer-full and documented to return NULL when accessibility is off;
    // `from_glib_full` into an `Option` takes the reference and maps NULL to `None`.
    let context: Option<gtk::ATContext> = unsafe {
        from_glib_full(gtk::ffi::gtk_accessible_get_at_context(
            widget.as_ref().to_glib_none().0,
        ))
    };
    context.is_some_and(|context| context.type_().name() != "GtkTestATContext")
}

#[allow(deprecated)]
fn named(widget: &gtk::Widget, name: &str) -> Option<gdk::RGBA> {
    widget.style_context().lookup_color(name)
}

/// A colour the **document** chose — `fo:color`, `fo:background-color`, a border's third field —
/// as a colour to paint with; a cell that says `#ffff00` is yellow in every session. `None` for
/// ODF's `transparent` (GDK would parse it as opaque black) and for anything unrecognised, dropped
/// at painting rather than at reading, where it would lose the cell.
pub fn color(value: &str) -> Option<gdk::RGBA> {
    match value.trim() {
        "transparent" | "none" => None,
        value => value.parse().ok(),
    }
}

pub fn with_alpha(color: gdk::RGBA, alpha: f32) -> gdk::RGBA {
    gdk::RGBA::new(color.red(), color.green(), color.blue(), alpha)
}

#[cfg(test)]
mod tests {
    use super::*;
    use grind_sheet::view::CellRole;

    fn ground(background: gdk::RGBA, foreground: gdk::RGBA) -> Palette {
        Palette {
            background,
            foreground,
            lines: foreground,
            header: background,
            header_text: foreground,
            accent: BLUE,
            misspelt: RED,
        }
    }

    /// Role hues are the palette's and readability the theme's (§4.5): every role colour separates
    /// from the ground it is drawn on, on both grounds.
    #[test]
    fn every_role_colour_separates_from_the_ground_it_is_drawn_on() {
        let luminance = |c: gdk::RGBA| 0.299 * c.red() + 0.587 * c.green() + 0.114 * c.blue();
        for palette in [ground(WHITE, BLACK), ground(BLACK, WHITE)] {
            let ground = luminance(palette.background);
            for role in CellRole::ALL {
                let Some(color) = role_color(role, &palette) else {
                    assert_eq!(role, CellRole::Empty, "{} has no colour", role.name());
                    continue;
                };
                // A label is the foreground, muted — legible by construction, and the alpha
                // is what makes its luminance say otherwise.
                if role == CellRole::Label {
                    continue;
                }
                assert!(
                    (luminance(color) - ground).abs() >= 0.35,
                    "{} is {color:?} on a ground of {ground}",
                    role.name()
                );
            }
        }
    }

    /// Every palette colour a document can give its text reads on a dark sheet, a fill it chose
    /// gets automatic ink that reads on it, and in a light theme the document's colours are drawn
    /// as they are.
    #[test]
    fn a_documents_ink_reads_on_whatever_it_is_drawn_on() {
        use grind_core::color::{TEXT, contrast};
        let dark = ground(
            gdk::RGBA::new(0.12, 0.12, 0.12, 1.0),
            gdk::RGBA::new(0.93, 0.93, 0.93, 1.0),
        );
        let light = ground(gdk::RGBA::WHITE, gdk::RGBA::BLACK);
        for (name, hex) in grind_sheet::style::PALETTE {
            let drawn = ink(Some(hex), None, &dark);
            assert!(
                contrast(rgb8(drawn), rgb8(dark.background)) >= TEXT,
                "{name} text is unreadable on a dark sheet"
            );
            assert_eq!(ink(Some(hex), None, &light), color(hex).unwrap(), "{name}");
            // Automatic ink on the document's own fill, in either theme.
            for palette in [&dark, &light] {
                let on_fill = ink(None, Some(hex), palette);
                let fill = rgb8(color(hex).unwrap());
                assert!(
                    contrast(rgb8(on_fill), fill) >= TEXT
                        || contrast(rgb8(on_fill), fill)
                            >= contrast((0, 0, 0), fill).max(contrast((255, 255, 255), fill)),
                    "automatic ink on {name} does not read"
                );
            }
        }
        // The silver heading row that started it: white theme ink gives way to black.
        assert_eq!(ink(None, Some("#dddddd"), &dark), gdk::RGBA::BLACK);
    }

    /// Roles are read by colour first, so two sharing one is the mode failing at its job.
    #[test]
    fn the_roles_do_not_share_a_colour() {
        let palette = ground(WHITE, BLACK);
        let mut seen: Vec<gdk::RGBA> = Vec::new();
        for role in CellRole::ALL {
            let Some(color) = role_color(role, &palette) else {
                continue;
            };
            assert!(!seen.contains(&color), "{} repeats a colour", role.name());
            seen.push(color);
        }
    }
}
