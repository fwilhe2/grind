// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `word/styles.xml` — the document's named styles and its defaults (§17.7), and the theme's
//! two fonts (`word/theme/theme1.xml`, §20.1.4.1.18) that a style may name instead of a family.
//!
//! A Word style becomes an ODF **named** style of the same family — paragraph styles
//! `style:family="paragraph"`, character styles `text` — with `w:basedOn` as
//! `style:parent-style-name`. That is ODF's own inheritance (rng:12081), so nothing is
//! flattened: a document whose headings are all `Heading 1` arrives with a `Heading 1` the reader
//! resolves, a person can see, and LibreOffice lists. `w:docDefaults` is the
//! `style:default-style` under every chain, as it is under every chain in Word.
//!
//! Table styles are read for what a table *built* from one needs (its borders, cell margins and
//! fill), since ODF has no table-style family a text table can name; `tables.rs` applies them.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::props::{self, Fonts, ParaProps, Props};
use crate::xml::{Handled, Reader, Word as _, WordAttrs as _};

/// Which kind of style (`w:type`, §17.18.83).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Paragraph,
    Character,
    Table,
    Numbering,
}

/// What a table style says about a table built from it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TableLook {
    /// `w:tblBorders`: `top`, `left`, `bottom`, `right`, `insideH`, `insideV`, as ODF lines.
    pub borders: BTreeMap<&'static str, Option<String>>,
    /// `w:tblCellMar`, in twips: `top`, `left`, `bottom`, `right`.
    pub margins: BTreeMap<&'static str, i64>,
    /// The cells' fill, `w:tcPr/w:shd`.
    pub fill: Option<String>,
    /// Whether the style has conditional parts (`w:tblStylePr` — banded rows, a first row in
    /// bold), which a cell cannot carry and `tables.rs` counts.
    pub conditional: bool,
}

#[derive(Clone, Debug)]
pub struct Style {
    pub id: String,
    pub kind: Kind,
    /// `w:name`, as Word spells it — `heading 1`, `Normal`.
    pub name: Option<String>,
    pub based_on: Option<String>,
    pub next: Option<String>,
    /// The paragraph and text properties it states itself, not inherited.
    pub props: ParaProps,
    pub numbering: Option<(Option<i64>, Option<i64>)>,
    pub outline: Option<i64>,
    pub table: TableLook,
}

/// Every style in the document, and the defaults under them.
#[derive(Clone, Debug, Default)]
pub struct Styles {
    pub by_id: HashMap<String, Style>,
    /// The ids in the order the file declares them, so the output is too.
    pub order: Vec<String>,
    /// `w:docDefaults`, plus what Word assumes where even those say nothing.
    pub defaults: ParaProps,
    /// The paragraph style a paragraph naming none is in (`w:default="1"`, usually `Normal`).
    pub default_paragraph: Option<String>,
    /// The table style a table naming none is built from.
    pub default_table: Option<String>,
    /// Each style's ODF name, by id — unique, and an XML NCName.
    pub odf_names: HashMap<String, String>,
}

impl Styles {
    pub fn get(&self, id: &str) -> Option<&Style> {
        self.by_id.get(id)
    }

    /// The chain from `id` up through `w:basedOn`, nearest first, stopping at a cycle — a
    /// style based on itself is a malformed file that must still load.
    pub fn chain(&self, id: &str) -> Vec<&Style> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        let mut at = Some(id);
        while let Some(id) = at {
            if !seen.insert(id) {
                break;
            }
            let Some(style) = self.by_id.get(id) else {
                break;
            };
            out.push(style);
            at = style.based_on.as_deref();
        }
        out
    }

    /// The outline level a paragraph in this style has, 0-based, if it is a heading: stated
    /// (`w:outlineLvl`) somewhere up the chain. Level 9 is "body text" and no heading.
    ///
    /// A style *named* `heading 1` that states no level is not a heading — measured against the
    /// oracle (`doc/docx-format.md` §3.2); Word itself writes the level into every heading
    /// style it saves, so the name is never needed for a document Word wrote.
    pub fn outline(&self, id: &str) -> Option<i64> {
        self.chain(id)
            .into_iter()
            .find_map(|style| style.outline)
            .filter(|level| (0..9).contains(level))
    }

    /// The numbering a paragraph in this style has, `(numId, ilvl)`, if any style up the
    /// chain gives one.
    pub fn numbering(&self, id: &str) -> Option<(Option<i64>, Option<i64>)> {
        self.chain(id).into_iter().find_map(|s| s.numbering)
    }

    /// A table style's look, its own over what it is based on.
    pub fn table_look(&self, id: &str) -> TableLook {
        let mut look = TableLook::default();
        for style in self.chain(id).into_iter().rev() {
            for (k, v) in &style.table.borders {
                look.borders.insert(k, v.clone());
            }
            for (k, v) in &style.table.margins {
                look.margins.insert(k, *v);
            }
            if style.table.fill.is_some() {
                look.fill.clone_from(&style.table.fill);
            }
            look.conditional |= style.table.conditional;
        }
        look
    }

    /// The ODF name of a paragraph or character style, by id.
    pub fn odf_name(&self, id: &str) -> Option<&str> {
        self.odf_names.get(id).map(String::as_str)
    }
}

