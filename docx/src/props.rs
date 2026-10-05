// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Word's run and paragraph properties (`w:rPr`, `w:pPr`) as ODF's — **one translation, read
//! once**, the way `grind_xlsx::styles` turns a cell format into a `CellStyle`.
//!
//! The output is not a model type but ODF's own attribute vocabulary: `fo:font-weight` →
//! `bold`, `fo:margin-left` → `36pt`. That is deliberate and it is the shape of this whole
//! filter (`doc/docx-import.md`, "Why ODF and not the model"): an imported document is written
//! as ODF and then read by `grind_text`'s own reader, so a property the text model has no field
//! for — a superscript, a letter spacing, a paragraph border — still arrives in the file a save
//! writes and in what LibreOffice shows of it, instead of being lost at the model's edge.
//!
//! **Units.** Word measures in twentieths of a point (*twips*, §17.18.105's `ST_TwipsMeasure`),
//! half-points for a font size, eighths of a point for a border and English Metric Units for a
//! drawing. All of them divide into points exactly or nearly so, so every length written here is
//! in `pt` — `708` twips is `35.4pt`, not a rounded centimetre.

use std::collections::BTreeMap;

use crate::xml::{Attrs, Handled, Reader, Word as _, WordAttrs as _};

/// ODF style properties, keyed by qualified attribute name — `fo:font-weight` → `bold`.
///
/// Ordered so that a style written from it is the same bytes every time (an import of the same
/// file twice is byte-identical), and hashable so equal formattings pool into one automatic
/// style.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Props(pub BTreeMap<&'static str, String>);

impl Props {
    pub fn set(&mut self, key: &'static str, value: impl Into<String>) {
        self.0.insert(key, value.into());
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(String::as_str)
    }

    pub fn remove(&mut self, key: &str) {
        self.0.remove(key);
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Lay `over` on top: every property it states replaces this one's.
    pub fn layer(&mut self, over: &Props) {
        for (k, v) in &over.0 {
            self.0.insert(k, v.clone());
        }
    }

    /// The properties as XML attributes, each preceded by a space, escaped.
    pub fn attributes(&self) -> String {
        let mut out = String::new();
        for (k, v) in &self.0 {
            out.push_str(&format!(" {k}=\"{}\"", grind_core::odf::xml::esc(v)));
        }
        out
    }
}

/// A tab stop (`w:tab` inside `w:tabs`): where, which way, and what fills the gap.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Tab {
    /// In twips, from the paragraph's own indent — `doc/docx-format.md` §3.4.
    pub position: i64,
    /// ODF's `style:type`: `left`, `center`, `right`, `char`.
    pub kind: &'static str,
    /// `style:leader-text`, when the gap is filled.
    pub leader: Option<char>,
    /// `w:val="clear"` — removes a stop a style set, rather than adding one.
    pub clear: bool,
}

/// A paragraph's formatting: the ODF paragraph properties, the text properties the paragraph
/// mark's style carries (a paragraph *style* has both), and its tab stops.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ParaProps {
    pub para: Props,
    pub text: Props,
    /// `None` when nothing was said; an empty list is a style clearing every stop.
    pub tabs: Option<Vec<Tab>>,
}

impl ParaProps {
    pub fn is_empty(&self) -> bool {
        self.para.is_empty() && self.text.is_empty() && self.tabs.is_none()
    }

    pub fn layer(&mut self, over: &ParaProps) {
        self.para.layer(&over.para);
        self.text.layer(&over.text);
        if let Some(tabs) = &over.tabs {
            let mut merged = self.tabs.clone().unwrap_or_default();
            for tab in tabs {
                merged.retain(|t| t.position != tab.position);
                if !tab.clear {
                    merged.push(tab.clone());
                }
            }
            merged.sort();
            self.tabs = Some(merged);
        }
    }
}

