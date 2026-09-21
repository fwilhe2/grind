// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `xl/styles.xml` — every cell format, translated once: its **number format** (X3) and its
//! **look** (X4), which is a `grind_sheet::style::CellStyle` plus whatever that could not hold.
//!
//! A cell's `s="12"` indexes `<cellXfs>`, whose twelfth `<xf>` names a `numFmtId`, a `fontId`,
//! a `fillId` and a `borderId`, and may carry an `<alignment>`. Styles are read **before**
//! sheets — the same order `odf/read.rs` reads `styles.xml` before `content.xml` — because a
//! serial cannot be corrected, or given its date kind, until its format is known. Each `<xf>`
//! is translated **once** and every cell that names it clones the result.
//!
//! **What a cell format means, decided:**
//!
//! - **The cell format's own ids are the ones in effect**, whatever its `applyFont` /
//!   `applyFill` / … flags say — `doc/xlsx-format.md` §3.3 and §4.4. The oracle measures the
//!   same way (`styles/named-styles.xlsx` A5 sets `applyFont="0"` and is drawn in its own
//!   bold, not its named style's heading font), and it is the reading that makes a
//!   `<cellXfs>` entry mean one thing. An id the entry leaves out comes from the named style
//!   it points at through `xfId`.
//! - **The workbook's default cell format is the document's default, and is not written onto
//!   every cell.** The cell format at index 0 is what a cell with no `s` gets; a property equal
//!   to its — a size, a colour, an alignment — is the absence of one, which is what ODF's
//!   automatic styles mean too, and writing `11pt` onto a million cells would make a million
//!   styles to say nothing. The cost is named in `grind_sheet::odf::read`'s ponytail on
//!   `style:default-style`: the model has no document default, so a shell draws those
//!   properties as its own — Excel's bottom alignment included, where the GNOME grid centres.
//! - **What the model has no property for is counted, never approximated** — [`Appearance`],
//!   one class per piece, surfacing per cell in `Report::appearance_lost` the way
//!   `Report::formats_lost` counts a number format. A pattern fill is not a background of its
//!   foreground colour, and a diagonal is not a border: the oracle approximates both, and
//!   loop D says so.
//!
//! The mapping tables — which border style is which width, which alignment is which ODF
//! value — are the oracle's own conversion read back, `doc/xlsx-format.md` §4.

use std::collections::BTreeMap;

use grind_sheet::model::NumberKind;
use grind_sheet::numfmt::Format;
use grind_sheet::style::CellStyle;

use crate::color::{self, Color, Missing, Palette, Resolved};
use crate::names::Ns;
use crate::numfmt::{self, Translation, Unspellable};
use crate::xml::{Attrs, Handled, Reader};

/// A piece of how the workbook **looks** that the model has no slot for.
///
/// `numfmt::Unspellable`'s sibling and `Dropped`'s admission rule: every variant names
/// something a `grind_sheet` document *cannot express*, not something this filter has not got
/// to. Counted per **cell** for the cell pieces, per **track** for [`Appearance::ZeroSize`],
/// and once per **sheet** for the three sheet-wide ones, which say so.
///
/// Two style losses are not here, because the report named them before this enum existed and
/// the corpus asks for them by that name: a font **family** is `Dropped::FontFamily` — the
/// model refuses one deliberately (`grind_sheet::style`, §5.4) — and a theme colour with no
/// theme to resolve it is `Dropped::ThemeColor`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Appearance {
    /// `<u/>`, any kind. `CellStyle` has no underline.
    Underline,
    /// `<strike/>`.
    Strike,
    /// `<vertAlign val="superscript"/>` or `subscript` — a text position, which a cell's
    /// style does not carry.
    Script,
    /// A two-colour pattern (`darkGrid`, `lightTrellis`, `gray125`, …): ink and paper and a
    /// texture, where ODF's cell background is one flat colour.
    PatternFill,
    /// `<gradientFill>` — not a colour at all.
    GradientFill,
    /// `dashDot`, `dashDotDot`, `mediumDashDot`, `mediumDashDotDot` or `slantDashDot`. The
    /// border is carried, as `dashed`, which is the nearest line XSL-FO names; the pattern is
    /// what is lost.
    BorderPattern,
    /// A diagonal rule. ODF has `style:diagonal-bl-tr` and `-tl-br`; `CellStyle` does not.
    Diagonal,
    /// `horizontal="fill"`: the content repeated to the cell's width. Carried as `start`.
    Fill,
    /// `horizontal="centerContinuous"`: centred across the empty cells to its right without
    /// merging them. Carried as `center`, within its own cell.
    CenterAcross,
    /// `horizontal="distributed"`: justified, with the space spread between the letters as
    /// well. Carried as `justify`.
    Distributed,
    /// `vertical="justify"` or `"distributed"`, which `style:vertical-align` has no value
    /// for — its whole vocabulary is top, middle, bottom and automatic.
    VerticalJustify,
    /// `shrinkToFit`.
    Shrink,
    /// `indent`, in character widths. ODF would say it as a paragraph margin.
    Indent,
    /// `textRotation`, including 255, which is not a rotation but stacked letters.
    Rotation,
    /// An `indexed` colour past the workbook's palette.
    UnknownColour,
    /// A row with `ht="0"` or a column with `width="0"`: invisible, and not hidden. ODF's
    /// sizes are positive lengths, so the track is carried **hidden** instead — which unhides
    /// to the default size rather than to nothing. Per track.
    ZeroSize,
    /// Rows or columns grouped into an outline (`outlineLevel`). What a collapsed group
    /// hides is carried, as hidden tracks; the grouping is not. Per sheet.
    Outline,
    /// Frozen or split panes. Per sheet. The selection, the zoom and whether gridlines and
    /// headers show are view state rather than document content, and are not counted.
    Pane,
    /// An autofilter criterion other than a set of values — a custom comparison, a top-ten
    /// rule, a dynamic date band, a colour. The rows Excel hid for it stay hidden; the rule
    /// that hid them is gone. Per filtered column.
    FilterCriterion,
    /// A sheet default width derived from `baseColWidth` with no `defaultColWidth` stated
    /// beside it — arithmetic this build has not measured — or a hidden or author-sized
    /// `<col>` run to the sheet's edge, which is carried only as far as the sheet's content
    /// goes. The model has no sheet default for a column. Per sheet.
    SheetDefaultWidth,
}

