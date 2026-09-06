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
    /// The hairlines **past the last row and column the document uses** — the same lines, drawn
    /// quieter, so the sheet somebody wrote reads as the subject and the empty rest of an
    /// infinite grid as the paper it is on.
    ///
    /// Not a border round the used range and not a different ground: both of those would be this
    /// shell inventing a page boundary the document does not have. One tone of grey is the whole
    /// of it, and `App::used_extent` — the same answer the status bar already reports — is where
    /// the line falls.
    pub grid_line_soft: Rgb,
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
    ///
    /// **The suite's own blue** (`grind_core::style::PALETTE`'s `blue`, deepened for the light
    /// palette), not the user's Windows accent colour, and that is a decision W9 made rather than
    /// an omission. The accent colour *is* readable — it is a `DWORD` under `HKCU\…\DWM` — but it
    /// is chosen for a title bar and a taskbar, and half the values in that picker are colours a
    /// one-pixel selection edge disappears into or a grid cannot be read through. One accent this
    /// shell owns is one it can guarantee reads against both grounds, and it ties the window to
    /// the same mark the icon and the two GTK apps already use. Following the system accent is
    /// therefore a named gap, not a missing feature.
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
    /// The stripe down the notice bar's leading edge. The one saturated thing about that band,
    /// which is what lets the ground behind it stay quiet enough to read a sentence on.
    pub banner_edge: Rgb,
    /// The assist band under the strip while a formula is being typed — the completion offers and
    /// the signature hint (`sheet/assist.rs`). Its own ground, tinted towards the accent rather
    /// than towards the notice bar's amber, because it is *help while typing* and not a state the
    /// document is in — the two can be up at once and must not read as the same thing.
    pub hint: Rgb,
    pub hint_text: Rgb,
}

/// The light palette.
///
/// Refreshed in W9, and the direction of every change is the same one: **quieter chrome, one
/// louder accent**. The hairlines lost a step of contrast (a grid is read *through*, and lines
/// dark enough to count are lines that fight the numbers), the header band and the status bar
/// moved a shade closer to the paper, and what was spent there was put back into the accent —
/// which is now the suite's own blue and is the only saturated colour on screen that the document
/// did not choose itself.
const LIGHT: Theme = Theme {
    mode: Mode::Light,
    background: Rgb(0xff, 0xff, 0xff),
    grid_line: Rgb(0xdf, 0xdf, 0xdf),
    grid_line_soft: Rgb(0xef, 0xef, 0xef),
    text: Rgb(0x1a, 0x1a, 0x1a),
    header: Rgb(0xf7, 0xf7, 0xf7),
    header_text: Rgb(0x5a, 0x5a, 0x5a),
    header_line: Rgb(0xd2, 0xd2, 0xd2),
    status: Rgb(0xf7, 0xf7, 0xf7),
    status_text: Rgb(0x5a, 0x5a, 0x5a),
    selection: Rgb(0x00, 0x74, 0xd9),
    selection_edge: Rgb(0x00, 0x5c, 0xae),
    header_active: Rgb(0xdc, 0xe9, 0xf8),
    field: Rgb(0xff, 0xff, 0xff),
    field_line: Rgb(0xc8, 0xc8, 0xc8),
    banner: Rgb(0xff, 0xf6, 0xdb),
    banner_text: Rgb(0x4d, 0x3a, 0x00),
    banner_edge: Rgb(0xe0, 0xa8, 0x00),
    hint: Rgb(0xf1, 0xf7, 0xfd),
    hint_text: Rgb(0x1a, 0x1a, 0x1a),
};

/// The dark palette. Not an inversion of the light one: the grid lines are *lighter* than the
/// ground here and darker than it there, because a line has to be visible against what it sits
/// on and inverting a light theme's greys puts them the wrong side of it.
///
/// The same holds for the two W9 additions and is why neither is computed from its light twin:
/// `grid_line_soft` is *darker* than `grid_line` here and lighter than it there — in both cases
/// nearer the ground, which is what "quieter" means — and the accent is the palette's own blue
/// rather than the light theme's, since the deepened one disappears into a dark ground.
const DARK: Theme = Theme {
    mode: Mode::Dark,
    background: Rgb(0x1e, 0x1e, 0x1e),
    grid_line: Rgb(0x3a, 0x3a, 0x3a),
    grid_line_soft: Rgb(0x2b, 0x2b, 0x2b),
    text: Rgb(0xe8, 0xe8, 0xe8),
    header: Rgb(0x27, 0x27, 0x27),
    header_text: Rgb(0xb6, 0xb6, 0xb6),
    header_line: Rgb(0x40, 0x40, 0x40),
    status: Rgb(0x27, 0x27, 0x27),
    status_text: Rgb(0xb6, 0xb6, 0xb6),
    selection: Rgb(0x4c, 0xa0, 0xff),
    selection_edge: Rgb(0x60, 0xac, 0xff),
    header_active: Rgb(0x22, 0x3d, 0x58),
    field: Rgb(0x17, 0x17, 0x17),
    field_line: Rgb(0x4a, 0x4a, 0x4a),
    banner: Rgb(0x3d, 0x34, 0x12),
    banner_text: Rgb(0xf5, 0xdd, 0x8e),
    banner_edge: Rgb(0xc6, 0x9a, 0x2e),
    hint: Rgb(0x16, 0x23, 0x2f),
    hint_text: Rgb(0xe8, 0xe8, 0xe8),
};