/// What a `w:pPr` says beside its formatting: the structural facts a paragraph is read by.
#[derive(Clone, Debug, Default)]
pub struct ParaFacts {
    pub props: ParaProps,
    /// `w:pStyle` — a style *id*, resolved by `styles.rs`.
    pub style: Option<String>,
    /// `w:numPr`: `(numId, ilvl)`. A `numId` of `0` is *no numbering*, stated — it removes the
    /// numbering a style would have given the paragraph (§17.9.18).
    pub numbering: Option<(Option<i64>, Option<i64>)>,
    /// `w:outlineLvl`, 0-based; `9` is body text (§17.3.1.20).
    pub outline: Option<i64>,
    /// A `w:sectPr`: this paragraph is the last of a section, and this is that section's page.
    pub section: Option<Box<crate::section::Section>>,
    /// `w:framePr` — a paragraph positioned on the page, which is kept where it stands.
    pub frame: bool,
    /// `w:framePr/@w:dropCap`: the paragraph is the large first letter of the next one.
    pub drop_cap: bool,
}

/// What a `w:rPr` says: its formatting, and the character style it names.
#[derive(Clone, Debug, Default)]
pub struct RunFacts {
    pub props: Props,
    /// `w:rStyle` — a style id.
    pub style: Option<String>,
}

/// The theme's two font families, for `w:asciiTheme="minorHAnsi"` and friends.
#[derive(Clone, Debug, Default)]
pub struct Fonts {
    pub major: Option<String>,
    pub minor: Option<String>,
    pub major_east_asia: Option<String>,
    pub minor_east_asia: Option<String>,
}

/// A number of points, as ODF writes a length: up to four decimals, no trailing zeros.
pub fn pt(points: f64) -> String {
    let rounded = (points * 10_000.0).round() / 10_000.0;
    let mut s = format!("{rounded:.4}");
    while s.ends_with('0') {
        s.pop();
    }
    if s.ends_with('.') {
        s.pop();
    }
    if s == "-0" {
        s = "0".into();
    }
    format!("{s}pt")
}

/// Twentieths of a point.
pub fn twips(t: i64) -> String {
    pt(t as f64 / 20.0)
}

/// English Metric Units — 12,700 to the point (§20.1.2.1, `ST_Coordinate`).
pub fn emu(e: i64) -> String {
    pt(e as f64 / 12_700.0)
}

/// A colour Word spelled as six hex digits (`FF0000`) or `auto`; `#ff0000` or nothing.
pub fn color(hex: &str) -> Option<String> {
    let hex = hex.trim();
    (hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit()))
        .then(|| format!("#{}", hex.to_ascii_lowercase()))
}

/// `w:highlight`'s sixteen names (§17.18.40, `ST_HighlightColor`), as colours.
pub fn highlight(name: &str) -> Option<&'static str> {
    Some(match name {
        "black" => "#000000",
        "blue" => "#0000ff",
        "cyan" => "#00ffff",
        "green" => "#00ff00",
        "magenta" => "#ff00ff",
        "red" => "#ff0000",
        "yellow" => "#ffff00",
        "white" => "#ffffff",
        "darkBlue" => "#000080",
        "darkCyan" => "#008080",
        "darkGreen" => "#008000",
        "darkMagenta" => "#800080",
        "darkRed" => "#800000",
        "darkYellow" => "#808000",
        "darkGray" => "#808080",
        "lightGray" => "#c0c0c0",
        _ => return None,
    })
}

/// `w:shd` as one colour: the fill, unless the pattern is `solid`, which paints the *pattern*
/// colour over all of it (§17.3.5, `ST_Shd`). Any other pattern — stripes, a percentage — is
/// the fill and nothing more, which is what most of them nearly look like.
pub fn shading(attrs: &Attrs) -> Option<String> {
    match attrs.val() {
        Some("solid") => attrs.w("color").and_then(color),
        Some("nil") => None,
        _ => attrs.w("fill").and_then(color),
    }
}

