// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What colour everything is, how big it is, and which of the two sets Windows is asking for.
//!
//! Split the way the whole crate is: the **tokens and the choice between the two are portable**
//! and tested on any host, and only the registry read, the dark title bar and the rounded window
//! corner need Windows.
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
//!
//! ## Where the numbers come from — *W10*
//!
//! The tables below are **Fluent 2's neutral ramp**, resolved to opaque values. Fluent states
//! most of its surface tokens as a white or black at some alpha over the layer beneath
//! (`ControlFillColorDefault` is `#FFFFFF` at 70% in light, `#FFFFFF` at 5.8% in dark, and so
//! on), and GDI has no alpha in `FillRect` — so the composite is done once, here, and written
//! down as the colour it comes out as. That is the same move [`Rgb::blend`] already made for the
//! selection wash, applied to the whole palette.
//!
//! The consequence worth knowing: this shell has **three grounds and not one**. A window is a
//! [`Theme::backdrop`] with the chrome on it, a [`Theme::background`] where the document is (the
//! grid's paper, the text pane's page), and a [`Theme::card`] for the things you type in and
//! click. Fluent calls those the base, a layer and a control fill; the whole reason the W9 chrome
//! looked flat is that it had one ground for all three.

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

    /// WCAG relative luminance — the perceptual weight of a colour, on 0.0 to 1.0.
    ///
    /// Public rather than a test helper because [`Rgb::contrast`] is what decides whether a
    /// **user's own accent colour** is legible on this shell's grounds, and that decision has to
    /// be made at run time on a machine nobody here can see (see [`accent_for`]).
    pub fn luminance(self) -> f64 {
        let channel = |value: u8| {
            let v = f64::from(value) / 255.0;
            match v <= 0.040_45 {
                true => v / 12.92,
                false => ((v + 0.055) / 1.055).powf(2.4),
            }
        };
        0.2126 * channel(self.0) + 0.7152 * channel(self.1) + 0.0722 * channel(self.2)
    }

    /// The WCAG contrast ratio between two colours, from 1.0 (identical) to 21.0 (black on
    /// white). 3.0 is the floor for a line or a shape you have to be able to see; 4.5 is the one
    /// for small text.
    pub fn contrast(self, other: Rgb) -> f64 {
        let (a, b) = (self.luminance(), other.luminance());
        let (light, dark) = match a > b {
            true => (a, b),
            false => (b, a),
        };
        (light + 0.05) / (dark + 0.05)
    }

    /// Hue, saturation and lightness — the space a tint ramp is built in.
    ///
    /// Windows derives `AccentLight1..3` and `AccentDark1..3` from the accent the user picked by
    /// moving its *lightness* and leaving the hue alone, which is why this is here rather than a
    /// blend towards white: blending desaturates, and an accent that desaturates on a dark theme
    /// stops looking like the colour the user chose.
    pub fn hsl(self) -> (f64, f64, f64) {
        let (r, g, b) = (
            f64::from(self.0) / 255.0,
            f64::from(self.1) / 255.0,
            f64::from(self.2) / 255.0,
        );
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let l = (max + min) / 2.0;
        if (max - min).abs() < f64::EPSILON {
            return (0.0, 0.0, l);
        }
        let d = max - min;
        let s = match l > 0.5 {
            true => d / (2.0 - max - min),
            false => d / (max + min),
        };
        let h = if (max - r).abs() < f64::EPSILON {
            ((g - b) / d).rem_euclid(6.0)
        } else if (max - g).abs() < f64::EPSILON {
            (b - r) / d + 2.0
        } else {
            (r - g) / d + 4.0
        };
        (h * 60.0, s, l)
    }

    /// The inverse of [`Rgb::hsl`].
    pub fn from_hsl(h: f64, s: f64, l: f64) -> Self {
        let (h, s, l) = (h.rem_euclid(360.0), s.clamp(0.0, 1.0), l.clamp(0.0, 1.0));
        let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
        let x = c * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
        let m = l - c / 2.0;
        let (r, g, b) = match h as u32 / 60 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        let byte = |v: f64| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
        Rgb(byte(r), byte(g), byte(b))
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

/// The contrast a shape has to reach against the ground it is drawn on to count as visible.
///
/// WCAG's own floor for a non-text element. The accent is a one- to two-pixel line in places
/// (the selection's edge, the bar under a header button), so this is exactly the number that
/// decides whether a user's chosen accent can be used as it is or has to be moved.
const VISIBLE: f64 = 3.0;

/// The accent, as the tint of `base` that can actually be seen on `mode`'s ground.
///
/// **This is decision 9's answer, reversed in W10 and reversed for a stated reason.** W9 refused
/// to read the system accent at all, because "half the values in that picker are colours a
/// one-pixel selection edge disappears into". That is true of the accent *as the user picked it*
/// and false of the accent as Windows itself draws it: WinUI never paints `SystemAccentColor`
/// on a surface, it paints `SystemAccentColorDark1` on a light one and `SystemAccentColorLight2`
/// on a dark one, and those are tints of the same hue at a lightness chosen to read. So this
/// does the same thing — and then, because a ramp of fixed steps still cannot promise anything
/// about a *particular* colour, it keeps moving the lightness until [`Rgb::contrast`] against
/// the ground clears [`VISIBLE`].
///
/// The result is a function of the accent alone, so the whole of "does this shell work with the
/// user's colour" is testable on a machine with no Windows and no user — see the sweep in this
/// module's tests, which checks every hue at every lightness rather than the forty-eight swatches
/// the Settings app happens to offer today.
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

/// Every colour this shell draws with, other than the ones a document chose for itself.
///
/// A struct rather than a set of constants so that the drawing code takes the palette as an
/// argument and cannot reach past it — which is what makes `--render-to` able to produce a dark
/// screenshot on a light machine, and what would make a high-contrast palette a third table
/// rather than a third code path.
///
/// **Three grounds** (W10), which is the shape of the whole thing and the part worth reading
/// first: [`Theme::backdrop`] is the window, [`Theme::background`] is the document, and
/// [`Theme::card`] is a control. Everything else is ink on one of those or a line between two.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    pub mode: Mode,

    /// The window's own ground — Fluent's *Solid Background Base*. The strip, the header bands,
    /// the format strip and the status bar all sit on it, and so does the surround the text
    /// pane's page floats in. Never the ground of anything the *document* owns.
    pub backdrop: Rgb,
    /// The document's ground: the grid's paper, and the text pane's page. A layer *above* the
    /// backdrop in both palettes — lighter in light mode and lighter in dark mode too, which is
    /// Fluent's layering rather than an inversion, and is what makes the document read as a sheet
    /// of something laid on the window.
    pub background: Rgb,
    /// A control's ground at rest: the name box, the formula bar, a strip button. Fluent's
    /// *ControlFillColorDefault*, composited onto the backdrop.
    pub card: Rgb,
    /// The same control with the pointer on it, and pressed. Fluent's *Secondary* and *Tertiary*
    /// control fills — and in the light palette the pressed one is **quieter** than the hover,
    /// which is not a mistake: pressing recedes and hovering lifts.
    pub card_hover: Rgb,
    pub card_pressed: Rgb,
    /// The ground a control with **no** fill of its own takes when the pointer is on it —
    /// Fluent's *Subtle* fills, which is what a toggle in a strip uses so that a row of them
    /// reads as one band until you point at one.
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
    /// The hairlines **past the last row and column the document uses** — the same lines, drawn
    /// quieter, so the sheet somebody wrote reads as the subject and the empty rest of an
    /// infinite grid as the paper it is on.
    ///
    /// Not a border round the used range and not a different ground: both of those would be this
    /// shell inventing a page boundary the document does not have. One tone of grey is the whole
    /// of it, and `App::used_extent` — the same answer the status bar already reports — is where
    /// the line falls.
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

    /// The accent, painted neat: the outline round the selection, the bar under a selected header
    /// button, a checked toggle, the argument being typed.
    ///
    /// **The user's own Windows accent** since W10, moved to whichever tint of it reads on this
    /// palette's ground ([`accent_for`]), and the suite's blue when the system has not said.
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

    /// The assist band under the strip while a formula is being typed — the completion offers and
    /// the signature hint (`sheet/assist.rs`). Its own ground, tinted towards the accent rather
    /// than towards the notice bar's amber, because it is *help while typing* and not a state the
    /// document is in — the two can be up at once and must not read as the same thing.
    pub hint: Rgb,
    pub hint_text: Rgb,

    /// Fluent's focus rectangle: a stroke of [`Theme::focus`] with [`Theme::focus_inner`] just
    /// inside it, so that the ring is visible on a light control and on a dark one without
    /// knowing which it is drawn on.
    pub focus: Rgb,
    pub focus_inner: Rgb,
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
};