/// The name a style is shown by: Word's own, with the first letter capitalised — Word stores
/// its built-in styles lower-case (`heading 1`, `toc 1`) and shows them capitalised, and it is
/// the shown name a person searches for.
pub fn display_name(style: &Style) -> String {
    let name = style.name.clone().unwrap_or_else(|| style.id.clone());
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => style.id.clone(),
    }
}

/// An ODF style name for a display name: an NCName, with every character that may not be in one
/// written as `_xx_` in hex — the spelling LibreOffice uses (`Heading 1` → `Heading_20_1`), so a
/// style imported here and one LibreOffice wrote have the same name.
pub fn encode(display: &str) -> String {
    let mut out = String::new();
    for (i, c) in display.chars().enumerate() {
        let ok = c.is_ascii_alphabetic()
            || (i > 0 && (c.is_ascii_digit() || c == '-' || c == '.'))
            || (c != '_' && !c.is_ascii() && c.is_alphanumeric());
        if ok {
            out.push(c);
        } else {
            out.push_str(&format!("_{:x}_", c as u32));
        }
    }
    if out.is_empty() { "_".into() } else { out }
}

/// Read `word/styles.xml`.
pub fn read(bytes: &[u8], fonts: &Fonts) -> Styles {
    let mut styles = Styles::default();
    let mut reader = Reader::new(bytes);
    if !matches!(reader.root(), Ok(Some(_))) {
        styles.defaults = implicit_defaults(ParaProps::default());
        return styles;
    }
    let mut defaults = ParaProps::default();
    let _ = reader.children(|r, name, attrs| {
        if name.w("docDefaults") {
            r.children(|r, name, _| {
                if name.w("rPrDefault") || name.w("pPrDefault") {
                    r.children(|r, name, _| {
                        if name.w("rPr") {
                            defaults.text.layer(&props::read_rpr(r, fonts)?.props);
                        } else if name.w("pPr") {
                            let facts = props::read_ppr(r)?;
                            defaults.layer(&facts.props);
                        } else {
                            return Ok(Handled::No);
                        }
                        Ok(Handled::Yes)
                    })?;
                    Ok(Handled::Yes)
                } else {
                    Ok(Handled::No)
                }
            })?;
            return Ok(Handled::Yes);
        }
        if !name.w("style") {
            return Ok(Handled::No);
        }
        let kind = match attrs.w("type") {
            Some("character") => Kind::Character,
            Some("table") => Kind::Table,
            Some("numbering") => Kind::Numbering,
            _ => Kind::Paragraph,
        };
        let Some(id) = attrs.w("styleId").map(str::to_owned) else {
            return Ok(Handled::No);
        };
        let default = attrs
            .w("default")
            .is_some_and(|d| matches!(d, "1" | "true" | "on"));
        let mut style = Style {
            id: id.clone(),
            kind,
            name: None,
            based_on: None,
            next: None,
            props: ParaProps::default(),
            numbering: None,
            outline: None,
            table: TableLook::default(),
        };
        r.children(|r, name, attrs| {
            match name.local.as_str() {
                _ if name.ns != grind_ooxml::names::Ns::Word => return Ok(Handled::No),
                "name" => style.name = attrs.val().map(str::to_owned),
                "basedOn" => style.based_on = attrs.val().map(str::to_owned),
                "next" => style.next = attrs.val().map(str::to_owned),
                "rPr" => style.props.text.layer(&props::read_rpr(r, fonts)?.props),
                "pPr" => {
                    let facts = props::read_ppr(r)?;
                    style.props.layer(&facts.props);
                    style.numbering = facts.numbering;
                    style.outline = facts.outline;
                }
                "tblPr" => read_table_look(r, &mut style.table)?,
                "tcPr" => {
                    r.children(|_, name, attrs| {
                        if name.w("shd") {
                            style.table.fill = props::shading(attrs);
                        }
                        Ok(Handled::Yes)
                    })?;
                }
                "tblStylePr" => style.table.conditional = true,
                _ => return Ok(Handled::No),
            }
            Ok(Handled::Yes)
        })?;
        if default {
            match kind {
                Kind::Paragraph => styles.default_paragraph = Some(id.clone()),
                Kind::Table => styles.default_table = Some(id.clone()),
                _ => {}
            }
        }
        if !styles.by_id.contains_key(&id) {
            styles.order.push(id.clone());
        }
        styles.by_id.insert(id, style);
        Ok(Handled::Yes)
    });
    styles.defaults = implicit_defaults(defaults);
    name_styles(&mut styles);
    styles
}

