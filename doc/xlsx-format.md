<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Clean-room notes on OOXML SpreadsheetML

The `doc/ods-format.md` of the import filter: **facts about `.xlsx` that had to be measured
rather than read**, each recorded here *before* it reaches code. `doc/xlsx-import.md` is the
plan; this is the evidence it stands on.

The rule is the project's clean-room rule, unchanged. **ECMA-376 / ISO-IEC 29500 is the
specification.** LibreOffice's own filter (`sc/source/filter/oox/`) may be *read and cited by
`file:line`*, never copied, and a fact learned by reading it lands here with that citation
before it reaches a Rust file. LibreOffice is a conformance oracle and a corpus, never a
source. Excel is named freely — it is the format's name.

**Scope: reading, and only the XML form from Office 2007 onwards.** Not `.xls`, not `.xlsb`,
not SpreadsheetML 2003, and never writing. `doc/xlsx-import.md`'s "What this will not do" is
the full line.

## How to read the status markers

Every fact carries one, because the point of this document is to keep guesses out of the
reader:

- **`MEASURED`** — observed from a real file, with the command that produced it and the date.
  Reproducible.
- **`SPEC`** — stated by ECMA-376, cited by section, and *not yet* confirmed against a real
  file. Excel diverges from the spec often enough that this is a weaker claim than `MEASURED`.
- **`UNVERIFIED`** — written from memory or inference. **May not be implemented.** Same force
  as `doc/odt-format.md` §5's marker: it is a lead, not a fact.

A fact moves up the ladder; it never moves down quietly.

---

## 1. Flavours and namespaces

### 1.1 Transitional is what real files are — `MEASURED`

Measured 2026-09-14. `sheet/tests/data/kb/formula.fods` converted with the pinned oracle:

```sh
soffice --headless --convert-to xlsx formula.fods
unzip -p formula.xlsx xl/workbook.xml | head -c 900
```

`xl/workbook.xml` and `xl/worksheets/sheet1.xml` both carry, on the root element:

| Prefix | URI |
|---|---|
| (default) | `http://schemas.openxmlformats.org/spreadsheetml/2006/main` |
| `r` | `http://schemas.openxmlformats.org/officeDocument/2006/relationships` |
| `mc` | `http://schemas.openxmlformats.org/markup-compatibility/2006` |
| `xdr` | `http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing` |
| `x14`, `x15` | `http://schemas.microsoft.com/office/spreadsheetml/{2009/9,2010/11}/main` |
| `xr`, `xr2`, `xr6`, `xr10` | `http://schemas.microsoft.com/office/spreadsheetml/{2014,2015,2016}/revision*` |

`_rels/.rels` uses `http://schemas.openxmlformats.org/package/2006/relationships` for its own
namespace and `…/officeDocument/2006/relationships/officeDocument` as the `Type` of the
workbook relationship.

**Two things this measurement establishes.** First, the flavour: these are ECMA-376
Transitional URIs, and they are what the most minimal file this project can produce already
contains. Second, and the reason it is worth a section — *even this file declares seven
namespaces it makes no use of*, six of them Microsoft extensions. A reader that is not
tolerant of unknown namespaces by construction has no chance on a document somebody actually
worked in.

### 1.2 The Strict URIs — `MEASURED`

Written from memory first and then measured, which is the order this document exists to make
visible. The corpus turned out to contain three Strict workbooks:

```sh
for f in sc/qa/unit/data/xlsx/*.xlsx; do
    unzip -p "$f" xl/workbook.xml | head -c 2000 | grep -ql 'purl.oclc.org' && echo "$f"
done
# XlStartupExternal.xlsx, universal-content-strict.xlsx, user_defined_function.xlsx
```

From `universal-content-strict.xlsx`, measured 2026-09-14:

| | Strict |
|---|---|
| SpreadsheetML | `http://purl.oclc.org/ooxml/spreadsheetml/main` |
| `r:` attributes | `http://purl.oclc.org/ooxml/officeDocument/relationships` |
| Relationship `Type=` | `http://purl.oclc.org/ooxml/officeDocument/relationships/{officeDocument,worksheet,styles,theme,sharedStrings,calcChain,extendedProperties}` |
| OPC package rels | `http://schemas.openxmlformats.org/package/2006/relationships` — **unchanged**, confirming Part 2 does not vary |
| Markup compatibility | `http://schemas.openxmlformats.org/markup-compatibility/2006` — **unchanged**, Part 3 likewise |

