// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What colour everything is, how big it is, and which of the two sets Windows asks for. The
//! **tokens and the choice between them are portable** and tested on any host; only the registry
//! read, the dark title bar and the rounded corner need Windows.
//!
//! ## Why there is a palette here
//!
//! The GTK window takes every colour from the theme; Win32 has no theme that works to ask.
//! `GetSysColor` is the trap: its values have not followed light/dark since Windows 8 —
//! `COLOR_WINDOW` is white on a Windows 11 machine set to dark — so a grid drawn from it is white
//! inside a dark title bar. What Windows exposes is a **boolean**, `AppsUseLightTheme` under
//! `HKCU\…\Themes\Personalize`, and every application following the system theme, Microsoft's
//! included, reads it and supplies its own two palettes. Two exceptions, as in GTK: a colour the
//! **document** chose is drawn as it is, and `grind_core::style::PALETTE` is what a shell offers.
//!
//! ## Where the numbers come from
//!
//! **Fluent 2's neutral ramp**, resolved to opaque values: Fluent states most surface tokens as
//! white or black at some alpha over the layer beneath (`ControlFillColorDefault` is `#FFFFFF` at
//! 70% in light, 5.8% in dark), and `FillRect` has no alpha, so the composite is done once here —
//! [`Rgb::blend`]'s move applied to the palette.
//!
//! So there are **three grounds**: [`Theme::backdrop`], the window with its chrome;
//! [`Theme::background`], where the document is (the grid's paper, the text pane's page); and
//! [`Theme::card`], what you type in and click — Fluent's base, layer and control fill.

/// A colour as the three bytes a document and a human write it in — not a `COLORREF`, which has
/// the components the other way round, so the tables read against a hex colour. [`Rgb::colorref`]
/// swaps, once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    /// The same colour as GDI wants it: `0x00BBGGRR`.
    pub fn colorref(self) -> u32 {
        let Rgb(r, g, b) = self;
        u32::from(r) | (u32::from(g) << 8) | (u32::from(b) << 16)
    }

    /// This colour `t` of the way towards another, per channel. The selection wash needs it here,
    /// not at the drawing call: `FillRect` has no alpha, so "a translucent blue over whatever the
    /// cell is" is an opaque colour computed first — in portable code, so a wash over a document's
    /// red is testable without a device context.
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
        grind_core::color::parse(hex).map(Rgb::from)
    }

    /// WCAG relative luminance, 0.0 to 1.0. Public because [`Rgb::contrast`] decides at run time
    /// whether a **user's own accent** is legible on these grounds ([`accent_for`]).
    pub fn luminance(self) -> f64 {
        grind_core::color::luminance(self.into())
    }

    /// The WCAG contrast ratio between two colours, from 1.0 (identical) to 21.0 (black on
    /// white). 3.0 is the floor for a line or a shape you have to be able to see; 4.5 is the one
    /// for small text.
    pub fn contrast(self, other: Rgb) -> f64 {
        grind_core::color::contrast(self.into(), other.into())
    }

    /// Hue, saturation and lightness — the space a tint ramp is built in. Windows derives
    /// `AccentLight1..3` and `AccentDark1..3` from the user's accent by moving its *lightness* and
    /// keeping the hue; blending towards white desaturates, and an accent that desaturates stops
    /// looking like the one chosen.
    pub fn hsl(self) -> (f64, f64, f64) {
        grind_core::color::hsl(self.into())
    }

    /// The inverse of [`Rgb::hsl`].
    pub fn from_hsl(h: f64, s: f64, l: f64) -> Self {
        grind_core::color::from_hsl(h, s, l).into()
    }
}

/// The core's three bytes ([`grind_core::color`]), which is where the arithmetic on a colour
/// lives — this type is the Windows edge of it, carrying [`Rgb::colorref`].
impl From<grind_core::color::Rgb> for Rgb {
    fn from((r, g, b): grind_core::color::Rgb) -> Self {
        Rgb(r, g, b)
    }
}

impl From<Rgb> for grind_core::color::Rgb {
    fn from(Rgb(r, g, b): Rgb) -> Self {
        (r, g, b)
    }
}

/// Which of the two palettes the system is asking for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Light,
    Dark,
}

/// The accent this shell falls back to when the system has not told it one: the suite's own blue,
/// which the icon and both GTK apps already use.
pub const SUITE_BLUE: Rgb = Rgb(0x00, 0x74, 0xd9);

/// The contrast a shape must reach against its ground to count as visible — WCAG's floor for a
/// non-text element, since the accent is a one- or two-pixel line in places (the selection's edge,
/// a header button's bar).
const VISIBLE: f64 = 3.0;