/// `w:tblPr`'s borders and cell margins.
pub fn read_table_look(r: &mut Reader, look: &mut TableLook) -> grind_ooxml::Result<()> {
    r.children(|r, name, _| {
        Ok(if table_look_child(r, name, look)? {
            Handled::Yes
        } else {
            Handled::No
        })
    })
}

/// One child of a `w:tblPr` that has just been opened, if it is `w:tblBorders` or
/// `w:tblCellMar` — whether it was. Shared by a table style's `w:tblPr` and a table's own.
pub fn table_look_child(
    r: &mut Reader,
    name: &crate::xml::Name,
    look: &mut TableLook,
) -> grind_ooxml::Result<bool> {
    if name.w("tblBorders") {
        r.children(|_, name, attrs| {
            let side = match name.local.as_str() {
                "top" => "top",
                "bottom" => "bottom",
                "left" | "start" => "left",
                "right" | "end" => "right",
                "insideH" => "insideH",
                "insideV" => "insideV",
                _ => return Ok(Handled::No),
            };
            look.borders
                .insert(side, props::border(attrs).map(|(line, _)| line));
            Ok(Handled::Yes)
        })?;
        Ok(true)
    } else if name.w("tblCellMar") {
        r.children(|_, name, attrs| {
            let side = match name.local.as_str() {
                "top" => "top",
                "bottom" => "bottom",
                "left" | "start" => "left",
                "right" | "end" => "right",
                _ => return Ok(Handled::No),
            };
            // `w:type="dxa"` is twips; the other types (`nil`, `pct`) say nothing usable for a
            // margin.
            if matches!(attrs.w("type"), None | Some("dxa"))
                && let Some(w) = attrs.int("w")
            {
                look.margins.insert(side, w);
            }
            Ok(Handled::Yes)
        })?;
        Ok(true)
    } else {
        Ok(false)
    }
}

/// What is assumed when even `w:docDefaults` is silent: 11-point Calibri, the oracle's answer for
/// a document with no styles part at all (measured, `doc/docx-format.md` §3.3). ODF's own
/// defaults are a reader's choice (LibreOffice's is 12 points), so they are stated rather than
/// left to it.
fn implicit_defaults(mut defaults: ParaProps) -> ParaProps {
    if defaults.text.get("fo:font-size").is_none() {
        defaults.text.set("fo:font-size", "11pt");
    }
    if defaults.text.get("fo:font-family").is_none() {
        defaults.text.set("fo:font-family", "Calibri");
    }
    defaults
}

/// Give every paragraph and character style a unique ODF name.
fn name_styles(styles: &mut Styles) {
    let mut taken: HashSet<String> = HashSet::new();
    for id in &styles.order {
        let style = &styles.by_id[id];
        if !matches!(style.kind, Kind::Paragraph | Kind::Character) {
            continue;
        }
        let base = encode(&display_name(style));
        let mut name = base.clone();
        let mut n = 2;
        // A paragraph style and a character style may share a display name in Word (they live
        // in different families) — and may in ODF, but one name per id keeps every lookup
        // simple, and the shown name is still the same.
        while !taken.insert(name.to_ascii_lowercase()) {
            name = format!("{base}_{n}");
            n += 1;
        }
        styles.odf_names.insert(id.clone(), name);
    }
}