/// A family name as `fo:font-family` wants it: quoted when it holds a space, as XSL-FO does
/// and as `grind_text::style` decodes.
pub fn family(name: &str) -> String {
    if name.contains(' ') && !name.starts_with('\'') {
        format!("'{name}'")
    } else {
        name.to_owned()
    }
}

/// One border (`w:top`, `w:left`, … inside `w:pBdr` or `w:tcBorders`) as ODF's three-part
/// `0.5pt solid #000000`, and its `w:space` in points. `None` for `nil` and `none`.
pub fn border(attrs: &Attrs) -> Option<(String, Option<f64>)> {
    let style = match attrs.val()? {
        "nil" | "none" => return None,
        "double" | "triple" | "thinThickSmallGap" | "thickThinSmallGap" | "thinThickMediumGap"
        | "thickThinMediumGap" | "thinThickLargeGap" | "thickThinLargeGap" => "double",
        "dotted" => "dotted",
        "dashed" | "dashSmallGap" | "dotDash" | "dotDotDash" => "dashed",
        _ => "solid",
    };
    // Eighths of a point, and Word draws nothing thinner than a quarter.
    let width = attrs
        .int("sz")
        .map_or(0.5, |sz| (sz as f64 / 8.0).max(0.25));
    let color = attrs
        .w("color")
        .and_then(color)
        .unwrap_or_else(|| "#000000".into());
    let space = attrs.int("space").map(|s| s as f64);
    Some((format!("{} {style} {color}", pt(width)), space))
}

/// Read a `w:rPr` the reader has just opened.
pub fn read_rpr(r: &mut Reader, fonts: &Fonts) -> grind_ooxml::Result<RunFacts> {
    let mut out = RunFacts::default();
    r.children(|_, name, attrs| {
        if name.w("rStyle") {
            out.style = attrs.val().map(str::to_owned);
        } else {
            run_property(&mut out.props, &name.local, attrs, fonts);
        }
        Ok(Handled::Yes)
    })?;
    Ok(out)
}

