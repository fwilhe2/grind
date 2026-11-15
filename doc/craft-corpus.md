<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: CC-BY-4.0
-->

# The Craft corpus — a compatibility matrix and import timings

The vendored corpora (`xlsx/tests/data/corpus`, `docx/tests/data`) and LibreOffice's are small
files, each about one thing. Real Office files are large and do everything at once. This corpus
is the second kind, made on demand: **WordCraft** and **GridCraft**, two Office-compatible
applications with a scripting CLI, write `.docx` and `.xlsx` files the way Word and Excel do —
shared strings, cached formula values, theme parts, `w14`/`x14` extensions — from a script, so
the corpus is code and nothing large is checked in.

They are an approximation of Microsoft Office, not Office. Where the approximation shows, it is
named below; a matrix row is about grind only when Craft wrote what the probe asked for, which
the matrix checks for every probe.

## What is in it

`scripts/craft-corpus/large_corpus.py` writes two sets.

- **Probes.** One small file per feature: 67 workbooks and 56 documents, each carrying a unique
  token (`XTOKNOTE`, `XTOKFOOT`, …). Each is listed in `manifest.json` with two patterns: one
  over the Office file's XML (did Craft write the feature?) and one over grind's ODF (did it
  survive?), and sometimes a third, stricter one (did it survive *as written*?).
- **Scale tiers**, modelled on two fictional "Northstar" fixtures — an enterprise performance
  workbook and an operating playbook:
  - a workbook of seven sheets — an executive dashboard of cross-sheet `SUMIFS`, `COUNTIFS` and
    an `XLOOKUP`, a transaction ledger as an Excel table, a project portfolio, 36 months of
    financials with a chart, sparklines and outline groups, a risk register, a people model and
    reference data with edge cases — at 1k, 12k, 50k, 100k and 200k ledger rows;
  - a playbook with a title block, document control and revision tables, a table of contents,
    then per chapter headings, footnotes, links, lists, a shaded table with a repeated header
    and a caption, justified commentary, and now and then a comment, a picture or a chart — at
    12, 48, 150 and 450 chapters. The 12-chapter tier is the Northstar playbook's size
    (`document.xml` 338 kB against its 350 kB); the 450-chapter one is about 1,600 pages.

All names, figures and addresses are fabricated (`.test` domains, "FICTIONAL TEST DATA").

`compat_matrix.py` imports every file with a release `grind` and sorts each probe into one of:

| Status | Meaning |
|---|---|
| ✅ carried | the ODF has it |
| 🟡 kept, altered | it is there, not as it was written |
| ⚠️ dropped, reported | it is gone, and the import report says so |
| ❌ silent loss | it is gone, and the report says nothing — the bug worth chasing |
| ➖ Craft | Craft refused the script, or did not write the feature |

Losses every file of a kind reports (the plain-values probe's) are not charged to each feature.
For each tier it records the median of three imports, the peak RSS, reopening the result
(`grind info`), and either `grind sheet recalc` or `grind text export-pdf`.

## Running it

```sh
scripts/craft-corpus/install.sh                     # the pinned Craft packages, checksummed
cargo build --release -p grind-cli
GRIND_BIN=$PWD/target/release/grind scripts/craft-corpus/run.sh target/craft-corpus
```

or without installing anything but Docker:

```sh
docker build -f scripts/craft-corpus/Dockerfile -t grind-craft-corpus .
docker run --rm -v "$PWD/target/craft-corpus:/out" grind-craft-corpus
```

The matrix lands in `target/craft-corpus/grind/matrix.md`. `run.sh` holds every probe to
`scripts/craft-corpus/expected.json` **both ways**: a worse status is a regression, a better one
fails too until it is recorded with `run.sh target/craft-corpus --update-expect`, so the file
stays the truth. `.github/workflows/craft-corpus.yml` runs it on every change to an import
filter or what it stands on, and puts the matrix in the run's summary. The timings are never
gated on.

## Findings

Measured 2026-11-15 with WordCraft 0.16 and GridCraft 0.17 on a laptop (8 threads, 16 GB),
release build of `9ed97c5`. Every silent loss the first run found was confirmed by reading the
ODF, and every one that was a loss of the report rather than of the model is now counted:

- **Fixed — now reported.** Spreadsheets: a cell's hyperlink, an Excel table (its cells and
  structured references were and are carried), page setup — landscape, scaling, a header or
  footer other than the oracle's default, breaks, a print area or titles
  (`doc/xlsx-format.md` §7.3) — sparklines, and document properties. Text: a comment the body
  never anchors, a tracked change of formatting, line numbering, and document properties.