Every URI written from memory was right, and two things were found that had not been guessed:

1. **`<workbook conformance="strict">`** — the workbook says so itself, as well as through its
   namespaces. Corroboration rather than the decision, since a Strict file need not carry it.
2. **One file carries both families at once.** The same `_rels/.rels` holds Strict types
   beside the OPC ones, plus a fourth relationship typed
   `http://schemas.openxmlformats.org/officedocument/2006/relationships/metadata/core-properties`
   — note the lower-case `officedocument`, which matches nothing in either family. So
   `Flavour::Mixed` is a real answer rather than a hypothetical one, and a misspelt type must
   come back "unrecognised, therefore ignored" rather than be guessed at. Both are tests in
   `xlsx/src/names.rs`.

### 1.3 Markup compatibility — `SPEC`

ECMA-376 Part 3. `mc:AlternateContent` holds one or more `mc:Choice Requires="…"` and at most
one `mc:Fallback`; a consumer takes the first `mc:Choice` whose required namespaces it all
understands, and otherwise the `mc:Fallback`. We understand no extension namespace, so **every
`mc:Choice` is skipped and the `mc:Fallback` is read in its place**.

**How common — `MEASURED`.** 15 of the corpus's 360 workbooks carry an `mc:AlternateContent`
in a worksheet part (2026-09-14, scanning `xl/worksheets/sheet*.xml` of every file under
`sc/qa/unit/data/{xlsx,xlsm}`). That is a floor rather than a rate: the count ignores
`xl/workbook.xml`, which `universal-content-strict.xlsx` shows also carries them, and the
corpus is minimal bug reproductions rather than documents somebody worked in.

The rule itself is exercised by fixtures rather than by the corpus — a `Choice` and a
`Fallback` holding *different* sheet lists, so taking the wrong one, or both, is a visible
failure rather than a subtle one (`xlsx/tests/fixtures.rs`).

### 1.4 What real producers write — `MEASURED`

Scanned across all 360 corpus workbooks on 2026-09-14. Each is one rule in the reader, and
each would otherwise have arrived as a bug report:

| Quirk | Files |
|---|---|
| a `<row>` with no `r` attribute | 1 (`tdf121887.xlsx`) |
| a `<c>` with no `r` attribute | 2 (`row-index-1-based.xlsx`, `tdf121887.xlsx`) |
| `xml:space` in `sharedStrings.xml` | **51** |
| `mc:AlternateContent` in a worksheet | 15 |

**`mc:MustUnderstand` is reported by URI — decided at X1.** The attribute holds prefixes, and a
prefix is a local alias that tells a bug report nothing, so `xlsx/src/xml.rs` resolves each one
while the declaring element's namespace scope is still live — the only moment it can be — and
the report carries `http://schemas.microsoft.com/office/spreadsheetml/2010/11/main` where the
file wrote `x15`. An unbound prefix is a malformed file and is reported as written.

The `xml:space` figure is the one worth sitting with: **one workbook in seven** has a shared
string whose leading or trailing space is meaningful, so a reader that trims text by default
corrupts a seventh of this corpus silently. `xlsx/src/xml.rs` turns quick-xml's `trim_text`
off for exactly that reason, which is a decision that would have looked like a detail without
the number beside it.

---

## 2. Values and dates

### 2.1 The 1900 leap-year bug — `MEASURED`

ECMA-376 Part 1 §18.17.4: in the 1900 system serial 1 is 1900-01-01 **and** 1900 is treated as a
leap year, so serial 60 is 1900-02-29. ODF's default epoch is 1899-12-30, so the two agree from
serial 61 (1900-03-01) onwards and are one day apart below it. `xlsx/src/dates.rs` holds the
table and applies it: a **date** cell in a 1900 workbook with a serial in `[0, 60)` gains one
day; everything else is carried unchanged.

Measured on 2026-09-19 with loop D (`xlsx/tests/loop_d.rs`) against the pinned oracle
(LibreOffice 26.2.5.2, `ci/libreoffice-image`) over `values/dates-1900.xlsx`:

| Excel serial | Excel means | this import | the oracle |
|---|---|---|---|
| 1 | 1900-01-01 | 1900-01-01 | **1899-12-31** |
| 2 | 1900-01-02 | 1900-01-02 | **1900-01-01** |
| 59 | 1900-02-28 | 1900-02-28 | **1900-02-27** |
| 60 | 1900-02-29, which does not exist | 1900-02-28 | 1900-02-28 |
| 61 | 1900-03-01 | 1900-03-01 | 1900-03-01 |