/// The accent, as the tint of `base` that can be seen on `mode`'s ground. The accent as the user
/// picked it can vanish as a one-pixel edge; WinUI never paints `SystemAccentColor` on a surface
/// but `SystemAccentColorDark1` on a light one and `SystemAccentColorLight2` on a dark one, tints
/// chosen to read. This does the same, then keeps moving the lightness until [`Rgb::contrast`]
/// against the ground clears [`VISIBLE`], since fixed steps promise nothing about a particular
/// colour.
///
/// A function of the accent alone, so "does this work with the user's colour" is tested with no
/// Windows and no user: the sweep in the tests checks every hue at every lightness.
pub fn accent_for(base: Rgb, mode: Mode) -> Rgb {
    let ground = match mode {
        Mode::Light => LIGHT_GROUND,
        Mode::Dark => DARK_GROUND,
    };
    // The ramp's own step first — Dark1 on a light ground, Light2 on a dark one — which is where
    // an accent that was already fine ends up staying.
    let (h, s, l) = base.hsl();
    let (start, step) = match mode {
        Mode::Light => (l - 0.09, -0.02),
        Mode::Dark => (l + 0.18, 0.02),
    };
    let mut best = Rgb::from_hsl(h, s, start);
    // Then as far as it takes, in the same direction, stopping the moment it is legible. Fifty
    // steps of 0.02 covers the whole lightness axis from either end, so this always terminates
    // with either a legible colour or the nearest to legible one there is.
    for n in 0..50 {
        if best.contrast(ground) >= VISIBLE {
            return best;
        }
        best = Rgb::from_hsl(h, s, start + step * f64::from(n));
    }
    // Nothing on this hue reads on this ground — a fully saturated yellow on white is the real
    // case. Fall back to the ground's own ink, which always does.
    match best.contrast(ground) >= VISIBLE {
        true => best,
        false => match mode {
            Mode::Light => Rgb::from_hsl(h, s, 0.25),
            Mode::Dark => Rgb::from_hsl(h, s, 0.78),
        },
    }
}

/// The two document grounds, named here because [`accent_for`] has to reason about them before a
/// [`Theme`] exists.
const LIGHT_GROUND: Rgb = Rgb(0xff, 0xff, 0xff);
const DARK_GROUND: Rgb = Rgb(0x2b, 0x2b, 0x2b);

/// Every colour this shell draws with, other than a document's own. A struct taken as an argument,
/// so the drawing code cannot reach past it — which lets `--render-to` draw a dark frame on a
/// light machine, and would make high contrast a third table rather than a third code path.
/// **Three grounds**: [`Theme::backdrop`] the window, [`Theme::background`] the document,
/// [`Theme::card`] a control; everything else is ink on one of them or a line between two.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    pub mode: Mode,

    /// The window's own ground — Fluent's *Solid Background Base*. The strip, the header bands,
    /// the format strip and the status bar all sit on it, and so does the surround the text
    /// pane's page floats in. Never the ground of anything the *document* owns.
    pub backdrop: Rgb,
    /// The document's ground: the grid's paper and the text pane's page — a layer *above* the
    /// backdrop in both palettes, lighter in light and dark alike, Fluent's layering rather than
    /// an inversion.
    pub background: Rgb,
    /// A control's ground at rest: the name box, the formula bar, a strip button. Fluent's
    /// *ControlFillColorDefault*, composited onto the backdrop.
    pub card: Rgb,
    /// The same control hovered, and pressed — Fluent's *Secondary* and *Tertiary* fills. In
    /// light, pressed is quieter than hover on purpose: pressing recedes, hovering lifts.
    pub card_hover: Rgb,
    pub card_pressed: Rgb,
    /// The ground a control with **no** fill takes under the pointer — Fluent's *Subtle* fills, so
    /// a row of toggles reads as one band until one is pointed at.
    pub subtle_hover: Rgb,
    pub subtle_pressed: Rgb,

    /// A control's own outline — Fluent's *ControlStrokeColorDefault*.
    pub stroke: Rgb,
    /// The hairline between two bands of chrome, and the line closing the header band.
    /// Fluent's *DividerStrokeColorDefault*: quieter than [`Theme::stroke`], because a divider
    /// separates and an outline contains.
    pub divider: Rgb,
    /// The hairlines between cells.
    pub grid_line: Rgb,
    /// The hairlines **past the document's last used row and column** — the same lines, quieter,
    /// so what somebody wrote reads as the subject and the rest as paper. Not a border or a
    /// different ground, which would invent a page boundary; `App::used_extent` is where the line
    /// falls.
    pub grid_line_soft: Rgb,

    /// Text in a cell, or in a document, that has no colour of its own — Fluent's *TextFillColor
    /// Primary*.
    pub text: Rgb,
    /// A label rather than a value: a header button's letter, the status bar, a picker's caption.
    /// Fluent's *Secondary*.
    pub text_secondary: Rgb,
    /// Something present but not being said: a placeholder, a badge, the corner triangle.
    /// Fluent's *Tertiary*.
    pub text_tertiary: Rgb,
    /// Ink on a ground painted [`Theme::accent`] neat. Chosen for contrast against the accent
    /// rather than fixed, because a user's accent can be any lightness at all.
    pub on_accent: Rgb,

    /// The accent, painted neat: the selection's outline, a selected header button's bar, a
    /// checked toggle, the argument being typed. **The user's own Windows accent**, moved to the
    /// tint that reads on this palette ([`accent_for`]), or the suite's blue when the system says
    /// nothing.
    pub accent: Rgb,
    /// The accent as a *ground* — a wash of it over the backdrop, quiet enough that
    /// [`Theme::accent`] itself is legible on top. A selected header button and a checked strip
    /// toggle are drawn in it.
    pub accent_soft: Rgb,
    /// The colour a selected cell's ground is washed *towards* — never painted neat, always
    /// [`Rgb::blend`]ed over whatever the cell already is, so a document's own fill survives
    /// being selected. See `sheet::draw::WASH` for how far.
    pub selection: Rgb,

    /// The notice bar — a document that needs recalculating, a recalculation that was skipped, a
    /// save that failed. Fluent's *InfoBar*, severity caution: not an accent and not an error
    /// red, because a banner that shouts is one people learn to ignore.
    pub banner: Rgb,
    pub banner_text: Rgb,
    /// The stripe down the notice bar's leading edge. The one saturated thing about that band,
    /// which is what lets the ground behind it stay quiet enough to read a sentence on.
    pub banner_edge: Rgb,

    /// The assist band's ground while a formula is typed (`sheet/assist.rs`) — tinted towards the
    /// accent, not the notice bar's amber, since help while typing is not a state the document is
    /// in and the two can be up at once.
    pub hint: Rgb,
    pub hint_text: Rgb,

    /// Fluent's focus rectangle: a stroke of [`Theme::focus`] with [`Theme::focus_inner`] just
    /// inside it, so that the ring is visible on a light control and on a dark one without
    /// knowing which it is drawn on.
    pub focus: Rgb,
    pub focus_inner: Rgb,

    /// A misspelt word's squiggle in the text pane (`doc/spelling.md`) — Fluent's
    /// *SystemFillColorCritical*, the one place this shell paints an error red, since a word
    /// marked wrong is exactly the error that colour says.
    pub misspelt: Rgb,
}