/// The dark palette. Not an inversion of the light one: the grid lines are *lighter* than the
/// ground here and darker than it there, because a line has to be visible against what it sits
/// on and inverting a light theme's greys puts them the wrong side of it.
///
/// The same holds for the layering. [`Theme::background`] is lighter than [`Theme::backdrop`] in
/// **both** palettes — a document is a layer laid on the window, and a layer catches the light in
/// Fluent whichever mode it is in.
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
/// marker is the theme's ordinary text colour, and a label's is the theme's tertiary ink, which
/// is Fluent's own name for "present, but not what you are reading".
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
        R::Label => Some(theme.text_tertiary),
        R::Error => named("red"),
        R::Stale => named("maroon"),
    }
}

impl Theme {
    /// This palette with the suite's own accent — what `--render-to` draws and what a machine
    /// that has not said otherwise gets.
    pub fn of(mode: Mode) -> Self {
        Self::with_accent(mode, None)
    }

    /// This palette with the user's accent colour, if the system named one.
    ///
    /// Everything downstream of the accent is derived here rather than at the drawing call, so
    /// that a shell that reads `theme.accent_soft` cannot get one that does not go with
    /// `theme.accent`.
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
            // The wash a selected cell's ground moves towards is the accent as the *user* chose
            // it, not the tint — a wash is diluted to a fifth before it touches a cell, so the
            // legibility argument that moves the neat colour does not apply to it, and using the
            // tint here would make a light theme's selection paler than it should be.
            selection: system,
            on_accent: match accent.contrast(Rgb(0, 0, 0)) > accent.contrast(Rgb(0xff, 0xff, 0xff))
            {
                true => Rgb(0, 0, 0),
                false => Rgb(0xff, 0xff, 0xff),
            },
            ..base
        }
    }

    /// What the two registry values mean.
    ///
    /// `AppsUseLightTheme` is a `REG_DWORD`: 1 is light, 0 is dark, and **a missing value is
    /// light**. The last part is the one that matters and it is not a guess — a fresh Wine
    /// prefix has no `Personalize` key at all, which is exactly the case
    /// `doc/windows-shell.md` names as leaving dark mode untested there.
    ///
    /// That value is `0xAABBGGRR` — the ends the other way round from every hex colour a human
    /// writes, and the alpha byte meaningless — which is why the swap happens here, next to the
    /// sentence that says so, rather than at the registry read where it would be one more
    /// unexplained shift among the pointer casts.
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