/// Read the theme's major and minor fonts.
pub fn read_fonts(bytes: &[u8]) -> Fonts {
    use grind_ooxml::names::Ns;
    let mut fonts = Fonts::default();
    let mut reader = Reader::new(bytes);
    if !matches!(reader.root(), Ok(Some(_))) {
        return fonts;
    }
    fn walk(r: &mut Reader, fonts: &mut Fonts, major: Option<bool>) -> grind_ooxml::Result<()> {
        r.children(|r, name, attrs| {
            if name.ns != Ns::Drawing {
                return Ok(Handled::No);
            }
            match name.local.as_str() {
                "majorFont" => walk(r, fonts, Some(true))?,
                "minorFont" => walk(r, fonts, Some(false))?,
                "latin" | "ea" => {
                    let typeface = attrs.plain("typeface").filter(|t| !t.is_empty());
                    let slot = match (major, name.local.as_str()) {
                        (Some(true), "latin") => &mut fonts.major,
                        (Some(false), "latin") => &mut fonts.minor,
                        (Some(true), _) => &mut fonts.major_east_asia,
                        (Some(false), _) => &mut fonts.minor_east_asia,
                        (None, _) => return Ok(Handled::No),
                    };
                    *slot = typeface.map(str::to_owned);
                }
                _ => walk(r, fonts, major)?,
            }
            Ok(Handled::Yes)
        })
    }
    let _ = walk(&mut reader, &mut fonts, None);
    fonts
}

/// `word/fontTable.xml`: each font's name and, where Word says one, its ODF generic family —
/// `w:family` (§17.8.3.10) is `roman`, `swiss`, `modern`, `script`, `decorative` or `auto`, and
/// the first five are ODF's own words for the same classes (rng:10418). What a printer falls back
/// on when the family is not installed (`grind_text::Document::font_generics`).
pub fn read_font_table(bytes: &[u8]) -> Vec<(String, &'static str)> {
    let mut out = Vec::new();
    let mut reader = Reader::new(bytes);
    if !matches!(reader.root(), Ok(Some(_))) {
        return out;
    }
    let _ = reader.children(|r, name, attrs| {
        if !name.w("font") {
            return Ok(Handled::No);
        }
        let Some(font) = attrs.w("name").filter(|n| !n.is_empty()).map(str::to_owned) else {
            return Ok(Handled::No);
        };
        let mut generic = None;
        r.children(|_, name, attrs| {
            if name.w("family") {
                generic = match attrs.val() {
                    Some("roman") => Some("roman"),
                    Some("swiss") => Some("swiss"),
                    Some("modern") => Some("modern"),
                    Some("script") => Some("script"),
                    Some("decorative") => Some("decorative"),
                    _ => None,
                };
            }
            Ok(Handled::Yes)
        })?;
        if let Some(generic) = generic
            && out.len() < 1024
        {
            out.push((font, generic));
        }
        Ok(Handled::Yes)
    });
    out
}

/// Paragraph and text properties a style resolves to through its whole chain — used for a
/// table style, which ODF cannot name, and for nothing else: every other style stays a name.
pub fn resolved(styles: &Styles, id: &str) -> ParaProps {
    let mut out = ParaProps::default();
    for style in styles.chain(id).into_iter().rev() {
        out.layer(&style.props);
    }
    out
}

