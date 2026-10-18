<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: CC-BY-4.0
-->

# Function coverage — against LibreOffice, Excel, and what people use

**Descriptive, not normative.** `doc/small-group.md` is the scope line and `doc/not-doing.md`
the product line; this document is the *evidence* those two ask for before a function moves —
how many functions each application has, which ones real spreadsheets call, and where this
build's 138 stand against both. Measured 2026-10-11 and re-measured 2026-10-12, after the
classic band that needed no new machinery moved in; every figure says where it came from, and
the corpus figures re-derive with `scripts/function-usage.py`.

## 1. The answer in four lines

- **138 implemented**: the 110 of ODF 1.4 Part 4 §2.3.2 E) (the Small Group), `ROW` and
  `COLUMN`, and the 26 of Tier 1 below that needed no new machinery (`doc/small-group.md`'s
  second half). Every one exists under the same name in Excel and in LibreOffice.
- That is **every function among Enron's fifteen most used except `SUBTOTAL` and `CELL`**, and
  the formula cells of LibreOffice's own test documents call only implemented functions
  **73.5%** of the time (71.6% at 112).
- What is left of the classic band is **the part that needs an engine change** — `SUBTOTAL`
  (a row's filter state), `SUMPRODUCT` (ForceArray), `INDIRECT` and `OFFSET` (a function
  that returns a reference), `TEXT` (a format code), `CELL`, and `CEILING`/`FLOOR` (Excel's
  namesakes round differently) — and then **Excel's post-2010 additions, of which this build
  has two** (`IFNA` and `ISOWEEKNUM`, both 2013 and both in ODF).
- Most of those post-2010 additions are not in ODF 1.4 at all, and the most visible of them
  (`FILTER`, `SORT`, `UNIQUE`, `LET`, `LAMBDA`) need an array value the engine does not have.
  That is an engine decision, not a list of functions.

## 2. How many functions there are

| Catalogue | Functions | Implemented here | Source |
|---|---|---|---|
| ODF 1.4 Part 4 §6 | **393** — Small 110, Medium 162, Large 118, three in no group (`EASTERSUNDAY`, `IMCOSH`, `IMSINH`); `CELL` and `DOLLAR` are named as outside even the Large Group | 138 | `doc/OpenDocument-v1.4-os-part4-formula.html`, §2.3.2–§2.3.4 and the §6 headings |
| LibreOffice Calc | **~516** — all of Part 4 plus Excel's newer ones (`XLOOKUP`, `XMATCH`, `FILTER`, `SORT`, `SORTBY`, `UNIQUE`, `SEQUENCE`, `LET`, `LAMBDA`, `MAP`, `REDUCE`, `TEXTSPLIT`, `VSTACK`, `TAKE`, …), written as `COM.MICROSOFT.*` in a file | 138 | One test document per function in `sc/qa/unit/data/functions/*/fods/` of a LibreOffice checkout (516 files); a lower bound on its catalogue, not a census |
| Excel | **521** — 369 from 2007 or earlier, then 56 in 2010, 51 in 2013, 5 in 2016, 6 in 2019, 11 in 2021, 23 in 2024 | 136 of the 369; **2 of the 152** | Microsoft, *Excel functions (alphabetical)* |

Excel by its own categories, implemented over total:

| Category | | Category | |
|---|---|---|---|
| Database | 12 / 12 | Text | 17 / 50 |
| Logical | 8 / 19 | Lookup and reference | 11 / 37 (+3 filed as "Look and reference") |
| Date and time | 17 / 25 | Financial | 12 / 55 |
| Information | 10 / 22 | Statistical | 13 / 111 |
| Math and trigonometry | 33 / 80 | Engineering | 0 / 54 |
| Compatibility | 5 / 40 (`RANK`) | Cube, Web, Add-in | 0 / 13 |

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
`SUM`, `VLOOKUP`, `MATCH`, `INDEX`, `MONTH`, `DATE`, `YEAR`, `AVERAGE`, `CONCATENATE`,
`AND`, `LN`, `ABS`, `OFFSET` ✗, `ISERROR`, `ROUND`, `EOMONTH`, `SUMIF` — `IF` alone 2.4
million formulas, `VLOOKUP` 1.7 million. O'Beirne also notes many are `{=SUM(IF(…))}` array
formulas: how people wrote `SUMIFS` and `COUNTIFS` before Excel 2007 had them.