impl Appearance {
    /// Whether this piece would show on a cell with nothing in it — a fill or a diagonal does,
    /// an underline or an indent does not.
    pub fn shows_when_empty(self) -> bool {
        matches!(
            self,
            Appearance::PatternFill | Appearance::GradientFill | Appearance::Diagonal
        )
    }

    /// A plain-English name, for a report a person reads.
    pub fn label(self) -> &'static str {
        match self {
            Appearance::Underline => "underline",
            Appearance::Strike => "strikethrough",
            Appearance::Script => "superscript or subscript",
            Appearance::PatternFill => "pattern fill",
            Appearance::GradientFill => "gradient fill",
            Appearance::BorderPattern => "dash-dot border (drawn dashed)",
            Appearance::Diagonal => "diagonal border",
            Appearance::Fill => "fill alignment (repeated content)",
            Appearance::CenterAcross => "centred across a selection",
            Appearance::Distributed => "distributed alignment",
            Appearance::VerticalJustify => "justified vertical alignment",
            Appearance::Shrink => "shrink to fit",
            Appearance::Indent => "indent",
            Appearance::Rotation => "text rotation",
            Appearance::UnknownColour => "colour outside the palette",
            Appearance::ZeroSize => "zero-size row or column (carried hidden)",
            Appearance::Outline => "row or column outline (per sheet)",
            Appearance::Pane => "frozen or split panes (per sheet)",
            Appearance::FilterCriterion => "autofilter criterion (per column)",
            Appearance::SheetDefaultWidth => "sheet default column width (per sheet)",
        }
    }
}

/// What one cell format looks like, translated.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Look {
    /// `None` when there is nothing to carry: a plain cell has no style rather than an empty
    /// one, as in `odf/read.rs`.
    pub style: Option<CellStyle>,
    /// Sorted, each class once.
    pub lost: Vec<Appearance>,
    /// The font names a family other than the workbook's default font's — `Dropped::FontFamily`.
    pub family: bool,
    /// A theme colour with no theme to resolve it in — `Dropped::ThemeColor`.
    pub unresolved_theme: bool,
}

impl Look {
    /// Whether this look can be *seen* on a cell with nothing in it: a background or a border,
    /// carried or counted. A bold font or an alignment on an empty cell shows nothing, and
    /// dropping it from one loses nothing a person could have noticed.
    pub fn shows_when_empty(&self) -> bool {
        let carried = self.style.as_ref().is_some_and(|style| {
            style.background.is_some() || style.borders.iter().any(Option::is_some)
        });
        carried || self.lost.iter().any(|lost| lost.shows_when_empty())
    }
}

/// What the styles part says.
#[derive(Debug, Default)]
pub struct Styles {
    /// Per `<cellXfs>` entry, in order: what its number format translated to.
    xfs: Vec<Translation>,
    /// Per `<cellXfs>` entry, in order: what it looks like.
    looks: Vec<Look>,
}

impl Styles {
    /// The kind a cell with `s="index"` holds — `None` for a plain number, and for an index
    /// the part does not have. An out-of-range `s` is a claim about a style nobody wrote, and
    /// reading it as "no format" is what every consumer does.
    pub fn kind(&self, index: usize) -> Option<NumberKind> {
        self.xfs.get(index).and_then(|xf| xf.kind)
    }

    /// The format a cell with `s="index"` displays through, where one could be spelled.
    pub fn format(&self, index: usize) -> Option<&Format> {
        self.xfs.get(index).and_then(|xf| xf.format.as_ref())
    }

