// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What colour everything is, and which of the two sets Windows is asking for.
//!
//! Split the way the whole crate is: the **palette and the choice between the two are portable**
//! and tested on any host, and only the registry read and the dark title bar need Windows.
//!
//! ## Why there is a palette here at all
//!
//! `doc/sheet-shell.md` is emphatic that the GTK window takes every colour from the theme and
//! never writes a literal, and this file looks at first like a violation of that rule. It is
//! not, because the rule's *reason* does not carry across: GTK has a theme to ask, with named
//! colours that follow the user's choice, and Win32 does not have one that works.
//!
//! `GetSysColor` is the obvious candidate and it is a trap. Its values have not tracked the
//! user's light/dark choice since Windows 8 — `COLOR_WINDOW` is white on a Windows 11 machine
//! set to dark, because those constants are pinned to the old high-contrast-era semantics that
//! desktop applications depend on. An application that asked it would draw a white grid inside
//! a dark title bar. What Windows actually exposes is a **boolean** — `AppsUseLightTheme` under
//! `HKCU\…\Themes\Personalize` — and every application that follows the system theme, Microsoft's
//! own included, reads that and supplies its own two palettes. So that is what this does.
//!
//! Two exceptions, and they are the same two the GTK window makes: a colour the **document**
//! chose is the document's and is drawn verbatim, and `grind_core::style::PALETTE` is the list a
//! shell *offers*. Neither is theming.

/// A colour, as the three bytes a document and a human both write it in.
///
/// Not a `COLORREF`: that is a Windows type with the components in the other order, and keeping
/// this the right way round means the tables below can be read against a hex colour without
/// mentally swapping the ends. [`Rgb::colorref`] does the swap, once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    /// The same colour as GDI wants it: `0x00BBGGRR`.
    pub fn colorref(self) -> u32 {
        let Rgb(r, g, b) = self;
        u32::from(r) | (u32::from(g) << 8) | (u32::from(b) << 16)
    }

    /// This colour `t` of the way towards another, per channel.
    ///
    /// The selection wash is what needs it, and needs it *here* rather than at the drawing
    /// call: GDI has no alpha in `FillRect`, so "a translucent blue over whatever the cell
    /// already is" has to be computed as an opaque colour before anything is filled. Doing the
    /// arithmetic in portable code is what makes the wash over a document's own red the same
    /// question as the wash over the theme's ground, and testable without a device context.
    pub fn blend(self, other: Rgb, t: f64) -> Rgb {
        let t = t.clamp(0.0, 1.0);
        let mix = |a: u8, b: u8| (f64::from(a) + (f64::from(b) - f64::from(a)) * t).round() as u8;
        Rgb(
            mix(self.0, other.0),
            mix(self.1, other.1),
            mix(self.2, other.2),
        )
    }

    /// A colour parsed from `#rrggbb`, which is the only form ODF stores (§5.1).
    pub fn parse(hex: &str) -> Option<Self> {
        let digits = hex.strip_prefix('#')?;
        if digits.len() != 6 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let byte = |at: usize| u8::from_str_radix(&digits[at..at + 2], 16).ok();
        Some(Rgb(byte(0)?, byte(2)?, byte(4)?))
    }
}

/// Which of the two palettes the system is asking for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Light,
    Dark,
}

/// Every colour this shell draws with, other than the ones a document chose for itself.
///
/// A struct rather than a set of constants so that the drawing code takes the palette as an
/// argument and cannot reach past it — which is what makes `--render-to` able to produce a dark
/// screenshot on a light machine, and what would make a high-contrast palette a third table
/// rather than a third code path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    pub mode: Mode,
    /// The grid's own ground, behind unstyled cells.
    pub background: Rgb,
    /// The hairlines between cells.
    pub grid_line: Rgb,
    /// Text in a cell that has no colour of its own.
    pub text: Rgb,
    /// The row and column header buttons, their lettering, and the line under them.
    pub header: Rgb,
    pub header_text: Rgb,
    pub header_line: Rgb,
    /// The status bar at the foot of the window.
    pub status: Rgb,
    pub status_text: Rgb,
    /// The colour a selected cell's ground is washed *towards* — never painted neat, always
    /// [`Rgb::blend`]ed over whatever the cell already is, so a document's own fill survives
    /// being selected. See `sheet::draw::WASH` for how far.
    pub selection: Rgb,
    /// The outline around the selected rectangle, and the accent everything else in this shell
    /// borrows. Painted neat.
    pub selection_edge: Rgb,
    /// A header button belonging to a selected row or column.
    pub header_active: Rgb,
    /// The name box and the formula bar: inset fields on the strip, so they read as something to
    /// type in rather than as labels. Separate from `background` because the strip is not the
    /// grid.
    pub field: Rgb,
    pub field_line: Rgb,
    /// The notice bar under the strip — a document that needs recalculating, a recalculation
    /// that was skipped, a save that failed. Not an accent and not an error red: a banner that
    /// shouts is one people learn to ignore.
    pub banner: Rgb,
    pub banner_text: Rgb,
}