/// What colour `doc/view-modes.md`'s role overlay draws each [`grind_sheet::view::CellRole`] in
/// — `ui_sheet_gtk::theme::role_color`'s mapping, unchanged: the financial-modelling convention
/// it borrows (inputs blue, formulas the ordinary text colour, another sheet's a third hue) is a
/// property of the *mode*, not of a toolkit, so the two shells agree on what a colour means.
/// `None` for [`grind_sheet::view::CellRole::Empty`] — the one role drawn as nothing at all.
///
/// A name from `grind_core::style::PALETTE` rather than a literal, which is the one exception
/// `doc/sheet-shell.md` names for a colour a shell *offers* — this mode offers a legend, not a
/// document's own choice — for every role but the two that are not a hue at all: a formula's own
/// marker is the theme's ordinary text colour, and a label's is that colour blended towards the
/// ground, which is this shell's `Rgb::blend` doing what GTK's `with_alpha` does with an actual
/// alpha channel GDI does not have.
pub fn role_color(role: grind_sheet::view::CellRole, theme: Theme) -> Option<Rgb> {
    use grind_sheet::view::CellRole as R;
    let named = |name: &str| grind_core::style::palette(name).and_then(Rgb::parse);
    match role {
        R::Empty => None,
        R::InputNamed => named("blue"),
        R::InputUnnamed => named("navy"),
        R::ConstantUnnamed => named("orange"),
        R::ComputedLocal => Some(theme.text),
        R::ComputedCrossSheet => named("olive"),
        R::Label => Some(theme.text.blend(theme.background, 0.4)),
        R::Error => named("red"),
        R::Stale => named("maroon"),
    }
}

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

    /// Every role but the one drawn as nothing gets a colour, in both palettes.
    #[test]
    fn every_role_but_empty_has_a_marker_colour() {
        use grind_sheet::view::CellRole;
        for theme in [Theme::of(Mode::Light), Theme::of(Mode::Dark)] {
            for role in CellRole::ALL {
                assert_eq!(
                    role_color(role, theme).is_none(),
                    role == CellRole::Empty,
                    "{role:?} in {:?}",
                    theme.mode
                );
            }
        }
    }

    /// A label's marker is muted rather than a hue, and moving further towards the ground than
    /// a formula's own marker does — the same "quieter, not louder" rule the GTK window's
    /// `with_alpha` follows.
    #[test]
    fn a_label_is_quieter_than_a_computed_cell() {
        use grind_sheet::view::CellRole;
        for theme in [Theme::of(Mode::Light), Theme::of(Mode::Dark)] {
            let label = role_color(CellRole::Label, theme).unwrap();
            let computed = role_color(CellRole::ComputedLocal, theme).unwrap();
            assert_eq!(computed, theme.text);
            assert_ne!(label, computed, "{:?}", theme.mode);
        }
    }

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

    /// The unused part of the grid is drawn *quieter*, not differently: its hairline is nearer
    /// the ground than the used one in both palettes — which, since the dark theme's lines are
    /// lighter than its ground and the light theme's darker than its own, means the two soft
    /// tones sit on opposite sides. Asserted as a distance rather than as "lighter", which is
    /// what makes this one rule rather than two.
    #[test]
    fn the_grid_fades_where_the_document_stops() {
        for theme in [Theme::of(Mode::Light), Theme::of(Mode::Dark)] {
            let ground = luma(theme.background);
            let used = (luma(theme.grid_line) - ground).abs();
            let unused = (luma(theme.grid_line_soft) - ground).abs();
            assert!(
                unused < used,
                "{:?}: the empty grid is not quieter ({unused} vs {used})",
                theme.mode
            );
            assert!(
                unused > 4.0,
                "{:?}: the empty grid is invisible ({unused})",
                theme.mode
            );
        }
    }

    /// The accent is the suite's own blue rather than one borrowed from the system, and the
    /// wash and the edge are two tones *of the same hue* rather than two colours — the whole
    /// point of one shell having one accent.
    #[test]
    fn the_accent_is_the_suites_own_blue() {
        let blue = grind_core::style::palette("blue")
            .and_then(Rgb::parse)
            .unwrap();
        assert_eq!(Theme::of(Mode::Light).selection, blue);
        for theme in [Theme::of(Mode::Light), Theme::of(Mode::Dark)] {
            let Rgb(r, _, b) = theme.selection_edge;
            assert!(b > r, "{:?}: the edge is not a blue", theme.mode);
            let Rgb(r, _, b) = theme.selection;
            assert!(b > r, "{:?}: the wash is not a blue", theme.mode);
        }
    }

    /// The assist band is help while typing and the notice bar is a state the document is in.
    /// Both can be up at once, so they must not read as one band: different grounds, and each
    /// legible in its own.
    #[test]
    fn the_hint_band_is_readable_and_is_not_the_notice_bar() {
        for theme in [Theme::of(Mode::Light), Theme::of(Mode::Dark)] {
            assert_ne!(theme.hint, theme.banner, "{:?}", theme.mode);
            let gap = (luma(theme.hint_text) - luma(theme.hint)).abs();
            assert!(gap > 96.0, "{:?}: hint gap is {gap}", theme.mode);
            // And it is quiet: a band drawn under the strip on every keystroke must not be
            // further from the chrome around it than the notice bar, which is the loud one.
            let hint = (luma(theme.hint) - luma(theme.header)).abs();
            let banner = (luma(theme.banner) - luma(theme.header)).abs();
            assert!(hint < banner, "{:?}: {hint} vs {banner}", theme.mode);
        }
    }

    /// The relative luminance of a colour, which is what "contrast" means below.
    fn luma(Rgb(r, g, b): Rgb) -> f64 {
        0.2126 * f64::from(r) + 0.7152 * f64::from(g) + 0.0722 * f64::from(b)
    }

    /// A line has to be visible against the ground it sits on, in both palettes — which is why
    /// the dark theme is not the light one inverted.
    #[test]
    fn grid_lines_contrast_with_their_ground_in_both_palettes() {
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