    /// What that cell's format lost on the way — empty where nothing did.
    pub fn lost(&self, index: usize) -> &[Unspellable] {
        static NONE: &[Unspellable] = &[];
        self.xfs.get(index).map_or(NONE, |xf| &xf.lost)
    }

    /// What a cell with `s="index"` looks like. `None` for an index the part does not have.
    pub fn look(&self, index: usize) -> Option<&Look> {
        self.looks.get(index)
    }
}

// ---- what the part says, before anything is resolved ----

#[derive(Clone, Debug, Default)]
struct Font {
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    script: bool,
    size: Option<f64>,
    color: Option<Color>,
    /// `<name val>`, or the theme's font scheme a `<scheme val>` names instead — either way a
    /// family, and the model carries none.
    family: Option<String>,
}

#[derive(Clone, Debug, Default)]
enum Fill {
    #[default]
    None,
    Solid(Option<Color>),
    Pattern,
    Gradient,
}

#[derive(Clone, Debug, Default)]
struct Border {
    /// In [`grind_sheet::style::EDGES`] order: `(style, colour)`.
    edges: [Option<(String, Option<Color>)>; 4],
    diagonal: bool,
}

#[derive(Clone, Debug, Default)]
struct Align {
    horizontal: Option<String>,
    vertical: Option<String>,
    wrap: bool,
    shrink: bool,
    indent: bool,
    rotation: bool,
}

#[derive(Clone, Debug, Default)]
struct Xf {
    num_fmt: Option<u32>,
    font: Option<usize>,
    fill: Option<usize>,
    border: Option<usize>,
    align: Option<Align>,
    parent: Option<usize>,
}

#[derive(Default)]
struct Part {
    codes: BTreeMap<u32, String>,
    fonts: Vec<Font>,
    fills: Vec<Fill>,
    borders: Vec<Border>,
    style_xfs: Vec<Xf>,
    cell_xfs: Vec<Xf>,
    indexed: Option<Vec<[u8; 3]>>,
}

/// Read the styles part, with the theme's colour scheme (`theme::read`, in `theme` attribute
/// order; empty for a workbook with no theme).
///
/// Never fails: a styles part that will not parse leaves every cell a plain number with no
/// look, which loses date kinds and looks and nothing else — and losing one part is not a
/// reason to refuse a workbook (`package.rs`'s rule).
pub fn read(bytes: &[u8], theme: &[[u8; 3]]) -> Styles {
    let mut reader = Reader::new(bytes);
    if !matches!(reader.root(), Ok(Some((ref root, _))) if root.is("styleSheet")) {
        return Styles::default();
    }
    // Every table collected first and resolved after: ECMA-376 orders them `numFmts, fonts,
    // fills, borders, cellStyleXfs, cellXfs`, and a reader that relied on that order would be
    // relying on a producer doing what the schema says.
    let mut part = Part::default();
    let _ = reader.children(|reader, name, _| {
        if name.ns != Ns::Spreadsheet {
            return Ok(Handled::No);
        }
        match name.local.as_str() {
            "numFmts" => reader.children(|_, name, attrs| {
                if !name.is("numFmt") {
                    return Ok(Handled::No);
                }
                if let (Some(id), Some(code)) = (
                    attrs.plain("numFmtId").and_then(|id| id.parse().ok()),
                    attrs.plain("formatCode"),
                ) {
                    part.codes.insert(id, code.to_owned());
                }
                Ok(Handled::Yes)
            })?,
            "fonts" => reader.children(|reader, name, _| {
                if !name.is("font") {
                    return Ok(Handled::No);
                }
                part.fonts.push(font(reader)?);
                Ok(Handled::Yes)
            })?,
            "fills" => reader.children(|reader, name, _| {
                if !name.is("fill") {
                    return Ok(Handled::No);
                }
                part.fills.push(fill(reader)?);
                Ok(Handled::Yes)
            })?,
            "borders" => reader.children(|reader, name, attrs| {
                if !name.is("border") {
                    return Ok(Handled::No);
                }
                part.borders.push(border(reader, attrs)?);
                Ok(Handled::Yes)
            })?,
            "cellStyleXfs" => part.style_xfs = xfs(reader)?,
            "cellXfs" => part.cell_xfs = xfs(reader)?,
            "colors" => part.indexed = color::read_indexed(reader)?,
            _ => return Ok(Handled::No),
        }
        Ok(Handled::Yes)
    });
    part.resolve(theme)
}