### EUSES (Jansen, *Enron versus EUSES*, 2015)

A corpus collected from the web rather than one company. Its top fifteen: `SUM`, `IF`,
`ROUND`, `HYPERLINK`, `CONCATENATE`, `AND`, `COUNTIF`, `AVERAGE`, `OR`, `INDIRECT` ✗,
`MIN`, `ISNUMBER`, `MAX`, `VLOOKUP`, `ISBLANK`. Only six functions are in both corpora's top
fifteen; in both, the top fifteen cover about 70% of formulas.

### LibreOffice's test documents — measured here

`scripts/function-usage.py` over `sc/qa/unit/data` with the per-function directory left out
(`--skip functions`): 274 documents with formulas, 29,926 formula cells.

- **73.5% of formula cells and 171 of 274 files (62.4%)** call only implemented functions —
  71.6% and 156 (56.9%) at 112, before the classic band moved in.
- Most used, by files: `SUM` 43, `IF` 33, `VLOOKUP` 17, `INDIRECT` ✗ 12, `ROW` 11, `AND` 10,
  `SUBTOTAL` ✗ 10, `SUMPRODUCT` ✗ 10, `ROUND` 10, `INDEX` 9, `COUNTIF` 9.
- Missing, by files blocked: `INDIRECT` 12, `SUBTOTAL` 10, `SUMPRODUCT` 10, `TEXT` 7,
  `FORMULA` 6, `CELL` 6, `TRANSPOSE` 5, `CEILING` 5, `FLOOR` 5, `UNIQUE` 4, `LOOKUP` 3,
  `OFFSET` 3.
- **By files unlocked**, which is the better order to build in: a file that calls three missing
  functions is unlocked by none of them alone, so "files blocked" overstates each. A greedy
  pass — add whichever function makes the most further files fully computable, repeat — ran
  `SUBTOTAL`, `SUMPRODUCT`, `INDIRECT`, `CONCATENATE`, `TEXT`, `SEARCH`, `CELL`, `ADDRESS`,
  `QUOTIENT`, `ISOWEEKNUM`, … at 112 (156 → 196 files in eleven steps). At 138 it runs
  `INDIRECT` (+9 files), `SUBTOTAL` (+8), `SUMPRODUCT` (+7), `TEXT` (+3), `CELL` (+3),
  `FREQUENCY` (+2), and then single files. Weighting by formula *cells* instead is useless
  here: single test sheets of a thousand cells each (`BITAND`, `ACOT`, `ACOSH`, …) dominate it.

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

