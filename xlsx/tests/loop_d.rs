// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Loop D — import fidelity: our conversion of a workbook against **LibreOffice's** conversion of
//! it, cell by cell (`doc/xlsx-import.md`, "Verification").
//!
//! ```text
//! ours   = grind_xlsx::import_bytes(bytes)              → Document
//! theirs = soffice --headless --convert-to fods <file>  → read with our own reader → Document
//! compare(ours, theirs)
//! ```
//!
//! The corpus is the vendored generated one (`tests/data/corpus/`, `ooxmlgen.rs`), every fixture
//! whose manifest says `oracleOpens`, so it needs `soffice` and no checkout. `ooxmlgen.rs` holds
//! the files to the manifest's claims; this holds them to the oracle's *behaviour* — a manifest is
//! a belief about a file, the oracle a program that opened it.
//!
//! **Needs `soffice` on `PATH`; skips with a notice without one**, as loop C does. CI pins the
//! oracle to a container image (`doc/differential-fuzz.md`) and `scripts/soffice-tests.sh` puts it
//! first on `PATH` locally. Output is **cached** by fixture SHA-256 and the oracle's version
//! string.
//!
//! **Compared: every cell's value, kind and display, every carried cell's style, and every
//! column's and row's size and hidden-ness** ([`differences`]). A display is compared where
//! **both** sides format the cell, since a cell left unformatted had its code refused by name,
//! which `ooxmlgen.rs` asserts. **Formula text is not compared**: the oracle spells one expression
//! differently in ways the manifest records (`[$Data.A1]` for `[Data.A1]`,
//! `com.microsoft.xlookup`, lower-cased unknown names), so the manifest is the oracle for formula
//! text. Values compare at 15 significant digits (loop C's rule); a kind must match exactly.
//!
//! **Where the conversions differ on purpose** — usually the oracle being wrong, twice grind
//! choosing differently — the disagreement is a named [`Divergence`], a construct and never a file
//! name, asserted to still occur, so one LibreOffice stops having fails and is deleted.
//!
//! cargo test -p grind-xlsx --test loop_d -- --nocapture

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use grind_sheet::model::{CellValue, Document, NumberKind, Pos, Sheet};
use grind_sheet::style::{self, CellStyle};

/// How many fixtures must reach the comparison. The ratchet, as in every loop: raise it,
/// never lower it. 69 of the 76 import; the seven that do not are refused on purpose.
const FLOOR: usize = 60;

/// A construct on which LibreOffice's conversion is known to differ from ours, with the
/// evidence. Each is a predicate over one difference, and each must still match at least one
/// in the corpus — the day it does not, the entry is deleted. **First match wins**, so a
/// narrow entry goes before a broad one.
///
/// Every entry here was *looked at* before it was written: the oracle's `.fods` read for the
/// cell or sheet in question, and the fixture's own bytes beside it. None is a tolerance.
struct Divergence {
    name: &'static str,
    why: &'static str,
    scope: Scope,
}

enum Scope {
    /// One cell both sides have a sheet for.
    Cell(fn(ours: &Cell, theirs: &Cell) -> bool),
    /// One cell's **style**, where both sides carry one — its own scope, so a divergence about
    /// values cannot swallow a style difference on the same cell.
    Style(fn(ours: &CellStyle, theirs: &CellStyle) -> bool),
    /// One column or row of a sheet both sides have.
    Track(fn(&Track) -> bool),
    /// A sheet of ours the oracle's conversion lacks, by the name **the workbook** gave it — the
    /// import may have renamed it, and the construct is the spelling.
    OnlyOurs(fn(name: &str) -> bool),
    /// A sheet of the oracle's, by name, that ours does not have.
    OnlyTheirs(fn(name: &str) -> bool),
    /// A sheet both have, which the oracle's conversion holds nothing in and ours does.
    Unreached,
}

