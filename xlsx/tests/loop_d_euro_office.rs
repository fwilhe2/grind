// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Loop D against a second oracle, **Euro-Office**: the same comparison as `loop_d.rs`, with
//! Euro-Office's converter in LibreOffice's place.
//!
//! ```text
//! ours   = grind_xlsx::import_bytes(bytes)                → Document
//! theirs = x2t --convert-to ods --outdir <cache> <file>   → read with our own reader → Document
//! compare(ours, theirs)
//! ```
//!
//! Two oracles that agree with each other and not with us is a strong signal; one that disagrees
//! with both is usually that oracle's own bug, recorded here as a [`Divergence`] like loop D's.
//!
//! **Needs `x2t` on `PATH`; skips with a notice without one.** `scripts/euro-office-docker/x2t` is
//! the converter out of the image pinned in `ci/euro-office-image`, and
//! `scripts/euro-office-tests.sh` puts it first on `PATH`.
//!
//! scripts/euro-office-tests.sh

mod oracle;

use std::path::{Path, PathBuf};
use std::process::Command;

use grind_sheet::model::{CellValue, NumberKind};
use grind_sheet::style::{self, CellStyle};
use oracle::{Axis, Cell, Divergence, Oracle, Scope, TrackDifference, same_but, same_value};

/// The ratchet: raise it, never lower it. 77 of the 78 reach the comparison.
const FLOOR: usize = 77;

