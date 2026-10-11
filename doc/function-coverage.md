<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Function coverage — against LibreOffice, Excel, and what people use

**Descriptive, not normative.** `doc/small-group.md` is the scope line and `doc/not-doing.md`
the product line; this document is the *evidence* those two ask for before a function moves —
how many functions each application has, which ones real spreadsheets call, and where this
build's 112 stand against both. Measured 2026-10-11; every figure says where it came from, and
the corpus figures re-derive with `scripts/function-usage.py`.

## 1. The answer in four lines

- **112 implemented**: the 110 of ODF 1.4 Part 4 §2.3.2 E) (the Small Group) plus `ROW` and
  `COLUMN`. Every one exists under the same name in Excel and in LibreOffice.
- That is **every function among Enron's fifteen most used except `SUBTOTAL` and `CELL`**, and
  the formula cells of LibreOffice's own test documents call only implemented functions
  **71.6%** of the time.
- The gap is **a band of about thirty classic functions** (Excel 2007 and earlier, ODF Medium
  and Large Group) — `SUBTOTAL`, `SUMPRODUCT`, `IFERROR`, `SUMIFS`, `TEXT`, `CONCATENATE`,
  `ROUNDUP`, … — and then **Excel's post-2010 additions, of which this build has none**.
- Most of those post-2010 additions are not in ODF 1.4 at all, and the most visible of them
  (`FILTER`, `SORT`, `UNIQUE`, `LET`, `LAMBDA`) need an array value the engine does not have.
  That is an engine decision, not a list of functions.

## 2. How many functions there are

| Catalogue | Functions | Implemented here | Source |
|---|---|---|---|
| ODF 1.4 Part 4 §6 | **393** — Small 110, Medium 162, Large 118, three in no group (`EASTERSUNDAY`, `IMCOSH`, `IMSINH`); `CELL` and `DOLLAR` are named as outside even the Large Group | 112 | `doc/OpenDocument-v1.4-os-part4-formula.html`, §2.3.2–§2.3.4 and the §6 headings |
| LibreOffice Calc | **~516** — all of Part 4 plus Excel's newer ones (`XLOOKUP`, `XMATCH`, `FILTER`, `SORT`, `SORTBY`, `UNIQUE`, `SEQUENCE`, `LET`, `LAMBDA`, `MAP`, `REDUCE`, `TEXTSPLIT`, `VSTACK`, `TAKE`, …), written as `COM.MICROSOFT.*` in a file | 112 | One test document per function in `sc/qa/unit/data/functions/*/fods/` of a LibreOffice checkout (516 files); a lower bound on its catalogue, not a census |
| Excel | **521** — 369 from 2007 or earlier, then 56 in 2010, 51 in 2013, 5 in 2016, 6 in 2019, 11 in 2021, 23 in 2024 | 112 of the 369; **0 of the 152** | Microsoft, *Excel functions (alphabetical)* |

Excel by its own categories, implemented over total:

| Category | | Category | |
|---|---|---|---|
| Database | 12 / 12 | Text | 15 / 50 |
| Logical | 6 / 19 | Lookup and reference | 9 / 37 (+3 filed as "Look and reference") |
| Date and time | 11 / 25 | Financial | 10 / 55 |
| Information | 10 / 22 | Statistical | 8 / 111 |
| Math and trigonometry | 27 / 80 | Engineering | 0 / 54 |
| Compatibility | 4 / 40 | Cube, Web, Add-in | 0 / 13 |

**143 of Excel's 521 are not in ODF 1.4 Part 4.** They are of three kinds: the 2010 renames of
functions ODF already has under the old name (`STDEV.S`, `VAR.P`, `RANK.EQ`, `NORM.DIST`,
`PERCENTILE.INC`, …); scalar functions added in 2016–2019 (`IFS`, `SWITCH`, `MAXIFS`,
`MINIFS`, `CONCAT`, `TEXTJOIN`); and the dynamic-array and lambda families of 2021–2024.
LibreOffice reads and writes all of them under `COM.MICROSOFT.`, which is its own convention
and not a standard spelling — so carrying any of them through ODF is a decision about how to
write it, not only how to evaluate it.

## 3. What real spreadsheets call

### Enron — 15,770 workbooks (Hermans & Murphy-Hill, ICSE 2015)

The largest public corpus of spreadsheets people actually worked in. Of 9,012 workbooks that
call a function:

- across 913,472 distinct formulas **only 134 of the 329 functions Excel then had are ever
  used**;
- 54.6% use three or fewer distinct functions, 94.7% fewer than ten, and the workbook with
  the most uses 34;
- 75.6% use *only* the top fifteen.

Table V, by number of workbooks (the four arithmetic operators removed; ✗ = not implemented):