impl Part {
    fn resolve(self, theme: &[[u8; 3]]) -> Styles {
        let palette = Palette {
            indexed: self
                .indexed
                .clone()
                .unwrap_or_else(|| color::PALETTE.to_vec()),
            theme: theme.to_vec(),
        };
        // What an id the cell format leaves out means: its named style's, then the first
        // entry of the table, which is what every producer puts there for "nothing".
        let pick = |xf: &Xf, id: fn(&Xf) -> Option<usize>| -> usize {
            id(xf)
                .or_else(|| self.style_xfs.get(xf.parent.unwrap_or(0)).and_then(id))
                .unwrap_or(0)
        };
        let default_font = self
            .cell_xfs
            .first()
            .and_then(|xf| self.fonts.get(pick(xf, |x| x.font)))
            .cloned()
            .unwrap_or_default();
        let base = Base {
            size: default_font.size.unwrap_or(DEFAULT_SIZE),
            color: default_font
                .color
                .as_ref()
                .map_or(Resolved::Automatic, |c| palette.resolve(c)),
            family: default_font.family.clone(),
        };

        let none = Fill::None;
        let blank = Border::default();
        let mut xfs = Vec::with_capacity(self.cell_xfs.len());
        let mut looks = Vec::with_capacity(self.cell_xfs.len());
        for xf in &self.cell_xfs {
            // A file's own code wins over the built-in table, including for an id below 164:
            // the spec reserves those, and producers redefine them anyway. `applyNumberFormat`
            // is not consulted — §3.3.
            xfs.push(match xf.num_fmt {
                None => Translation::default(),
                Some(id) => match self.codes.get(&id) {
                    Some(code) => numfmt::of_code(code),
                    None => numfmt::of_builtin(id),
                },
            });
            let align = xf.align.as_ref().or_else(|| {
                self.style_xfs
                    .get(xf.parent.unwrap_or(0))
                    .and_then(|p| p.align.as_ref())
            });
            looks.push(look(
                self.fonts
                    .get(pick(xf, |x| x.font))
                    .unwrap_or(&default_font),
                self.fills.get(pick(xf, |x| x.fill)).unwrap_or(&none),
                self.borders.get(pick(xf, |x| x.border)).unwrap_or(&blank),
                align,
                &base,
                &palette,
            ));
        }
        // The default cell format *is* the document's default: whatever it says, every cell
        // that says the same carries nothing. `sample.xlsx`'s named style puts
        // `vertical="bottom"` on every cell, which would otherwise be a style on every cell
        // saying what the workbook's own default already says.
        let base = looks
            .first()
            .and_then(|look: &Look| look.style.clone())
            .unwrap_or_default();
        for look in &mut looks {
            if let Some(style) = look.style.take() {
                let style = without(style, &base);
                look.style = (!style.is_plain()).then_some(style);
            }
        }
        Styles { xfs, looks }
    }
}

/// `style` with every property equal to `base`'s taken out.
fn without(mut style: CellStyle, base: &CellStyle) -> CellStyle {
    let fields = [
        (&mut style.font_weight, &base.font_weight),
        (&mut style.font_style, &base.font_style),
        (&mut style.font_size, &base.font_size),
        (&mut style.color, &base.color),
        (&mut style.background, &base.background),
        (&mut style.align, &base.align),
        (&mut style.vertical_align, &base.vertical_align),
        (&mut style.wrap, &base.wrap),
    ];
    for (field, base) in fields {
        if field.is_some() && field == base {
            *field = None;
        }
    }
    for (edge, base) in style.borders.iter_mut().zip(&base.borders) {
        if edge.is_some() && edge == base {
            *edge = None;
        }
    }
    style
}

/// `<sz>` when a font states none: 11pt, the size Excel has given its default font since 2007
/// and the size the oracle writes for a workbook whose default font says nothing.
const DEFAULT_SIZE: f64 = 11.0;

/// The workbook's default font — what the cell format at index 0 is set in, which is what a
/// cell with no `s` gets. A size, colour or family equal to it is the document's default
/// rather than a property of the cell.
struct Base {
    size: f64,
    color: Resolved,
    family: Option<String>,
}