The oracle applies **no** correction, so everything before 1900-03-01 arrives a day early; the
generated corpus's manifest had already recorded the same in an `oracle` field (measured
2026-09-14), and loop D names it as a divergence rather than a failure. Serial 60 has no right
answer, and this import leaves it where the oracle does — on the same day as 59 — which keeps
the mapping monotone and puts a real date beside the one that was meant.

**Only a date is corrected.** A *time* format makes the whole serial a duration — 1 under
`[h]:mm:ss` is `PT24H` in either system, with no epoch in it (`values/times.xlsx`) — and a
number under a numeric format is a number (`values/date-vs-number.xlsx`, where one `<v>` is
four different things under four formats).

### 2.2 Cell types — `MEASURED`

ECMA-376 §18.18.11 (`ST_CellType`), mapped onto `CellValue` as `doc/xlsx-import.md` Part II §2
tabulates it, and held there by the generated corpus: `values/types.xlsx` has one cell of each of
the seven, and loop D agrees with the oracle about all of them but one — `t="e"` holding `#REF!`,
which the oracle turns into the formula `of:=#ref!` and evaluates to `#NAME?` (the other five
error values survive). `t` defaults to `n`, and `t="d"` (ISO 8601, 2nd edition) is written by
producers other than Excel.

### 2.3 What the oracle does with values that this import does not — `MEASURED`

Everything loop D's pinned run named on 2026-09-19, beside the two above. Each is a construct,
and each is asserted to still occur, so the list cannot go stale silently:

- **It recalculates.** A formula's cached `<v>` is replaced by the oracle's own evaluation on
  load: `formulas/shared-groups.xlsx` caches C2 = 4 and the oracle writes 6 (= A2+B2). Why it
  recalculates these files is not measured and not needed; that it does is. X2 carries the
  formula and this does not change: the oracle still evaluates on load, so the two numbers
  still come from two evaluators and a formula cell's *value* stays out of loop D.
- **An empty string is dropped.** `<t/>` is a string cell with no text; the oracle writes an
  empty cell.
- **A sheet name it does not allow drops the sheet**, cells and all — `Has[Brackets]` and
  `Has/Slash` in `document/sheets.xlsx`.
- **An external link's cache becomes a sheet**, named `'file:///…'#Sheet1`.
- **A part reached through `\` is not found**; the sheet arrives, empty.
- **A pivot table's output is regenerated**, with the oracle's own labels (`Total Result`).

### 2.4 Formula text: the three spellings the oracle differs in — `MEASURED`

Read out of `manifest.json`'s own `oracle` fields and confirmed against the pinned oracle's
output on 2026-09-20. Each is a difference in *spelling*, not in meaning, and each is why loop D
compares values and the generated corpus's manifest is the oracle for formula text:

| ours | the oracle's | what it is |
|---|---|---|
| `[Data.A1]` | `[$Data.A1]` | Excel has no sheet-relative form to distinguish, so both readings are defensible |
| `#REF!` | `[#REF!]` | §5.8's bracketed reference error and §5.12's constant error are one `Expr::Error` in this core, and its serialiser writes the name |
| `NOSUCHFUNCTION(…)` | `nosuchfunction(…)` | the oracle lower-cases a name it does not know |
| `XLOOKUP(…)` | `COM.MICROSOFT.XLOOKUP(…)` | it namespaces a function that is not OpenFormula's; this filter takes the `_xlfn.` marker off and leaves the name |

### 2.5 What producers write where a string delimiter belongs — `MEASURED`

`sc/qa/unit/data/xlsx/tdf165886.xlsx` writes `OR(D1=0,D1<>““)` — typographic quotation marks,
four spellings of them, where `"` belongs. They are not string delimiters in Excel's grammar
either, and the file is a LibreOffice bug reproduction rather than something a spreadsheet
produced. Twelve of the fifteen expressions X2's translator refuses as unreadable across the
whole 362-workbook corpus are in this one file.

---

## 3. Number formats

### 3.1 Built-in ids 0–49 — `SPEC` (§18.8.30), `MEASURED` against the oracle

The spec lists literal codes; Excel renders several of them in the user's locale, so the
mapping this filter wants is **by meaning**, not by code.