/// One child of a `w:rPr`, onto ODF text properties. Unknown ones are ignored, which is the
/// walker's tolerance applied one level down.
pub fn run_property(p: &mut Props, local: &str, attrs: &Attrs, fonts: &Fonts) {
    let on = attrs.on();
    match local {
        "b" => p.set("fo:font-weight", if on { "bold" } else { "normal" }),
        "bCs" => p.set(
            "style:font-weight-complex",
            if on { "bold" } else { "normal" },
        ),
        "i" => p.set("fo:font-style", if on { "italic" } else { "normal" }),
        "iCs" => p.set(
            "style:font-style-complex",
            if on { "italic" } else { "normal" },
        ),
        "u" => underline(p, attrs),
        "strike" | "dstrike" => {
            if on {
                p.set("style:text-line-through-style", "solid");
                if local == "dstrike" {
                    p.set("style:text-line-through-type", "double");
                }
            } else {
                p.set("style:text-line-through-style", "none");
            }
        }
        "color" => {
            if let Some(c) = attrs.val().and_then(color) {
                p.set("fo:color", c);
            }
        }
        "sz" => {
            if let Some(half) = attrs.int("val").filter(|h| *h > 0) {
                p.set("fo:font-size", pt(half as f64 / 2.0));
            }
        }
        "szCs" => {
            if let Some(half) = attrs.int("val").filter(|h| *h > 0) {
                p.set("style:font-size-complex", pt(half as f64 / 2.0));
            }
        }
        "rFonts" => {
            let theme = |key: &str| -> Option<String> {
                let which = attrs.w(key)?;
                if which.starts_with("major") {
                    if which.ends_with("EastAsia") {
                        fonts.major_east_asia.clone()
                    } else {
                        fonts.major.clone()
                    }
                } else if which.ends_with("EastAsia") {
                    fonts.minor_east_asia.clone()
                } else {
                    fonts.minor.clone()
                }
            };
            // A theme font wins over the literal name beside it (§17.3.2.26): the literal is a
            // cache of what the theme said when the file was written.
            let latin = theme("asciiTheme")
                .or_else(|| theme("hAnsiTheme"))
                .or_else(|| attrs.w("ascii").map(str::to_owned))
                .or_else(|| attrs.w("hAnsi").map(str::to_owned));
            if let Some(name) = latin.filter(|n| !n.is_empty()) {
                p.set("fo:font-family", family(&name));
            }
            let asian = theme("eastAsiaTheme").or_else(|| attrs.w("eastAsia").map(str::to_owned));
            if let Some(name) = asian.filter(|n| !n.is_empty()) {
                p.set("style:font-family-asian", family(&name));
            }
            let complex = theme("cstheme").or_else(|| attrs.w("cs").map(str::to_owned));
            if let Some(name) = complex.filter(|n| !n.is_empty()) {
                p.set("style:font-family-complex", family(&name));
            }
        }
        "highlight" => {
            if let Some(c) = attrs.val().and_then(highlight) {
                p.set("fo:background-color", c);
            } else if attrs.val() == Some("none") {
                p.set("fo:background-color", "transparent");
            }
        }
        "shd" => {
            // A highlight is the stronger statement; a shading only fills in where none is.
            if p.get("fo:background-color").is_none()
                && let Some(c) = shading(attrs)
            {
                p.set("fo:background-color", c);
            }
        }
        "vertAlign" => match attrs.val() {
            Some("superscript") => p.set("style:text-position", "super 58%"),
            Some("subscript") => p.set("style:text-position", "sub 58%"),
            Some("baseline") => p.set("style:text-position", "0% 100%"),
            _ => {}
        },
        "position" => {
            // Half-points up or down, at full size. ODF says it as a percentage of the font
            // size, which is not known here — so it is said in the one unit both share: the
            // raise as a share of a 12-point line is wrong for any other size, and a raise is
            // what the model could not hold at all. ponytail: resolve against the run's size.
            if let Some(half) = attrs.int("val").filter(|h| *h != 0) {
                let percent = (half as f64 / 2.0) / 12.0 * 100.0;
                p.set("style:text-position", format!("{}% 100%", percent.round()));
            }
        }
        "caps" => p.set("fo:text-transform", if on { "uppercase" } else { "none" }),
        "smallCaps" => p.set("fo:font-variant", if on { "small-caps" } else { "normal" }),
        "vanish" => {
            if on {
                p.set("text:display", "none");
            }
        }
        "spacing" => {
            if let Some(t) = attrs.int("val") {
                p.set("fo:letter-spacing", twips(t));
            }
        }
        "w" => {
            if let Some(scale) = attrs.int("val").filter(|s| *s > 0) {
                p.set("style:text-scale", format!("{scale}%"));
            }
        }
        "kern" => p.set("style:letter-kerning", if on { "true" } else { "false" }),
        "lang" => {
            if let Some((language, country)) = attrs.val().and_then(language) {
                p.set("fo:language", language);
                p.set("fo:country", country);
            }
        }
        "emboss" if on => p.set("style:font-relief", "embossed"),
        "imprint" if on => p.set("style:font-relief", "engraved"),
        "outline" => p.set("style:text-outline", if on { "true" } else { "false" }),
        "shadow" if on => p.set("fo:text-shadow", "1pt 1pt"),
        _ => {}
    }
}

/// `en-US` → `("en", "US")`. A bare `de` has no country, and ODF wants one or `none`.
fn language(tag: &str) -> Option<(String, String)> {
    let mut parts = tag.split(['-', '_']);
    let language = parts.next().filter(|l| (2..=3).contains(&l.len()))?;
    let country = parts
        .next()
        .filter(|c| c.len() == 2 && c.chars().all(|c| c.is_ascii_alphabetic()))
        .map_or("none".to_owned(), |c| c.to_ascii_uppercase());
    Some((language.to_ascii_lowercase(), country))
}

