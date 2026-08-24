// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Character formatting — the `style:style` of family `text` (doc/text-core.md). **\[ODT\]**
//!
//! `grind_sheet::style::CellStyle`'s counterpart, and the same split: the pieces a character
//! style is made of — `fo:font-weight`, an ODF length, a `#rrggbb` colour — are not a document
//! type's vocabulary and live in `grind_core::style`. What is here is the one thing that is
//! genuinely about a *run of text*: which of those pieces it carries.
//!
//! **The values are ODF's own, kept verbatim.** `bold` stays the string `"bold"` and `12pt`
//! stays `"12pt"`, because a model that normalises them has to choose a canonical spelling and
//! then every document that chose another one round-trips differently. The convenience readers
//! below ([`CharStyle::is_bold`] and friends) interpret without storing an interpretation.
//!
//! ## What this closes, and what it deliberately does not
//!
//! `doc/text-core.md` gates *style definitions* — reading `office:styles` and writing it back —
//! and this is **not** that gate opening. The split it draws is between two different things
//! that both arrive as a `text:style-name`:
//!
//! * **Automatic styles** (`office:automatic-styles`) are how every ODF producer spells
//!   *direct* formatting: LibreOffice writes bold-in-the-middle-of-a-sentence as a generated
//!   `T3` whose whole content is `fo:font-weight="bold"`. The name is a serialisation detail
//!   with no meaning outside the file, so this build resolves it to properties on the run and
//!   forgets the name — which is what makes "select this and press Ctrl+B" expressible at all.
//! * **Named styles** (`office:styles`, usually `styles.xml`) are the document's own
//!   vocabulary — `Emphasis`, `Source_20_Text`. Those keep their name and are *not* resolved,
//!   because the name is the meaning, and a UI that turned `Emphasis` into
//!   `fo:font-style="italic"` would have thrown the document's structure away to draw it.
//!
//! So a [`crate::model::Run`] carries both: a name this build does not interpret, and the
//! formatting it does. See `crate::odf::read` for where the line is drawn on the way in and
//! `crate::odf::write` for the pool that puts the names back.

use grind_core::style::TextStyle;

/// How one run of text looks. Every field is an ODF attribute value, and `None` means the
/// attribute is absent — which is not the same as `Some("none")`, the spelling ODF uses for
/// "explicitly not underlined".
///
/// The property set is `doc/text-core.md`'s "text family" row, unchanged and complete.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct CharStyle {
    /// `fo:font-family` — the family name, quoted exactly as the document spells it. A
    /// `style:font-name` pointing into `office:font-face-decls` is resolved to this on read,
    /// because the indirection is a container detail and the family is the fact.
    pub font_family: Option<String>,
    /// `fo:font-size` — a length, e.g. `12pt`, or a percentage.
    pub font_size: Option<String>,
    /// `fo:font-weight` — `normal`, `bold`, or a hundreds number.
    pub font_weight: Option<String>,
    /// `fo:font-style` — `normal`, `italic`, `oblique`.
    pub font_style: Option<String>,
    /// `style:text-underline-style` — `none`, `solid`, `dotted`, …
    pub underline: Option<String>,
    /// `style:text-line-through-style` — the same vocabulary, struck through instead.
    pub line_through: Option<String>,
    /// `fo:color` — `#rrggbb`.
    pub color: Option<String>,
    /// `fo:background-color` — `#rrggbb` or `transparent`. Text highlighting.
    pub background: Option<String>,
}

impl CharStyle {
    /// Nothing set at all — formatting worth neither writing nor pooling.
    pub fn is_plain(&self) -> bool {
        *self == CharStyle::default()
    }

    /// The metrics half, for `grind_core::layout`.
    ///
    /// Four of the eight fields change how *wide* text is; the other four change how it looks.
    /// A layout engine has no use for a colour, and `grind_core::style::TextStyle` says so by
    /// not having one — this is the projection that keeps that true.
    pub fn metrics(&self) -> TextStyle {
        TextStyle {
            font_family: self.font_family.clone(),
            font_size: self.font_size.clone(),
            font_weight: self.font_weight.clone(),
            font_style: self.font_style.clone(),
        }
    }

