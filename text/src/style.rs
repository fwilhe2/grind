// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Character formatting — the `style:style` of family `text` (`doc/text-core.md`). **\[ODT\]** The
//! pieces are `grind_core::style`'s; this is which of them a run of text carries, ODF values
//! verbatim (`bold`, `12pt`), interpreted by readers such as [`CharStyle::is_bold`] rather than
//! normalised.
//!
//! Two things arrive as a `text:style-name` and are treated differently:
//!
//! * **Automatic styles** are how every producer spells direct formatting — LibreOffice writes
//!   bold mid-sentence as a generated `T3`. The name means nothing outside the file, so it is
//!   resolved onto the run and forgotten, which is what makes "select this and press Ctrl+B"
//!   expressible.
//! * **Named styles** (`Emphasis`, `Source_20_Text`) are the document's vocabulary and keep their
//!   name: resolving `Emphasis` into italic would throw the structure away. They are read for
//!   showing ([`crate::paragraph::shown`]) and never written.
//!
//! A [`crate::model::Run`] carries both. `crate::odf::read` draws the line on the way in,
//! `crate::odf::write`'s pool puts the names back.

use grind_core::style::TextStyle;

/// How one run of text looks — `doc/text-core.md`'s text-family properties. Every field is an ODF
/// attribute value; `None` is absent, unlike `Some("none")`, ODF's "explicitly not underlined".
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct CharStyle {
    /// `fo:font-family`, the family name. A `style:font-name` reference into
    /// `office:font-face-decls` is resolved to this on reading.
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
    /// The automatic style this formatting was read from, when it says what the fields do not —
    /// superscript, a language, letter spacing. Not formatting, and equal to every other
    /// [`Origin`]; a save builds a restyled run's new style from it, so what the model never read
    /// survives (`grind_core::odf::envelope::patch_style`).
    pub origin: Origin,
}

/// Where a run's formatting came from, for a save ([`CharStyle::origin`]). Equal to every other,
/// so comparisons are about what formatting says; [`CharStyle::same`] sees it, for joining runs
/// and pooling styles, where a superscript run and a plain one must stay apart.
#[derive(Clone, Debug, Default)]
pub struct Origin(Option<std::sync::Arc<str>>);

impl Origin {
    pub fn new(name: &str) -> Self {
        Origin(Some(name.into()))
    }

    /// The automatic style's name in the file it was read from.
    pub fn name(&self) -> Option<&str> {
        self.0.as_deref()
    }
}

impl PartialEq for Origin {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl Eq for Origin {}

impl std::hash::Hash for Origin {
    fn hash<H: std::hash::Hasher>(&self, _: &mut H) {}
}

impl CharStyle {
    /// Nothing set at all. About what the formatting says, so a run distinguished only by its
    /// [`Origin`] is plain; a writer asks [`CharStyle::needs_style`].
    pub fn is_plain(&self) -> bool {
        *self == CharStyle::default()
    }

    /// Whether writing this formatting takes a style: anything set, or an [`Origin`] whose style
    /// says more (a superscript alone).
    pub fn needs_style(&self) -> bool {
        !self.is_plain() || self.origin.name().is_some()
    }

    /// Equal *and* read from the same style — [`Origin`] is the one difference `==` ignores.
    pub fn same(&self, other: &CharStyle) -> bool {
        self == other && self.origin.name() == other.origin.name()
    }

    /// The attributes that turn `base`'s properties into these — each differing field set or
    /// removed, nothing else — so a restyle built on the old element keeps the file's own spelling
    /// of the rest. A changed family also drops `style:font-name`, which would otherwise win.
    pub fn patch_against(&self, base: &CharStyle) -> Vec<(String, Option<String>)> {
        let mut out = Vec::new();
        for ((name, new), (_, old)) in self.pairs().into_iter().zip(base.pairs()) {
            if new == old {
                continue;
            }
            let value = new.map(|v| match name {
                "fo:font-family" => quote_family(v),
                _ => v.to_owned(),
            });
            if name == "fo:font-family" {
                out.push(("style:font-name".to_owned(), None));
            }
            out.push((name.to_owned(), value));
        }
        out
    }