/// `w:u` (§17.18.99, `ST_Underline`) onto ODF's three underline attributes.
fn underline(p: &mut Props, attrs: &Attrs) {
    let (style, kind, bold) = match attrs.val().unwrap_or("single") {
        "none" => {
            p.set("style:text-underline-style", "none");
            return;
        }
        "double" => ("solid", Some("double"), false),
        "thick" => ("solid", None, true),
        "dotted" => ("dotted", None, false),
        "dottedHeavy" => ("dotted", None, true),
        "dash" => ("dash", None, false),
        "dashedHeavy" => ("dash", None, true),
        "dashLong" => ("long-dash", None, false),
        "dashLongHeavy" => ("long-dash", None, true),
        "dotDash" => ("dot-dash", None, false),
        "dashDotHeavy" => ("dot-dash", None, true),
        "dotDotDash" => ("dot-dot-dash", None, false),
        "dashDotDotHeavy" => ("dot-dot-dash", None, true),
        "wave" => ("wave", None, false),
        "wavyHeavy" => ("wave", None, true),
        "wavyDouble" => ("wave", Some("double"), false),
        // `single`, `words`, and anything a later Word adds.
        _ => ("solid", None, false),
    };
    p.set("style:text-underline-style", style);
    p.set(
        "style:text-underline-width",
        if bold { "bold" } else { "auto" },
    );
    p.set(
        "style:text-underline-color",
        attrs
            .w("color")
            .and_then(color)
            .unwrap_or_else(|| "font-color".into()),
    );
    if let Some(kind) = kind {
        p.set("style:text-underline-type", kind);
    }
    if attrs.val() == Some("words") {
        p.set("style:text-underline-mode", "skip-white-space");
    }
}

/// Read a `w:pPr` the reader has just opened.
pub fn read_ppr(r: &mut Reader) -> grind_ooxml::Result<ParaFacts> {
    let mut out = ParaFacts::default();
    r.children(|r, name, attrs| {
        if name.ns != grind_ooxml::names::Ns::Word {
            return Ok(Handled::No);
        }
        let p = &mut out.props.para;
        let on = attrs.on();
        match name.local.as_str() {
            "pStyle" => out.style = attrs.val().map(str::to_owned),
            "numPr" => {
                let mut num = None;
                let mut level = None;
                r.children(|_, name, attrs| {
                    if name.w("numId") {
                        num = attrs.int("val");
                    } else if name.w("ilvl") {
                        level = attrs.int("val");
                    }
                    Ok(Handled::Yes)
                })?;
                out.numbering = Some((num, level));
            }
            "outlineLvl" => out.outline = attrs.int("val"),
            "keepNext" => p.set("fo:keep-with-next", if on { "always" } else { "auto" }),
            "keepLines" => p.set("fo:keep-together", if on { "always" } else { "auto" }),
            "pageBreakBefore" => {
                if on {
                    p.set("fo:break-before", "page");
                }
            }
            "widowControl" => {
                let lines = if on { "2" } else { "0" };
                p.set("fo:widows", lines);
                p.set("fo:orphans", lines);
            }
            "jc" => {
                if let Some(align) = attrs.val().and_then(alignment) {
                    p.set("fo:text-align", align);
                }
            }
            "ind" => indent(p, attrs),
            "spacing" => spacing(p, attrs),
            "contextualSpacing" => p.set(
                "style:contextual-spacing",
                if on { "true" } else { "false" },
            ),
            "shd" => {
                if let Some(c) = shading(attrs) {
                    p.set("fo:background-color", c);
                }
            }
            "pBdr" => {
                r.children(|_, name, attrs| {
                    let side = match name.local.as_str() {
                        "top" => ("fo:border-top", "fo:padding-top"),
                        "bottom" => ("fo:border-bottom", "fo:padding-bottom"),
                        "left" | "start" => ("fo:border-left", "fo:padding-left"),
                        "right" | "end" => ("fo:border-right", "fo:padding-right"),
                        _ => return Ok(Handled::No),
                    };
                    if let Some((line, space)) = border(attrs) {
                        p.set(side.0, line);
                        if let Some(space) = space {
                            p.set(side.1, pt(space));
                        }
                    }
                    Ok(Handled::Yes)
                })?;
            }
            "tabs" => {
                let mut tabs = Vec::new();
                r.children(|_, name, attrs| {
                    if name.w("tab")
                        && let Some(position) = attrs.int("pos")
                    {
                        let val = attrs.val().unwrap_or("left");
                        tabs.push(Tab {
                            position,
                            kind: match val {
                                "center" => "center",
                                "right" | "end" => "right",
                                "decimal" => "char",
                                _ => "left",
                            },
                            leader: match attrs.w("leader") {
                                Some("dot") => Some('.'),
                                Some("hyphen") => Some('-'),
                                Some("underscore" | "heavy") => Some('_'),
                                Some("middleDot") => Some('·'),
                                _ => None,
                            },
                            clear: val == "clear",
                        });
                    }
                    Ok(Handled::Yes)
                })?;
                out.props.tabs = Some(tabs);
            }
            "bidi" => {
                if on {
                    p.set("style:writing-mode", "rl-tb");
                }
            }
            "suppressAutoHyphens" => {
                if on {
                    out.props.text.set("fo:hyphenate", "false");
                }
            }
            "sectPr" => out.section = Some(Box::new(crate::section::read(r)?)),
            "framePr" => {
                out.frame = true;
                out.drop_cap = matches!(attrs.w("dropCap"), Some("drop" | "margin"));
                return Ok(Handled::No);
            }
            // The paragraph *mark*'s run properties: how the pilcrow is formatted, which is
            // invisible, and — in a style — the text properties of the whole style, which is
            // `styles.rs`'s to read rather than this function's.
            "rPr" => return Ok(Handled::No),
            _ => return Ok(Handled::No),
        }
        Ok(Handled::Yes)
    })?;
    Ok(out)
}