/// The text properties a character style chain sets — used for a hyperlink run, which names one.
pub fn text_props(styles: &Styles, id: &str) -> Props {
    resolved(styles, id).text
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: &str = grind_ooxml::names::WORD_T;

    fn styles(inner: &str) -> Styles {
        read(
            format!(r#"<w:styles xmlns:w="{W}">{inner}</w:styles>"#).as_bytes(),
            &Fonts::default(),
        )
    }

    #[test]
    fn a_style_reads_its_name_parent_and_properties() {
        let s = styles(
            r#"<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/>
                 <w:pPr><w:spacing w:after="160"/></w:pPr><w:rPr><w:sz w:val="22"/></w:rPr></w:style>
               <w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/>
                 <w:basedOn w:val="Normal"/><w:next w:val="Normal"/>
                 <w:pPr><w:keepNext/><w:outlineLvl w:val="0"/></w:pPr><w:rPr><w:b/></w:rPr></w:style>"#,
        );
        assert_eq!(s.default_paragraph.as_deref(), Some("Normal"));
        let h = s.get("Heading1").unwrap();
        assert_eq!(h.based_on.as_deref(), Some("Normal"));
        assert_eq!(h.props.text.get("fo:font-weight"), Some("bold"));
        assert_eq!(s.outline("Heading1"), Some(0));
        assert_eq!(s.outline("Normal"), None);
        assert_eq!(s.odf_name("Heading1"), Some("Heading_20_1"));
        assert_eq!(s.odf_name("Normal"), Some("Normal"));
    }

    /// A style called `heading 2` that says no level is not a heading (`doc/docx-format.md`
    /// §3.2); a level is inherited, and level 9 is body text.
    #[test]
    fn a_heading_is_a_stated_level_not_a_name() {
        let s = styles(
            r#"<w:style w:type="paragraph" w:styleId="berschrift2"><w:name w:val="heading 2"/></w:style>
               <w:style w:type="paragraph" w:styleId="H"><w:pPr><w:outlineLvl w:val="1"/></w:pPr></w:style>
               <w:style w:type="paragraph" w:styleId="Mine"><w:basedOn w:val="H"/></w:style>
               <w:style w:type="paragraph" w:styleId="Body"><w:pPr><w:outlineLvl w:val="9"/></w:pPr></w:style>"#,
        );
        assert_eq!(s.outline("berschrift2"), None);
        assert_eq!(s.outline("Mine"), Some(1), "inherited");
        assert_eq!(s.outline("Body"), None, "level 9 is body text");
    }

    #[test]
    fn a_cycle_of_based_on_still_loads() {
        let s = styles(
            r#"<w:style w:type="paragraph" w:styleId="A"><w:basedOn w:val="B"/></w:style>
               <w:style w:type="paragraph" w:styleId="B"><w:basedOn w:val="A"/></w:style>"#,
        );
        assert_eq!(s.chain("A").len(), 2);
        assert_eq!(s.outline("A"), None);
    }

    #[test]
    fn defaults_are_read_and_what_word_assumes_is_stated() {
        let s = styles(
            r#"<w:docDefaults><w:rPrDefault><w:rPr><w:sz w:val="24"/></w:rPr></w:rPrDefault>
               <w:pPrDefault><w:pPr><w:spacing w:after="200"/></w:pPr></w:pPrDefault></w:docDefaults>"#,
        );
        assert_eq!(s.defaults.text.get("fo:font-size"), Some("12pt"));
        assert_eq!(s.defaults.text.get("fo:font-family"), Some("Calibri"));
        assert_eq!(s.defaults.para.get("fo:margin-bottom"), Some("10pt"));
    }

    #[test]
    fn names_are_ncnames_and_unique() {
        assert_eq!(encode("Heading 1"), "Heading_20_1");
        assert_eq!(encode("1st"), "_31_st");
        assert_eq!(encode("Überschrift"), "Überschrift");
        let s = styles(
            r#"<w:style w:type="paragraph" w:styleId="a"><w:name w:val="Quote"/></w:style>
               <w:style w:type="character" w:styleId="b"><w:name w:val="Quote"/></w:style>"#,
        );
        assert_eq!(s.odf_name("a"), Some("Quote"));
        assert_eq!(s.odf_name("b"), Some("Quote_2"));
    }

    #[test]
    fn a_table_style_inherits_its_borders() {
        let s = styles(
            r#"<w:style w:type="table" w:styleId="Base"><w:tblPr><w:tblBorders>
                 <w:top w:val="single" w:sz="4"/><w:insideH w:val="single" w:sz="4"/></w:tblBorders>
                 <w:tblCellMar><w:left w:w="108" w:type="dxa"/></w:tblCellMar></w:tblPr></w:style>
               <w:style w:type="table" w:styleId="Grid"><w:basedOn w:val="Base"/><w:tblPr><w:tblBorders>
                 <w:top w:val="nil"/></w:tblBorders></w:tblPr></w:style>"#,
        );
        let look = s.table_look("Grid");
        assert_eq!(look.borders["top"], None, "the nearer style's nil wins");
        assert_eq!(
            look.borders["insideH"].as_deref(),
            Some("0.5pt solid #000000")
        );
        assert_eq!(look.margins["left"], 108);
    }

    #[test]
    fn the_themes_fonts() {
        let a = grind_ooxml::names::DRAWING_T;
        let xml = format!(
            r#"<a:theme xmlns:a="{a}"><a:themeElements><a:fontScheme name="Office">
                 <a:majorFont><a:latin typeface="Calibri Light"/><a:ea typeface=""/></a:majorFont>
                 <a:minorFont><a:latin typeface="Calibri"/></a:minorFont>
               </a:fontScheme></a:themeElements></a:theme>"#
        );
        let fonts = read_fonts(xml.as_bytes());
        assert_eq!(fonts.major.as_deref(), Some("Calibri Light"));
        assert_eq!(fonts.minor.as_deref(), Some("Calibri"));
        assert_eq!(fonts.major_east_asia, None);
    }
}
