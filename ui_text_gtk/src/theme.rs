// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Every colour the document view draws, resolved from the running theme in one place. **Nothing
//! here is a literal**: a custom-drawn widget that picks its own colours stays white in a dark
//! theme. Rebuilt on every style change.
//!
//! `lookup_color` is deprecated in GTK 4.10 with no replacement, and is the only way to read a
//! named theme colour; it is called here alone, with fallbacks. A page has fewer parts than a
//! grid: paper, ink, a dimmed ink for furniture, the caret's accent and a table's rules.

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
    /// The band behind selected text.
    pub selection: gdk::RGBA,
    /// A table's rules: quieter than the text, stronger than the furniture, derived from the
    /// theme's ink.
    pub rule: gdk::RGBA,
    /// A misspelt word's wavy underline — the theme's own error colour, the red libadwaita
    /// uses for a field that will not take what was typed.
    pub misspelt: gdk::RGBA,
}

impl Palette {
    /// The page and the ink a document's own colours are read against
    /// ([`crate::metrics::Paper`]).
    pub fn paper(&self) -> crate::metrics::Paper {
        let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        let rgb = |c: gdk::RGBA| (byte(c.red()), byte(c.green()), byte(c.blue()));
        crate::metrics::Paper {
            ink: rgb(self.foreground),
            page: rgb(self.background),
            link: rgb(self.accent),
        }
    }

    /// Read the palette out of `widget`'s current style.
    pub fn of(widget: &impl IsA<gtk::Widget>) -> Self {
        let widget = widget.as_ref();
        let foreground = named(widget, "view_fg_color")
            .or_else(|| named(widget, "theme_fg_color"))
            .unwrap_or(BLACK);
        let accent = named(widget, "accent_color")
            .or_else(|| named(widget, "theme_selected_bg_color"))
            .unwrap_or(foreground);
        Palette {
            background: named(widget, "view_bg_color")
                .or_else(|| named(widget, "theme_base_color"))
                .unwrap_or(WHITE),
            foreground,
            dim: with_alpha(foreground, 0.55),
            accent,
            selection: with_alpha(accent, 0.35),
            rule: with_alpha(foreground, 0.28),
            misspelt: named(widget, "error_color").unwrap_or(gdk::RGBA::new(0.88, 0.11, 0.14, 1.0)),
        }
    }
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

fn with_alpha(color: gdk::RGBA, alpha: f32) -> gdk::RGBA {
    gdk::RGBA::new(color.red(), color.green(), color.blue(), alpha)
}