fn look(
    font: &Font,
    fill: &Fill,
    border: &Border,
    align: Option<&Align>,
    base: &Base,
    palette: &Palette,
) -> Look {
    let mut style = CellStyle::default();
    let mut lost = Vec::new();
    let mut unresolved_theme = false;
    let mut resolve = |color: &Color, lost: &mut Vec<Appearance>| match palette.resolve(color) {
        Resolved::Unknown(Missing::Theme) => {
            unresolved_theme = true;
            Resolved::Automatic
        }
        Resolved::Unknown(Missing::Index) => {
            lost.push(Appearance::UnknownColour);
            Resolved::Automatic
        }
        resolved => resolved,
    };

    // The font.
    style.font_weight = font.bold.then(|| "bold".to_owned());
    style.font_style = font.italic.then(|| "italic".to_owned());
    let size = font.size.unwrap_or(base.size);
    if size != base.size {
        style.font_size = Some(format!("{size}pt"));
    }
    let ink = font
        .color
        .as_ref()
        .map_or(Resolved::Automatic, |c| resolve(c, &mut lost));
    if ink != base.color {
        style.color = ink.hex().map(str::to_owned);
    }
    for (on, class) in [
        (font.underline, Appearance::Underline),
        (font.strike, Appearance::Strike),
        (font.script, Appearance::Script),
    ] {
        if on {
            lost.push(class);
        }
    }
    let family = font.family.is_some() && font.family != base.family;

    // The fill: only `solid` has a translation, and its colour is the *foreground* — a
    // pattern's ink — whatever the name `bgColor` suggests.
    match fill {
        Fill::None => {}
        Fill::Solid(color) => {
            style.background = color
                .as_ref()
                .map(|c| resolve(c, &mut lost))
                .and_then(|r| r.hex().map(str::to_owned));
        }
        Fill::Pattern => lost.push(Appearance::PatternFill),
        Fill::Gradient => lost.push(Appearance::GradientFill),
    }

    // The border. An edge whose colour is automatic is drawn in the ink, which for a line is
    // black: ODF's border has no automatic colour, and the oracle writes `#000000` there.
    for (slot, edge) in style.borders.iter_mut().zip(&border.edges) {
        let Some((name, color)) = edge else { continue };
        let Some((width, line, pattern)) = line(name) else {
            continue;
        };
        if pattern {
            lost.push(Appearance::BorderPattern);
        }
        let color = color
            .as_ref()
            .map(|c| resolve(c, &mut lost))
            .and_then(|r| r.hex().map(str::to_owned))
            .unwrap_or_else(|| "#000000".to_owned());
        *slot = Some(format!("{width} {line} {color}"));
    }
    if border.diagonal {
        lost.push(Appearance::Diagonal);
    }

    // The alignment.
    if let Some(align) = align {
        let (horizontal, class) = match align.horizontal.as_deref() {
            Some("left") => (Some("start"), None),
            Some("center") => (Some("center"), None),
            Some("right") => (Some("end"), None),
            Some("justify") => (Some("justify"), None),
            Some("fill") => (Some("start"), Some(Appearance::Fill)),
            Some("centerContinuous") => (Some("center"), Some(Appearance::CenterAcross)),
            Some("distributed") => (Some("justify"), Some(Appearance::Distributed)),
            // `general` — text to the start, numbers to the end — is ODF's own default
            // (`style:text-align-source="value-type"`), which is no value at all.
            _ => (None, None),
        };
        style.align = horizontal.map(str::to_owned);
        lost.extend(class);
        style.vertical_align = match align.vertical.as_deref() {
            Some("top") => Some("top".to_owned()),
            Some("center") => Some("middle".to_owned()),
            Some("bottom") => Some("bottom".to_owned()),
            Some("justify" | "distributed") => {
                lost.push(Appearance::VerticalJustify);
                None
            }
            _ => None,
        };
        style.wrap = align.wrap.then(|| "wrap".to_owned());
        for (on, class) in [
            (align.shrink, Appearance::Shrink),
            (align.indent, Appearance::Indent),
            (align.rotation, Appearance::Rotation),
        ] {
            if on {
                lost.push(class);
            }
        }
    }

    lost.sort();
    lost.dedup();
    Look {
        style: (!style.is_plain()).then_some(style),
        lost,
        family,
        unresolved_theme,
    }
}

/// A border style's name as ODF's three parts: width, line, and whether a dash pattern was
/// lost. `None` for `none` and for a name ECMA-376 does not define.
///
/// The widths are the oracle's, read back from its conversion of `styles/borders.xlsx`
/// (`doc/xlsx-format.md` §4.3): 0.74pt for Excel's thin weight, 1.76pt for medium, 2.49pt for
/// thick and 0.06pt for hair. The lines are XSL-FO's, which `fo:border` takes (§20.183) —
/// `solid`, `dotted`, `dashed`, `double` — where the oracle writes LibreOffice's own
/// `fine-dashed`, `dash-dot`, `dash-dot-dot` and `double-thin`.
fn line(name: &str) -> Option<(&'static str, &'static str, bool)> {
    Some(match name {
        "thin" => ("0.74pt", "solid", false),
        "medium" => ("1.76pt", "solid", false),
        "thick" => ("2.49pt", "solid", false),
        "hair" => ("0.06pt", "solid", false),
        "double" => ("1.76pt", "double", false),
        "dotted" => ("0.74pt", "dotted", false),
        "dashed" => ("0.74pt", "dashed", false),
        "mediumDashed" => ("1.76pt", "dashed", false),
        "dashDot" | "dashDotDot" => ("0.74pt", "dashed", true),
        "mediumDashDot" | "mediumDashDotDot" | "slantDashDot" => ("1.76pt", "dashed", true),
        _ => return None,
    })
}

// ---- the readers, one per table ----

/// `<b/>`, `<i/>`, `<strike/>`: on unless `val` says otherwise (§18.8.2's `CT_BooleanProperty`,
/// whose `val` defaults to true).
fn on(attrs: &Attrs) -> bool {
    attrs.plain("val").is_none() || attrs.flag("val")
}