**Which ids make a number a date or a time** (X1's half): 14–17 and 22 are dates, 18–21 and
45–47 are times, and every other id is a number. Measured by `numfmt/builtins.xlsx` (one cell
per id) against both the manifest and the pinned oracle, 2026-09-19. The East Asian date ids
(27–36, 50–58) are **not** counted: §18.8.30 does not define them, and a cell using one keeps
its serial as a plain number, which is visible and recoverable rather than a wrong date.
`UNVERIFIED` which of them Excel writes.

**What each id becomes** (X3's half). Measured by converting `numfmt/builtins.xlsx` with the
oracle and reading the `number:*-style` each cell's style points at, 2026-09-20. The oracle
translates a built-in by **interpreting its literal code**, with the locale applied to the date
ones: id 14 (`mm-dd-yy`) comes out as `m/d/yyyy` from an en-US machine, and would come out
differently from a German one. So `xlsx/src/numfmt.rs` keeps a code per id and runs the ordinary
parser over it, with three departures, each of which is a decision rather than a measurement:

| id(s) | the table's code | what this filter uses | why |
|---|---|---|---|
| 14, 22 | `mm-dd-yy`, `m/d/yy h:mm` | `yyyy-mm-dd`, `yyyy-mm-dd hh:mm` | the oracle's answer is a fact about the converting machine; ISO means the same day everywhere, which is `date.rs`'s own reason. Loop D names the difference |
| 47 | `mmss.0` | `mm:ss.0` | minutes and seconds run together are a spelling this model does not produce; the oracle inserts the same separator |
| 5–8, 41–44 | `"$"#,##0_);("$"#,##0)` … | **nothing** | the symbol is the *reader's* locale and appears nowhere in the file. `Unspellable::LocaleCurrency`, counted |

Ids 11, 12, 13, 46 and 48 are carried by the oracle as `number:scientific-number`,
`number:fraction` and `number:truncate-on-overflow="false"`, none of which this model has a
`Part` for; they are refused and counted (§3.6). Everything else — 1–4, 9, 10, 15–21, 37–40, 45,
49 — comes through as the oracle spells it, checked cell by cell by converting both sides to CSV
and diffing them.

### 3.2 Format code grammar — `SPEC`, and one rule `MEASURED`

§18.8.31. The pieces the parser must handle are listed in `doc/xlsx-import.md` Part II §4.
What is *not* spec is how Excel resolves a four-section code against ODF's two-branch
`style:map`; that is a measurement and goes here.

**`m` is a month or minutes by position — `MEASURED`.** `m` and `mm` are minutes when the
nearest date/time token before them is an hour (`h:mm`, `[h]:mm`) or the nearest after is a
second (`mm:ss`), and a month everywhere else; three or more `m` are always a month.
`numfmt/datetime-codes.xlsx` has one cell per spelling and the manifest and the oracle agree:
`m`, `mm`, `mmm`, `mmmm` and `mmmmm` alone are months, so the cell is a **date**. The first
version of `numfmt::classify` read a lone `m` as minutes — "no year or day beside it, so no
month for it to be" — and was caught by that fixture on 2026-09-19. The same rule makes
`[$-407]TT.MM.JJJJ` (German letters for day and year, which are not tokens) a date, because
what is left is a lone `MM`.

### 3.3 `applyNumberFormat` — `SPEC`, and its sibling `MEASURED`

On a `<cellXfs>` entry it says whether the cell format overrides its parent cell style; the
`numFmtId` it carries is the one in effect either way (§18.8.45), and `xlsx/src/styles.rs`
does not consult it. Not yet asked of the corpus whether any producer disagrees about this flag
— but its sibling `applyFont` was measured at X4, and the oracle reads it the same way (§4.4).

### 3.4 Sections, and which one becomes the style — `MEASURED`

Excel's code has up to four sections (`positive;negative;zero;text`) plus optional conditions;
ODF has one style with `style:map` branches, first match wins (§16.3). How the two meet is not
in either spec. Measured by converting `numfmt/sections.xlsx` and `numfmt/conditions.xlsx` with
the oracle and reading what it wrote, 2026-09-20 — the style is always the **fallback**, the
section that applies when no condition holds:

| sections | the style | the maps |
|---|---|---|
| `pos` | `pos` | — |
| `pos;neg` | `neg` | `value()>=0` → `pos` |
| `pos;neg;zero` | `zero` | `value()>0` → `pos`, `value()<0` → `neg` |
| `pos;neg;zero;text` | `text`, as a `number:text-style` | the three above |
| any with `[>=100]`-style conditions | the last section carrying no condition | each conditioned section, in order |

Two consequences worth writing down, both measured by converting the oracle's own output to
CSV and reading the rendered text:

- **A condition is not a loss.** `doc/xlsx-import.md` Part II §4 expected to drop `[>=100]`;
  §16.3's `style:condition` is the same six operators, so every condition in
  `numfmt/conditions.xlsx` comes through exactly.
- **An empty section hides what it covers.** `0.0;;` becomes a style with no parts at all and
  two maps, which renders as nothing — the oracle writes `<number:text/>` and means the same.

**Where ODF's renderers differ from Excel — `MEASURED`.** For `[>=100]#,##0;0.00` at −50 the
oracle renders `50.00`, unsigned: a style carrying any `style:map` spells its own sign, and the
fallback of a *conditional* code carries none. Excel signs it. This build agrees with the
oracle, because the rule is ODF's rather than this filter's (`doc/ods-format.md` §5.2).

### 3.5 Date and time pieces — `MEASURED`

From the same conversion of `numfmt/datetime-codes.xlsx`, one cell per spelling. The lengths are
§18.8.31's and the parts are the oracle's:

| code | `Part` | note |
|---|---|---|
| `yy`, `yyyy` | `Year { long }` | three or more `y` is long |
| `m`, `mm` | `Month { long }` | numeric; and see §3.2 for when it is minutes instead |
| `mmm`, `mmmm` | `Month { textual, long }` | `Mar`, `March` |
| `mmmmm` | `Month { textual, short }` | the month's **initial** in Excel; neither ODF nor the oracle has it, and both render `Mar` |
| `d`, `dd` | `Day { long }` | |
| `ddd`, `dddd` | `DayOfWeek { long }` | a weekday, not a wider day |
| `h`, `hh` | `Hours { long }` | |
| `s`, `ss` | `Seconds { long }` | |
| `ss.0`, `ss.000` | `Seconds { decimals }` | the point and the zeros belong to the seconds, not to a literal |
| `AM/PM` | `AmPm` | and it is what makes the hours a 12-hour clock |
| `A/P` | `AmPm` | the one-letter marker has no ODF spelling |
| `[$€-407]` | `Currency("€")` | the symbol carries; the LCID does not |

### 3.6 What has no `Part` at all — `MEASURED`

The oracle writes `number:fraction`, `number:scientific-number`, `number:display-factor`,
`number:fill-character`, `number:truncate-on-overflow="false"` and its own
`loext:blank-width-char`; `grind_sheet::numfmt::Part` has none of these, and inventing one is a
decision about the core's format model rather than about an import filter.
`grind_xlsx::numfmt::Unspellable` is the list, and it is split in two by one question — *would
losing this piece misstate the number?* A fraction, an exponent, an elapsed hour count, a ×1000
factor and a `General` section beside others all would, so the cell's **whole** format is
dropped and it shows its plain value; a fill character, a blank-width pad, a colour, the
one-letter month and meridiem markers and a format's LCID would not, so the format is carried
without them. Either way the class is counted per cell in the report, and
`xlsx/tests/ooxmlgen.rs`'s `UNSPELLABLE` table holds each one to a fixture that still fails.

---

## 4. Styles and geometry

Everything in this section was measured on 2026-09-21 against **LibreOffice 26.8.0.3** — the
oracle on this machine, not the pinned 26.2.5.2 CI uses — by converting each fixture with
`soffice --headless --convert-to fods` and reading the automatic style every cell's
`table:style-name` points at. The `styles/` and `geometry/` fixtures of the generated corpus
are the inputs, and three workbooks were generated for the purpose (§4.1, §4.2, §4.5); the
commands are beside each fact. Loop D holds every one of them to the pinned oracle in CI,
which is where a difference between the two versions would surface.

### 4.1 Indexed colours — `MEASURED`

ECMA-376 §18.8.27's legacy palette, **read back rather than transcribed**: a workbook with one
font per index 0–65, one cell per font, converted and read. Indices 0–7 repeat as 8–15, and
16–63 are the Excel 97 palette; `xlsx/src/color.rs`'s `PALETTE` is the result. Two notes:

- 0 and 8 came back with no `fo:color` at all — black is the default ink, so the oracle wrote
  nothing — and are black in the table, which is what §18.8.27 says.
- **64 and 65 are the system's foreground and background**, not palette entries. The oracle
  resolves them inconsistently: as no colour on a plain font, as `#000000` and `#ffffff` in
  `styles/colors.xlsx` B12 and B13. Both are ODF's *automatic* colour, and this filter carries
  them as no colour — a fixed black is black text on a dark theme. Loop D names it.

A workbook's own `<colors><indexedColors>` replaces the table entry for entry (§18.8.27); it
is read, and a slot that will not parse keeps its place so the indices after it still mean what
the file meant. `SPEC`: no fixture carries one yet.

### 4.2 Theme colours and tint — `MEASURED`

**Slot order.** A `theme` attribute counts `lt1, dk1, lt2, dk2, accent1…accent6, hlink,
folHlink`; the scheme lists `dk1, lt1, dk2, lt2, …`. Measured: theme 0 is `#ffffff` and theme 1
`#000000` in `styles/colors.xlsx`, whose scheme puts `dk1` first — and every accent comes
through by its own index. A reader that indexes the scheme in document order gets white and
black backwards and everything else right.

A `sysClr` is read by its `lastClr`, the value it had when the file was saved.

**Tint.** ECMA-376's formula moves the colour's HSL luminance: `L × (1 + tint)` below zero,
`L × (1 − tint) + tint` above. Hue and saturation are untouched. A workbook with every theme
slot under tints −0.5, −0.25, 0.2, 0.4, 0.6 and 0.8 — 72 colours — plus `colors.xlsx`'s own
three non-zero tints of accent1, 75 in all, converted and read back:

| arithmetic | exact | off |
|---|---|---|
| floating point, rounded half up — **what `color::tint` does** | 69 | 6, each by one step in one channel or two |
| floating point, truncated | 24 | 51 |
| integer HLS, `HLSMAX` 255 | 21 | 54 |
| integer HLS, `HLSMAX` 240 (the Windows routine) | 15 | 60 |

No quantisation tried reproduces all 75, so the oracle rounds somewhere the specification does
not say to. The float formula is the specification's and is kept; loop D names the one-step
difference. Accent1 (`4472C4`) at 0.5 is one of the six: `#a2b9e2` here, `#a1b8e1` there.

**No theme.** `styles/borders.xlsx` names `theme="4"` on two edges and has no theme part. The
oracle draws the edges `#ffffff`, and for the same reference on a *font* writes no colour at
all — two guesses. This filter makes none: the colour is unresolved, the edge is drawn in the
ink, and the cell is counted as `Dropped::ThemeColor`, which is what that kind means.

**Strict's DrawingML namespace is `UNVERIFIED`.** The Transitional one,
`http://schemas.openxmlformats.org/drawingml/2006/main`, is read out of `colors.xlsx`; no
workbook this project holds has a Strict theme, so a Strict theme is not read and its colours
are counted as unresolved. `xlsx/src/names.rs` has the reason beside the constant.

### 4.3 Borders — `MEASURED`

`styles/borders.xlsx` has every style on every side and on one side. The oracle's widths:

| Excel | the oracle | this filter |
|---|---|---|
| `thin` | `0.74pt solid` | `0.74pt solid` |
| `medium` | `1.76pt solid` | `1.76pt solid` |
| `thick` | `2.49pt solid` | `2.49pt solid` |
| `hair` | `0.06pt solid` | `0.06pt solid` |
| `double` | `1.76pt double-thin` + `style:border-line-width` | `1.76pt double` |
| `dotted` | `0.74pt dotted` | `0.74pt dotted` |
| `dashed` | `0.74pt fine-dashed` | `0.74pt dashed` |
| `dashDot`, `dashDotDot` | `0.74pt dash-dot`, `dash-dot-dot` | `0.74pt dashed`, counted |
| `mediumDashed` | `1.76pt dashed` | `1.76pt dashed` |
| `mediumDashDot`, `mediumDashDotDot` | `1.76pt dash-dot`, `dash-dot-dot` | `1.76pt dashed`, counted |
| `slantDashDot` | `1.76pt fine-dashed` | `1.76pt dashed`, counted |

The widths are the oracle's. The *lines* are not: `fo:border` is XSL-FO's shorthand (ODF Part 3
§20.183), whose line styles include `solid`, `dotted`, `dashed` and `double` and not
LibreOffice's `fine-dashed`, `dash-dot`, `dash-dot-dot` or `double-thin`. A dash pattern XSL-FO
has no name for is drawn `dashed` and counted as `Appearance::BorderPattern`.

An edge with no `<color>`, or `auto`, is `#000000`: ODF's border has no automatic colour, and
the oracle writes black. `diagonalUp`/`diagonalDown` with a `<diagonal>` line become the
oracle's `style:diagonal-bl-tr`/`-tl-br`, which `CellStyle` does not carry — counted.

### 4.4 Which ids a cell format uses — `MEASURED`

`styles/named-styles.xlsx` A5's cell format names `fontId="3"` (bold), `xfId="1"` (the
`Heading 1` style, 16pt navy) and `applyFont="0"`. The corpus's note expects the heading's font
to win. The oracle draws A5 in the cell format's own bold at 11pt: **the `applyX` flags do not
switch a cell format's own ids off.** The same reading §3.3 takes for `applyNumberFormat`, now
measured for one flag rather than specified for another, and the one that makes a `<cellXfs>`
entry mean one thing. An id the entry leaves out comes from the named style `xfId` points at.

A `<row s="…" customFormat="1">`'s style reaches every cell in the row that names none:
`geometry/rows.xlsx` A10 has no `s` and the oracle draws it in its row's bold. `SPEC`
(§18.3.1.73) for the `customFormat` condition. The same question for a `<col style>` is not
measured and is not implemented.

**What the oracle writes on every cell.** Every automatic style it writes spells the defaults
out — `fo:font-size="11pt"`, `fo:font-style="normal"`, `fo:font-weight`,
`fo:wrap-option="no-wrap"`, `style:vertical-align="bottom"` — on cells that set none of them.
Loop D normalises those away before comparing. A workbook whose cell styles include no `Normal`
(`named-styles.xlsx` declares only `Heading 1` and `Note`) leaves the oracle's `Default` cell
style at LibreOffice's own 10pt, so its `11pt` on each cell is a real statement against that;
this filter's default is the workbook's default font, which is 11pt, and writes none.

### 4.5 Column width — `MEASURED`, and a fact about the converting machine

ECMA-376 §18.3.1.13 counts `<col width>` in the maximum digit width of the default font, and
takes Calibri 11 at 96 dpi — seven pixels — as its worked example. Excel's default column is
`width="9.140625"`: 64/7 truncated to a 256th, the 64-pixel column the UI calls 8.43.

The oracle's widths, from `geometry/columns.xlsx`: 4 → 0.389in, 20 → 1.9445in, 50.5 →
4.9098in — **exactly `width × 7pt`**. Changing the fixture's default font to 20pt Arial moved
them to `width × 12.75pt`. The converting machine had nine fonts, all DejaVu (`fc-list`), and
DejaVu Sans's digit is 0.636 em: 7.0pt at 11pt, 12.7pt at 20pt. So the oracle measures the
digit **in whatever font the machine substitutes for the default one**, in points, with no
padding — its widths are a fact about the converting machine, the way id 14's date order is
(§3.1). Calibri itself is 5.58pt at 11pt, so the same workbook converted on a machine with
Calibri or Carlito would come out a fifth narrower.

This filter uses the specification's reading with its example's constant: `width × 7` pixels at
96 dpi (`grind_xlsx::sheet::DIGIT_PX`), which makes Excel's default column 16.93 mm, two-thirds
of an inch, and every width three-quarters of this oracle's. It has no font metrics — the core
measures text only through a shell — so a workbook whose default font is not Calibri 11 is
converted in Calibri's unit, a few percent off; the ponytail beside the constant has the
upgrade. Loop D names the ratio.

A hidden column at `width="0"` arrives from the oracle at its own default width; this filter
hides it and gives it none. A `<col>` running further than `grind_sheet::MAX_TRACK_RUN` columns,
and `<sheetFormatPr defaultColWidth>`, are carried onto the columns the sheet uses that no
`<col>` mentions — LibreOffice's own `.xlsx` writes `defaultColWidth="11.53515625"` on every
sheet — and no further.

### 4.6 Row height — `MEASURED`

Points, and the oracle agrees to its own rounding: 15 → 0.2083in, 7.5 → 0.1043in, 120.75 →
1.6772in, 409 → 5.6807in (`geometry/rows.xlsx`). Carried where `customHeight="1"`; a height
without it is Excel's measurement of the row's content, which a shell makes again.

`ht="0" customHeight="1"`, not hidden: invisible in Excel. The oracle ignores the height and
shows the row at the default. ODF's `style:row-height` is a `positiveLength` (the schema), so
zero cannot be written; this filter hides the row and counts `Appearance::ZeroSize`. A hidden
row with a height (row 7, 25pt) keeps it: unhiding restores 25pt, as in Excel.

### 4.7 Alignment — `MEASURED`

From `styles/alignment.xlsx`:

| Excel | the oracle | this filter |
|---|---|---|
| `general` | no `fo:text-align`, `text-align-source="value-type"` | nothing |
| `left`, `center`, `right`, `justify` | `start`, `center`, `end`, `justify` | the same |
| `fill` | `start` + `style:repeat-content="true"` | `start`, counted |
| `centerContinuous` | `center` | `center`, counted — it spans the empty cells to its right |
| `distributed` | `justify` + `css3t:text-justify="distribute"` | `justify`, counted |
| vertical `top`, `center`, `bottom` | `top`, `middle`, `bottom` | the same |
| vertical `justify`, `distributed` | `justify`, and wrapping on | nothing, counted — not an ODF value |
| `wrapText` | `fo:wrap-option="wrap"` | the same |
| `shrinkToFit` | `style:shrink-to-fit="true"` | counted |
| `indent` | `fo:margin-left`, 0.1457in per level | counted |
| `textRotation` 45, 90, 135, 180 | `style:rotation-angle` 45, 90, 315, 270 | counted |
| `textRotation` 255 | `style:direction="ttb"` | counted |

ODF's cell `style:vertical-align` is `top | middle | bottom | automatic` (the 1.4 schema), so
the oracle's `justify` there is its own. It also adds `fo:text-align` to a rotated cell whose
`horizontal` is `general` — `start` or `end` by the angle — which the file does not say.

### 4.8 Fills — `MEASURED`

`solid`'s colour is its `fgColor` — the pattern's ink — which is the whole translation. The
oracle **blends** every other pattern into one colour by coverage (`darkGray` over accent1 is
`#668dd9`) and a gradient into its midpoint (`#7f0080`); this filter carries no background and
counts the fill, which is the corpus's own instruction. `patternType` absent is `none` (§18.8.32).

### 4.9 What has no home

Everything above that ends "counted" is `grind_xlsx::Appearance`, one class per piece, in
`Report::appearance_lost`. Two style losses were named before that enum existed and keep their
`Dropped` names because the corpus asks for them by those: a font family other than the default
font's (`FontFamily`, which `grind_sheet::style` refuses on purpose, §5.4 of `ods-format.md`)
and a theme colour with no theme (`ThemeColor`). Underline, strikethrough and a text position
are not in `CellStyle` and are counted rather than carried — `grind_text::style::CharStyle` has
all three, so the vocabulary exists in the suite, and adding them to a cell is a decision about
the spreadsheet's model rather than about this filter.

---

## 5. Functions whose semantics differ under the same name

`doc/xlsx-import.md` Part II §3 takes the decision: the importer carries the name and Excel's
cached value, and never renames or rewrites. This section is the **list** — the names where
Excel and OpenFormula disagree about the answer, so that a future warning can name them
instead of saying "some formulas may differ".

Known candidates, all `UNVERIFIED` until each is run against the oracle: `CEILING` and `FLOOR`
with a negative significand, `MOD` with operands of opposite sign, `ROUND`'s tie rule, and the
error produced by `x^y` for a negative `x` and a fractional `y`. Loop E's machinery already
generates and compares exactly this shape of case, and is the cheap way to build the list.

One thing X2 did settle: of that list, **`CEILING`, `FLOOR`, `ROUNDDOWN` and `ROUNDUP` are not
in the Small Group at all**, so an imported workbook that calls them reports them as unknown
functions and recalculating replaces the value with `#NAME?` rather than with a differently
rounded number. `MOD`, `ROUND`, `INT` and `TRUNC` *are* implemented, and are where the divergence
this section is about actually bites.

---

## 6. Where facts come from

- ECMA-376 / ISO-IEC 29500, cited by part and section. Not vendored here: unlike the OASIS
  ODF specs in `doc/`, it is not redistributed with this repository.
- The pinned oracle (`ci/libreoffice-image`, via `scripts/soffice-docker/soffice`), for
  anything the spec leaves room in. Every measurement records the command and the date.
- LibreOffice's `sc/source/filter/oox/`, read and cited by `file:line`, never copied — and
  only after the specification has been consulted first.