- **Fixed — now carried.** A defined name keeps its spelling: the model keys names lower-case,
  and every writer wrote the key, so `XTOKNAME` came back `xtokname`
  (`Document::name_spellings`).
- **Left, on purpose.** Zoom, gridlines and the tab colour are view state, which the import
  does not count (`grind_xlsx::Appearance::Pane`); a manual calculation mode is the same kind of
  setting. A sheet-local name with no rival is carried document-wide (`doc/xlsx-format.md`,
  `workbook::names`), which the matrix marks as altered.
- **Not grind's.** The comment reply, the cell-style and the slicer rows are Craft's (below).

What remains reported is what the models have no home for: charts and drawings in a workbook,
conditional formats other than a cell-value or formula rule, data validation, pivot tables,
cell comments, frozen panes, outline groups, protection and a font family other than the
default (counted per cell, which on the openpyxl-written Northstar workbook is 168,870 times);
in a document, fields (kept as their text), equations, shapes, charts, watermarks, multi-column
sections, drop caps and text boxes (their text kept).

The timings are the median of three runs. Import is linear in cells — about 200,000 a second
and 200 bytes of peak memory per cell — split evenly between reading the `.xlsx` and writing
the ODF, with no single hot spot. A recalc that changes many cached values used to cost four
times one that changes none: the never-worse check on save tested every element of the original
against one byte range per changed cell. With the ranges sorted and searched
(`envelope::vocabulary_within`), the Northstar workbook, whose 25,120 formulas carry no cached
values, recalculates and saves in 3.0 s instead of 10.9. A one-cell `grind sheet set` on the
12k tier still takes about 3 s, spread over several whole-document passes of the save (the
style scan, the never-worse check, the re-read that verifies the result); none dominates.

### Spreadsheets (`.xlsx` → `.ods`)

✅ carried: 30 · 🟡 kept, altered: 1 · ⚠️ dropped, reported: 31 · ❌ silent loss: 3 · ➖ Craft did not write it: 2