/// The light palette — Fluent 2's light neutrals, composited (see the module comment).
const LIGHT: Theme = Theme {
    mode: Mode::Light,
    backdrop: Rgb(0xf3, 0xf3, 0xf3),
    background: LIGHT_GROUND,
    card: Rgb(0xfb, 0xfb, 0xfb),
    card_hover: Rgb(0xf6, 0xf6, 0xf6),
    card_pressed: Rgb(0xf2, 0xf2, 0xf2),
    subtle_hover: Rgb(0xea, 0xea, 0xea),
    subtle_pressed: Rgb(0xed, 0xed, 0xed),
    stroke: Rgb(0xe1, 0xe1, 0xe1),
    divider: Rgb(0xe6, 0xe6, 0xe6),
    grid_line: Rgb(0xe4, 0xe4, 0xe4),
    grid_line_soft: Rgb(0xf1, 0xf1, 0xf1),
    text: Rgb(0x1a, 0x1a, 0x1a),
    text_secondary: Rgb(0x5d, 0x5d, 0x5d),
    text_tertiary: Rgb(0x8d, 0x8d, 0x8d),
    on_accent: Rgb(0xff, 0xff, 0xff),
    accent: Rgb(0x00, 0x5c, 0xae),
    accent_soft: Rgb(0xdd, 0xe9, 0xf6),
    selection: SUITE_BLUE,
    banner: Rgb(0xff, 0xf4, 0xce),
    banner_text: Rgb(0x3b, 0x2c, 0x00),
    banner_edge: Rgb(0x9d, 0x5d, 0x00),
    hint: Rgb(0xf2, 0xf6, 0xfb),
    hint_text: Rgb(0x1a, 0x1a, 0x1a),
    focus: Rgb(0x1a, 0x1a, 0x1a),
    focus_inner: Rgb(0xff, 0xff, 0xff),
    misspelt: Rgb(0xc4, 0x2b, 0x1c),
};

/// The dark palette — not an inversion of the light one: grid lines are *lighter* than the ground
/// here and darker there, since a line must show against what it sits on. [`Theme::background`] is
/// still lighter than [`Theme::backdrop`]: a layer catches the light in either mode.
const DARK: Theme = Theme {
    mode: Mode::Dark,
    backdrop: Rgb(0x20, 0x20, 0x20),
    background: DARK_GROUND,
    card: Rgb(0x2f, 0x2f, 0x2f),
    card_hover: Rgb(0x35, 0x35, 0x35),
    card_pressed: Rgb(0x2a, 0x2a, 0x2a),
    subtle_hover: Rgb(0x2d, 0x2d, 0x2d),
    subtle_pressed: Rgb(0x28, 0x28, 0x28),
    stroke: Rgb(0x3a, 0x3a, 0x3a),
    divider: Rgb(0x33, 0x33, 0x33),
    grid_line: Rgb(0x3b, 0x3b, 0x3b),
    grid_line_soft: Rgb(0x31, 0x31, 0x31),
    text: Rgb(0xf2, 0xf2, 0xf2),
    text_secondary: Rgb(0xcf, 0xcf, 0xcf),
    text_tertiary: Rgb(0x96, 0x96, 0x96),
    on_accent: Rgb(0x00, 0x00, 0x00),
    accent: Rgb(0x60, 0xac, 0xff),
    accent_soft: Rgb(0x21, 0x33, 0x45),
    selection: Rgb(0x4c, 0xa0, 0xff),
    banner: Rgb(0x43, 0x35, 0x19),
    banner_text: Rgb(0xf7, 0xef, 0xdc),
    banner_edge: Rgb(0xfc, 0xe1, 0x00),
    hint: Rgb(0x23, 0x2b, 0x33),
    hint_text: Rgb(0xf2, 0xf2, 0xf2),
    focus: Rgb(0xff, 0xff, 0xff),
    focus_inner: Rgb(0x00, 0x00, 0x00),
    misspelt: Rgb(0xff, 0x99, 0xa4),
};

/// The colour the role overlay draws each [`grind_sheet::view::CellRole`] in — `CellRole::hue`,
/// the core's table, so every client agrees what a colour means (inputs blue, formulas the
/// ordinary text colour, another sheet's a third hue); `None` for
/// [`grind_sheet::view::CellRole::Empty`]. A `PALETTE` name rather than a literal, as a colour a
/// shell offers, except the two roles that are not hues: a formula's marker is the theme's text
/// colour, a label's its tertiary ink — Fluent's "present, but not what you are reading".
pub fn role_color(role: grind_sheet::view::CellRole, theme: Theme) -> Option<Rgb> {
    use grind_sheet::view::Hue;
    match role.hue() {
        Hue::None => None,
        Hue::Palette(name) => grind_core::style::palette(name).and_then(Rgb::parse),
        Hue::Ink => Some(theme.text),
        Hue::Quiet => Some(theme.text_tertiary),
    }
}

