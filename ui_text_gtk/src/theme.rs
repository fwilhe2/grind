// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Every colour the document view draws, resolved from the running theme in one place.
//!
//! `ui_sheet_gtk/src/theme.rs`'s rule, unchanged and worth repeating: **nothing here is a
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
    /// The band behind selected text.
    pub selection: gdk::RGBA,
    /// A table's rules. Quieter than the text and stronger than the furniture — a grid a reader
    /// can see the shape of without it competing with what is written in it. Derived from the
    /// theme's own ink like everything else here, because a literal is what makes a page stay
    /// white in a dark theme.
    pub rule: gdk::RGBA,
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
        }
    }
}

/// Whether an announcement from `widget` reaches anybody.
///
/// Two ways it does not, and both used to cost something. GTK's fallback accessibility context —
/// the one it uses when there is no AT-SPI bus, as in a container, a VM or a minimal session —
/// has no `announce` hook, and `gtk_accessible_announce` calls it anyway: a jump to address zero
/// on every move (measured on GTK 4.18 under Xvfb, with and without a session bus). And with
/// `GTK_A11Y=none` there is **no context at all**: `gtk_accessible_get_at_context` returns NULL,
/// which the generated binding asserts against, so the first click in the window aborted it. The
/// raw call is made here because it is the one spelling that can say "none".
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