| Feature | Probe | Result | Grind reported |
|---|---|---|---|
| Text, numbers, booleans | `values` | ✅ carried |  |
| Alignment, wrap, rotation, indent | `alignment` | ✅ carried |  |
| AutoFilter with active criterion | `autofilter` | ✅ carried |  |
| Manual calculation mode | `calc-manual` | ❌ silent loss |  |
| Named cell styles (Good, Bad, Heading 1) | `cell-style` | ➖ Craft did not write it |  |
| Conditional format: cell value | `cf-cellis` | ✅ carried |  |
| Conditional format: colour scale | `cf-colorscale` | ⚠️ dropped, reported | conditional format ×1 |
| Conditional format: data bar | `cf-databar` | ⚠️ dropped, reported | conditional format ×1 |
| Conditional format: formula | `cf-expression` | ✅ carried |  |
| Conditional format: icon set | `cf-iconset` | ⚠️ dropped, reported | conditional format ×1 |
| Conditional format: top 10 and duplicates | `cf-top10-dup` | ⚠️ dropped, reported | conditional format ×2 |
| Column chart | `chart-column` | ⚠️ dropped, reported | chart ×1; drawing ×1 |
| Waterfall chart (chartex) | `chart-combo-modern` | ⚠️ dropped, reported | drawing ×1 |
| Line and pie charts | `chart-line-pie` | ⚠️ dropped, reported | chart ×2; drawing ×1 |
| Cell checkboxes | `checkbox` | ✅ carried |  |
| Column widths and row heights | `col-width-row-height` | ✅ carried |  |
| Dates and times | `dates` | ✅ carried |  |
| Workbook defined name | `defined-name` | ✅ carried |  |
| Sheet-scoped defined name | `defined-name-sheet` | 🟡 kept, altered |  |
| Fill colour and borders | `fill-borders` | ✅ carried |  |
| Font name, size, bold, italic, colour | `font` | ✅ carried | font family ×4 |
| Non-default font family carried | `font-family` | ⚠️ dropped, reported | font family ×24 |
| Legacy CSE array formula | `formula-array` | ⚠️ dropped, reported | array formula ×1; array formula ×1 |
| Arithmetic and SUM | `formula-basic` | ✅ carried |  |
| SUMIFS, COUNTIFS, AVERAGEIFS | `formula-conditional` | ✅ carried |  |
| Formulas referencing another sheet | `formula-cross-sheet` | ✅ carried |  |
| Dynamic arrays: UNIQUE, FILTER, SORT | `formula-dynamic` | ⚠️ dropped, reported | array formula ×3; array formula ×3 |
| IFERROR and error values | `formula-iferror` | ✅ carried |  |
| VLOOKUP, INDEX/MATCH, XLOOKUP | `formula-lookup` | ✅ carried | COM.MICROSOFT.XLOOKUP ×1 |
| Structured table references | `formula-structured` | ✅ carried | table (its data kept) ×1 |
| Text and date functions | `formula-text-date` | ✅ carried | TEXT ×1 |
| Frozen panes | `freeze-panes` | ⚠️ dropped, reported | frozen or split panes (per sheet) ×1 |
| Row and column outline groups | `grouping` | ⚠️ dropped, reported | row or column outline (per sheet) ×1 |
| Header and footer with fields | `header-footer` | ⚠️ dropped, reported | page setup (per sheet) ×1 |
| Hidden rows and columns | `hidden-rows-cols` | ✅ carried |  |
| Hyperlink to a URL | `hyperlink` | ⚠️ dropped, reported | hyperlink (its text kept) ×1 |
| Hyperlink to a cell in another sheet | `hyperlink-internal` | ⚠️ dropped, reported | hyperlink (its text kept) ×1 |
| Numeric-looking text kept as text | `leading-apostrophe` | ✅ carried |  |
| Text near the 32,767-character cell limit | `long-text` | ✅ carried |  |
| Forty sheets | `many-sheets` | ✅ carried |  |
| Merged cells | `merge` | ✅ carried |  |
| Legacy note | `note` | ⚠️ dropped, reported | comment ×1 |
| Currency number format | `number-currency` | ✅ carried |  |
| Custom format with colour and sections | `number-custom` | ✅ carried |  |
| Long date format | `number-date` | ✅ carried | number-format locale ×1 |
| Percentage format | `number-percent` | ✅ carried |  |
| Manual page break | `page-breaks` | ⚠️ dropped, reported | page setup (per sheet) ×1 |
| Landscape, margins, scale to fit | `page-setup` | ⚠️ dropped, reported | page setup (per sheet) ×1 |
| Embedded picture | `picture` | ⚠️ dropped, reported | drawing ×1 |
| Pivot table | `pivot` | ⚠️ dropped, reported | pivot table ×1 |
| Print area and print titles | `print-area-titles` | ⚠️ dropped, reported | page setup (per sheet) ×1 |
| Document properties | `properties` | ⚠️ dropped, reported | document properties ×1 |
| Shape and text box | `shape-textbox` | ⚠️ dropped, reported | drawing ×1 |
| Hidden sheet | `sheet-hidden` | ⚠️ dropped, reported | hidden sheet (the data was kept) ×1 |
| Protected sheet | `sheet-protection` | ⚠️ dropped, reported | protection ×1 |
| Table slicer | `slicer` | ➖ Craft did not write it | table (its data kept) ×1 |
| Sorted range | `sort-state` | ✅ carried |  |
| Sparklines | `sparkline` | ⚠️ dropped, reported | sparkline ×1 |
| Sheet tab colour | `tab-color` | ❌ silent loss |  |
| Excel table (ListObject) with style | `table` | ⚠️ dropped, reported | table (its data kept) ×1 |
| Table total row | `table-total-row` | ✅ carried | table (its data kept) ×1 |
| Threaded comment with reply | `threaded-comment` | ⚠️ dropped, reported | comment ×1 |
| Unicode, emoji, RTL, CJK text | `unicode` | ✅ carried |  |
| List data validation | `validation-list` | ⚠️ dropped, reported | data validation ×1 |
| Whole-number validation with error message | `validation-whole` | ⚠️ dropped, reported | data validation ×1 |
| Protected workbook structure | `workbook-protection` | ⚠️ dropped, reported | protection ×1 |
| Zoom and hidden gridlines | `zoom-gridlines` | ❌ silent loss |  |