/// The light palette — Windows 11's own surface greys rather than pure white, so that the grid
/// lines have something to be lighter than.
const LIGHT: Theme = Theme {
    mode: Mode::Light,
    background: Rgb(0xff, 0xff, 0xff),
    grid_line: Rgb(0xd6, 0xd6, 0xd6),
    text: Rgb(0x1a, 0x1a, 0x1a),
    header: Rgb(0xf3, 0xf3, 0xf3),
    header_text: Rgb(0x44, 0x44, 0x44),
    header_line: Rgb(0xc4, 0xc4, 0xc4),
    status: Rgb(0xf3, 0xf3, 0xf3),
    status_text: Rgb(0x44, 0x44, 0x44),
    selection: Rgb(0x00, 0x67, 0xc0),
    selection_edge: Rgb(0x00, 0x5a, 0x9e),
    header_active: Rgb(0xd8, 0xe6, 0xf4),
    field: Rgb(0xff, 0xff, 0xff),
    field_line: Rgb(0xb4, 0xb4, 0xb4),
    banner: Rgb(0xff, 0xf4, 0xce),
    banner_text: Rgb(0x4d, 0x3a, 0x00),
};

/// The dark palette. Not an inversion of the light one: the grid lines are *lighter* than the
/// ground here and darker than it there, because a line has to be visible against what it sits
/// on and inverting a light theme's greys puts them the wrong side of it.
const DARK: Theme = Theme {
    mode: Mode::Dark,
    background: Rgb(0x1e, 0x1e, 0x1e),
    grid_line: Rgb(0x3a, 0x3a, 0x3a),
    text: Rgb(0xe8, 0xe8, 0xe8),
    header: Rgb(0x2b, 0x2b, 0x2b),
    header_text: Rgb(0xc0, 0xc0, 0xc0),
    header_line: Rgb(0x45, 0x45, 0x45),
    status: Rgb(0x2b, 0x2b, 0x2b),
    status_text: Rgb(0xc0, 0xc0, 0xc0),
    selection: Rgb(0x4c, 0xa0, 0xff),
    selection_edge: Rgb(0x60, 0xac, 0xff),
    header_active: Rgb(0x1f, 0x3a, 0x52),
    field: Rgb(0x1a, 0x1a, 0x1a),
    field_line: Rgb(0x55, 0x55, 0x55),
    banner: Rgb(0x3d, 0x34, 0x12),
    banner_text: Rgb(0xf5, 0xdd, 0x8e),
};

impl Theme {
    pub fn of(mode: Mode) -> Self {
        match mode {
            Mode::Light => LIGHT,
            Mode::Dark => DARK,
        }
    }

    /// What the registry value means.
    ///
    /// `AppsUseLightTheme` is a `REG_DWORD`: 1 is light, 0 is dark, and **a missing value is
    /// light**. The last part is the one that matters and it is not a guess — a fresh Wine
    /// prefix has no `Personalize` key at all, which is exactly the case
    /// `doc/windows-shell.md` names as leaving dark mode untested there.
    pub fn from_registry_value(value: Option<u32>) -> Self {
        match value {
            Some(0) => DARK,
            _ => LIGHT,
        }
    }
}