impl Theme {
    /// This palette with the suite's own accent — what `--render-to` draws and what a machine
    /// that has not said otherwise gets.
    pub fn of(mode: Mode) -> Self {
        Self::with_accent(mode, None)
    }

    /// This palette with the user's accent, if the system named one. Everything downstream of the
    /// accent is derived here, so `theme.accent_soft` always goes with `theme.accent`.
    pub fn with_accent(mode: Mode, system: Option<Rgb>) -> Self {
        let base = match mode {
            Mode::Light => LIGHT,
            Mode::Dark => DARK,
        };
        let Some(system) = system else {
            return base;
        };
        let accent = accent_for(system, mode);
        Self {
            accent,
            // A wash of the accent over the backdrop rather than a fixed pair of blues: it has to
            // stay quiet enough that a header button's letter, drawn in `accent` on top of it, is
            // still readable. A tenth is what does that for every hue.
            accent_soft: base.backdrop.blend(accent, 0.14),
            // The selection wash moves towards the accent as the user chose it, not the tint: a
            // wash is diluted to a fifth, so the legibility argument does not apply, and the tint
            // would make a light theme's selection too pale.
            selection: system,
            on_accent: match accent.contrast(Rgb(0, 0, 0)) > accent.contrast(Rgb(0xff, 0xff, 0xff))
            {
                true => Rgb(0, 0, 0),
                false => Rgb(0xff, 0xff, 0xff),
            },
            ..base
        }
    }

    /// What the two registry values mean. `AppsUseLightTheme` is a `REG_DWORD`: 1 light, 0 dark,
    /// and **a missing value is light** — a fresh Wine prefix has no `Personalize` key at all.
    /// `AccentColor` is `0xAABBGGRR` — the ends the other way round from a hex colour, the alpha
    /// meaningless — so the swap is here, beside the sentence saying so.
    pub fn from_registry_values(theme: Option<u32>, accent: Option<u32>) -> Self {
        let mode = match theme {
            Some(0) => Mode::Dark,
            _ => Mode::Light,
        };
        let accent = accent.map(|packed| {
            Rgb(
                (packed & 0xff) as u8,
                ((packed >> 8) & 0xff) as u8,
                ((packed >> 16) & 0xff) as u8,
            )
        });
        Self::with_accent(mode, accent)
    }
}

/// The type ramp, in pixels at 100% — Fluent 2's, by its own names. Windows 11's body text is **14
/// epx**, not the 12 Win32 windows have drawn since 1995, and a status bar and a formula bar are
/// not the same kind of text.
pub mod text {
    /// *Caption* — a label on something else: the header bands, the status bar, a picker's
    /// caption. The one size below body, and never used for anything a person reads a sentence of.
    pub const CAPTION: f64 = 12.0;
    /// *Body* — the shell's own voice: the name box, the formula bar, the notice bar, a menu.
    pub const BODY: f64 = 14.0;
    /// A cell's own text, which is **not** body: a document's column widths were chosen against a
    /// size near this, and raising it makes the numbers stop fitting the columns sized for them.
    pub const CELL: f64 = 13.0;
    /// Prose in the text pane. Larger than [`CELL`] on purpose: a spreadsheet is read by
    /// scanning and a document by reading, and every word processor there has ever been sets body
    /// text bigger than a cell.
    pub const PROSE: f64 = 15.0;
}

/// The spacing ramp, in pixels at 100% — Fluent's 4-pixel grid; every chrome measurement is one of
/// these or a small multiple. Two radii, Fluent's: 4 for anything clicked or typed in, 8 for a
/// surface holding other things.
pub mod space {
    /// `controlCornerRadius`, and the corner of every field, button and swatch.
    pub const RADIUS: f64 = 4.0;
    /// `overlayCornerRadius`, and the corner of the text pane's page and the two inset bands.
    pub const RADIUS_SURFACE: f64 = 8.0;
    /// The gap between two controls in a row.
    pub const GAP: f64 = 4.0;
    /// The gap between two *groups* of controls, and between a band's edge and what is in it.
    pub const GROUP: f64 = 12.0;
    /// A control's height — Fluent's standard.
    pub const CONTROL_H: f64 = 32.0;
}

/// The ink for text the document gave no colour, on a ground the document *did* choose — ODF's
/// *automatic* colour, resolved: the theme's ink where it reads, else black or white, whichever
/// reads better. Without the second clause a dark theme draws near-white text on a pale heading
/// row the document chose, and a light theme the reverse on a dark fill. The threshold is WCAG's
/// floor for small text.
pub fn automatic_ink(ground: Rgb, theme: Theme) -> Rgb {
    grind_core::color::automatic_ink(ground.into(), theme.text.into()).into()
}

/// The ink for a cell or a run: [`automatic_ink`] when the document chose no colour, else its own
/// colour, **lifted along its hue until it reads when it lands on the theme's dark ground**
/// (`grind_core::color::legible`) — a navy chosen for white paper is invisible on the dark page,
/// and a lighter navy is still the document's. On a fill the document chose, or in the light
/// palette, the colour is drawn as it is — every client's three rules over the same arithmetic.
/// `ground` is where the text lands (the document's fill, or the theme's ground or a wash over
/// it); `filled` says which.
pub fn document_ink(own: Option<Rgb>, ground: Rgb, filled: bool, theme: Theme) -> Rgb {
    match own {
        // Automatic against wherever the text lands, the theme's ground or a selection wash
        // over it included — not only against a fill the document chose.
        None => automatic_ink(ground, theme),
        Some(own) => grind_core::color::document_ink(
            Some(own.into()),
            filled.then_some(ground.into()),
            ground.into(),
            theme.text.into(),
            theme.mode == Mode::Dark,
        )
        .into(),
    }
}