/// `w:jc` (§17.18.44, `ST_Jc`). `start`/`end` are the 2nd edition's spellings of `left`/`right`,
/// and LibreOffice writes them (`doc/docx-format.md` §3.1).
pub fn alignment(jc: &str) -> Option<&'static str> {
    Some(match jc {
        "left" | "start" => "start",
        "right" | "end" => "end",
        "center" => "center",
        "both" | "distribute" | "thaiDistribute" | "lowKashida" | "mediumKashida"
        | "highKashida" => "justify",
        _ => return None,
    })
}

/// `w:ind` — the left and right indents and the first line's, in twips. `w:start`/`w:end` are
/// the same as `w:left`/`w:right` (`doc/docx-format.md` §3.1), and a `w:hanging` is a negative
/// first-line indent.
pub fn indent(p: &mut Props, attrs: &Attrs) {
    if let Some(left) = attrs.int("left").or_else(|| attrs.int("start")) {
        p.set("fo:margin-left", twips(left));
    }
    if let Some(right) = attrs.int("right").or_else(|| attrs.int("end")) {
        p.set("fo:margin-right", twips(right));
    }
    if let Some(hanging) = attrs.int("hanging") {
        p.set("fo:text-indent", twips(-hanging));
    } else if let Some(first) = attrs.int("firstLine") {
        p.set("fo:text-indent", twips(first));
    }
}

