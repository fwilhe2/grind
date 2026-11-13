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
//! column's and row's size and hidden-ness** (`oracle::differences`). A display is compared where
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
//! The comparison and the runs are `oracle/mod.rs`'s, shared with `loop_d_euro_office.rs`.
//!
//! cargo test -p grind-xlsx --test `loop_d` -- --nocapture

mod oracle;

use std::path::{Path, PathBuf};
use std::process::Command;

use grind_sheet::model::{CellValue, NumberKind};
use grind_sheet::style::{self, CellStyle};
use oracle::{Axis, Divergence, Oracle, Scope, TrackDifference, same_but};

/// How many fixtures must reach the comparison. The ratchet, as in every loop: raise it,
/// never lower it. 69 of the 76 import; the seven that do not are refused on purpose.
const FLOOR: usize = 60;

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

const LIBREOFFICE: Oracle = Oracle {
    name: "LibreOffice",
    cache: "grind-loop-d",
    output: "fods",
    version: soffice_version,
    convert,
    unconverted: &[],
    divergences: DIVERGENCES,
};

fn soffice_version() -> Option<String> {
    let out = Command::new("soffice").arg("--version").output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// Convert whatever is not cached yet, in **one** `soffice` invocation — startup dominates, a
/// couple of seconds each time against milliseconds per document once it is up. The private
/// `UserInstallation` is not optional: without it this fights the developer's own running
/// LibreOffice for the profile lock and either blocks or silently does nothing.
fn convert(cache: &Path, pending: &[(String, PathBuf)]) {
    if pending.is_empty() {
        return;
    }
    let (stage, inputs) = oracle::stage(cache, pending);
    Command::new("soffice")
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
}

#[test]
fn our_import_agrees_with_the_oracles() {
    oracle::run(&LIBREOFFICE, FLOOR);
}

/// `GRIND_LOOP_D_CORPUS`=~/samples cargo test -p grind-xlsx --release --test `loop_d` outside -- --nocapture
#[test]
fn outside_corpus_against_the_oracle() {
    oracle::run_outside(&LIBREOFFICE);
}