fn font(reader: &mut Reader<'_>) -> crate::Result<Font> {
    let mut font = Font::default();
    reader.children(|_, name, attrs| {
        if name.ns != Ns::Spreadsheet {
            return Ok(Handled::No);
        }
        match name.local.as_str() {
            "b" => font.bold = on(attrs),
            "i" => font.italic = on(attrs),
            "strike" => font.strike = on(attrs),
            // `single` when `val` is absent; `none` is the one spelling of no underline.
            "u" => font.underline = attrs.plain("val") != Some("none"),
            "vertAlign" => {
                font.script = matches!(attrs.plain("val"), Some("superscript" | "subscript"))
            }
            "sz" => {
                font.size = attrs
                    .plain("val")
                    .and_then(|v| v.trim().parse::<f64>().ok())
                    .filter(|v| v.is_finite() && *v > 0.0)
            }
            "color" => font.color = Some(color::read(attrs)),
            "name" => font.family = attrs.plain("val").map(str::to_owned),
            "scheme" if font.family.is_none() => {
                font.family = attrs
                    .plain("val")
                    .filter(|v| *v != "none")
                    .map(|v| format!("scheme:{v}"))
            }
            _ => return Ok(Handled::No),
        }
        Ok(Handled::Yes)
    })?;
    Ok(font)
}

fn fill(reader: &mut Reader<'_>) -> crate::Result<Fill> {
    let mut fill = Fill::None;
    reader.children(|reader, name, attrs| {
        if name.is("gradientFill") {
            fill = Fill::Gradient;
            return Ok(Handled::No);
        }
        if !name.is("patternFill") {
            return Ok(Handled::No);
        }
        // §18.8.32: an absent `patternType` is `none`.
        match attrs.plain("patternType").unwrap_or("none") {
            "none" => {}
            "solid" => {
                let mut fg = None;
                reader.children(|_, name, attrs| {
                    if name.is("fgColor") {
                        fg = Some(color::read(attrs));
                    }
                    Ok(Handled::No)
                })?;
                fill = Fill::Solid(fg);
            }
            _ => fill = Fill::Pattern,
        }
        Ok(Handled::Yes)
    })?;
    Ok(fill)
}

fn border(reader: &mut Reader<'_>, attrs: &Attrs) -> crate::Result<Border> {
    let mut out = Border::default();
    let runs_either_way = attrs.flag("diagonalUp") || attrs.flag("diagonalDown");
    reader.children(|reader, name, attrs| {
        if name.ns != Ns::Spreadsheet {
            return Ok(Handled::No);
        }
        // `start` and `end` are the 2nd edition's names for left and right.
        let slot = match name.local.as_str() {
            "left" | "start" => Some(0),
            "right" | "end" => Some(1),
            "top" => Some(2),
            "bottom" => Some(3),
            "diagonal" => None,
            _ => return Ok(Handled::No),
        };
        let style = attrs.plain("style").unwrap_or("none").to_owned();
        let mut ink = None;
        reader.children(|_, name, attrs| {
            if name.is("color") {
                ink = Some(color::read(attrs));
            }
            Ok(Handled::No)
        })?;
        match slot {
            Some(i) if style != "none" => out.edges[i] = Some((style, ink)),
            Some(_) => {}
            // The `<diagonal>` supplies the line and the two flags say which way it runs; a
            // line with neither flag is drawn nowhere.
            None => out.diagonal = style != "none" && runs_either_way,
        }
        Ok(Handled::Yes)
    })?;
    Ok(out)
}