const DIVERGENCES: &[Divergence] = &[
    Divergence {
        name: "an error value #REF! becomes a formula",
        why: "`<c t=\"e\"><v>#REF!</v></c>` — a cached error with no `<f>` — arrives in the \
              oracle's output as `table:formula=\"of:=#ref!\"`, lower-cased and unparseable, \
              which it then evaluates to `#NAME?`. The other five error values in \
              `values/types.xlsx` survive. Read from the oracle's `.fods` on 2026-09-19.",
        scope: Scope::Cell(|ours, theirs| {
            ours.value == CellValue::Text("#REF!".into())
                && theirs.value == CellValue::Text("#NAME?".into())
        }),
    },
    Divergence {
        name: "a formula cell's value is the oracle's own evaluation",
        why: "The oracle **recalculates** these workbooks on load rather than carrying the \
              cached `<v>`: `formulas/shared-groups.xlsx` caches C2 = 4 and the oracle writes \
              6, which is A2+B2. So for a formula cell this loop would compare two evaluators \
              rather than two importers. The cached values are not unchecked — `ooxmlgen.rs` \
              holds every one of them to the manifest, C2 = 4 included. X2 carries the formula \
              as well and does not change this: the oracle still recalculates on load, so the \
              two numbers still come from two evaluators.",
        scope: Scope::Cell(|_, theirs| theirs.formula),
    },
    Divergence {
        name: "the 1900 system below the phantom day",
        why: "Excel's serials 1–60 are one day later than ODF's epoch makes them, because Excel \
              believes 1900-02-29 existed (`doc/xlsx-format.md` §2.1). The oracle applies no \
              correction and so shows every date before 1900-03-01 a day early — 1900-01-01 \
              arrives as 1899-12-31. `values/dates-1900.xlsx`'s manifest records the oracle's \
              answer in an `oracle` field for exactly this reason. Ours shifts by one.",
        scope: Scope::Cell(|ours, theirs| {
            ours.kind == Some(NumberKind::Date)
                && theirs.kind == Some(NumberKind::Date)
                && matches!((&ours.value, &theirs.value), (CellValue::Number(a), CellValue::Number(b))
                    if *a <= 61.0 && (a - b - 1.0).abs() < 1e-9)
        }),
    },
    Divergence {
        name: "an empty string is dropped",
        why: "`<t/>` is a string cell whose text is empty — not an empty cell, which is what the \
              manifest says and what this import carries. The oracle writes a bare \
              `<table:table-cell/>`, so the cell is gone. The hostile entity fixtures come out \
              the same way: an entity nothing defines resolves to no text, leaving an empty \
              string.",
        scope: Scope::Cell(|ours, theirs| {
            ours.value == CellValue::Text(String::new()) && theirs.value == CellValue::Empty
        }),
    },
    Divergence {
        name: "a pivot table's output is regenerated",
        why: "The oracle's conversion of a pivot table carries its own labels rather than the \
              workbook's — `Total Result` where the cell says `grand total` — so the output is \
              regenerated rather than read. This import \
              carries the cells the workbook holds and counts the pivot table as dropped \
              (X5), which is the decision that nothing is evaluated on import.",
        scope: Scope::Cell(|ours, theirs| {
            matches!(ours.value, CellValue::Text(_))
                && theirs.value == CellValue::Text("Total Result".into())
        }),
    },
    Divergence {
        name: "a built-in date id is spelled in the reader's locale",
        why: "ECMA-376 §18.8.30 prints id 14 as `mm-dd-yy` and id 22 as `m/d/yy h:mm`, and \
              Excel renders both in the *reader's* locale rather than in that US order. The \
              oracle does the same with its own — `3/17/2024` from an en-US machine, and \
              something else from a German one, which makes its answer a fact about the \
              converting machine. This import maps the two ids onto `numfmt::preset`'s ISO \
              spelling (`doc/xlsx-import.md` Part II §4, *by meaning, not by their literal \
              code*), which means the same day in every country. The corpus agrees that this \
              is unclaimable: `numfmt/builtins.xlsx` states a `display` for no cell at all.",
        scope: Scope::Cell(|ours, theirs| {
            ours.kind == Some(NumberKind::Date)
                && ours.value == theirs.value
                && iso_for(&theirs.display) == ours.display
        }),
    },
    Divergence {
        name: "a blank-width pad has no ODF spelling",
        why: "`0.00_);(0.00)` — `_)` is a blank as wide as `)`, which lines a bracketed \
              negative up under a positive one. LibreOffice carries it as \
              `loext:blank-width-char`, its own extension rather than ODF, and renders the \
              space; this build counts `Unspellable::BlankWidth` and keeps the rest of the \
              format. One cell, `numfmt/custom-numeric.xlsx` B17.",
        scope: Scope::Cell(|ours, theirs| {
            ours.value == theirs.value && theirs.display.trim_end() == ours.display
        }),
    },
    // ---- styles ----
    Divergence {
        name: "the oracle names its own border lines",
        why: "`fo:border` is XSL-FO's shorthand (ODF §20.183), whose line styles are `solid`, \
              `dotted`, `dashed` and `double` among others. The oracle writes LibreOffice's own \
              — `fine-dashed` for Excel's `dashed` and `slantDashDot`, `dash-dot`, \
              `dash-dot-dot`, and `double-thin` with a `style:border-line-width` beside it for \
              `double` — where this import writes the nearest XSL-FO name and counts the lost \
              dash pattern (`Appearance::BorderPattern`). Read from the oracle's conversion of \
              `styles/borders.xlsx`, 2026-09-21. Everything else about each edge must agree.",
        scope: Scope::Style(|ours, theirs| same_but(ours, theirs, |a, b| a == b)),
    },
    Divergence {
        name: "a theme colour with no theme is drawn white by the oracle",
        why: "`styles/borders.xlsx` B16 names `theme=\"4\"` on two edges and the package has no \
              theme part. There is nothing to resolve the slot against: this import draws the \
              edge in the ink and counts `Dropped::ThemeColor`; the oracle draws it `#ffffff` \
              — and, measured on a font the same way, writes no colour at all. Every other \
              edge of the cell must agree.",
        scope: Scope::Style(|ours, theirs| {
            same_but(ours, theirs, |a, b| {
                a == b || (a == "#000000" && b == "#ffffff")
            })
        }),
    },
    Divergence {
        name: "a system colour is a fixed colour in the oracle",
        why: "`indexed=\"64\"` and `\"65\"` are the system's foreground and background \
              (ECMA-376 §18.8.27), which is ODF's automatic colour, and this import carries them \
              as no colour. The oracle fixes them at `#000000` and `#ffffff` — on a dark theme, \
              black text. `styles/colors.xlsx` B12 and B13.",
        scope: Scope::Style(|ours, theirs| {
            ours.color.is_none()
                && matches!(theirs.color.as_deref(), Some("#000000" | "#ffffff"))
                && CellStyle {
                    color: None,
                    ..theirs.clone()
                } == *ours
        }),
    },
    Divergence {
        name: "a tint rounds one step differently",
        why: "ECMA-376's tint moves a colour's HSL luminance. This import computes it in \
              floating point and rounds each channel half up, which reproduces the oracle's \
              colour exactly for 69 of the 75 tinted theme colours measured \
              (`doc/xlsx-format.md` §4.2) and is one step off in one or more channels for the \
              other six: the oracle quantises somewhere the specification does not say to. \
              `styles/colors.xlsx` B27, accent1 at tint 0.5, is one of them.",
        scope: Scope::Style(|ours, theirs| {
            let (Some(a), Some(b)) = (ours.color.as_deref(), theirs.color.as_deref()) else {
                return false;
            };
            let channel = |hex: &str, i: usize| i32::from_str_radix(&hex[1 + 2 * i..3 + 2 * i], 16);
            (0..3).all(|i| match (channel(a, i), channel(b, i)) {
                (Ok(x), Ok(y)) => (x - y).abs() <= 1,
                _ => false,
            }) && CellStyle {
                color: None,
                ..ours.clone()
            } == CellStyle {
                color: None,
                ..theirs.clone()
            }
        }),
    },
    Divergence {
        name: "a pattern fill is blended into one colour",
        why: "A pattern fill is two colours and a texture; ODF's cell background is one flat \
              colour. The oracle blends ink and paper by the pattern's coverage — `darkGray` \
              becomes `#668dd9` — and a gradient into its midpoint; this import carries no \
              background and counts `Appearance::PatternFill` or `GradientFill`, which is the \
              manifest's own instruction (\"dropped and counted, not approximated by its \
              foreground\"). `styles/fills.xlsx` B4–B21.",
        scope: Scope::Style(|ours, theirs| {
            ours.background.is_none()
                && theirs.background.is_some()
                && CellStyle {
                    background: None,
                    ..theirs.clone()
                } == *ours
        }),
    },
    Divergence {
        name: "the oracle aligns rotated text",
        why: "`textRotation` with no `horizontal`: the oracle adds `fo:text-align` by the \
              angle — `start` at 45° and 180°, `end` at 90°, 135° and stacked — where the file \
              says `general`, which this import carries as no alignment beside the angle. \
              `styles/alignment.xlsx` B19–B23.",
        scope: Scope::Style(|ours, theirs| {
            ours.align.is_none()
                && matches!(theirs.align.as_deref(), Some("start" | "end"))
                && CellStyle {
                    align: None,
                    ..theirs.clone()
                } == *ours
        }),
    },
    Divergence {
        name: "the oracle justifies vertically, which ODF cannot say",
        why: "`vertical=\"justify\"` and `\"distributed\"` arrive in the oracle's output as \
              `style:vertical-align=\"justify\"` — not a value ODF's cell vertical alignment \
              has (top, middle, bottom, automatic; OpenDocument 1.4 schema) — and with wrapping \
              switched on. This import counts `Appearance::VerticalJustify` and writes neither. \
              `styles/alignment.xlsx` B13 and B14.",
        scope: Scope::Style(|ours, theirs| {
            theirs.vertical_align.as_deref() == Some("justify")
                && CellStyle {
                    vertical_align: None,
                    wrap: None,
                    ..theirs.clone()
                } == *ours
        }),
    },
    Divergence {
        name: "a workbook with no Normal style gets the oracle's own 10pt",
        why: "`styles/named-styles.xlsx` declares cell styles `Heading 1` and `Note` and no \
              `Normal`, so the oracle's `Default` cell style keeps LibreOffice's own 10pt, and \
              each automatic style spells `11pt` — the size a font with no `<sz>` has — against \
              it. This import takes the workbook's default font as the document's default \
              whatever the cell styles are called, and that font is 11pt, so the same cells \
              carry no size. Both say the cell is set in 11pt.",
        scope: Scope::Style(|ours, theirs| {
            ours.font_size.is_none()
                && theirs.font_size.as_deref() == Some("11pt")
                && CellStyle {
                    font_size: None,
                    ..theirs.clone()
                } == *ours
        }),
    },
    Divergence {
        name: "the oracle frames a pivot table in its own borders",
        why: "The pivot table's output is regenerated by the oracle (the divergence above), and \
              regenerated with LibreOffice's own frame round it: borders 2.01pt and 0.99pt wide \
              — widths no Excel border style becomes, since those are 0.06, 0.74, 1.76 and \
              2.49pt (`doc/xlsx-format.md` §4.3) — and in its own pivot cell styles, which a \
              reader following `style:parent-style-name` sees: a bold heading and labels set \
              at the start. This import carries the cells the workbook holds, unstyled, as the \
              workbook styles them.",
        scope: Scope::Style(|ours, theirs| {
            ours.is_plain()
                && matches!(theirs.font_weight.as_deref(), None | Some("bold"))
                && matches!(theirs.align.as_deref(), None | Some("start"))
                && CellStyle {
                    borders: Default::default(),
                    font_weight: None,
                    align: None,
                    ..theirs.clone()
                }
                .is_plain()
                && theirs.borders.iter().flatten().all(|edge| {
                    style::border_parts(edge).is_some_and(|(width, _, _)| {
                        [0.99, 2.01].iter().any(|w| (width - w).abs() < 0.005)
                    })
                })
        }),
    },
    // ---- geometry ----
    Divergence {
        name: "the oracle measures a column in the font it substituted",
        why: "ECMA-376 §18.3.1.13 counts a column's width in the maximum digit width of the \
              workbook's default font. This import takes that to be Calibri 11's seven pixels at \
              96 dpi, the specification's own example (`grind_xlsx::sheet::DIGIT_PX`); the \
              oracle measures the digit in whatever font the converting machine substitutes \
              for the default — DejaVu Sans on the pinned image, whose digit at 11pt is 7.0 \
              *points* — which makes every width four-thirds of this import's. Measured \
              2026-09-21 by changing the fixture's default font and watching the oracle's \
              widths follow it (`doc/xlsx-format.md` §4.5): its answer is a fact about the \
              machine, which is id 14's date-order problem again.",
        scope: Scope::Track(|track| {
            track.axis == Axis::Column
                && matches!(track.what, TrackDifference::Size { ours, theirs: Some(theirs) }
                    if (theirs / ours - 4.0 / 3.0).abs() < 0.005)
        }),
    },
    Divergence {
        name: "the oracle's indent step depends on the converting machine",
        why: "`indent=\"n\"` has no length in ECMA-376 beyond \"n steps\". The oracle converted \
              one step to 2.65mm on a desktop, to 2.54mm under this loop's profile there, and to \
              3.70mm on the pinned image (`doc/xlsx-format.md` §4.14) — a step measured, like a \
              column's width, in whatever font the machine has. This import takes the first, \
              and the indent is the only property that differs. `styles/alignment.xlsx`.",
        scope: Scope::Style(|ours, theirs| {
            let (Some(a), Some(b)) = (ours.indent_mm(), theirs.indent_mm()) else {
                return false;
            };
            (0.9..1.5).contains(&(b / a))
                && CellStyle {
                    indent: None,
                    ..theirs.clone()
                } == CellStyle {
                    indent: None,
                    ..ours.clone()
                }
        }),
    },
    Divergence {
        name: "a zero-height row is shown by the oracle",
        why: "`geometry/rows.xlsx` row 8 is `ht=\"0\" customHeight=\"1\"` and not hidden: \
              invisible in Excel. ODF's row height is a positive length, so this import carries \
              the row hidden and counts `Appearance::ZeroSize`; the oracle ignores the height \
              and shows the row at the default.",
        scope: Scope::Track(|track| {
            track.axis == Axis::Row
                && matches!(
                    track.what,
                    TrackDifference::Hidden {
                        ours: true,
                        theirs: false
                    }
                )
        }),
    },
    Divergence {
        name: "a sheet whose name the oracle does not allow is dropped",
        why: "`document/sheets.xlsx` has ten sheets and the pinned oracle's conversion has \
              eight: `Has[Brackets]` and `Has/Slash` are gone, cells and all (LibreOffice 26.8 \
              keeps them as `Sheet9` and `Sheet10` instead, with their cells). This import keeps \
              both, renamed `Has_Brackets_` and `Has_Slash` with every reference following \
              (X5), so the sheet is matched here by the workbook's own spelling.",
        scope: Scope::OnlyOurs(|name| name.contains(['[', ']', '/', '\\', '?', '*', ':'])),
    },
    Divergence {
        name: "an external link's cache becomes a sheet",
        why: "The oracle keeps the cached values of an external workbook link in a sheet of \
              its own, named `'file:///…'#Sheet1`. It is not a sheet of the workbook; this \
              import follows no link and counts it as dropped (X5).",
        scope: Scope::OnlyTheirs(|name| name.starts_with("'file:")),
    },
    Divergence {
        name: "a part reached through a backslash is not found",
        why: "`realworld/part-targets.xlsx` reaches its third sheet through a relationship \
              target written with `\\`. The oracle's conversion has the sheet, by name, with \
              nothing in it; this import normalises the separator (`package.rs`) and reads the \
              cell. ECMA-376 Part 2 does not allow `\\` in a part name, and real producers \
              write it anyway.",
        scope: Scope::Unreached,
    },
];