/// The type ramp, in pixels at 100% — Fluent 2's, and the sizes are its own names.
///
/// Windows 11's body text is **14 epx**, not the 12 a Win32 window has drawn since 1995, and the
/// single most dated thing about the W9 chrome was that everything in it was set at 13 or below.
/// A ramp rather than one size is the other half: a status bar and a formula bar are not the same
/// kind of text, and a window where they are set identically reads as undesigned.
pub mod text {
    /// *Caption* — a label on something else: the header bands, the status bar, a picker's
    /// caption. The one size below body, and never used for anything a person reads a sentence of.
    pub const CAPTION: f64 = 12.0;
    /// *Body* — the shell's own voice: the name box, the formula bar, the notice bar, a menu.
    pub const BODY: f64 = 14.0;
    /// A cell's own text, which is **not** body: a grid is read by scanning a table, and a
    /// document's own column widths were chosen against a size near this. Raising it is data
    /// loss dressed as typography — the numbers stop fitting the columns the document sizes for
    /// them, which a rendered frame said before this comment did.
    pub const CELL: f64 = 13.0;
    /// Prose in the text pane. Larger than [`CELL`] on purpose: a spreadsheet is read by
    /// scanning and a document by reading, and every word processor there has ever been sets body
    /// text bigger than a cell.
    pub const PROSE: f64 = 15.0;
}

/// The spacing ramp, in pixels at 100% — Fluent's 4-pixel grid.
///
/// Every measurement in this shell's chrome is one of these or a small multiple, which is what
/// stops the geometry drifting into arbitrary numbers as features arrive. The two radii are
/// Fluent's own tokens and there are only two on purpose: 4 for anything you click or type in,
/// 8 for a surface that holds other things.
pub mod space {
    /// `controlCornerRadius`, and the corner of every field, button and swatch.
    pub const RADIUS: f64 = 4.0;
    /// `overlayCornerRadius`, and the corner of the text pane's page and the two inset bands.
    pub const RADIUS_SURFACE: f64 = 8.0;
    /// The gap between two controls in a row.
    pub const GAP: f64 = 4.0;
    /// The gap between two *groups* of controls, and between a band's edge and what is in it.
    pub const GROUP: f64 = 12.0;
    /// A control's height — Fluent's standard, and the reason the strips grew in W10.
    pub const CONTROL_H: f64 = 32.0;
}

/// The ink for text the document did **not** give a colour to, on a ground the document *did*
/// choose — ODF's *automatic* colour, resolved.
///
/// The theme's own ink wherever that reads, which is every ordinary cell and every ordinary run;
/// black or white, whichever reads better, where it does not. That last clause is not a nicety —
/// it is what "automatic" means, and without it a dark theme draws its near-white text on the
/// pale grey heading row a document chose for itself and the row becomes unreadable. A dark
/// palette showed exactly that the first time one could be rendered at all (W10's `--dark`), and
/// the light palette had the same bug waiting on any document with a dark fill.
///
/// The threshold is WCAG's floor for small text, because that is what this is.
pub fn automatic_ink(ground: Rgb, theme: Theme) -> Rgb {
    if theme.text.contrast(ground) >= 4.5 {
        return theme.text;
    }
    let (black, white) = (Rgb(0, 0, 0), Rgb(0xff, 0xff, 0xff));
    match black.contrast(ground) > white.contrast(ground) {
        true => black,
        false => white,
    }
}