### Text documents (`.docx` → `.odt`)

✅ carried: 41 · 🟡 kept, altered: 4 · ⚠️ dropped, reported: 11

| Feature | Probe | Result | Grind reported |
|---|---|---|---|
| Centred and justified paragraphs | `alignment` | ✅ carried |  |
| Bold, italic, underline | `bold-italic` | ✅ carried |  |
| Bookmark and cross-reference | `bookmark-xref` | ✅ carried | field (kept as its text) ×1 |
| Bulleted list | `bullets` | ✅ carried |  |
| Table and figure captions | `caption` | ⚠️ dropped, reported | field (kept as its text) ×1 |
| Font, size and colour | `char-color-size` | ✅ carried |  |
| Embedded chart | `chart` | ⚠️ dropped, reported | drawing ×1 |
| Two-column section | `columns` | ⚠️ dropped, reported | multi-column section ×1 |
| Comment | `comment` | ✅ carried |  |
| Comment reply thread | `comment-reply` | 🟡 kept, altered | comment ×1 |
| Checkbox and drop-down content controls | `content-controls` | 🟡 kept, altered | content control (kept as its text) ×2 |
| Custom paragraph style | `custom-style` | ✅ carried |  |
| Drop cap | `drop-cap` | ⚠️ dropped, reported | positioned paragraph ×1 |
| Endnote | `endnote` | ✅ carried |  |
| Office Math equation | `equation` | ⚠️ dropped, reported | equation ×1 |
| Date and author fields | `fields-date` | ⚠️ dropped, reported | field (kept as its text) ×2 |
| Different first page header | `first-page-header` | ✅ carried |  |
| Footnote | `footnote` | ✅ carried |  |
| Header and footer | `header-footer` | ✅ carried |  |
| Heading 1–3 with outline levels | `headings` | ✅ carried |  |
| Highlight and shading | `highlight` | ✅ carried |  |
| External hyperlink | `hyperlink` | ✅ carried |  |
| Left, right and hanging indents | `indents` | ✅ carried |  |
| Link to a bookmark | `internal-link` | ✅ carried |  |
| Keep with next and page break before | `keep-pagebreak` | ✅ carried |  |
| Landscape section between portrait ones | `landscape-section` | ✅ carried |  |
| Manual line break | `line-break` | ✅ carried |  |
| Line numbering | `line-numbers` | ⚠️ dropped, reported | line numbering ×1 |
| Multilevel list | `multilevel` | ✅ carried |  |
| Numbered list | `numbering` | ✅ carried |  |
| Manual page break | `page-break` | ✅ carried |  |
| Page X of Y in footer | `page-number` | ✅ carried |  |
| A4 paper and custom margins | `paper-a4-margins` | ✅ carried |  |
| Paragraph borders and shading | `para-borders-shading` | ✅ carried |  |
| Plain paragraphs | `paragraphs` | ✅ carried |  |
| Inline picture | `picture` | ✅ carried |  |
| Document properties | `properties` | ⚠️ dropped, reported | document properties ×1 |
| Quote, Intense Quote and No Spacing styles | `quote-styles` | ✅ carried |  |
| Drawing shape | `shape` | ⚠️ dropped, reported | drawing ×1 |
| Line spacing and space before/after | `spacing` | ✅ carried |  |
| Strikethrough, small caps, all caps | `strike-caps` | ✅ carried |  |
| Superscript and subscript | `super-sub` | ✅ carried |  |
| Symbols, special characters, unicode | `symbols` | ✅ carried |  |
| Simple table | `table` | ✅ carried |  |
| Table with 500 rows | `table-large` | ✅ carried |  |
| Merged table cells | `table-merge` | ✅ carried |  |
| Text before, between and after tables | `table-nested` | ✅ carried |  |
| Table style, header shading, repeat header | `table-styled` | ✅ carried | table style banding ×1 |
| Tab stops with leaders | `tabs` | ✅ carried |  |
| Floating text box | `text-box` | 🟡 kept, altered | text box (its text kept in the flow) ×1 |
| Title and Subtitle styles | `title-subtitle` | ✅ carried |  |
| Table of contents | `toc` | ⚠️ dropped, reported | field (kept as its text) ×3 |
| Tracked deletion | `track-delete` | ✅ carried |  |
| Tracked formatting change | `track-format` | 🟡 kept, altered | tracked formatting change (applied) ×1 |
| Tracked insertion | `track-insert` | ✅ carried |  |
| Watermark | `watermark` | ⚠️ dropped, reported | drawing ×1 |