/// Which theme the user has chosen, read from the registry.
///
/// Failure in any form — no key, no value, the wrong type — is light mode rather than an error.
/// A shell that refused to start because it could not learn a colour would be worse than one
/// that started in the wrong one.
#[cfg(windows)]
pub fn current() -> Theme {
    use windows::Win32::System::Registry::{
        HKEY_CURRENT_USER, KEY_READ, REG_VALUE_TYPE, RegCloseKey, RegOpenKeyExW, RegQueryValueExW,
    };
    use windows::core::{HSTRING, PCWSTR};

    let path = HSTRING::from(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let name = HSTRING::from("AppsUseLightTheme");
    let mut key = Default::default();
    // SAFETY: every pointer is to a live local that outlives the call, and the key is closed
    // on both paths below.
    let value = unsafe {
        if RegOpenKeyExW(HKEY_CURRENT_USER, &path, None, KEY_READ, &mut key).is_err() {
            return Theme::of(Mode::Light);
        }
        let mut data = 0u32;
        let mut size = u32::try_from(std::mem::size_of::<u32>()).expect("four");
        let mut kind = REG_VALUE_TYPE::default();
        let read = RegQueryValueExW(
            key,
            PCWSTR(name.as_ptr()),
            None,
            Some(&mut kind),
            Some(std::ptr::from_mut(&mut data).cast()),
            Some(&mut size),
        );
        let _ = RegCloseKey(key);
        read.is_ok().then_some(data)
    };
    Theme::from_registry_value(value)
}

/// Ask the compositor to draw this window's title bar dark.
///
/// The one piece of "native chrome" this shell asks for, and the only one it can get without a
/// manifest. Deliberately best-effort: `DwmSetWindowAttribute` fails on Windows 10 builds older
/// than the attribute and is largely inert under Wine, and in both cases the right response is
/// a light title bar over a dark grid rather than a refusal to open.
#[cfg(windows)]
pub fn apply_title_bar(hwnd: windows::Win32::Foundation::HWND, theme: Theme) {
    use windows::Win32::Graphics::Dwm::{DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute};

    let dark = windows::core::BOOL::from(theme.mode == Mode::Dark);
    // SAFETY: `hwnd` is this window's, and the buffer is a live local of the size given.
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            std::ptr::from_ref(&dark).cast(),
            u32::try_from(std::mem::size_of::<windows::core::BOOL>()).expect("four"),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_colorref_swaps_the_ends_and_nothing_else() {
        assert_eq!(Rgb(0x12, 0x34, 0x56).colorref(), 0x0056_3412);
        assert_eq!(Rgb(0xff, 0x00, 0x00).colorref(), 0x0000_00ff);
    }

    #[test]
    fn a_document_colour_is_read_the_way_a_document_spells_it() {
        let hex = "#0074d9"; // PALETTE's blue
        assert_eq!(Rgb::parse(hex), Some(Rgb(0x00, 0x74, 0xd9)));
    }

    #[test]
    fn a_blend_is_a_straight_line_between_two_colours() {
        let black = Rgb(0, 0, 0);
        let white = Rgb(0xff, 0xff, 0xff);
        assert_eq!(black.blend(white, 0.0), black);
        assert_eq!(black.blend(white, 1.0), white);
        assert_eq!(black.blend(white, 0.5), Rgb(128, 128, 128));
        // Out-of-range mixes are clamped rather than wrapping a `u8` round.
        assert_eq!(black.blend(white, 2.0), white);
        assert_eq!(black.blend(white, -1.0), black);
    }

    /// The selection wash has to be visible over a cell the *document* coloured, not only over
    /// the theme's ground — that is the case a solid highlight would erase.
    #[test]
    fn the_wash_moves_a_documents_own_colour_without_erasing_it() {
        for theme in [Theme::of(Mode::Light), Theme::of(Mode::Dark)] {
            let red = Rgb(0xff, 0x41, 0x36); // PALETTE's red, as a document would store it
            let washed = red.blend(theme.selection, crate::sheet::draw::WASH);
            assert_ne!(washed, red, "{:?}: the wash is invisible", theme.mode);
            assert_ne!(
                washed, theme.selection,
                "{:?}: the wash erased the document's colour",
                theme.mode
            );
        }
    }

    #[test]
    fn nonsense_is_not_a_colour() {
        assert_eq!(Rgb::parse("blue"), None);
        assert_eq!(Rgb::parse("#abc"), None);
        assert_eq!(Rgb::parse("#gggggg"), None);
    }

    /// Every colour the shell offers is one it can also draw. `PALETTE` is the core's list, so
    /// this is a check that the two spellings of a colour agree rather than a check on a table.
    #[test]
    fn the_cores_palette_is_all_parseable() {
        for (name, hex) in grind_core::style::PALETTE {
            assert!(Rgb::parse(hex).is_some(), "{name} = {hex}");
        }
    }

    #[test]
    fn the_registry_decides_and_a_missing_value_is_light() {
        assert_eq!(Theme::from_registry_value(Some(1)).mode, Mode::Light);
        assert_eq!(Theme::from_registry_value(Some(0)).mode, Mode::Dark);
        assert_eq!(Theme::from_registry_value(None).mode, Mode::Light);
    }

    /// A line has to be visible against the ground it sits on, in both palettes — which is why
    /// the dark theme is not the light one inverted.
    #[test]
    fn grid_lines_contrast_with_their_ground_in_both_palettes() {
        let luma = |Rgb(r, g, b): Rgb| {
            0.2126 * f64::from(r) + 0.7152 * f64::from(g) + 0.0722 * f64::from(b)
        };
        for theme in [Theme::of(Mode::Light), Theme::of(Mode::Dark)] {
            let gap = (luma(theme.grid_line) - luma(theme.background)).abs();
            assert!(gap > 16.0, "{:?}: grid line gap is {gap}", theme.mode);
            let text = (luma(theme.text) - luma(theme.background)).abs();
            assert!(text > 128.0, "{:?}: text gap is {text}", theme.mode);
            // A notice nobody can read is a notice nobody acts on, and the banner is the one
            // place this shell paints a ground of its own that is neither the grid's nor the
            // chrome's.
            let banner = (luma(theme.banner_text) - luma(theme.banner)).abs();
            assert!(banner > 96.0, "{:?}: banner gap is {banner}", theme.mode);
        }
    }
}