/// Which of the three grounds a strip control stands on right now — the hover-and-press feedback
/// as a portable function. A control with a fill moves between [`Theme::card`],
/// [`Theme::card_hover`] and [`Theme::card_pressed`]; one without takes the *subtle* pair,
/// transparent at rest. `checked` outranks both — a toggle that is on looks on wherever the
/// pointer is — taking [`Theme::accent_soft`] and moving a step per state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Interaction {
    #[default]
    Rest,
    Hover,
    Pressed,
}

/// A control's ground: its fill, or `None` for "draw nothing, the band behind it is the fill".
pub fn control_fill(theme: Theme, state: Interaction, checked: bool, filled: bool) -> Option<Rgb> {
    if checked {
        let ground = theme.accent_soft;
        return Some(match state {
            Interaction::Rest => ground,
            Interaction::Hover => ground.blend(theme.accent, 0.16),
            Interaction::Pressed => ground.blend(theme.backdrop, 0.3),
        });
    }
    match (filled, state) {
        (true, Interaction::Rest) => Some(theme.card),
        (true, Interaction::Hover) => Some(theme.card_hover),
        (true, Interaction::Pressed) => Some(theme.card_pressed),
        (false, Interaction::Rest) => None,
        (false, Interaction::Hover) => Some(theme.subtle_hover),
        (false, Interaction::Pressed) => Some(theme.subtle_pressed),
    }
}

/// Which theme the user has chosen, from the registry — the mode and the accent. Any failure — no
/// key, no value, the wrong type — is the default rather than an error: no accent is the suite's
/// blue, which every `--render-to` frame and Wine run gets.
#[cfg(windows)]
pub fn current() -> Theme {
    Theme::from_registry_values(
        read_dword(
            r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
            "AppsUseLightTheme",
        ),
        read_dword(r"Software\Microsoft\Windows\DWM", "AccentColor"),
    )
}