| Rank | Function | Workbooks | Share |
|---|---|---|---|
| 1 | `SUM` | 6,493 | 72.0% |
| 6 | `IF` | 1,827 | 20.3% |
| 7 | `NOW` | 1,501 | 16.7% |
| 8 | `AVERAGE` | 879 | 9.8% |
| 9 | `VLOOKUP` | 763 | 8.5% |
| 10 | `ROUND` | 606 | 6.7% |
| 11 | `TODAY` | 537 | 6.0% |
| 12 | `SUBTOTAL` ✗ | 385 | 4.3% |
| 13 | `MONTH` | 325 | 3.6% |
| 14 | `CELL` ✗ | 321 | 3.6% |
| 15 | `YEAR` | 287 | 3.2% |

Counted by **formula** rather than by workbook (O'Beirne, EuSpRIG 2015), the order is `IF`,
`SUM`, `VLOOKUP`, `MATCH`, `INDEX`, `MONTH`, `DATE`, `YEAR`, `AVERAGE`, `CONCATENATE` ✗,
`AND`, `LN`, `ABS`, `OFFSET` ✗, `ISERROR`, `ROUND`, `EOMONTH` ✗, `SUMIF` — `IF` alone 2.4
million formulas, `VLOOKUP` 1.7 million. O'Beirne also notes many are `{=SUM(IF(…))}` array
formulas: how people wrote `SUMIFS` and `COUNTIFS` before Excel 2007 had them.

### EUSES (Jansen, *Enron versus EUSES*, 2015)

A corpus collected from the web rather than one company. Its top fifteen: `SUM`, `IF`,
`ROUND`, `HYPERLINK` ✗, `CONCATENATE` ✗, `AND`, `COUNTIF`, `AVERAGE`, `OR`, `INDIRECT` ✗,
`MIN`, `ISNUMBER`, `MAX`, `VLOOKUP`, `ISBLANK`. Only six functions are in both corpora's top
fifteen; in both, the top fifteen cover about 70% of formulas.

### LibreOffice's test documents — measured here

`scripts/function-usage.py` over `sc/qa/unit/data` with the per-function directory left out
(`--skip functions`): 274 documents with formulas, 29,926 formula cells.

- **71.6% of formula cells and 156 of 274 files (56.9%)** call only implemented functions.
- Most used, by files: `SUM` 43, `IF` 33, `VLOOKUP` 17, `INDIRECT` ✗ 12, `ROW` 11, `AND` 10,
  `SUBTOTAL` ✗ 10, `SUMPRODUCT` ✗ 10, `ROUND` 10, `INDEX` 9, `COUNTIF` 9.
- Missing, by files blocked: `INDIRECT` 12, `SUBTOTAL` 10, `SUMPRODUCT` 10, `TEXT` 7,
  `FORMULA` 6, `CELL` 6, `ADDRESS` 6, `CONCATENATE` 6, `TRANSPOSE` 5, `FLOOR` 5, `CEILING` 5,
  `IFERROR` 5, `UNIQUE` 4, `SEARCH` 4, `LARGE` 4, `SMALL` 4.

This corpus tests LibreOffice rather than recording use, so it over-represents the odd
(`FORMULA`, `CELL`, `AREAS`); it is useful because it agrees with Enron and EUSES about the
ordinary ones.

### What none of these measures

Enron is from about 2001 and EUSES from the mid-2000s, before `IFERROR`, `SUMIFS` and
`COUNTIFS` existed. A workbook written today leans on those and, since 2021, on `XLOOKUP`,
`FILTER` and `UNIQUE`. No public frequency data for the newer functions was found, so their
place in §4 is inference.

## 4. The gaps, ranked

Each function's ODF group says how far from the Small Group the standard itself places it.

### Tier 1 — classic, in every corpus

| Function | ODF group | Evidence |
|---|---|---|
| `SUBTOTAL` | Medium | Enron #12 by workbook; LO corpus blocks 10 files |
| `SUMPRODUCT` | Medium | LO corpus blocks 10; the pre-`SUMIFS` idiom |
| `CONCATENATE` | Medium | Enron (formulas) #10, EUSES #5, LO 6 |
| `IFERROR` | Large | LO 5; the standard wrapper since Excel 2007 |
| `SUMIFS`, `COUNTIFS`, `AVERAGEIFS` | Large | What Enron's `{=SUM(IF(…))}` became |
| `TEXT` | Large | LO 7 |
| `INDIRECT` | Large | EUSES #10, LO 12 — the most-blocking function measured |
| `OFFSET` | Medium | Enron #7 (Jansen), O'Beirne #14 |
| `CELL` | outside every group | Enron #14 — and a variance swamp, Part 4 leaves much of it implementation-defined |
| `ROUNDUP`, `ROUNDDOWN`, `CEILING`, `FLOOR`, `MROUND` | Medium | LO 3–5 each |
| `LARGE`, `SMALL`, `MEDIAN`, `RANK` | Medium | LO 4 each |
| `SEARCH` | Large | LO 4 |
| `LOOKUP`, `ADDRESS` | Medium | Enron #14 (Jansen); LO 3, 6 |
| `EOMONTH`, `EDATE`, `NETWORKDAYS`, `WORKDAY`, `DATEDIF`, `DATEVALUE`, `WEEKNUM` | Medium; `EDATE` and `DATEDIF` Large | O'Beirne #19 (`EOMONTH`) |
| `HYPERLINK` | Large | EUSES #4 |
| `IPMT`, `PPMT` | Large | The loan-schedule pair beside the `PMT` we have |

### Tier 2 — Excel 2010 and later, scalar

`IFNA` (Large), `IFS`, `SWITCH`, `MAXIFS`, `MINIFS`, `CONCAT`, `TEXTJOIN`, and `XLOOKUP` and
`XMATCH` in their single-value use; and the dotted renames of functions already implemented
(`STDEV.S`, `STDEV.P`, `VAR.S`, `VAR.P`). None but `IFNA` is in ODF 1.4, so each needs a
written decision about its spelling as well as its semantics.

### Tier 3 — needs arrays, not functions

`FILTER`, `SORT`, `SORTBY`, `UNIQUE`, `SEQUENCE`, `TRANSPOSE`, `MMULT`, `TEXTSPLIT`, `LET`,
`LAMBDA` and the rest of the 2021–2024 families. They return a matrix that spills into the
neighbouring cells; the engine has no array value, and inline arrays are a syntactic exclusion
(`doc/not-doing.md`). Adding one of these means adding the array value first.

### Tier 4 — low evidence

Engineering (54), complex numbers, the statistical distributions beyond the basics (most of
Statistical's 111), Cube, Web.

## 5. What a gap does to a user

A formula calling an unimplemented function **still opens with its value**. An `.xlsx` import
carries Excel's cached value verbatim (`doc/xlsx-import.md`), and an ODF file carries the
cached value LibreOffice wrote. The gap appears on **recalculation**, where the cell becomes
`#NAME?` — and `grind sheet recalc` counts a value about to be lost that way as `spoiled`
before it replaces it. So a missing function is experienced as "recalculating broke my
workbook", never as "it would not open".

## 6. How this sits with the scope line

`doc/not-doing.md` admits functions one at a time on written evidence (`ROW` and `COLUMN` were
the first, `doc/small-group.md`'s second half). §3 and §4 are that evidence for Tier 1. One
mismatch is worth knowing when reading that document: its *Large Group* row describes the group
as "26 complex-number functions and specialised conversions", but §2.3.4's list also holds
`IFERROR`, `SUMIFS`, `COUNTIFS`, `AVERAGEIFS`, `TEXT`, `SEARCH`, `INDIRECT`, `EDATE`,
`DATEDIF`, `IPMT`, `PPMT` and `HYPERLINK` — a good third of Tier 1.

## 7. Re-measuring

```sh
python3 -I scripts/function-usage.py "$GRIND_LO_CORPUS/sc/qa/unit/data" --skip functions
```

The implemented set is read out of `sheet/src/formula/funcs/mod.rs`, so the figures move with
the engine. The Excel catalogue was read from Microsoft's alphabetical list (the table's first
column is the name, its year in brackets, the second column's bold prefix the category); the
ODF groups from Part 4 §2.3.2–§2.3.4; LibreOffice's catalogue from the file names under
`sc/qa/unit/data/functions/`.

## Sources

- Hermans, F., Murphy-Hill, E. *Enron's Spreadsheets and Related Emails: A Dataset and
  Analysis.* ICSE 2015 — Tables IV and V.
  <https://repository.tudelft.nl/file/File_6407791c-a000-4062-acaf-63e04502dfdc?preview=1>
- Jansen, B. *Enron versus EUSES: A Comparison of Two Spreadsheet Corpora.* 2015 — Table IV.
  <https://arxiv.org/html/1503.04055>
- O'Beirne, P. *VBA in the spreadsheets from the Enron email corpus.* EuSpRIG 2015.
  <https://eusprig.org/wp-content/uploads/VBA-in-spreadsheets-from-EnronPOBeirne-2015.pdf>
- Microsoft. *Excel functions (alphabetical).*
  <https://support.microsoft.com/en-us/office/excel-functions-alphabetical-b3944572-255d-4efb-bb96-c6d90033e188>
- OASIS. *OpenDocument v1.4 Part 4: Recalculated Formula (OpenFormula) Format*, §2.3 and §6 —
  `doc/OpenDocument-v1.4-os-part4-formula.html`.
- LibreOffice `sc/qa/unit/data` — a checkout's test documents, read as data.