/// `w:spacing` on a paragraph: space before and after in twips, and the line (§17.3.1.33).
fn spacing(p: &mut Props, attrs: &Attrs) {
    if let Some(before) = attrs.int("before") {
        p.set("fo:margin-top", twips(before));
    }
    if let Some(after) = attrs.int("after") {
        p.set("fo:margin-bottom", twips(after));
    }
    if let Some(line) = attrs.int("line") {
        match attrs.w("lineRule").unwrap_or("auto") {
            // In 240ths of a line.
            "auto" => p.set(
                "fo:line-height",
                format!("{}%", (line as f64 / 240.0 * 100.0).round()),
            ),
            "exact" => p.set("fo:line-height", twips(line)),
            _ => p.set("style:line-height-at-least", twips(line)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: &str = grind_ooxml::names::WORD_T;

    fn rpr(inner: &str) -> RunFacts {
        let xml = format!(r#"<w:rPr xmlns:w="{W}">{inner}</w:rPr>"#);
        let mut reader = Reader::new(xml.as_bytes());
        reader.root().unwrap();
        read_rpr(&mut reader, &Fonts::default()).unwrap()
    }

    fn ppr(inner: &str) -> ParaFacts {
        let xml = format!(r#"<w:pPr xmlns:w="{W}">{inner}</w:pPr>"#);
        let mut reader = Reader::new(xml.as_bytes());
        reader.root().unwrap();
        read_ppr(&mut reader).unwrap()
    }

    #[test]
    fn lengths_are_points_exactly() {
        assert_eq!(twips(708), "35.4pt");
        assert_eq!(twips(1440), "72pt");
        assert_eq!(twips(-360), "-18pt");
        assert_eq!(emu(914_400), "72pt");
        assert_eq!(pt(0.0), "0pt");
        assert_eq!(pt(1.0 / 3.0), "0.3333pt");
    }

    #[test]
    fn the_four_everyday_run_properties() {
        let got = rpr(r#"<w:b/><w:i/><w:sz w:val="28"/><w:color w:val="1F4E79"/>"#).props;
        assert_eq!(got.get("fo:font-weight"), Some("bold"));
        assert_eq!(got.get("fo:font-style"), Some("italic"));
        assert_eq!(got.get("fo:font-size"), Some("14pt"));
        assert_eq!(got.get("fo:color"), Some("#1f4e79"));
    }

    #[test]
    fn a_toggle_turned_off_says_so() {
        let got = rpr(r#"<w:b w:val="0"/><w:i w:val="false"/>"#).props;
        assert_eq!(got.get("fo:font-weight"), Some("normal"));
        assert_eq!(got.get("fo:font-style"), Some("normal"));
    }

    #[test]
    fn automatic_colour_is_no_colour() {
        assert!(rpr(r#"<w:color w:val="auto"/>"#).props.is_empty());
    }

    #[test]
    fn a_highlight_beats_a_shading() {
        let got = rpr(r#"<w:shd w:val="clear" w:fill="00FF00"/><w:highlight w:val="yellow"/>"#);
        assert_eq!(got.props.get("fo:background-color"), Some("#ffff00"));
        let got = rpr(r#"<w:highlight w:val="yellow"/><w:shd w:val="clear" w:fill="00FF00"/>"#);
        assert_eq!(got.props.get("fo:background-color"), Some("#ffff00"));
    }

    #[test]
    fn underlines_and_strikes() {
        let got = rpr(r#"<w:u w:val="double"/><w:dstrike/>"#).props;
        assert_eq!(got.get("style:text-underline-style"), Some("solid"));
        assert_eq!(got.get("style:text-underline-type"), Some("double"));
        assert_eq!(got.get("style:text-line-through-type"), Some("double"));
        let got = rpr(r#"<w:u w:val="none"/>"#).props;
        assert_eq!(got.get("style:text-underline-style"), Some("none"));
    }

    #[test]
    fn a_superscript_has_somewhere_to_go() {
        let got = rpr(r#"<w:vertAlign w:val="superscript"/>"#).props;
        assert_eq!(got.get("style:text-position"), Some("super 58%"));
    }

    #[test]
    fn a_character_style_is_named_not_resolved() {
        assert_eq!(
            rpr(r#"<w:rStyle w:val="Strong"/>"#).style.as_deref(),
            Some("Strong")
        );
    }

    #[test]
    fn fonts_take_the_theme_over_the_cached_name() {
        let xml = format!(
            r#"<w:rPr xmlns:w="{W}"><w:rFonts w:ascii="Calibri" w:asciiTheme="majorHAnsi"/></w:rPr>"#
        );
        let mut reader = Reader::new(xml.as_bytes());
        reader.root().unwrap();
        let fonts = Fonts {
            major: Some("Calibri Light".into()),
            ..Fonts::default()
        };
        let got = read_rpr(&mut reader, &fonts).unwrap().props;
        assert_eq!(got.get("fo:font-family"), Some("'Calibri Light'"));
    }

    #[test]
    fn a_language_tag_splits_into_odfs_two_attributes() {
        let got = rpr(r#"<w:lang w:val="de-DE"/>"#).props;
        assert_eq!(got.get("fo:language"), Some("de"));
        assert_eq!(got.get("fo:country"), Some("DE"));
        assert_eq!(language("de"), Some(("de".into(), "none".into())));
    }

    #[test]
    fn paragraph_spacing_indent_and_alignment() {
        let got = ppr(
            r#"<w:pStyle w:val="Heading1"/><w:spacing w:before="240" w:after="120" w:line="360" w:lineRule="auto"/>
               <w:ind w:left="720" w:hanging="360"/><w:jc w:val="both"/><w:keepNext/>"#,
        );
        assert_eq!(got.style.as_deref(), Some("Heading1"));
        let p = &got.props.para;
        assert_eq!(p.get("fo:margin-top"), Some("12pt"));
        assert_eq!(p.get("fo:margin-bottom"), Some("6pt"));
        assert_eq!(p.get("fo:line-height"), Some("150%"));
        assert_eq!(p.get("fo:margin-left"), Some("36pt"));
        assert_eq!(p.get("fo:text-indent"), Some("-18pt"));
        assert_eq!(p.get("fo:text-align"), Some("justify"));
        assert_eq!(p.get("fo:keep-with-next"), Some("always"));
    }

    /// LibreOffice writes the 2nd edition's `start`/`end` (measured, `doc/docx-format.md` §3.1).
    #[test]
    fn start_and_end_are_left_and_right() {
        let got = ppr(r#"<w:ind w:start="567" w:end="0" w:hanging="0"/><w:jc w:val="end"/>"#);
        assert_eq!(got.props.para.get("fo:margin-left"), Some("28.35pt"));
        assert_eq!(got.props.para.get("fo:text-align"), Some("end"));
    }

    #[test]
    fn numbering_and_outline_are_facts_not_formatting() {
        let got = ppr(
            r#"<w:numPr><w:ilvl w:val="1"/><w:numId w:val="3"/></w:numPr><w:outlineLvl w:val="0"/>"#,
        );
        assert_eq!(got.numbering, Some((Some(3), Some(1))));
        assert_eq!(got.outline, Some(0));
        assert!(got.props.is_empty());
    }

    #[test]
    fn a_paragraph_border_is_odfs_three_part_line() {
        let got = ppr(
            r#"<w:pBdr><w:bottom w:val="single" w:sz="6" w:space="1" w:color="auto"/></w:pBdr>"#,
        );
        assert_eq!(
            got.props.para.get("fo:border-bottom"),
            Some("0.75pt solid #000000")
        );
        assert_eq!(got.props.para.get("fo:padding-bottom"), Some("1pt"));
    }

    #[test]
    fn tab_stops_layer_and_clear() {
        let base = ppr(
            r#"<w:tabs><w:tab w:val="left" w:pos="720"/><w:tab w:val="right" w:leader="dot" w:pos="9000"/></w:tabs>"#,
        );
        let over = ppr(
            r#"<w:tabs><w:tab w:val="clear" w:pos="720"/><w:tab w:val="center" w:pos="4500"/></w:tabs>"#,
        );
        let mut props = base.props.clone();
        props.layer(&over.props);
        let tabs = props.tabs.unwrap();
        assert_eq!(
            tabs.iter()
                .map(|t| (t.position, t.kind))
                .collect::<Vec<_>>(),
            [(4500, "center"), (9000, "right")]
        );
        assert_eq!(tabs[1].leader, Some('.'));
    }
}