/// One `REG_DWORD` under `HKEY_CURRENT_USER`, or `None` for every way that can fail.
#[cfg(windows)]
fn read_dword(path: &str, name: &str) -> Option<u32> {
    use windows::Win32::System::Registry::{
        HKEY_CURRENT_USER, KEY_READ, REG_VALUE_TYPE, RegCloseKey, RegOpenKeyExW, RegQueryValueExW,
    };
    use windows::core::{HSTRING, PCWSTR};

    let path = HSTRING::from(path);
    let name = HSTRING::from(name);
    let mut key = Default::default();
    // SAFETY: every pointer is to a live local that outlives the call, and the key is closed on
    // both paths below.
    unsafe {
        if RegOpenKeyExW(HKEY_CURRENT_USER, &path, None, KEY_READ, &mut key).is_err() {
            return None;
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
    }
}

/// Ask the compositor for this theme's window chrome: a dark title bar in dark mode, the **caption
/// painted in the application's backdrop**, and Windows 11's rounded corner — the one native
/// chrome this shell asks for, best-effort throughout: each attribute fails on older builds and is
/// mostly inert under Wine, and then the system's title bar stays. Painting the caption makes the
/// window one surface, as Explorer, Settings and Terminal do, which is why [`Theme::backdrop`] is
/// a token.
#[cfg(windows)]
pub fn apply_window_chrome(hwnd: windows::Win32::Foundation::HWND, theme: Theme) {
    dark_controls(hwnd, theme);
    use windows::Win32::Foundation::COLORREF;
    use windows::Win32::Graphics::Dwm::{
        DWMWA_BORDER_COLOR, DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE,
        DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute,
    };

    let dark = windows::core::BOOL::from(theme.mode == Mode::Dark);
    let corner = DWMWCP_ROUND;
    let caption = COLORREF(theme.backdrop.colorref());
    let ink = COLORREF(theme.text.colorref());
    let border = COLORREF(theme.divider.colorref());
    // SAFETY: `hwnd` is this window's, and every buffer is a live local of the size given.
    unsafe {
        let set = |attribute, value: *const std::ffi::c_void, size: usize| {
            let _ =
                DwmSetWindowAttribute(hwnd, attribute, value, u32::try_from(size).expect("small"));
        };
        set(
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            std::ptr::from_ref(&dark).cast(),
            std::mem::size_of::<windows::core::BOOL>(),
        );
        set(
            DWMWA_WINDOW_CORNER_PREFERENCE,
            std::ptr::from_ref(&corner).cast(),
            std::mem::size_of_val(&corner),
        );
        set(
            DWMWA_CAPTION_COLOR,
            std::ptr::from_ref(&caption).cast(),
            std::mem::size_of::<COLORREF>(),
        );
        set(
            DWMWA_TEXT_COLOR,
            std::ptr::from_ref(&ink).cast(),
            std::mem::size_of::<COLORREF>(),
        );
        set(
            DWMWA_BORDER_COLOR,
            std::ptr::from_ref(&border).cast(),
            std::mem::size_of::<COLORREF>(),
        );
    }
}

/// The parts of the window Windows draws — popup menus, scroll bars — in the same palette. **The
/// one undocumented API this shell uses**, since there is no documented way: a Win32 program's
/// popups are light whatever the theme, and Notepad, Explorer and Settings are dark only because
/// they call `uxtheme`'s `SetPreferredAppMode` (ordinal 135) and `FlushMenuThemes` (136) and give
/// scroll bars the `DarkMode_Explorer` class. Stable since Windows 10 1809, hence the build check;
/// every step is looked up rather than linked, so an older Windows or Wine keeps light menus and
/// nothing worse.
#[cfg(windows)]
fn dark_controls(hwnd: windows::Win32::Foundation::HWND, theme: Theme) {
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    use windows::Win32::UI::Controls::SetWindowTheme;
    use windows::core::{PCSTR, PCWSTR, w};

    if windows_build() < 17763 {
        return;
    }
    let dark = theme.mode == Mode::Dark;
    // SAFETY: the two entry points are looked up by ordinal and called with the one integer
    // argument each has always taken; `hwnd` is this window's.
    unsafe {
        let Ok(uxtheme) = LoadLibraryW(w!("uxtheme.dll")) else {
            return;
        };
        let ordinal = |n: usize| GetProcAddress(uxtheme, PCSTR(n as *const u8));
        // `PreferredAppMode`: 2 is ForceDark, 3 ForceLight. On 1809, where the same ordinal was
        // `AllowDarkModeForApp(BOOL)`, either is true, and the system's own mode — which is what
        // this palette was read from — decides.
        if let Some(set_mode) = ordinal(135) {
            let set_mode: unsafe extern "system" fn(i32) -> i32 = std::mem::transmute(set_mode);
            set_mode(if dark { 2 } else { 3 });
        }
        if let Some(allow) = ordinal(133) {
            let allow: unsafe extern "system" fn(windows::Win32::Foundation::HWND, i32) -> i32 =
                std::mem::transmute(allow);
            allow(hwnd, i32::from(dark));
        }
        if let Some(flush) = ordinal(136) {
            let flush: unsafe extern "system" fn() = std::mem::transmute(flush);
            flush();
        }
        let class = match dark {
            true => w!("DarkMode_Explorer"),
            false => w!("Explorer"),
        };
        let _ = SetWindowTheme(hwnd, class, PCWSTR::null());
    }
}

/// Windows' build number, as Windows itself reports it — `RtlGetVersion`, which unlike
/// `GetVersionExW` is not shimmed to say whatever the manifest asked for. Zero when it cannot be
/// asked, which every check here reads as "too old".
#[cfg(windows)]
pub fn windows_build() -> u32 {
    use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
    use windows::core::{s, w};

    #[repr(C)]
    struct Version {
        size: u32,
        major: u32,
        minor: u32,
        build: u32,
        platform: u32,
        service_pack: [u16; 128],
    }
    let mut version = Version {
        size: std::mem::size_of::<Version>() as u32,
        major: 0,
        minor: 0,
        build: 0,
        platform: 0,
        service_pack: [0; 128],
    };
    // SAFETY: `RtlGetVersion` writes an `OSVERSIONINFOW`, whose layout `Version` is, and reads
    // its size from the first field.
    unsafe {
        let Ok(ntdll) = GetModuleHandleW(w!("ntdll.dll")) else {
            return 0;
        };
        let Some(get) = GetProcAddress(ntdll, s!("RtlGetVersion")) else {
            return 0;
        };
        let get: unsafe extern "system" fn(*mut Version) -> i32 = std::mem::transmute(get);
        if get(&mut version) != 0 {
            return 0;
        }
    }
    version.build
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every palette this shell can produce: the two it ships, and the two it would build for a
    /// user whose accent is one of the awkward ones.
    fn palettes() -> Vec<Theme> {
        let mut out = vec![Theme::of(Mode::Light), Theme::of(Mode::Dark)];
        for accent in [
            Rgb(0xff, 0xff, 0x00), // a yellow: the case a light theme cannot paint neat
            Rgb(0x00, 0x00, 0x00), // black, which is a real choice in that picker
            Rgb(0xff, 0xff, 0xff),
            Rgb(0x74, 0x4d, 0xa9),
        ] {
            out.push(Theme::with_accent(Mode::Light, Some(accent)));
            out.push(Theme::with_accent(Mode::Dark, Some(accent)));
        }
        out
    }

    /// Every role but the one drawn as nothing gets a colour, in both palettes.
    #[test]
    fn every_role_but_empty_has_a_marker_colour() {
        use grind_sheet::view::CellRole;
        for theme in palettes() {
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
            assert!(
                label.contrast(theme.background) < computed.contrast(theme.background),
                "{:?}",
                theme.mode
            );
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
        for theme in palettes() {
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
        assert_eq!(Theme::from_registry_values(Some(1), None).mode, Mode::Light);
        assert_eq!(Theme::from_registry_values(Some(0), None).mode, Mode::Dark);
        assert_eq!(Theme::from_registry_values(None, None).mode, Mode::Light);
        // No accent is the suite's own blue, which is what `--render-to` draws.
        assert_eq!(
            Theme::from_registry_values(None, None).accent,
            Theme::of(Mode::Light).accent
        );
    }

    /// `AccentColor` is stored ends-first from a hex colour, the alpha meaningless; backwards
    /// would tint the window with the accent's complement, a bug that looks deliberate.
    #[test]
    fn the_accent_dword_is_read_ends_first() {
        // 0xAABBGGRR for a pure red.
        let theme = Theme::from_registry_values(Some(1), Some(0xff00_00ff));
        assert_eq!(theme.selection, Rgb(0xff, 0x00, 0x00));
        // And a pure blue, whose bytes are the other end of the word.
        let theme = Theme::from_registry_values(Some(1), Some(0xffff_0000));
        assert_eq!(theme.selection, Rgb(0x00, 0x00, 0xff));
    }

    /// The unused part of the grid is drawn *quieter*: its hairline nearer the ground than the
    /// used one in both palettes — so, with dark lines lighter than their ground and light ones
    /// darker, the two soft tones sit on opposite sides. Asserted as a distance, which makes it
    /// one rule.
    #[test]
    fn the_grid_fades_where_the_document_stops() {
        for theme in [Theme::of(Mode::Light), Theme::of(Mode::Dark)] {
            let used = theme.grid_line.contrast(theme.background);
            let unused = theme.grid_line_soft.contrast(theme.background);
            assert!(
                unused < used,
                "{:?}: the empty grid is not quieter ({unused} vs {used})",
                theme.mode
            );
            assert!(
                unused > 1.02,
                "{:?}: the empty grid is invisible ({unused})",
                theme.mode
            );
        }
    }

    /// **The three grounds are three**, in both palettes: a document is a layer on the window and
    /// a control a third thing; if two collapsed to one value, nothing else here would fail.
    #[test]
    fn the_document_and_the_window_are_different_surfaces() {
        for theme in palettes() {
            assert_ne!(theme.backdrop, theme.background, "{:?}", theme.mode);
            assert_ne!(theme.card, theme.backdrop, "{:?}", theme.mode);
            // And the document catches the light in both modes — Fluent's layering rather than
            // an inversion of it.
            assert!(
                theme.background.luminance() > theme.backdrop.luminance(),
                "{:?}: the page is not a layer above the window",
                theme.mode
            );
        }
    }

    /// The type ramp is a ramp: each step is bigger than the one below, and body is Windows 11's
    /// own 14 rather than Win32's inherited 12.
    #[test]
    fn the_type_ramp_ascends_and_body_is_fourteen() {
        const {
            assert!(text::CAPTION < text::CELL);
            assert!(text::CELL < text::BODY);
            assert!(text::BODY < text::PROSE);
            assert!(text::BODY == 14.0);
        }
    }

    /// The accent is the suite's own blue when nothing else is said, and the wash and the neat
    /// colour are two tones *of the same hue* rather than two colours.
    #[test]
    fn the_default_accent_is_the_suites_own_blue() {
        let blue = grind_core::style::palette("blue")
            .and_then(Rgb::parse)
            .unwrap();
        assert_eq!(blue, SUITE_BLUE);
        assert_eq!(Theme::of(Mode::Light).selection, blue);
        for theme in [Theme::of(Mode::Light), Theme::of(Mode::Dark)] {
            let Rgb(r, _, b) = theme.accent;
            assert!(b > r, "{:?}: the accent is not a blue", theme.mode);
            let Rgb(r, _, b) = theme.selection;
            assert!(b > r, "{:?}: the wash is not a blue", theme.mode);
        }
    }

    /// After [`accent_for`] **no** accent is invisible: every hue at every lightness and both
    /// saturations, on both grounds, clears the floor a one-pixel line needs — a sweep, since the
    /// forty-eight swatches Settings offers are a fact about one Windows build.
    #[test]
    fn any_accent_at_all_is_legible_once_it_has_been_tinted() {
        for hue in (0..360).step_by(5) {
            for lightness in (0..=100).step_by(5) {
                for saturation in [1.0, 0.55, 0.2] {
                    let base =
                        Rgb::from_hsl(f64::from(hue), saturation, f64::from(lightness) / 100.0);
                    for mode in [Mode::Light, Mode::Dark] {
                        let theme = Theme::with_accent(mode, Some(base));
                        let on_paper = theme.accent.contrast(theme.background);
                        assert!(
                            on_paper >= VISIBLE,
                            "{base:?} in {mode:?}: {:?} is {on_paper:.2} on the page",
                            theme.accent
                        );
                        // And it is still legible on its own soft ground, which is where a
                        // selected header button's letter is drawn.
                        let on_soft = theme.accent.contrast(theme.accent_soft);
                        assert!(
                            on_soft >= 2.0,
                            "{base:?} in {mode:?}: {on_soft:.2} on its own wash"
                        );
                        // Whatever is written *on* the accent reads too.
                        assert!(theme.on_accent.contrast(theme.accent) >= 3.0, "{base:?}");
                    }
                }
            }
        }
    }

    /// A tint keeps the hue it was given — the property that makes the accent still look like the
    /// colour the user picked. Greys have no hue to keep and are the exception.
    #[test]
    fn tinting_moves_the_lightness_and_leaves_the_hue() {
        for hue in (0..360).step_by(15) {
            let base = Rgb::from_hsl(f64::from(hue), 0.8, 0.5);
            for mode in [Mode::Light, Mode::Dark] {
                let (h, _, _) = accent_for(base, mode).hsl();
                let drift = (h - f64::from(hue))
                    .abs()
                    .min(360.0 - (h - f64::from(hue)).abs());
                assert!(drift < 4.0, "{hue} moved to {h} in {mode:?}");
            }
        }
    }

    /// HSL is a round trip, or the ramp above would be quietly shifting every accent it touched.
    #[test]
    fn hsl_survives_the_return_journey() {
        for colour in [
            Rgb(0x00, 0x74, 0xd9),
            Rgb(0xff, 0x41, 0x36),
            Rgb(0x2e, 0xcc, 0x40),
            Rgb(0x80, 0x80, 0x80),
            Rgb(0, 0, 0),
            Rgb(0xff, 0xff, 0xff),
        ] {
            let (h, s, l) = colour.hsl();
            let back = Rgb::from_hsl(h, s, l);
            for (a, b) in [(colour.0, back.0), (colour.1, back.1), (colour.2, back.2)] {
                assert!(a.abs_diff(b) <= 1, "{colour:?} came back {back:?}");
            }
        }
    }

    /// The dark frame, as a test: navy the document chose is lifted on the dark page until it
    /// reads, left alone in the light palette and on a fill of its own, and a cell with no
    /// colour still gets automatic ink.
    #[test]
    fn a_documents_own_colour_reads_on_the_dark_page() {
        let navy = Rgb(0x00, 0x1f, 0x3f);
        let dark = Theme::of(Mode::Dark);
        let lifted = document_ink(Some(navy), dark.background, false, dark);
        assert!(lifted.contrast(dark.background) >= 4.5, "{lifted:?}");
        let light = Theme::of(Mode::Light);
        assert_eq!(
            document_ink(Some(navy), light.background, false, light),
            navy
        );
        let silver = Rgb(0xdd, 0xdd, 0xdd);
        assert_eq!(document_ink(Some(navy), silver, true, dark), navy);
        assert_eq!(
            document_ink(None, silver, true, dark),
            automatic_ink(silver, dark)
        );
    }

    /// **Automatic is a colour that can be read** on whatever ground the document chose — the
    /// ordinary case being a heading row filled pale grey with nothing said about the text.
    #[test]
    fn automatic_ink_reads_on_whatever_the_document_chose() {
        for theme in palettes() {
            // The theme's own ink on the theme's own grounds — nothing clever wanted here.
            assert_eq!(automatic_ink(theme.background, theme), theme.text);
            for ground in [
                Rgb(0xee, 0xee, 0xee), // the pale fill `examples/sample-sheet.sh` uses
                Rgb(0x11, 0x11, 0x11),
                Rgb(0xff, 0xdc, 0x00), // PALETTE's yellow, as a highlight
                Rgb(0x00, 0x1f, 0x3f), // PALETTE's navy
                theme.background,
                theme.backdrop,
            ] {
                let ink = automatic_ink(ground, theme);
                assert!(
                    ink.contrast(ground) >= 4.5,
                    "{:?}: {ink:?} on {ground:?} is {:.2}",
                    theme.mode,
                    ink.contrast(ground)
                );
            }
        }
    }

    /// A control that is on looks on whether or not the pointer is over it, and one that is off
    /// with no pointer on it draws no ground at all — which is what lets a row of toggles read as
    /// one band.
    #[test]
    fn a_control_grounds_itself_by_state() {
        let theme = Theme::of(Mode::Light);
        assert_eq!(control_fill(theme, Interaction::Rest, false, false), None);
        assert_eq!(
            control_fill(theme, Interaction::Hover, false, false),
            Some(theme.subtle_hover)
        );
        assert_eq!(
            control_fill(theme, Interaction::Rest, false, true),
            Some(theme.card)
        );
        // Checked outranks every state, and every one of its three is still a wash of the accent
        // rather than the accent neat.
        for state in [Interaction::Rest, Interaction::Hover, Interaction::Pressed] {
            let fill = control_fill(theme, state, true, false).expect("checked always fills");
            assert!(
                fill.contrast(theme.accent) > 1.6,
                "{state:?}: {fill:?} swallows its own label"
            );
        }
    }

    /// A line has to be visible against the ground it sits on, in both palettes — which is why
    /// the dark theme is not the light one inverted. Stated as contrast ratios rather than as
    /// luminance distances, so the numbers are the ones an accessibility checker would report.
    #[test]
    fn every_ink_reads_on_the_ground_it_is_drawn_on() {
        for theme in palettes() {
            let checks = [
                ("text on the page", theme.text, theme.background, 7.0),
                ("text on the window", theme.text, theme.backdrop, 7.0),
                (
                    "a label on the window",
                    theme.text_secondary,
                    theme.backdrop,
                    4.5,
                ),
                (
                    "a placeholder on a control",
                    theme.text_tertiary,
                    theme.card,
                    2.4,
                ),
                ("a notice", theme.banner_text, theme.banner, 7.0),
                ("a hint", theme.hint_text, theme.hint, 7.0),
                ("a grid line", theme.grid_line, theme.background, 1.08),
                ("a control's outline", theme.stroke, theme.backdrop, 1.05),
            ];
            for (what, ink, ground, floor) in checks {
                let ratio = ink.contrast(ground);
                assert!(
                    ratio >= floor,
                    "{:?}: {what} is {ratio:.2}, wanted {floor}",
                    theme.mode
                );
            }
        }
    }

    /// The assist band is help while typing and the notice bar is a state the document is in.
    /// Both can be up at once, so they must not read as one band: different grounds, and each
    /// legible in its own.
    #[test]
    fn the_hint_band_is_readable_and_is_not_the_notice_bar() {
        for theme in [Theme::of(Mode::Light), Theme::of(Mode::Dark)] {
            assert_ne!(theme.hint, theme.banner, "{:?}", theme.mode);
            // And quiet: a band drawn under the strip every keystroke must not stand further off
            // the chrome than the notice bar, the loud one. Measured in **all three channels**,
            // not luminance: an amber and a pale blue of one lightness are not equally loud.
            let distance = |a: Rgb, b: Rgb| {
                u32::from(a.0.abs_diff(b.0))
                    + u32::from(a.1.abs_diff(b.1))
                    + u32::from(a.2.abs_diff(b.2))
            };
            let hint = distance(theme.hint, theme.backdrop);
            let banner = distance(theme.banner, theme.backdrop);
            assert!(hint < banner, "{:?}: {hint} vs {banner}", theme.mode);
        }
    }
}