    /// Layer `other` over `self`: every property `other` sets wins, and the rest are kept.
    ///
    /// What nesting means. `<span T1><span T2>x</span></span>` gives `x` both styles with the
    /// inner one winning, which is CSS's rule and ODF's, and it is why reading can flatten the
    /// tree (`doc/text-core.md`) without losing how the text renders.
    pub fn layer(&mut self, other: &CharStyle) {
        let fields: [(&mut Option<String>, &Option<String>); 8] = [
            (&mut self.font_family, &other.font_family),
            (&mut self.font_size, &other.font_size),
            (&mut self.font_weight, &other.font_weight),
            (&mut self.font_style, &other.font_style),
            (&mut self.underline, &other.underline),
            (&mut self.line_through, &other.line_through),
            (&mut self.color, &other.color),
            (&mut self.background, &other.background),
        ];
        for (mine, theirs) in fields {
            if theirs.is_some() {
                mine.clone_from(theirs);
            }
        }
    }

    /// Whatever `self` and `other` agree about, and nothing else.
    ///
    /// What a toolbar shows over a selection that is not uniform: a range that is bold
    /// throughout reads as bold, and one that is half bold reads as neither, which is what
    /// every word processor does and the only answer that makes a toggle predictable.
    pub fn common(&self, other: &CharStyle) -> CharStyle {
        fn agreed(a: &Option<String>, b: &Option<String>) -> Option<String> {
            (a == b).then(|| a.clone()).flatten()
        }
        CharStyle {
            font_family: agreed(&self.font_family, &other.font_family),
            font_size: agreed(&self.font_size, &other.font_size),
            font_weight: agreed(&self.font_weight, &other.font_weight),
            font_style: agreed(&self.font_style, &other.font_style),
            underline: agreed(&self.underline, &other.underline),
            line_through: agreed(&self.line_through, &other.line_through),
            color: agreed(&self.color, &other.color),
            background: agreed(&self.background, &other.background),
        }
    }

    /// Whether this run reads as bold.
    ///
    /// `fo:font-weight` is `normal`, `bold`, or a hundreds number, and CSS's threshold for
    /// "bold" is 600 — interpreted here rather than stored, so the document's own spelling
    /// survives a round trip whichever one it chose.
    pub fn is_bold(&self) -> bool {
        match self.font_weight.as_deref() {
            None | Some("normal") => false,
            Some("bold") => true,
            Some(other) => other.trim().parse::<u32>().is_ok_and(|n| n >= 600),
        }
    }

    pub fn set_bold(&mut self, on: bool) {
        self.font_weight = on.then(|| "bold".to_owned());
    }

    /// Whether this run reads as italic. `oblique` counts: it is a slanted face by another
    /// name, and no UI this project will build distinguishes them.
    pub fn is_italic(&self) -> bool {
        matches!(self.font_style.as_deref(), Some("italic" | "oblique"))
    }

    pub fn set_italic(&mut self, on: bool) {
        self.font_style = on.then(|| "italic".to_owned());
    }

    pub fn is_underlined(&self) -> bool {
        is_line(&self.underline)
    }

    pub fn set_underlined(&mut self, on: bool) {
        self.underline = on.then(|| "solid".to_owned());
    }

    pub fn is_struck(&self) -> bool {
        is_line(&self.line_through)
    }

    pub fn set_struck(&mut self, on: bool) {
        self.line_through = on.then(|| "solid".to_owned());
    }

    /// The `style:text-properties` attributes, ready to drop into a start tag.
    ///
    /// In one place because the writer emits them and nothing else may — a second speller would
    /// be a second chance to disagree with the reader about which attribute holds what.
    pub fn attributes(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        for (name, value) in self.pairs() {
            let Some(value) = value else { continue };
            let value = match name {
                "fo:font-family" => quote_family(value),
                _ => value.to_owned(),
            };
            let _ = write!(out, " {name}=\"{}\"", grind_core::odf::xml::esc(&value));
        }
        out
    }