/// One side's view of one cell.
#[derive(Clone, Debug, PartialEq)]
struct Cell {
    value: CellValue,
    kind: Option<NumberKind>,
    /// Whether the cell holds a formula — ours except where a named class stopped it, the oracle's
    /// wherever the workbook had one. Read to *exclude* such cells from the value comparison: the
    /// oracle recalculates them and this import does not.
    formula: bool,
    formatted: bool,
    display: String,
    /// The cell's style after [`normalise`] — `None` where it looks like the document's
    /// default. Compared only where both sides carry a value: the ODF reader drops a styled
    /// empty cell (its `TODO:`), so the oracle's blanks arrive unstyled whatever it wrote.
    style: Option<CellStyle>,
}

fn cell(sheet: &Sheet, pos: Pos, null_date: i64, defaults: &Defaults) -> Cell {
    let value = sheet.get(pos);
    Cell {
        style: match value {
            CellValue::Empty => None,
            _ => normalise(sheet.style(pos), defaults),
        },
        value,
        kind: sheet.kind(pos),
        formula: sheet.formula(pos).is_some(),
        formatted: sheet.format(pos).is_some(),
        display: grind_sheet::render(sheet, pos, null_date),
    }
}

/// What a document's default cell looks like, as far as the comparison needs: the size and
/// colour its `Default` cell style carries. Ours has none — the model has no document default
/// (`odf/read.rs`'s ponytail on `style:default-style`) — and the oracle's is read out of its
/// output here, because the oracle **spells the default out on every automatic style**
/// (`fo:font-size="11pt"` on each of them, measured 2026-09-21), and a size equal to the
/// document's own default is the absence of one.
#[derive(Default)]
struct Defaults {
    size: Option<String>,
    color: Option<String>,
}