    /// The four fields that change how wide text is, for `grind_core::layout`.
    pub fn metrics(&self) -> TextStyle {
        TextStyle {
            font_family: self.font_family.clone(),
            font_size: self.font_size.clone(),
            font_weight: self.font_weight.clone(),
            font_style: self.font_style.clone(),
        }
    }

    /// Layer `other` over `self`: what `other` sets wins. What nested spans mean, inner winning,
    /// which is why reading can flatten them (`doc/text-core.md`).
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

    /// What `self` and `other` agree about, and nothing else — so a half-bold selection reads as
    /// not bold.
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
            // A reading for a toolbar, not a run: it came from nowhere.
            origin: Origin::default(),
        }
    }

    /// Whether this run reads as bold: `fo:font-weight` of `bold` or 600 and up, interpreted
    /// rather than stored.
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

    /// Whether this run reads as italic; `oblique` counts.
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

    /// The `style:text-properties` attributes, ready for a start tag — the one speller of them.
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

    /// Every property as the attribute carrying it — the one list the reader and writer both use.
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

/// Whether a `*-style` attribute names a drawn line: anything but `none`.
fn is_line(value: &Option<String>) -> bool {
    !matches!(value.as_deref(), None | Some("none"))
}

/// A font family with XSL-FO's quoting removed. `fo:font-family` is a font list, so `'Liberation
/// Serif'`'s quotes are list syntax, not the name; keeping them would hand a shell a name no font
/// system resolves. Decoded on reading and re-encoded on writing (`quote_family`), as `text:s` is.
/// A value with a comma is a list and is left as it is.
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

/// Quote a family whose name would not survive as a bare XSL-FO token, as LibreOffice writes it.
fn quote_family(value: &str) -> String {
    let needs = !value.contains(',')
        && value.contains(char::is_whitespace)
        && !value.starts_with(['\'', '"']);
    match needs {
        true => format!("'{value}'"),
        false => value.to_owned(),
    }
}

/// A style's name as its author wrote it — `Text_20_body` is *Text body*. ODF spells a character
/// that is not allowed in an XML name as `_xx_` in hex; every shell's status bar reads names
/// through this. Only ASCII codes are decoded; other underscores stay.
pub fn readable_name(name: &str) -> String {
    let mut out = String::new();
    let mut rest = name;
    while let Some(at) = rest.find('_') {
        out.push_str(&rest[..at]);
        let tail = &rest[at + 1..];
        let decoded = tail
            .get(..3)
            .filter(|code| code.ends_with('_'))
            .and_then(|code| u8::from_str_radix(&code[..2], 16).ok())
            .filter(u8::is_ascii);
        match decoded {
            Some(byte) => {
                out.push(char::from(byte));
                rest = &tail[3..];
            }
            None => {
                out.push('_');
                rest = tail;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ODF's `_xx_` escaping undone, and nothing that merely contains an underscore mangled.
    #[test]
    fn a_style_name_is_shown_as_its_author_wrote_it() {
        assert_eq!(readable_name("Text_20_body"), "Text body");
        assert_eq!(readable_name("Heading_20_1"), "Heading 1");
        assert_eq!(readable_name("snake_case_name"), "snake_case_name");
        assert_eq!(readable_name("trailing_"), "trailing_");
        assert_eq!(readable_name("A_2e_B"), "A.B");
    }
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

    /// `Some("none")` and `None` are different attributes and the same rendering.
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

    /// XSL-FO's quotes are not part of a family's name, and LibreOffice writes `'Liberation
    /// Serif'` for `Liberation Serif`, so keeping them would fail loop C over nothing.
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
        // Apostrophes XML-escaped in the attribute, as a Writer document has them.
        assert_eq!(
            quoted("Liberation Serif"),
            format!(" fo:font-family=\"{}\"", esc("'Liberation Serif'"))
        );
        assert_eq!(quoted("Georgia"), " fo:font-family=\"Georgia\"");
        assert_eq!(quoted(list), format!(" fo:font-family=\"{}\"", esc(list)));
    }

    /// Only the four metric properties reach the layout engine.
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