/// Where Euro-Office's conversion differs from ours. Every entry was read in the oracle's `.ods`
/// on 2026-11-06, against the image in `ci/euro-office-image`, and in each LibreOffice agrees
/// with this import (`loop_d.rs`). **First match wins.**
const DIVERGENCES: &[Divergence] = &[
    // ---- values ----
    Divergence {
        name: "a time is written without a valid duration",
        why: "`office:time-value` is an `xsd:duration` (`PT06H00M00S`); the oracle writes \
              `T06:00:00`, and for midnight no `office:time-value` at all, only the text `0`. \
              Either way the cell carries no time ODF can read, so it arrives as text. \
              `values/times.xlsx`, `functions/datetime.xlsx`, `values/dates-1904.xlsx` B2.",
        scope: Scope::Cell(|ours, theirs| {
            matches!(ours.kind, Some(NumberKind::Time | NumberKind::Date))
                && matches!(&theirs.value, CellValue::Text(t) if t == "0"
                || t.strip_prefix('T').is_some_and(|rest| {
                    rest.chars().all(|c| c.is_ascii_digit() || c == ':')
                }))
        }),
    },
    Divergence {
        name: "a duration past one day is a date one day late",
        why: "`[h]:mm:ss` over 1.5 is a duration of a day and a half. The oracle writes it as \
              `office:date-value` counted from Excel's 1900-01-01 rather than ODF's null date, \
              which against `1899-12-30` is serial 2.5. `values/times.xlsx` rows 6 to 8.",
        scope: Scope::Cell(|ours, theirs| {
            ours.kind == Some(NumberKind::Time)
                && theirs.kind == Some(NumberKind::Date)
                && matches!((&ours.value, &theirs.value), (CellValue::Number(a), CellValue::Number(b))
                    if *a >= 1.0 && (b - a - 1.0).abs() < 1e-9)
        }),
    },
    Divergence {
        name: "the 1904 date system is read as the 1900 one",
        why: "`<workbookPr date1904=\"1\"/>` counts from 1904-01-01; the oracle counts the same \
              serials from 1900-01-01 and writes those dates, four years early (five across a \
              year end). `values/dates-1904.xlsx`.",
        scope: Scope::Cell(|ours, theirs| {
            let year = |c: &Cell| c.display.get(..4)?.parse::<i32>().ok();
            ours.kind == Some(NumberKind::Date)
                && theirs.kind == Some(NumberKind::Date)
                && matches!((year(ours), year(theirs)), (Some(a), Some(b)) if (4..=5).contains(&(a - b)))
        }),
    },
    Divergence {
        name: "a time format over a dated serial is a date",
        why: "`h:mm` over 45368.75 shows only the time, and the value still has its date. The \
              oracle writes it as `office:date-value` with a time style, which keeps the value \
              and makes the cell a date; this import keeps the format's kind. \
              `numfmt/builtins.xlsx`, `numfmt/datetime-codes.xlsx`, `values/date-vs-number.xlsx`.",
        scope: Scope::Cell(|ours, theirs| {
            ours.kind == Some(NumberKind::Time)
                && theirs.kind == Some(NumberKind::Date)
                && same_value(&ours.value, &theirs.value)
        }),
    },
    Divergence {
        name: "a date or time format is written as a number format, or not at all",
        why: "Formats of one date token (`yyyy`, `mmm`, `dddd`, `hh`, `ss.000`) and a \
              locale-prefixed one (`[$-407]TT.MM.JJJJ`) arrive as a cell style whose \
              `style:data-style-name` names a style the package never defines (`NF10001` in \
              `numfmt/datetime-codes.xlsx`), or as a plain number style. \
              `numfmt/datetime-codes.xlsx`, `numfmt/currency-locale.xlsx` B10.",
        scope: Scope::Cell(|ours, theirs| {
            matches!(ours.kind, Some(NumberKind::Date | NumberKind::Time))
                && theirs.kind.is_none()
                && same_value(&ours.value, &theirs.value)
        }),
    },
    Divergence {
        name: "a number format is written lossily",
        why: "The value and its kind agree and the oracle's number style says less than the \
              format did: only the first section survives (`#,##0 ;(#,##0)`, `[>100]…`), \
              literal text moves after the number (`\"total: \"0` as `0 total: `), a \
              currency symbol gains a space, `#` digits are dropped (`0.#` as `0`), `mm` after \
              a date is `number:minutes`, `AM/PM` and the `]` of `[h]` are literal text, and \
              `hh:mm:ss` after a date loses its leading zeros. Every one is in the oracle's \
              `number:*-style` element. `numfmt/*.xlsx`, `values/date-vs-number.xlsx` B4.",
        scope: Scope::Cell(|ours, theirs| {
            same_value(&ours.value, &theirs.value)
                && ours.kind == theirs.kind
                && ours.display != theirs.display
        }),
    },
    Divergence {
        name: "a number under the text format becomes a string",
        why: "`<c><v>1234.5678</v></c>` with numFmtId 49 (`@`) is a number shown as typed. \
              The oracle writes `office:value-type=\"string\"`. `numfmt/builtins.xlsx` C29.",
        scope: Scope::Cell(|ours, theirs| {
            matches!((&ours.value, &theirs.value), (CellValue::Number(n), CellValue::Text(t))
                if t.parse::<f64>().ok() == Some(*n))
        }),
    },
    Divergence {
        name: "markup compatibility's fallback is dropped",
        why: "A row inside `<mc:AlternateContent>` whose `<mc:Choice Requires=\"x14\">` the \
              reader does not take must be read from `<mc:Fallback>` (ECMA-376 Part 3 §10.2.1). \
              The oracle writes neither branch: A2 and A4 are empty. \
              `realworld/mce-alternate-content.xlsx`.",
        scope: Scope::Cell(|ours, theirs| {
            matches!(ours.value, CellValue::Number(_)) && theirs.value == CellValue::Empty
        }),
    },
    Divergence {
        name: "a string past the oracle's size is dropped",
        why: "`hostile/zip-bomb.xlsx` A5 is one 256 MiB text node; the oracle writes A5 as a \
              string cell with no text.",
        scope: Scope::Cell(|ours, theirs| {
            matches!(&ours.value, CellValue::Text(t) if t.len() > 1 << 20)
                && theirs.value == CellValue::Text(String::new())
        }),
    },
    Divergence {
        name: "an empty string is dropped",
        why: "An entity nothing defines resolves to no text, leaving a string cell whose text is \
              empty; the oracle writes no cell. `hostile/external-entity.xlsx` B1, as LibreOffice \
              does.",
        scope: Scope::Cell(|ours, theirs| {
            ours.value == CellValue::Text(String::new()) && theirs.value == CellValue::Empty
        }),
    },
    Divergence {
        name: "an inline string is dropped",
        why: "`<c r=\"B1\" t=\"inlineStr\"><is><t>quantity</t></is></c>`, in a worksheet \
              written with the `x:` prefix, arrives as an empty cell, while A1 beside it, \
              spelled the same way, arrives. `values/inline-strings.xlsx` B1.",
        scope: Scope::Cell(|ours, theirs| {
            matches!(&ours.value, CellValue::Text(t) if !t.is_empty())
                && theirs.value == CellValue::Empty
        }),
    },
    Divergence {
        name: "unpreserved whitespace is trimmed",
        why: "`<t>  no preserve attribute  </t>` has no `xml:space=\"preserve\"`; the oracle \
              trims it, where XML keeps character data as written and LibreOffice keeps it too. \
              `realworld/xml-space-preserve.xlsx` A4.",
        scope: Scope::Cell(|ours, theirs| {
            matches!((&ours.value, &theirs.value), (CellValue::Text(a), CellValue::Text(b))
                if a != b && a.trim() == b)
        }),
    },
    // ---- styles ----
    Divergence {
        name: "general, centre-across and distributed text is aligned at the start",
        why: "`horizontal` absent (general), `centerContinuous` and `distributed` all arrive \
              as `fo:text-align=\"start\"`. `styles/alignment.xlsx` B2, B8, B9.",
        scope: Scope::Style(|ours, theirs| {
            theirs.align.as_deref() == Some("start")
                && ours.align.as_deref() != Some("start")
                && equal_without(ours, theirs, |s| s.align = None)
        }),
    },
    Divergence {
        name: "vertical justify and distributed are written as values ODF lacks",
        why: "`style:vertical-align` is `top`, `middle`, `bottom` or `automatic`; the oracle \
              writes `justify` and `auto`, which this import reads as given and LibreOffice's \
              schema refuses. `styles/alignment.xlsx` B13, B14.",
        scope: Scope::Style(|ours, theirs| {
            ours.vertical_align.is_none()
                && matches!(theirs.vertical_align.as_deref(), Some("justify" | "auto"))
                && equal_without(ours, theirs, |s| s.vertical_align = None)
        }),
    },
    Divergence {
        name: "an indent is a different step, or dropped beside a rotation",
        why: "`indent=\"1\"` arrives as `9pt` and `indent=\"5\"` as `30pt`, which is no \
              fixed step; with a `textRotation` beside it the indent is gone. \
              `styles/alignment.xlsx` B17, B18, B24.",
        scope: Scope::Style(|ours, theirs| {
            ours.indent.is_some()
                && ours.indent != theirs.indent
                && equal_without(ours, theirs, |s| s.indent = None)
        }),
    },
    Divergence {
        name: "a downward rotation is written as Excel's number",
        why: "`textRotation` 91 to 180 means 1 to 90 degrees *below* the horizontal \
              (ECMA-376 §18.8.1), which is 360 minus (value − 90) in ODF's counter-clockwise \
              `style:rotation-angle`. The oracle writes the number unchanged: 135 for 315. \
              `styles/alignment.xlsx` B21, B22.",
        scope: Scope::Style(|ours, theirs| {
            let angle = |s: &CellStyle| s.rotation.as_deref()?.parse::<i32>().ok();
            matches!((angle(ours), angle(theirs)), (Some(a), Some(b))
                if (91..=180).contains(&b) && a == 360 - (b - 90))
                && equal_without(ours, theirs, |s| s.rotation = None)
        }),
    },
    Divergence {
        name: "the oracle names its own border lines",
        why: "`dashDot`, `dashDotDot`, `slantDashDot` and the medium dashes arrive as \
              LibreOffice's `dash-dot`, `dash-dot-dot` and `fine-dashed`, where this import \
              writes XSL-FO's `dashed` and counts the lost pattern; as in `loop_d.rs`. \
              `styles/borders.xlsx`.",
        scope: Scope::Style(|ours, theirs| same_but(ours, theirs, |a, b| a == b)),
    },
    Divergence {
        name: "a double border is drawn at the thin width",
        why: "Excel's `double` is three lines' worth; this import draws it 1.76pt as \
              LibreOffice does, the oracle 0.74pt. `styles/borders.xlsx` B5, C5.",
        scope: Scope::Style(|ours, theirs| {
            let edges = ours.borders.iter().zip(&theirs.borders).all(|(a, b)| {
                a == b
                    || matches!((a.as_deref().and_then(style::border_parts),
                                 b.as_deref().and_then(style::border_parts)),
                        (Some((_, "double", ca)), Some((wb, "double", cb)))
                            if ca == cb && (wb - 0.74).abs() < 0.01)
            });
            edges && equal_without(ours, theirs, |s| s.borders = Default::default())
        }),
    },
    Divergence {
        name: "a tint rounds differently",
        why: "A theme colour with a `tint` is computed in HLS; the oracle's result differs \
              from this import's (and LibreOffice's) by up to two steps a channel: `#a1b8e1` \
              for `#a2b9e2`, `#fefefe` for `#ffffff`. `styles/colors.xlsx` B27 to B29.",
        scope: Scope::Style(|ours, theirs| {
            let rgb = |s: &CellStyle| {
                let hex = s.color.as_deref()?.strip_prefix('#')?;
                (0..3)
                    .map(|i| u8::from_str_radix(hex.get(2 * i..2 * i + 2)?, 16).ok())
                    .collect::<Option<Vec<u8>>>()
            };
            matches!((rgb(ours), rgb(theirs)), (Some(a), Some(b))
                if a.iter().zip(&b).all(|(x, y)| x.abs_diff(*y) <= 3))
                && equal_without(ours, theirs, |s| s.color = None)
        }),
    },
    Divergence {
        name: "a pattern fill is drawn in its foreground colour",
        why: "`<patternFill patternType=\"darkGrid\">` and the other seventeen patterns have \
              no ODF spelling; this import draws no background and counts the pattern, the \
              oracle fills the cell with the foreground colour. `styles/fills.xlsx` B3 to B20.",
        scope: Scope::Style(|ours, theirs| {
            ours.background.is_none()
                && theirs.background.is_some()
                && equal_without(ours, theirs, |s| s.background = None)
        }),
    },
    Divergence {
        name: "a strikethrough is dropped",
        why: "`<strike/>` arrives with no `style:text-line-through-style`. \
              `styles/fonts.xlsx` B9, B21.",
        scope: Scope::Style(|ours, theirs| {
            ours.line_through.is_some()
                && theirs.line_through.is_none()
                && equal_without(ours, theirs, |s| s.line_through = None)
        }),
    },
    Divergence {
        name: "the default font size is stated on a cell",
        why: "The oracle's default cell style (`Normal`, and `style:default-style`) states no \
              size, so a cell at the workbook's own 11pt carries it as its own. \
              `styles/fonts.xlsx` B11.",
        scope: Scope::Style(|ours, theirs| {
            ours.font_size.is_none()
                && theirs.font_size.as_deref() == Some("11pt")
                && equal_without(ours, theirs, |s| s.font_size = None)
        }),
    },
    Divergence {
        name: "a named style's size and colour are dropped",
        why: "A cell whose `<xf>` takes its font from a named style keeps the style's weight \
              and loses its size and colour. `styles/named-styles.xlsx` A1.",
        scope: Scope::Style(|ours, theirs| {
            ours.font_size.is_some()
                && ours.color.is_some()
                && theirs.font_size.is_none()
                && theirs.color.is_none()
                && equal_without(ours, theirs, |s| {
                    s.font_size = None;
                    s.color = None;
                })
        }),
    },
    // ---- columns, rows and sheets ----
    Divergence {
        name: "a column width counts the padding twice",
        why: "`<col width>` already includes Excel's five pixels of padding (ECMA-376 \
              §18.3.1.13); the oracle adds them again, 1.2 to 1.4 mm on every sized column. \
              `geometry/columns.xlsx`, `geometry/outlines.xlsx`.",
        scope: Scope::Track(|track| {
            matches!(track.what, TrackDifference::Size { ours, theirs: Some(theirs) }
                if (1.1..1.45).contains(&(theirs - ours)))
        }),
    },
    Divergence {
        name: "a hidden column is narrowed to its padding",
        why: "A hidden column with a width arrives 1.323 mm wide (five pixels) and hidden, \
              where this import keeps the width it will have when shown. \
              `geometry/columns.xlsx` column 11, `geometry/outlines.xlsx` column 5.",
        scope: Scope::Track(|track| {
            matches!(track.what, TrackDifference::Size { theirs: Some(theirs), .. }
                if (theirs - 1.323).abs() < 0.01)
        }),
    },
    Divergence {
        name: "a column range at the default width has no width",
        why: "`<col min=\"14\" max=\"16384\" width=\"8.43\"/>` arrives with no \
              `style:column-width`. `geometry/columns.xlsx` column 14.",
        scope: Scope::Track(|track| {
            track.axis == Axis::Column
                && matches!(track.what, TrackDifference::Size { theirs: None, .. })
        }),
    },
    Divergence {
        name: "a row a filter hid, or a collapsed group's summary row, is hidden",
        why: "Rows an `<autoFilter>` hid are filtered, not hidden by hand, and arrive as \
              `table:visibility=\"collapse\"`; a row with `collapsed=\"1\"` and no `hidden` is \
              the visible summary of a collapsed group, and arrives collapsed too. \
              `functions/math.xlsx` Receipts, `geometry/outlines.xlsx` row 8.",
        scope: Scope::Track(|track| {
            track.axis == Axis::Row
                && matches!(
                    track.what,
                    TrackDifference::Hidden {
                        ours: false,
                        theirs: true
                    }
                )
        }),
    },
    Divergence {
        name: "a zero-height row is shown",
        why: "`<row ht=\"0\" customHeight=\"1\">` is a row nobody can see; the oracle shows it, \
              as LibreOffice does. `geometry/rows.xlsx` row 8.",
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
        name: "a sheet name with brackets or a slash is kept (theirs)",
        why: "`Has[Brackets]` and `Has/Slash` are kept as `table:name`; this import renames \
              them, because an ODF reference cannot spell them. `document/sheets.xlsx`.",
        scope: Scope::OnlyTheirs(|name| name.contains(['[', ']', '/'])),
    },
    Divergence {
        name: "a sheet name with brackets or a slash is kept (ours)",
        why: "The same two sheets, seen from this import's side under their workbook names.",
        scope: Scope::OnlyOurs(|name| name.contains(['[', ']', '/'])),
    },
    Divergence {
        name: "a workbook with a document type declaration is empty",
        why: "`hostile/billion-laughs.xlsx` declares nested entities; the oracle's sheet holds \
              nothing, where this import caps the expansion and keeps the cells.",
        scope: Scope::Unreached,
    },
];