### Import timings

| File | Input | Output | Import | Peak RSS | Reopen | Recalc / PDF | Notes |
|---|---:|---:|---:|---:|---:|---:|---|
| `docx/northstar-playbook-12ch` | 26 kB | 17 kB | 0.06 s | 22 MiB | 0.02 s | 0.17 s, 41 pp | blocks 1,875, paragraphs 1,893, headings 65, tables 15, pictures 3, notes 12 |
| `docx/northstar-playbook-48ch` | 56 kB | 44 kB | 0.23 s | 57 MiB | 0.04 s | 0.43 s, 154 pp | blocks 7,339, paragraphs 7,405, headings 251, tables 51, pictures 12, notes 48 |
| `docx/northstar-playbook-150ch` | 137 kB | 119 kB | 0.72 s | 154 MiB | 0.14 s | 1.42 s, 476 pp | blocks 22,406, paragraphs 22,608, headings 778, tables 153, pictures 38, notes 150 |
| `docx/northstar-playbook-450ch` | 388 kB | 349 kB | 2.33 s | 432 MiB | 0.52 s | 5.39 s, 1426 pp | blocks 67,216, paragraphs 67,818, headings 2,334, tables 459, pictures 114, notes 450 |
| `xlsx/northstar-workbook-1k` | 140 kB | 110 kB | 0.07 s | 14 MiB | 0.04 s | 0.16 s | sheets 7, cells 15,036, formulas 2,294, styled 58 |
| `xlsx/northstar-workbook-12k` | 1.3 MB | 1.2 MB | 0.71 s | 48 MiB | 0.47 s | 1.93 s | sheets 7, cells 168,816, formulas 25,554, styled 418 |
| `xlsx/northstar-workbook-50k` | 5.4 MB | 4.8 MB | 2.98 s | 146 MiB | 2.03 s | 8.11 s | sheets 7, cells 701,442, formulas 105,986, styled 1,684 |
| `xlsx/northstar-workbook-100k` | 10.8 MB | 9.6 MB | 6.27 s | 282 MiB | 4.18 s | 17.02 s | sheets 7, cells 1,402,279, formulas 211,820, styled 3,351 |
| `xlsx/northstar-workbook-200k` | 21.6 MB | 19.1 MB | 13.65 s | 554 MiB | 9.26 s | 36.98 s | sheets 7, cells 2,803,942, formulas 423,486, styled 6,684 |
| Northstar workbook (openpyxl) | 1.1 MB | 841 kB | 1.08 s | 61 MiB | 0.62 s | 3.04 s | sheets 7, cells 168,877, formulas 25,120, styled 168,879 |
| Northstar playbook (python-docx) | 49 kB | 18 kB | 0.08 s | 20 MiB | 0.01 s | 0.18 s, 20 pp | blocks 1,485, paragraphs 1,487, headings 69, tables 20, pictures 0, notes 0 |

## Where Craft is not Office

- **A reply to a comment** is written into `comments.xml` and `commentsExtended.xml` only, with
  no anchor in the body; Word anchors a reply over its parent's range. Grind carries every
  anchored comment, so a Word reply would most likely come through; this one is counted as a
  dropped comment.
- **WordCraft leaves the caret inside a new footnote or endnote**, so text typed next goes into
  the note. The generator leaves with `insert.closeHeader`.
- **The blank template** has no `List Bullet`/`List Number` styles, which documents made from
  Word's Normal template use, and names its table styles `Grid Table 4 – Blue` rather than
  Word's `Grid Table 4 Accent 1`. The generator uses direct bullets and numbering instead.
- **GridCraft writes no named cell styles** for `home.cellStyle` and no slicer part; both probes
  are marked ➖.
- **Size.** WordCraft refuses scripts over 4 MiB and slows down superlinearly with their length
  (150 chapters take about 90 s). Tiers past 150 chapters are the 150-chapter file appended to
  itself with `insert.textFromFile`, which takes a second.

GridCraft's own calculation check (`gridcraft-cli check`) agrees with every cached value the
corpus holds, which is what makes the cached values worth comparing: grind does not recalculate
on import.