✓ marks the 26 that moved in on 2026-10-12 (`doc/small-group.md`'s second half); each row
left says, in its last column, what stands in its way.

| Function | ODF group | Evidence |
|---|---|---|
| `SUBTOTAL` | Medium | Enron #12 by workbook; LO corpus blocks 10 files — needs a row's filter and collapse state inside evaluation |
| `SUMPRODUCT` | Medium | LO corpus blocks 10; the pre-`SUMIFS` idiom — needs ForceArray, operators applied cell by cell |
| ✓ `CONCATENATE` | Medium | Enron (formulas) #10, EUSES #5, LO 6 |
| ✓ `IFERROR` | Large | LO 5; the standard wrapper since Excel 2007 |
| ✓ `SUMIFS`, `COUNTIFS`, `AVERAGEIFS` | Large | What Enron's `{=SUM(IF(…))}` became |
| `TEXT` | Large | LO 7 — a format code is implementation-defined, and LibreOffice reads it in the document's locale |
| `INDIRECT` | Large | EUSES #10, LO 12 — the most-blocking function measured; needs a function that returns a reference |
| `OFFSET` | Medium | Enron #7 (Jansen), O'Beirne #14; as `INDIRECT` |
| `CELL` | outside every group | Enron #14 — and a variance swamp, Part 4 leaves much of it implementation-defined |
| ✓ `ROUNDUP`, `ROUNDDOWN`, `MROUND` | Medium | LO 3–5 each |
| ✓ `CEILING`, `FLOOR` | Medium | LO 5 each — moved in on 2026-10-18 with Excel's own two beside them, `COM.MICROSOFT.CEILING` and `COM.MICROSOFT.FLOOR`, which is what LibreOffice's import (and now ours) calls Excel's; the worry about imported workbooks was about those names (`doc/xlsx-format.md` §5.1) |
| ✓ `LARGE`, `SMALL`, `MEDIAN`, `RANK` | Medium | LO 4 each |
| ✓ `SEARCH` | Large | LO 4 |
| ✓ `ADDRESS` | Medium | LO 6 |
| `LOOKUP` | Medium | Enron #14 (Jansen); LO 3 — its array form is ForceArray |
| ✓ `EOMONTH`, `EDATE`, `NETWORKDAYS`, `WORKDAY`, `WEEKNUM`, `ISOWEEKNUM` | Medium; `EDATE` Large | O'Beirne #19 (`EOMONTH`) |
| `DATEDIF`, `DATEVALUE` | `DATEDIF` Large | `DATEVALUE` parses locale-dependent text |
| ✓ `HYPERLINK` | Large | EUSES #4 |
| ✓ `IPMT`, `PPMT` | Large | The loan-schedule pair beside the `PMT` we have |
| ✓ `QUOTIENT`, `SIGN` | Medium | LO 3 and 2 |

### Tier 2 — Excel 2010 and later, scalar

✓ `IFNA` (Large, and the one of these ODF 1.4 has); `IFS`, `SWITCH`, `MAXIFS`, `MINIFS`,
`CONCAT`, `TEXTJOIN`, and `XLOOKUP` and `XMATCH` in their single-value use; and the dotted
renames of functions already implemented (`STDEV.S`, `STDEV.P`, `VAR.S`, `VAR.P`). None of the
rest is in ODF 1.4, so each needs a written decision about its spelling as well as its
semantics — and `MAXIFS`/`MINIFS` are, past that decision, `criterion::several` with a new
`Mode`.

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
the first, `doc/small-group.md`'s second half). §3 and §4 are that evidence for Tier 1, and
the 26 of it that needed no new machinery have moved on it. One
mismatch was worth knowing when reading that document: its *Large Group* row described the
group as "26 complex-number functions and specialised conversions", but §2.3.4's list also
holds `IFERROR`, `SUMIFS`, `COUNTIFS`, `AVERAGEIFS`, `TEXT`, `SEARCH`, `INDIRECT`, `EDATE`,
`DATEDIF`, `IPMT`, `PPMT` and `HYPERLINK` — a good third of Tier 1. The row was amended on
2026-10-12 when the classic band moved in.

## 7. Where the work stands

As of 2026-10-12. §1–§4 are the measurements; this is the state of the work they led to, and
what each next step is waiting on.

**Done — the classic band that needed no new machinery.** The 26 functions marked ✓ in §4,
admitted in `doc/small-group.md`'s second half with their evidence. How they were built and
where each piece is:

- **Expectations measured, not assumed.** `sheet/src/formula/funcs/beyond.rs` holds every
  one: each value comes from Part 4, or — where Part 4 is silent — from LibreOffice 26.8
  converting a flat document that holds the formula with no cached value, under
  `table:calculation-settings table:use-wildcards="true"`. Without that setting LibreOffice
  reads every pattern as a regular expression. `doc/ods-format.md` §4.1 is the list of what
  the oracle answered and what this build decided.
- **Where the spec and the oracle disagree, the spec wins**, and the test says so at the line:
  `CONCATENATE` writes a true Logical as `"TRUE"` (§6.3.14; LibreOffice writes `1`), `MROUND`
  takes "the greater one" on a tie (−5 for `MROUND(-7.5;-5)`; LibreOffice and Excel give −10),
  `ROUNDUP(8.234567890134;11)` rounds up (LibreOffice gives …013), `WORKDAY` takes §6.10.23's
  fourth parameter (LibreOffice 26.8 refuses it), and a text `N` in `LARGE` converts (§6.3.5).
  `WEEKNUM` puts the last days of December in week 1 of the next year, read literally, and the
  oracle agrees (Excel says 53). The `IPMT` cells loop B still disagrees about are
  LibreOffice's precision rather than ours — exact rational arithmetic over the same doubles
  sides with this build from the twelfth digit.
- **One change beyond the 26.** `ROUND`, `TRUNC` and `INT` now step the value a cell *shows*
  — read to fifteen significant digits, and only below 10^11 (`math::shown`) — so
  `ROUND(0.285;2)` is 0.29 as in LibreOffice and Excel, where it used to be 0.28. Above 10^11
  the oracle's answer depends on its own binary steps and no rule in digits reproduces it; a
  16-digit integer such as `2^52+1` is left exact.
- **Two pieces of shared machinery.** `wildcard::find` (an unanchored match, for `SEARCH`) and
  `criterion::several` (the `IFS` functions). Their signatures repeat a *pair* of parameters,
  so `friendly.rs` reads a repeated group and `assist::part_for` is the one answer to which
  part of a signature a hint emphasises, used by the core's band, the GNOME window and the
  browser.
- **The ratchets.** Loop B's `FLOOR` went from 13320 to 13900 (13395 → 14116 cells on a
  2026-10 checkout, less a margin since CI's clone need not match). Loop E holds at 917 of
  1000 with no new class of disagreement (`doc/differential-fuzz.md`).
  `examples/sample-sheet.sh` uses ten of the functions, and `cli/tests/cli.rs` asserts five.

**Known and left alone.** `FIND("";"abc")` is 1 here and `#VALUE!` in LibreOffice — older than
this work and not yet decided. `ROUNDUP(8.06-8;2)` is 0.06 there and 0.07 here: LibreOffice
tidies `6.0000000000000497` more coarsely than fifteen digits, and the measurements could not
pin down the rule (`sal/rtl/math.cxx`, which would say, is not in the checkout used).
LibreOffice's own fixtures for `SEARCH`, `COUNTIFS` and `AVERAGEIFS` use regular expressions,
which this build reads as wildcards by `wildcard.rs`'s decision.

**Next — each waits on a decision, not on code.** In greedy order (§3), with what it needs:

| Function | Unlocks | Needs | Decision owed |
|---|---|---|---|
| `INDIRECT`, `OFFSET` | 9 files, then 3 | a function that returns a reference: `funcs::call` hands back an `Operand`, and `INDEX` should too | `graph.rs` cannot see a reference built at run time, so cell roles, lint and sheet-rename rewriting go wrong for a cell read only through one — name that gap or close it |
| `SUBTOTAL` (and `AGGREGATE`) | 8 files | a row's filter and collapse state inside evaluation, and whether a referenced cell's formula calls `SUBTOTAL` | none in principle — the model already separates filter-hidden rows from rows hidden by hand, which is exactly §6.16.60's split |
| `SUMPRODUCT` | 7 files | ForceArray: operators applied cell by cell over ranges, kept inside the function with no array value that spills | whether that narrows or breaks `doc/not-doing.md`'s "Array/matrix formulas" row; the same machinery would give `LOOKUP`'s array form and single-cell `{=SUM(IF(…))}` |
| `TEXT` | 3 files | a format-code language | the code is implementation-defined and LibreOffice reads it in the document's locale; `numfmt` holds no format-code strings by rule, though `grind-xlsx`'s `numfmt.rs` parses Excel's and could be hoisted |
| `CELL` | 3 files | an info-type vocabulary | outside every ODF group, and much of it implementation-defined |
| `MAXIFS`, `MINIFS` and the rest of Tier 2 | — | `criterion::several` with a new `Mode`, for those two | each is outside ODF 1.4 and needs its spelling decided (`COM.MICROSOFT.*` is LibreOffice's convention, not a standard) |

## 8. Re-measuring

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