fn xfs(reader: &mut Reader<'_>) -> crate::Result<Vec<Xf>> {
    let mut out = Vec::new();
    reader.children(|reader, name, attrs| {
        if !name.is("xf") {
            return Ok(Handled::No);
        }
        let id = |local: &str| attrs.plain(local).and_then(|v| v.trim().parse().ok());
        let mut xf = Xf {
            num_fmt: id("numFmtId").and_then(|v: usize| u32::try_from(v).ok()),
            font: id("fontId"),
            fill: id("fillId"),
            border: id("borderId"),
            align: None,
            parent: id("xfId"),
        };
        reader.children(|_, name, attrs| {
            if !name.is("alignment") {
                return Ok(Handled::No);
            }
            let number = |local: &str| {
                attrs
                    .plain(local)
                    .and_then(|v| v.trim().parse::<i64>().ok())
                    .unwrap_or(0)
            };
            xf.align = Some(Align {
                horizontal: attrs.plain("horizontal").map(str::to_owned),
                vertical: attrs.plain("vertical").map(str::to_owned),
                wrap: attrs.flag("wrapText"),
                shrink: attrs.flag("shrinkToFit"),
                indent: number("indent") > 0,
                rotation: number("textRotation") != 0,
            });
            Ok(Handled::Yes)
        })?;
        out.push(xf);
        Ok(Handled::Yes)
    })?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAIN: &str = crate::names::MAIN_T;

    #[test]
    fn a_cell_format_is_a_date_through_its_number_format() {
        let xml = format!(
            r#"<styleSheet xmlns="{MAIN}">
                 <cellXfs count="4">
                   <xf numFmtId="0"/>
                   <xf numFmtId="14"/>
                   <xf numFmtId="164"/>
                   <xf numFmtId="165" applyNumberFormat="0"/>
                 </cellXfs>
                 <numFmts>
                   <numFmt numFmtId="164" formatCode="hh:mm:ss"/>
                   <numFmt numFmtId="165" formatCode="0.00"/>
                 </numFmts>
               </styleSheet>"#
        );
        let styles = read(xml.as_bytes(), &[]);
        assert_eq!(styles.kind(0), None);
        assert_eq!(styles.kind(1), Some(NumberKind::Date));
        // `numFmts` after `cellXfs` — against the schema's order, and still read.
        assert_eq!(styles.kind(2), Some(NumberKind::Time));
        assert_eq!(styles.kind(3), None);
        assert_eq!(styles.kind(99), None, "an index past the table");
    }

    #[test]
    fn a_file_may_redefine_a_built_in_id() {
        let xml = format!(
            r#"<styleSheet xmlns="{MAIN}">
                 <numFmts><numFmt numFmtId="14" formatCode="0.00"/></numFmts>
                 <cellXfs><xf numFmtId="14"/></cellXfs>
               </styleSheet>"#
        );
        assert_eq!(read(xml.as_bytes(), &[]).kind(0), None);
    }

    /// A styles part with the given tables, and cell formats whose `fontId`/`fillId`/
    /// `borderId` and alignment are given per entry. Entry 0 is the workbook's default.
    fn part(tables: &str, xfs: &str) -> Styles {
        let xml =
            format!(r#"<styleSheet xmlns="{MAIN}">{tables}<cellXfs>{xfs}</cellXfs></styleSheet>"#);
        read(xml.as_bytes(), &[])
    }

    fn style(styles: &Styles, index: usize) -> Option<CellStyle> {
        styles.look(index).and_then(|look| look.style.clone())
    }

    /// The workbook's default font is 11pt and black; a cell in it carries nothing, and a cell
    /// that differs carries exactly the difference.
    #[test]
    fn a_font_carries_what_differs_from_the_default_one() {
        let styles = part(
            r#"<fonts>
                 <font><sz val="11"/><color rgb="FF000000"/><name val="Calibri"/></font>
                 <font><b/><sz val="11"/><color rgb="FF000000"/><name val="Calibri"/></font>
                 <font><i val="0"/><sz val="14"/><color rgb="FFFF0000"/><name val="Calibri"/></font>
                 <font><u/><strike/><vertAlign val="superscript"/><name val="Georgia"/></font>
               </fonts>"#,
            r#"<xf fontId="0"/><xf fontId="1"/><xf fontId="2"/><xf fontId="3"/>"#,
        );
        assert_eq!(style(&styles, 0), None);
        assert_eq!(
            style(&styles, 1),
            Some(CellStyle {
                font_weight: Some("bold".into()),
                ..CellStyle::default()
            })
        );
        assert_eq!(
            style(&styles, 2),
            Some(CellStyle {
                font_size: Some("14pt".into()),
                color: Some("#ff0000".into()),
                ..CellStyle::default()
            }),
            "`<i val=\"0\"/>` is not italic"
        );
        let look = styles.look(3).unwrap();
        assert!(look.family, "Georgia is not the default Calibri");
        assert_eq!(
            look.lost,
            [
                Appearance::Underline,
                Appearance::Strike,
                Appearance::Script
            ]
        );
        assert!(
            !styles.look(1).unwrap().family,
            "the default family costs nothing"
        );
    }

    /// `applyFont="0"` does not switch the cell format's own font off — `doc/xlsx-format.md`
    /// §4.4 — and an id the cell format leaves out comes from its named style.
    #[test]
    fn a_cell_format_uses_its_own_ids_and_inherits_the_missing_ones() {
        let styles = part(
            r#"<fonts><font/><font><b/></font><font><i/></font></fonts>
               <fills><fill><patternFill/></fill>
                 <fill><patternFill patternType="solid"><fgColor rgb="FFFFFF00"/><bgColor indexed="64"/></patternFill></fill>
               </fills>
               <cellStyleXfs><xf fontId="0"/><xf fontId="2" fillId="1"/></cellStyleXfs>"#,
            r#"<xf fontId="0"/><xf fontId="1" xfId="1" applyFont="0"/>"#,
        );
        assert_eq!(
            style(&styles, 1),
            Some(CellStyle {
                font_weight: Some("bold".into()),
                background: Some("#ffff00".into()),
                ..CellStyle::default()
            }),
            "its own bold font, its named style's fill — the *foreground* of a solid one"
        );
    }

    #[test]
    fn only_a_solid_fill_is_a_background() {
        let styles = part(
            r#"<fills><fill><patternFill patternType="none"/></fill>
                 <fill><patternFill patternType="darkGrid"><fgColor rgb="FF0000FF"/></patternFill></fill>
                 <fill><gradientFill degree="90"><stop position="0"><color rgb="FFFFFFFF"/></stop></gradientFill></fill>
               </fills>"#,
            r#"<xf fillId="0"/><xf fillId="1"/><xf fillId="2"/>"#,
        );
        assert_eq!(style(&styles, 1), None, "a pattern is not its foreground");
        assert_eq!(styles.look(1).unwrap().lost, [Appearance::PatternFill]);
        assert_eq!(styles.look(2).unwrap().lost, [Appearance::GradientFill]);
    }

    /// Excel names a border; ODF spells one. The widths are the oracle's (§4.3).
    #[test]
    fn a_border_is_a_width_a_line_and_a_colour() {
        let styles = part(
            r#"<borders><border/>
                 <border><left style="thin"><color rgb="FFFF0000"/></left><right style="medium"/>
                   <top style="dashDot"/><bottom style="double"><color indexed="12"/></bottom></border>
                 <border diagonalUp="1"><diagonal style="thin"/></border>
               </borders>"#,
            r#"<xf borderId="0"/><xf borderId="1"/><xf borderId="2"/>"#,
        );
        let got = style(&styles, 1).unwrap();
        assert_eq!(
            got.borders,
            [
                Some("0.74pt solid #ff0000".into()),
                Some("1.76pt solid #000000".into()),
                Some("0.74pt dashed #000000".into()),
                Some("1.76pt double #0000ff".into()),
            ]
        );
        assert_eq!(styles.look(1).unwrap().lost, [Appearance::BorderPattern]);
        assert_eq!(style(&styles, 2), None);
        assert_eq!(styles.look(2).unwrap().lost, [Appearance::Diagonal]);
    }

    #[test]
    fn alignment_is_odfs_where_odf_has_a_word_for_it() {
        let styles = part(
            "",
            r#"<xf/>
               <xf><alignment horizontal="right" vertical="center" wrapText="1"/></xf>
               <xf><alignment horizontal="general" vertical="bottom"/></xf>
               <xf><alignment horizontal="centerContinuous" vertical="distributed" indent="2" textRotation="255"/></xf>"#,
        );
        assert_eq!(
            style(&styles, 1),
            Some(CellStyle {
                align: Some("end".into()),
                vertical_align: Some("middle".into()),
                wrap: Some("wrap".into()),
                ..CellStyle::default()
            })
        );
        assert_eq!(
            style(&styles, 2),
            Some(CellStyle {
                vertical_align: Some("bottom".into()),
                ..CellStyle::default()
            }),
            "general is no alignment; an explicit bottom is one"
        );
        assert_eq!(
            styles.look(3).unwrap().lost,
            [
                Appearance::CenterAcross,
                Appearance::VerticalJustify,
                Appearance::Indent,
                Appearance::Rotation
            ]
        );
    }

    /// Whatever the default cell format says, a cell saying the same carries nothing — the
    /// shape of `sample.xlsx`, whose named style bottom-aligns every cell.
    #[test]
    fn the_default_cell_format_is_the_documents_default() {
        let styles = part(
            r#"<fonts><font/><font><b/></font></fonts>
               <cellStyleXfs><xf><alignment vertical="bottom"/></xf></cellStyleXfs>"#,
            r#"<xf fontId="0" xfId="0"/><xf fontId="1" xfId="0"/><xf xfId="0"><alignment vertical="top"/></xf>"#,
        );
        assert_eq!(style(&styles, 0), None);
        assert_eq!(
            style(&styles, 1),
            Some(CellStyle {
                font_weight: Some("bold".into()),
                ..CellStyle::default()
            }),
            "bold, and not bottom-aligned by inheritance"
        );
        assert_eq!(
            style(&styles, 2).and_then(|s| s.vertical_align),
            Some("top".into())
        );
    }

    #[test]
    fn a_theme_colour_resolves_through_the_theme_or_is_counted() {
        let xml = format!(
            r#"<styleSheet xmlns="{MAIN}"><fonts><font/><font><color theme="4" tint="-0.25"/></font></fonts>
                 <cellXfs><xf fontId="0"/><xf fontId="1"/></cellXfs></styleSheet>"#
        );
        let mut theme = vec![
            [0xFF, 0xFF, 0xFF],
            [0, 0, 0],
            [0xE7, 0xE6, 0xE6],
            [0x44, 0x54, 0x6A],
        ];
        theme.push([0x44, 0x72, 0xC4]);
        let styles = read(xml.as_bytes(), &theme);
        assert_eq!(
            style(&styles, 1).and_then(|s| s.color),
            Some("#2f5597".into())
        );
        assert!(!styles.look(1).unwrap().unresolved_theme);

        let styles = read(xml.as_bytes(), &[]);
        assert_eq!(style(&styles, 1), None, "no theme: the ink, not a guess");
        assert!(styles.look(1).unwrap().unresolved_theme);
    }

    #[test]
    fn a_broken_part_is_no_formats_rather_than_no_workbook() {
        assert_eq!(read(b"not xml at all <<<", &[]).kind(0), None);
        assert_eq!(read(b"", &[]).kind(0), None);
    }
}