/// Which of the three grounds a strip control is standing on right now.
///
/// The whole of this shell's hover-and-press feedback, as a portable function: a control with a
/// fill of its own moves between [`Theme::card`], [`Theme::card_hover`] and
/// [`Theme::card_pressed`], and one without takes the *subtle* pair instead — transparent at
/// rest, so that a row of toggles reads as one band until the pointer is on one of them.
///
/// `checked` outranks both, because a toggle that is on has to look on whether or not the pointer
/// happens to be over it; it takes [`Theme::accent_soft`] and moves one step per state from
/// there.
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

/// Which theme the user has chosen, read from the registry — the mode **and**, since W10, the
/// accent colour.
///
/// Failure in any form — no key, no value, the wrong type — is the default rather than an error.
/// A shell that refused to start because it could not learn a colour would be worse than one
/// that started in the wrong one, and that goes double for the accent: no accent is the suite's
/// own blue, which is what every `--render-to` frame and every Wine run gets.
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

/// Ask the compositor for the window chrome this theme wants: a dark title bar in dark mode, the
/// **caption painted in the application's own backdrop**, and Windows 11's rounded corner.
///
/// The one piece of "native chrome" this shell asks for, and the only one it can get without a
/// manifest. Deliberately best-effort in every part: each attribute fails on Windows builds older
/// than itself and is largely inert under Wine, and in all of those cases the right response is
/// the system's own title bar over our grid rather than a refusal to open.
///
/// Painting the caption is what makes the window read as one surface instead of as a document
/// with a lid on it — it is what Explorer, Settings and Terminal all do, and it is the reason
/// [`Theme::backdrop`] is a token rather than a shade of the header band.
#[cfg(windows)]
pub fn apply_window_chrome(hwnd: windows::Win32::Foundation::HWND, theme: Theme) {
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

    /// `AccentColor` is stored the other way round from every hex colour a human writes, and the
    /// alpha byte is meaningless. Getting this backwards would tint the whole window with the
    /// user's accent read as its own complement, which is exactly the kind of bug that looks
    /// deliberate.
    #[test]
    fn the_accent_dword_is_read_ends_first() {
        // 0xAABBGGRR for a pure red.
        let theme = Theme::from_registry_values(Some(1), Some(0xff00_00ff));
        assert_eq!(theme.selection, Rgb(0xff, 0x00, 0x00));
        // And a pure blue, whose bytes are the other end of the word.
        let theme = Theme::from_registry_values(Some(1), Some(0xffff_0000));
        assert_eq!(theme.selection, Rgb(0x00, 0x00, 0xff));
    }

    /// The unused part of the grid is drawn *quieter*, not differently: its hairline is nearer
    /// the ground than the used one in both palettes — which, since the dark theme's lines are
    /// lighter than its ground and the light theme's darker than its own, means the two soft
    /// tones sit on opposite sides. Asserted as a distance rather than as "lighter", which is
    /// what makes this one rule rather than two.
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

    /// **The three grounds are three grounds**, in both palettes: a document is a layer laid on
    /// the window, and a control is a third thing again. If any two of them collapsed to one
    /// value the whole W10 look would be W9's back again, and nothing else here would fail.
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

    /// **The whole of decision 9, reversed and made checkable.** W9 would not read the user's
    /// accent because some of the values in that picker cannot be seen on a grid. This asserts
    /// that after [`accent_for`] *none* of them is: every hue at every lightness and both
    /// saturations, on both grounds, clears the contrast floor a one-pixel line needs.
    ///
    /// A sweep rather than the forty-eight swatches Settings offers today, because the swatch
    /// list is a fact about one Windows build and this is a fact about the function.
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

    /// **Automatic is a colour that can be read**, on whatever ground the document chose — which
    /// is the whole of what ODF means by leaving one out. The failing case is the ordinary one:
    /// a document that fills its heading row pale grey and says nothing about the text.
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
            // And it is quiet: a band drawn under the strip on every keystroke must not stand
            // further off the chrome around it than the notice bar does, which is the loud one.
            //
            // Measured as a distance in **all three channels** rather than in luminance, and
            // that is the point rather than a detail: an amber and a pale blue of the same
            // lightness are not equally loud, and the first version of this test said the
            // notice bar was the quieter of the two because it happened to weigh the same.
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