/// Is `ours` the same style as `theirs` once `clear` has taken the field in question out of both?
fn equal_without(ours: &CellStyle, theirs: &CellStyle, clear: fn(&mut CellStyle)) -> bool {
    let (mut a, mut b) = (ours.clone(), theirs.clone());
    clear(&mut a);
    clear(&mut b);
    a == b
}

const EURO_OFFICE: Oracle = Oracle {
    name: "Euro-Office",
    cache: "grind-loop-d-euro-office",
    output: "ods",
    version: x2t_version,
    convert,
    unconverted: &[(
        "values/types.xlsx",
        "x2t aborts on it with `boost::bad_lexical_cast`.",
    )],
    divergences: DIVERGENCES,
};

fn x2t_version() -> Option<String> {
    let out = Command::new("x2t").arg("--version").output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// Everything not cached yet, in one container. A file x2t fails on leaves no output, which the
/// run then holds against [`Oracle::unconverted`].
fn convert(cache: &Path, pending: &[(String, PathBuf)]) {
    if pending.is_empty() {
        return;
    }
    let (stage, inputs) = oracle::stage(cache, pending);
    Command::new("x2t")
        .args(["--convert-to", "ods", "--outdir"])
        .arg(cache)
        .args(&inputs)
        .status()
        .expect("x2t failed to start");
    let _ = std::fs::remove_dir_all(&stage);
}

#[test]
fn our_import_agrees_with_euro_office() {
    oracle::run(&EURO_OFFICE, FLOOR);
}

/// GRIND_LOOP_D_CORPUS=~/samples scripts/euro-office-tests.sh outside -- --nocapture
#[test]
fn outside_corpus_against_euro_office() {
    oracle::run_outside(&EURO_OFFICE);
}