impl Defaults {
    /// The `Default` table-cell style's attributes, read out of the `.fods` text. A substring
    /// search rather than a parse, because the style is one element whose shape the oracle
    /// writes the same way every time, and our own reader deliberately does not keep it.
    fn of(fods: &str) -> Self {
        // What `Default` leaves unsaid, the default style says.
        let fallback = Self::in_element(fods, r#"<style:default-style style:family="table-cell""#);
        let own = Self::in_element(fods, r#"style:name="Default" style:family="table-cell""#);
        Defaults {
            size: own.size.or(fallback.size),
            color: own.color.or(fallback.color),
        }
    }

    fn in_element(fods: &str, opening: &str) -> Self {
        let Some(start) = fods.find(opening) else {
            return Defaults::default();
        };
        let element = &fods[start..];
        // A self-closed `<style:style …/>` carries nothing, and reading on to the next
        // `</style:style>` would take another style's attributes for this one's.
        let open = &element[..element.find('>').unwrap_or(element.len())];
        if open.ends_with('/') {
            return Defaults::default();
        }
        let element = &element[..element.find("</style:style>").unwrap_or(element.len())];
        let attr = |name: &str| {
            let at = element.find(&format!(" {name}=\""))? + name.len() + 3;
            Some(element[at..at + element[at..].find('"')?].to_owned())
        };
        Defaults {
            size: attr("fo:font-size"),
            color: attr("fo:color"),
        }
    }
}

/// A style with every property that says "the default" taken out, so that the two sides are
/// compared on what they *mean*. The oracle writes `fo:font-weight="normal"`,
/// `fo:font-style="normal"`, `fo:wrap-option="no-wrap"` and `style:vertical-align="bottom"`
/// onto styles that set none of them, where this import writes nothing; both are the same cell.
fn normalise(style: Option<&CellStyle>, defaults: &Defaults) -> Option<CellStyle> {
    let mut s = style?.clone();
    let clear = |field: &mut Option<String>, plain: &[&str]| {
        if field.as_deref().is_some_and(|v| plain.contains(&v)) {
            *field = None;
        }
    };
    clear(&mut s.font_weight, &["normal"]);
    clear(&mut s.font_style, &["normal"]);
    clear(&mut s.wrap, &["no-wrap"]);
    clear(&mut s.vertical_align, &["bottom", "automatic"]);
    clear(&mut s.background, &["transparent"]);
    if s.font_size.is_some() && s.font_size == defaults.size {
        s.font_size = None;
    }
    if s.color.is_some() && s.color == defaults.color {
        s.color = None;
    }
    (!s.is_plain()).then_some(s)
}

/// Loop C's rule for two styles (`sheet/tests/roundtrip.rs`'s `same_style`): a border's width
/// numerically, since LibreOffice re-quantises it, and everything else exactly.
fn same_style(a: &Option<CellStyle>, b: &Option<CellStyle>) -> bool {
    let (Some(a), Some(b)) = (a, b) else {
        return a.is_none() && b.is_none();
    };
    let borders = a.borders.iter().zip(&b.borders).all(|(a, b)| {
        match (
            a.as_deref().and_then(style::border_parts),
            b.as_deref().and_then(style::border_parts),
        ) {
            (Some((wa, sa, ca)), Some((wb, sb, cb))) => {
                (wa - wb).abs() < 0.05 && sa == sb && ca == cb
            }
            _ => a == b,
        }
    });
    // An indent is a length the oracle spells in the converting machine's unit (`0.1in` for
    // `0.265cm`), so it is compared as one.
    // A tenth of a millimetre either way: the same step converted on a cm machine came back
    // `0.265cm` and on an inch machine `0.1in`, which is 0.11mm apart.
    let indents = match (a.indent_mm(), b.indent_mm()) {
        (Some(x), Some(y)) => (x - y).abs() < 0.15,
        (x, y) => x == y,
    };
    let bare = |s: &CellStyle| CellStyle {
        borders: Default::default(),
        indent: None,
        ..s.clone()
    };
    borders && indents && bare(a) == bare(b)
}

/// Are these the same style except where `colour` excuses a border colour — with the oracle's
/// own line names read as the XSL-FO ones this import writes? The divergences about borders
/// are built on it, so that each excuses exactly one thing.
fn same_but(ours: &CellStyle, theirs: &CellStyle, colour: fn(&str, &str) -> bool) -> bool {
    fn xsl(line: &str) -> &str {
        match line {
            "fine-dashed" | "dash-dot" | "dash-dot-dot" => "dashed",
            "double-thin" => "double",
            other => other,
        }
    }
    let edges = ours.borders.iter().zip(&theirs.borders).all(|(a, b)| {
        match (
            a.as_deref().and_then(style::border_parts),
            b.as_deref().and_then(style::border_parts),
        ) {
            (Some((wa, la, ca)), Some((wb, lb, cb))) => {
                (wa - wb).abs() < 0.05 && la == xsl(lb) && colour(ca, cb)
            }
            _ => a == b,
        }
    });
    let bare = |s: &CellStyle| CellStyle {
        borders: Default::default(),
        ..s.clone()
    };
    edges && bare(ours) == bare(theirs)
}

/// One column or row on which the two conversions disagree.
#[derive(Debug)]
struct Track {
    axis: Axis,
    index: u32,
    what: TrackDifference,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Axis {
    Column,
    Row,
}

#[derive(Debug)]
enum TrackDifference {
    /// A size ours carries, in millimetres, against the oracle's — which it has for every
    /// track, since it measures the ones nobody sized.
    Size {
        ours: f64,
        theirs: Option<f64>,
    },
    Hidden {
        ours: bool,
        theirs: bool,
    },
}

/// Loop C's rule, restated: equal at 15 significant digits (`sheet/tests/roundtrip.rs`'s
/// `same`, which has the arithmetic for why 1e-14 is that), everything else exactly.
fn same(a: &Cell, b: &Cell) -> bool {
    let values = match (&a.value, &b.value) {
        (CellValue::Number(x), CellValue::Number(y)) => {
            x == y || (x - y).abs() <= 1e-14 * x.abs().max(y.abs())
        }
        (x, y) => x == y,
    };
    // The display is compared only where **both** sides format the cell: an unformatted one had
    // its code refused by a named class, asserted in `ooxmlgen.rs`, and the oracle would say the
    // same in a worse vocabulary — `"3.75" vs "3 3/4"` rather than `Fraction`.
    let display = !(a.formatted && b.formatted) || a.display == b.display;
    values && a.kind == b.kind && display
}

/// One way the two conversions differ.
enum Difference {
    Cell(String, Pos, Cell, Cell),
    Style(String, Pos, Option<CellStyle>, Option<CellStyle>),
    Track(String, Track),
    OnlyOurs(String),
    OnlyTheirs(String),
    Unreached(String),
}

impl Difference {
    fn explained_by(&self, divergence: &Divergence) -> bool {
        match (self, &divergence.scope) {
            (Difference::Cell(_, _, a, b), Scope::Cell(applies)) => applies(a, b),
            (Difference::Style(_, _, a, b), Scope::Style(applies)) => {
                let plain = CellStyle::default();
                applies(a.as_ref().unwrap_or(&plain), b.as_ref().unwrap_or(&plain))
            }
            (Difference::Track(_, track), Scope::Track(applies)) => applies(track),
            (Difference::OnlyOurs(name), Scope::OnlyOurs(applies)) => applies(name),
            (Difference::OnlyTheirs(name), Scope::OnlyTheirs(applies)) => applies(name),
            (Difference::Unreached(_), Scope::Unreached) => true,
            _ => false,
        }
    }

    fn describe(&self) -> String {
        match self {
            Difference::Cell(sheet, pos, a, b) => format!(
                "{sheet}!{} is {:?} {:?}, the oracle's {:?} {:?}",
                grind_sheet::a1::format(None, *pos),
                a.value,
                a.kind,
                b.value,
                b.kind
            ),
            Difference::Style(sheet, pos, a, b) => format!(
                "{sheet}!{} is styled {a:?}, the oracle's {b:?}",
                grind_sheet::a1::format(None, *pos),
            ),
            Difference::Track(sheet, track) => format!(
                "{sheet}: {} {} is {:?}",
                match track.axis {
                    Axis::Column => "column",
                    Axis::Row => "row",
                },
                track.index + 1,
                track.what
            ),
            Difference::OnlyOurs(name) => format!("sheet {name:?} is not in the oracle's"),
            Difference::OnlyTheirs(name) => format!("sheet {name:?} is only in the oracle's"),
            Difference::Unreached(name) => {
                format!("sheet {name:?} holds nothing in the oracle's and cells in ours")
            }
        }
    }
}

/// Every way the two documents disagree. Sheets are matched **by name** — the oracle drops
/// some and invents others, and matching by position would then compare the wrong pairs.
///
/// Cells are walked over the rows either side *carries* rather than over the used rectangle,
/// which for `scale/wide-and-sparse.xlsx` is the whole grid — seventeen billion cells.
fn differences(
    ours: &Document,
    theirs: &Document,
    defaults: &Defaults,
    renamed: &[(String, String)],
) -> Vec<Difference> {
    differences_with(ours, theirs, (&Defaults::default(), defaults), renamed)
}

/// [`differences`], with what each side's default cell looks like.
fn differences_with(
    ours: &Document,
    theirs: &Document,
    (own, defaults): (&Defaults, &Defaults),
    renamed: &[(String, String)],
) -> Vec<Difference> {
    let mut out = Vec::new();
    for other in &theirs.sheets {
        if !ours.sheets.iter().any(|s| s.name == other.name) {
            out.push(Difference::OnlyTheirs(other.name.clone()));
        }
    }
    for mine in &ours.sheets {
        let Some(other) = theirs.sheets.iter().find(|s| s.name == mine.name) else {
            let original = renamed
                .iter()
                .find(|(_, to)| *to == mine.name)
                .map_or(&mine.name, |(from, _)| from);
            out.push(Difference::OnlyOurs(original.clone()));
            continue;
        };
        if other.rows_carrying().is_empty() && !mine.rows_carrying().is_empty() {
            out.push(Difference::Unreached(mine.name.clone()));
            continue;
        }
        let cols = mine.used_cols().max(other.used_cols());
        let mut rows: Vec<u32> = mine
            .rows_carrying()
            .into_iter()
            .chain(other.rows_carrying())
            .flatten()
            .collect();
        rows.sort_unstable();
        rows.dedup();
        for row in rows {
            for col in 0..cols {
                let pos = Pos::new(row, col);
                let (a, b) = (
                    cell(mine, pos, ours.null_date, own),
                    cell(other, pos, theirs.null_date, defaults),
                );
                if !same_style(&a.style, &b.style) {
                    out.push(Difference::Style(
                        mine.name.clone(),
                        pos,
                        a.style.clone(),
                        b.style.clone(),
                    ));
                }
                if !same(&a, &b) {
                    out.push(Difference::Cell(mine.name.clone(), pos, a, b));
                }
            }
        }
        for track in tracks(mine, other) {
            out.push(Difference::Track(mine.name.clone(), track));
        }
    }
    out
}

/// The columns and rows on which two sheets disagree. A size is compared where **ours** carries
/// one — the oracle carries one for every track, measuring unsized rows in its own fonts — at loop
/// C's tolerance of a tenth of a millimetre; hidden-ness everywhere.
fn tracks(mine: &Sheet, other: &Sheet) -> Vec<Track> {
    let mut out = Vec::new();
    let mm = |s: Option<&str>| s.and_then(style::length_mm);
    let mut size = |axis, index, ours: Option<&str>, theirs: Option<&str>| {
        if let Some(ours) = mm(ours) {
            let theirs = mm(theirs);
            if theirs.is_none_or(|t| (t - ours).abs() >= 0.1) {
                out.push(Track {
                    axis,
                    index,
                    what: TrackDifference::Size { ours, theirs },
                });
            }
        }
    };
    for (col, width) in mine.col_widths() {
        size(Axis::Column, col, Some(width), other.col_width(col));
    }
    // A measured height is Excel's measurement of the row, and the oracle's another in its own
    // fonts (`doc/xlsx-format.md` §4.6): two measurements, not a size either chose.
    for (row, height) in mine
        .row_heights()
        .filter(|(row, _)| !mine.row_measured(*row))
    {
        size(Axis::Row, row, Some(height), other.row_height(row));
    }
    let hidden = |axis, a: Vec<u32>, b: Vec<u32>, out: &mut Vec<Track>| {
        for index in a
            .iter()
            .chain(&b)
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
        {
            let (ours, theirs) = (a.contains(&index), b.contains(&index));
            if ours != theirs {
                out.push(Track {
                    axis,
                    index,
                    what: TrackDifference::Hidden { ours, theirs },
                });
            }
        }
    };
    hidden(
        Axis::Column,
        mine.hidden_cols().collect(),
        other.hidden_cols().collect(),
        &mut out,
    );
    hidden(
        Axis::Row,
        mine.manually_hidden_rows().collect(),
        other.manually_hidden_rows().collect(),
        &mut out,
    );
    out
}

/// `3/17/2024 18:00` as `2024-03-17 18:00` — the oracle's en-US date spelling in the ISO one,
/// so that the divergence above can say *this is the same day* rather than merely *these are
/// two strings*. Anything that is not month/day/year comes back unchanged and so matches
/// nothing.
fn iso_for(display: &str) -> String {
    let (date, rest) = match display.split_once(' ') {
        Some((date, rest)) => (date, format!(" {rest}")),
        None => (display, String::new()),
    };
    let parts: Vec<&str> = date.split('/').collect();
    let [month, day, year] = parts[..] else {
        return display.to_owned();
    };
    match (
        month.parse::<u32>(),
        day.parse::<u32>(),
        year.parse::<i32>(),
    ) {
        (Ok(m), Ok(d), Ok(y)) => format!("{y:04}-{m:02}-{d:02}{rest}"),
        _ => display.to_owned(),
    }
}

// ---- the oracle ----

fn soffice_version() -> Option<String> {
    let out = Command::new("soffice").arg("--version").output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// Where one oracle build's answers are kept: under the **system temp directory**, named by the
/// oracle's version string with everything that is not safe in a path replaced.
///
/// The temp directory rather than `CARGO_TARGET_TMPDIR` because it is the one directory
/// `scripts/soffice-docker/soffice` mounts into the pinned oracle's container — loops C and E
/// stage there for the same reason, and a cache the container cannot see is a loop that
/// passes on a laptop and cannot run in CI.
fn cache_dir(version: &str) -> PathBuf {
    let safe: String = version
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    std::env::temp_dir().join("grind-loop-d").join(safe)
}

/// Convert whatever is not cached yet, in **one** `soffice` invocation — startup dominates, a
/// couple of seconds each time against milliseconds per document once it is up. The private
/// `UserInstallation` is not optional: without it this fights the developer's own running
/// LibreOffice for the profile lock and either blocks or silently does nothing. False when the
/// oracle failed on any of them.
fn convert(cache: &Path, pending: &[(String, PathBuf)]) -> bool {
    if pending.is_empty() {
        return true;
    }
    let stage = cache.join("stage");
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir_all(&stage).unwrap();
    // Staged under their digest, so two fixtures called `sheets.xlsx` in different families
    // cannot overwrite each other's output.
    let inputs: Vec<PathBuf> = pending
        .iter()
        .map(|(digest, source)| {
            let ext = source
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("xlsx");
            let staged = stage.join(format!("{digest}.{ext}"));
            std::fs::copy(source, &staged).unwrap();
            staged
        })
        .collect();
    let status = Command::new("soffice")
        .arg("--headless")
        .arg(format!(
            "-env:UserInstallation=file://{}",
            cache.join("profile").display()
        ))
        .args(["--convert-to", "fods", "--outdir"])
        .arg(cache)
        .args(&inputs)
        .status()
        .expect("soffice failed to start");
    let _ = std::fs::remove_dir_all(&stage);
    status.success()
}

// ---- the corpus ----

struct Fixture {
    file: String,
    sha256: String,
}

fn corpus() -> (PathBuf, Vec<Fixture>) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/corpus");
    let text = std::fs::read_to_string(root.join("manifest.json")).expect("the manifest");
    let json: serde_json::Value = serde_json::from_str(&text).expect("the manifest is JSON");
    let fixtures = json["fixtures"]
        .as_array()
        .expect("a fixtures array")
        .iter()
        // A fixture the oracle cannot open has no answer to compare against, and one that
        // must be refused has no document.
        .filter(|f| f["oracleOpens"].as_bool() == Some(true) && f.get("error").is_none())
        .map(|f| Fixture {
            file: f["file"].as_str().unwrap().to_owned(),
            sha256: f["sha256"].as_str().unwrap().to_owned(),
        })
        .collect();
    (root, fixtures)
}

#[test]
fn our_import_agrees_with_the_oracles() {
    let Some(version) = soffice_version() else {
        eprintln!("skipping loop D: no soffice on PATH");
        return;
    };
    let cache = cache_dir(&version);
    std::fs::create_dir_all(&cache).unwrap();

    let (root, fixtures) = corpus();
    let pending: Vec<(String, PathBuf)> = fixtures
        .iter()
        .filter(|f| !cache.join(format!("{}.fods", f.sha256)).exists())
        .map(|f| (f.sha256.clone(), root.join(&f.file)))
        .collect();
    let converted_now = pending.len();
    assert!(convert(&cache, &pending), "soffice failed");

    let mut compared = 0usize;
    let mut cells = 0usize;
    let mut failures = Vec::new();
    let mut matched: BTreeMap<&str, usize> = BTreeMap::new();
    for fixture in &fixtures {
        let theirs_path = cache.join(format!("{}.fods", fixture.sha256));
        let Ok(theirs) = grind_sheet::read_file(&theirs_path) else {
            failures.push(format!(
                "{}: the oracle produced nothing we can read",
                fixture.file
            ));
            continue;
        };
        let bytes = std::fs::read(root.join(&fixture.file)).unwrap();
        let (ours, report) = match grind_xlsx::import_bytes(&bytes) {
            Ok(pair) => pair,
            Err(e) => {
                failures.push(format!(
                    "{}: the oracle opens it and we do not: {e}",
                    fixture.file
                ));
                continue;
            }
        };
        compared += 1;
        cells += report.cells;

        let defaults = Defaults::of(&std::fs::read_to_string(&theirs_path).unwrap_or_default());
        for difference in differences(&ours, &theirs, &defaults, &report.renamed) {
            match DIVERGENCES.iter().find(|d| difference.explained_by(d)) {
                Some(d) => *matched.entry(d.name).or_default() += 1,
                None => failures.push(format!("{}: {}", fixture.file, difference.describe())),
            }
        }
    }

    // The other direction: a divergence nothing matched any more is one LibreOffice fixed, or
    // a predicate that was never right.
    for d in DIVERGENCES {
        if !matched.contains_key(d.name) {
            failures.push(format!(
                "divergence `{}` matched no cell — delete it if the oracle changed ({})",
                d.name, d.why
            ));
        }
    }

    eprintln!(
        "loop D: {version}; {compared} workbooks, {cells} cells, {} disagreements, \
         {} named divergences ({converted_now} converted now, the rest cached)",
        failures.len(),
        matched.values().sum::<usize>(),
    );
    for (name, count) in &matched {
        eprintln!("  divergence ×{count}: {name}");
    }
    for failure in failures.iter().take(100) {
        eprintln!("  {failure}");
    }
    if failures.len() > 100 {
        eprintln!("  ... and {} more", failures.len() - 100);
    }
    assert!(
        compared >= FLOOR,
        "{compared} workbooks reached the comparison, fewer than the ratchet's {FLOOR}"
    );
    assert!(
        failures.is_empty(),
        "{} disagreements with the oracle",
        failures.len()
    );
}

/// Loop D over a directory of workbooks from elsewhere — `GRIND_LOOP_D_CORPUS=<dir>`, skipped
/// without it. Nothing there has a manifest, so this asserts only that every workbook the oracle
/// converts imports, and prints the disagreements by class, most widespread first.
///
/// GRIND_LOOP_D_CORPUS=~/samples cargo test -p grind-xlsx --release --test loop_d outside -- --nocapture
#[test]
fn outside_corpus_against_the_oracle() {
    let Some(dir) = std::env::var_os("GRIND_LOOP_D_CORPUS") else {
        return;
    };
    let Some(version) = soffice_version() else {
        eprintln!("skipping loop D outside: no soffice on PATH");
        return;
    };
    let cache = cache_dir(&version);
    std::fs::create_dir_all(&cache).unwrap();

    let mut files = Vec::new();
    let mut stack = vec![PathBuf::from(dir)];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
                ["xlsx", "xlsm", "ods", "fods"]
                    .iter()
                    .any(|x| e.eq_ignore_ascii_case(x))
            }) {
                files.push(path);
            }
        }
    }
    files.sort();
    let digests: Vec<(String, PathBuf)> = files
        .iter()
        .filter_map(|path| Some((sha256(path)?, path.clone())))
        .collect();
    let pending: Vec<(String, PathBuf)> = digests
        .iter()
        .filter(|(digest, _)| {
            !cache.join(format!("{digest}.fods")).exists()
                && !cache.join(format!("{digest}.unopened")).exists()
        })
        .cloned()
        .collect();
    // A workbook the oracle cannot open is remembered as such, rather than retried every run.
    for chunk in pending.chunks(20) {
        convert(&cache, chunk);
        for (digest, _) in chunk {
            if !cache.join(format!("{digest}.fods")).exists() {
                let _ = std::fs::write(cache.join(format!("{digest}.unopened")), "");
            }
        }
    }

    let mut refused = Vec::new();
    let mut compared = 0usize;
    // class → (files, instances, examples)
    let mut classes: BTreeMap<String, (std::collections::BTreeSet<String>, usize, Vec<String>)> =
        BTreeMap::new();
    let mut matched: BTreeMap<&str, usize> = BTreeMap::new();
    // Files shown per class, one example each: `GRIND_LOOP_D_EXAMPLES`, four by default.
    let examples: usize = std::env::var("GRIND_LOOP_D_EXAMPLES")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(4);
    for (digest, path) in &digests {
        let theirs_path = cache.join(format!("{digest}.fods"));
        let Ok(theirs) = grind_sheet::read_file(&theirs_path) else {
            continue;
        };
        let file = path.display().to_string();
        let bytes = std::fs::read(path).unwrap();
        // An ODF file is read as it is: the comparison is then our reading of it against our
        // reading of the oracle's re-save, which states everything it inherited.
        let odf = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("ods") || e.eq_ignore_ascii_case("fods"));
        let read = match odf {
            true => grind_sheet::read_bytes(&file, &bytes)
                .map(|doc| (doc, Vec::new()))
                .map_err(|e| e.to_string()),
            false => grind_xlsx::import_bytes(&bytes)
                .map(|(doc, report)| (doc, report.renamed))
                .map_err(|e| e.to_string()),
        };
        let (ours, renamed) = match read {
            Ok(pair) => pair,
            Err(e) => {
                refused.push(format!("{file}: {e}"));
                continue;
            }
        };
        compared += 1;
        let defaults = Defaults::of(&std::fs::read_to_string(&theirs_path).unwrap_or_default());
        let mine = match odf {
            true => Defaults::of(&std::fs::read_to_string(path).unwrap_or_default()),
            false => Defaults::default(),
        };
        for difference in differences_with(&ours, &theirs, (&mine, &defaults), &renamed) {
            if let Some(d) = DIVERGENCES.iter().find(|d| difference.explained_by(d)) {
                *matched.entry(d.name).or_default() += 1;
                continue;
            }
            let entry = classes.entry(difference.class()).or_default();
            entry.0.insert(file.clone());
            entry.1 += 1;
            if entry.2.len() < examples && !entry.2.iter().any(|e| e.starts_with(&file)) {
                entry.2.push(format!("{file}: {}", difference.describe()));
            }
        }
    }

    eprintln!("loop D outside: {version}; {compared} workbooks compared");
    for (name, count) in &matched {
        eprintln!("  divergence ×{count}: {name}");
    }
    let mut ranked: Vec<_> = classes.into_iter().collect();
    ranked.sort_by_key(|(_, (files, n, _))| std::cmp::Reverse((files.len(), *n)));
    for (class, (files, n, examples)) in &ranked {
        eprintln!("{class}: {} files, {n} times", files.len());
        for example in examples {
            eprintln!("    {example}");
        }
    }
    for line in &refused {
        eprintln!("  refused: {line}");
    }
    assert!(
        refused.is_empty(),
        "{} workbooks the oracle opens and we do not",
        refused.len()
    );
}