    /// Every property as the attribute that carries it. The single ordered list this module's
    /// reader and writer both work from.
    fn pairs(&self) -> [(&'static str, Option<&str>); 8] {
        [
            ("fo:font-family", self.font_family.as_deref()),
            ("fo:font-size", self.font_size.as_deref()),
            ("fo:font-weight", self.font_weight.as_deref()),
            ("fo:font-style", self.font_style.as_deref()),
            ("style:text-underline-style", self.underline.as_deref()),
            (
                "style:text-line-through-style",
                self.line_through.as_deref(),
            ),
            ("fo:color", self.color.as_deref()),
            ("fo:background-color", self.background.as_deref()),
        ]
    }
}

/// Whether a `*-style` attribute names a line that is actually drawn. ODF's `none` is an
/// explicit absence and every other value is some kind of line.
fn is_line(value: &Option<String>) -> bool {
    !matches!(value.as_deref(), None | Some("none"))
}

/// A font family with XSL-FO's quoting taken off, if it had any.
///
/// **The one value in this module that is not kept verbatim, and it has to be.**
/// `fo:font-family` is an XSL-FO font list, so a name containing a space is written
/// `'Liberation Serif'` — the quotes are the *list's* syntax and not part of the family's name.
/// Keeping them would make `font_family == Some("Georgia")` false for a document that wrote
/// `'Georgia'`, and would hand a shell a name no font system resolves. The same trade `text:s`
/// gets: decode on the way in, re-encode on the way out ([`quote_family`]).
///
/// A value with a comma in it is a *list* of families and is left exactly as it was: picking
/// one of them would be this build choosing a font, which is a renderer's business.
pub fn unquote_family(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.contains(',') {
        return value.to_owned();
    }
    for quote in ['\'', '"'] {
        if let Some(inner) = trimmed
            .strip_prefix(quote)
            .and_then(|v| v.strip_suffix(quote))
        {
            return inner.to_owned();
        }
    }
    value.to_owned()
}

/// The inverse: quote a family whose name would not survive as a bare XSL-FO token.
///
/// LibreOffice writes `'Liberation Serif'` and reads a bare `Liberation Serif` as one family
/// anyway, so this is about producing what the format says rather than about being understood.
fn quote_family(value: &str) -> String {
    let needs = !value.contains(',')
        && value.contains(char::is_whitespace)
        && !value.starts_with(['\'', '"']);
    match needs {
        true => format!("'{value}'"),
        false => value.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grind_core::odf::xml::esc;

    fn bold() -> CharStyle {
        CharStyle {
            font_weight: Some("bold".into()),
            ..CharStyle::default()
        }
    }

    #[test]
    fn a_weight_is_interpreted_and_never_normalised() {
        let mut style = CharStyle::default();
        assert!(!style.is_bold());

        style.font_weight = Some("700".into());
        assert!(style.is_bold(), "CSS's threshold is 600");
        assert_eq!(
            style.font_weight.as_deref(),
            Some("700"),
            "the document's own spelling survives"
        );

        style.font_weight = Some("400".into());
        assert!(!style.is_bold());
        style.font_weight = Some("normal".into());
        assert!(!style.is_bold());
    }

    /// `Some("none")` and `None` are different attributes and the same rendering — the
    /// distinction ODF makes and this model keeps.
    #[test]
    fn an_explicit_none_is_not_a_line_and_is_still_a_value() {
        let mut style = CharStyle {
            underline: Some("none".into()),
            ..CharStyle::default()
        };
        assert!(!style.is_underlined());
        assert!(!style.is_plain(), "the attribute is still there to write");

        style.underline = Some("dotted".into());
        assert!(style.is_underlined());
    }

    #[test]
    fn the_inner_style_wins_where_it_says_anything() {
        let mut outer = CharStyle {
            font_size: Some("14pt".into()),
            color: Some("#001f3f".into()),
            ..bold()
        };
        outer.layer(&CharStyle {
            font_weight: Some("normal".into()),
            font_style: Some("italic".into()),
            ..CharStyle::default()
        });
        assert_eq!(outer.font_weight.as_deref(), Some("normal"), "overridden");
        assert_eq!(outer.font_style.as_deref(), Some("italic"), "added");
        assert_eq!(outer.font_size.as_deref(), Some("14pt"), "untouched");
        assert_eq!(outer.color.as_deref(), Some("#001f3f"), "untouched");
    }

    #[test]
    fn a_mixed_selection_agrees_about_nothing() {
        let italic = CharStyle {
            font_style: Some("italic".into()),
            ..CharStyle::default()
        };
        assert_eq!(bold().common(&bold()), bold());
        assert!(bold().common(&italic).is_plain());
        // Agreeing that a property is *absent* is still agreement.
        assert_eq!(
            CharStyle::default().common(&CharStyle::default()),
            CharStyle::default()
        );
    }

    #[test]
    fn the_toggles_set_and_clear_odfs_own_spelling() {
        let mut style = CharStyle::default();
        style.set_bold(true);
        style.set_italic(true);
        style.set_underlined(true);
        style.set_struck(true);
        assert_eq!(
            style.attributes(),
            concat!(
                " fo:font-weight=\"bold\"",
                " fo:font-style=\"italic\"",
                " style:text-underline-style=\"solid\"",
                " style:text-line-through-style=\"solid\"",
            )
        );

        style.set_bold(false);
        style.set_italic(false);
        style.set_underlined(false);
        style.set_struck(false);
        assert!(style.is_plain(), "off means absent, not \"normal\"");
    }

    /// XSL-FO's quotes are the font *list*'s syntax, not part of a family's name — and a
    /// measured fact about the oracle: LibreOffice writes `'Liberation Serif'` for what it was
    /// given as `Liberation Serif`, so a model that kept the quotes would fail loop C for a
    /// difference that is not one.
    #[test]
    fn a_font_family_loses_its_quoting_and_gets_it_back() {
        assert_eq!(unquote_family("'Liberation Serif'"), "Liberation Serif");
        assert_eq!(unquote_family("\"Georgia\""), "Georgia");
        assert_eq!(unquote_family("Georgia"), "Georgia");
        // A list is somebody's fallback chain and stays exactly as written.
        let list = "'Liberation Serif', 'Times New Roman', serif";
        assert_eq!(unquote_family(list), list);

        let quoted = |family: &str| {
            CharStyle {
                font_family: Some(family.to_owned()),
                ..CharStyle::default()
            }
            .attributes()
        };
        // The apostrophes are XML-escaped on the way into the attribute, which is what
        // LibreOffice writes too — `svg:font-family="&apos;Liberation Sans&apos;"` is verbatim
        // out of a Writer document.
        assert_eq!(
            quoted("Liberation Serif"),
            format!(" fo:font-family=\"{}\"", esc("'Liberation Serif'"))
        );
        assert_eq!(quoted("Georgia"), " fo:font-family=\"Georgia\"");
        assert_eq!(quoted(list), format!(" fo:font-family=\"{}\"", esc(list)));
    }

    /// The projection into the layout engine's vocabulary carries the four properties that
    /// change how wide text is, and drops the four that do not.
    #[test]
    fn only_the_metric_properties_reach_the_layout_engine() {
        let style = CharStyle {
            font_family: Some("Georgia".into()),
            font_size: Some("11pt".into()),
            color: Some("#ff4136".into()),
            underline: Some("solid".into()),
            ..bold()
        };
        let metrics = style.metrics();
        assert_eq!(metrics.font_family.as_deref(), Some("Georgia"));
        assert_eq!(metrics.font_size.as_deref(), Some("11pt"));
        assert_eq!(metrics.font_weight.as_deref(), Some("bold"));
        assert_eq!(metrics.font_style, None);
    }
}