/// The cache's key, from `sha256sum` rather than a dependency for a loop that runs on request.
fn sha256(path: &Path) -> Option<String> {
    let out = Command::new("sha256sum").arg(path).output().ok()?;
    let text = String::from_utf8(out.stdout).ok()?;
    Some(text.split_whitespace().next()?.to_owned())
}

impl Difference {
    /// A coarse name for what kind of disagreement this is, so a corpus's thousands of them
    /// can be counted by cause.
    fn class(&self) -> String {
        match self {
            Difference::Cell(_, _, a, b) => {
                if a.value != b.value {
                    let shape = |v: &CellValue| match v {
                        CellValue::Empty => "empty",
                        CellValue::Number(_) => "number",
                        CellValue::Text(_) => "text",
                        CellValue::Bool(_) => "bool",
                    };
                    format!(
                        "cell value: ours {} theirs {}{}",
                        shape(&a.value),
                        shape(&b.value),
                        if b.formula { " (formula)" } else { "" }
                    )
                } else if a.kind != b.kind {
                    format!("cell kind: ours {:?} theirs {:?}", a.kind, b.kind)
                } else {
                    "cell display".to_owned()
                }
            }
            Difference::Style(_, _, a, b) => {
                let plain = CellStyle::default();
                let (a, b) = (a.as_ref().unwrap_or(&plain), b.as_ref().unwrap_or(&plain));
                let mut fields = Vec::new();
                macro_rules! field {
                    ($($f:ident),*) => {$(
                        if a.$f != b.$f { fields.push(stringify!($f)); }
                    )*};
                }
                field!(
                    font_weight,
                    font_style,
                    font_size,
                    color,
                    background,
                    align,
                    vertical_align,
                    wrap,
                    underline,
                    line_through,
                    indent,
                    rotation,
                    borders
                );
                format!("style: {}", fields.join(","))
            }
            Difference::Track(_, track) => match (&track.axis, &track.what) {
                (axis, TrackDifference::Size { .. }) => format!("{axis:?} size"),
                (axis, TrackDifference::Hidden { ours, .. }) => {
                    format!("{axis:?} hidden: ours {ours}")
                }
            },
            Difference::OnlyOurs(_) => "sheet only ours".to_owned(),
            Difference::OnlyTheirs(_) => "sheet only theirs".to_owned(),
            Difference::Unreached(_) => "sheet unreached".to_owned(),
        }
    }
}
