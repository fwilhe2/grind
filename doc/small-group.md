<!--
SPDX-FileCopyrightText: 2025 OASIS Open
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: LicenseRef-OASIS-IPR AND CC-BY-4.0
-->

# OpenFormula Small Group — the phase 4 work list

Extracted verbatim from **ODF 1.4 Part 4 §2.3.2 E)** (`OpenDocument-v1.4-os-part4-formula.html`),
not estimated. Implementing all of these, plus §2.3.2 A–D and G, makes the engine a
*conforming OpenDocument Formula Small Group Evaluator* — a citable claim rather than a
vague one.

Section numbers point at each function's normative definition in Part 4. That definition —
arguments, types, constraints, error behaviour — is the spec for the Rust function; read it
before writing one. Loop B decides the order to write them in.

**Total: 110 functions.**

Note what Small Group already includes and my first estimate did not: 12 database functions
and 10 financial ones. It excludes inline arrays, complex numbers, and the reference union
operator `~` (§2.3.2 G), and need not evaluate multi-area references (F).

## Mathematical (24)

- `ABS` — §6.16.2
- `ACOS` — §6.16.3
- `ASIN` — §6.16.7
- `ATAN` — §6.16.9
- `ATAN2` — §6.16.10
- `COS` — §6.16.19
- `DEGREES` — §6.16.25
- `EVEN` — §6.16.30
- `EXP` — §6.16.31
- `FACT` — §6.16.32
- `LN` — §6.16.39
- `LOG` — §6.16.40
- `LOG10` — §6.16.41
- `MOD` — §6.16.42
- `ODD` — §6.16.44
- `PI` — §6.16.45
- `POWER` — §6.16.46
- `PRODUCT` — §6.16.47
- `RADIANS` — §6.16.49
- `SIN` — §6.16.55
- `SQRT` — §6.16.58
- `SUM` — §6.16.61
- `SUMIF` — §6.16.62
- `TAN` — §6.16.69

## Information (17)

- `COLUMNS` — §6.13.5
- `COUNT` — §6.13.6
- `COUNTA` — §6.13.7
- `COUNTBLANK` — §6.13.8
- `COUNTIF` — §6.13.9
- `ISBLANK` — §6.13.14
- `ISERR` — §6.13.15
- `ISERROR` — §6.13.16
- `ISLOGICAL` — §6.13.19
- `ISNA` — §6.13.20
- `ISNONTEXT` — §6.13.21
- `ISNUMBER` — §6.13.22
- `ISTEXT` — §6.13.25
- `N` — §6.13.26
- `NA` — §6.13.27
- `ROWS` — §6.13.30
- `VALUE` — §6.13.34

## Text (14)

- `EXACT` — §6.20.8
- `FIND` — §6.20.9
- `LEFT` — §6.20.12
- `LEN` — §6.20.13
- `LOWER` — §6.20.14
- `MID` — §6.20.15
- `PROPER` — §6.20.16
- `REPLACE` — §6.20.17
- `REPT` — §6.20.18
- `RIGHT` — §6.20.19
- `SUBSTITUTE` — §6.20.21
- `T` — §6.20.22
- `TRIM` — §6.20.24
- `UPPER` — §6.20.27

## Database (12)

- `DAVERAGE` — §6.9.2
- `DCOUNT` — §6.9.3
- `DCOUNTA` — §6.9.4
- `DGET` — §6.9.5
- `DMAX` — §6.9.6
- `DMIN` — §6.9.7
- `DPRODUCT` — §6.9.8
- `DSTDEV` — §6.9.9
- `DSTDEVP` — §6.9.10
- `DSUM` — §6.9.11
- `DVAR` — §6.9.12
- `DVARP` — §6.9.13

## Date and Time (11)

- `DATE` — §6.10.2
- `DAY` — §6.10.5
- `HOUR` — §6.10.11
- `MINUTE` — §6.10.13
- `MONTH` — §6.10.14
- `NOW` — §6.10.16
- `SECOND` — §6.10.17
- `TIME` — §6.10.18
- `TODAY` — §6.10.20
- `WEEKDAY` — §6.10.21
- `YEAR` — §6.10.24

## Financial (10)

- `DDB` — §6.12.14
- `FV` — §6.12.20
- `IRR` — §6.12.24
- `NPER` — §6.12.29
- `NPV` — §6.12.30
- `PMT` — §6.12.36
- `PV` — §6.12.41
- `RATE` — §6.12.42
- `SLN` — §6.12.45
- `SYD` — §6.12.46

## Statistical (8)

- `AVERAGE` — §6.18.3
- `AVERAGEIF` — §6.18.5
- `MAX` — §6.18.45
- `MIN` — §6.18.48
- `STDEV` — §6.18.72
- `STDEVP` — §6.18.74
- `VAR` — §6.18.82
- `VARP` — §6.18.84

## Logical (6)

- `AND` — §6.15.2
- `FALSE` — §6.15.3
- `IF` — §6.15.4
- `NOT` — §6.15.7
- `OR` — §6.15.8
- `TRUE` — §6.15.9

## Lookup (5)

- `CHOOSE` — §6.14.3
- `HLOOKUP` — §6.14.5
- `INDEX` — §6.14.6
- `MATCH` — §6.14.9
- `VLOOKUP` — §6.14.12

## Rounding (3)

- `INT` — §6.17.2
- `ROUND` — §6.17.5
- `TRUNC` — §6.17.8

---

# Beyond the Small Group

**Nothing below is from the spec's list.** Everything above is §2.3.2 E) verbatim and stays
exactly 110; this section is the anti-bloat rule's escape hatch, and the plan's own gate
applies — one item at a time, by explicit decision, with the evidence written down. A
function here is still a Part 4 function with a normative definition; what it is not is part
of the Small Group conformance claim, which remains "all 110 of §2.3.2 E)" and is unaffected
by anything on this list.

Every function admitted, on either side of the line, also gets **samples**: realistic calls in
odsgen's `function_samples.py`, recalculated by LibreOffice and vendored by
`scripts/function-samples.sh` into `sheet/tests/data/functions/`, where
`sheet/tests/function_samples.rs` fails until the new function is called there.

`doc/function-coverage.md` is the measured side of the gate: which functions real
spreadsheets call, how far this build is from LibreOffice and Excel, and the candidates ranked
by that evidence. It recommends nothing on its own; an entry here still needs its own decision.

- `ROW` — §6.13.29
- `COLUMN` — §6.13.4

  **Evidence:** `fizzbuzz.fods` is an R7 document (`doc/plan.md`), and its entire content is
  eighteen copies of `IF(MOD(ROW();15)=0;"fizzbuzz";…)`. It read, wrote and round-tripped
  while recalculating to eighteen `#NAME?`. §2.3.2 E) admits `ROWS` and `COLUMNS` — the
  shape of a reference — and not the singulars, which are the *position* of one; the two are
  a natural pair and the omission reads as a line drawn at array-returning functions rather
  than at these. Eight lines in `info.rs`, no new machinery: `Args` already carries the
  address the formula sits at, because that is what a reference is resolved against.
  Decided 2026-08-16. The array form of `ROW` over a multi-row reference stays out with the
  rest of inline arrays.

- `IFERROR` — §6.15.5
- `IFNA` — §6.15.6
- `CONCATENATE` — §6.20.6
- `SEARCH` — §6.20.20
- `HYPERLINK` — §6.11.3
- `QUOTIENT` — §6.16.48
- `SIGN` — §6.16.54
- `ROUNDUP` — §6.17.7
- `ROUNDDOWN` — §6.17.6
- `MROUND` — §6.17.4
- `LARGE` — §6.18.40
- `SMALL` — §6.18.70
- `MEDIAN` — §6.18.47
- `RANK` — §6.18.65
- `SUMIFS` — §6.16.63
- `COUNTIFS` — §6.13.10
- `AVERAGEIFS` — §6.18.6
- `EOMONTH` — §6.10.10
- `EDATE` — §6.10.9
- `WEEKNUM` — §6.10.22
- `ISOWEEKNUM` — §6.10.12
- `NETWORKDAYS` — §6.10.15
- `WORKDAY` — §6.10.23
- `IPMT` — §6.12.23
- `PPMT` — §6.12.37
- `ADDRESS` — §6.14.2

  **Evidence:** `doc/function-coverage.md` §3 and §4's Tier 1 — the classic band every corpus
  calls. Measured over LibreOffice's test documents, a greedy pass adding one function at a
  time unlocks whole files in this order: `SUBTOTAL`, `SUMPRODUCT`, `INDIRECT`, `CONCATENATE`,
  `TEXT`, `SEARCH`, …; `CONCATENATE` is also Enron's tenth function by formula and EUSES's
  fifth, `IFERROR` the standard wrapper since Excel 2007, and `SUMIFS`/`COUNTIFS` what Enron's
  `{=SUM(IF(…))}` array formulas became. This entry admits the part of that band which needs
  **no new machinery** — each one a scalar function over the argument layer, the criterion
  matcher, the calendar or the annuity equation that already exist — and leaves out what does
  need it, each for its own decision: `SUBTOTAL` (the row's filter state), `SUMPRODUCT`
  (ForceArray), `INDIRECT`/`OFFSET` (a function that returns a reference), `TEXT` (a format
  code is implementation-defined), `CELL` (outside every group), and `CEILING`/`FLOOR` (whose
  Excel namesakes round differently, so implementing them changes what an imported workbook
  recalculates to — `doc/xlsx-format.md` §5 has to be measured first). Every expectation in
  `sheet/src/formula/funcs/beyond.rs` was measured against LibreOffice 26.8; where it answers
  what the spec leaves open, `doc/ods-format.md` §4.1 records it, and where it contradicts the
  spec the spec wins and the test says so. Decided 2026-10-12.

- `CEILING` — §6.17.1
- `FLOOR` — §6.17.3

  **Evidence:** five of LibreOffice's test documents each, and `doc/function-coverage.md`'s
  Tier 1. Held back on 2026-10-12 because Excel's two-parameter namesakes round differently
  and an imported workbook would recalculate to a different number; measured on 2026-10-18,
  that worry turned out to be about a different function. LibreOffice writes Excel's
  `CEILING` and `FLOOR` into ODF as `COM.MICROSOFT.CEILING` and `COM.MICROSOFT.FLOOR` (three
  of its test documents carry each), so an ODF file that says `CEILING` means §6.17.1's,
  and the oracle computes that one — the Mode rule, and opposite signs refused
  (`doc/ods-format.md` §4.1). What an `.xlsx` import should spell Excel's as is
  `doc/xlsx-format.md` §5.1's question, and the next entry's answer. No new machinery.
  Decided 2026-10-18.

- `SUBTOTAL` — §6.16.60

  **Evidence:** the first function `doc/function-coverage.md`'s greedy pass adds — it unlocks
  more of LibreOffice's test documents than any other missing function — and Enron's twelfth
  by workbook. It needed one piece of new machinery, and only that: a way for evaluation to ask
  which rows a filter hides, which the model already answers for every renderer
  (`Sheet::row_hidden`) and already keeps apart from rows hidden by hand — exactly §6.16.60's
  split between 1–11 and 101–111. Measured against the oracle with both kinds of hidden row;
  what it answers where §6.16.60 is silent is `doc/ods-format.md` §4.1. Decided 2026-10-18.

- `SUMPRODUCT` — §6.16.64

  **Evidence:** the second function the greedy pass adds — seven of LibreOffice's test
  documents — and the idiom every spreadsheet used for several conditions before `SUMIFS`:
  `SUMPRODUCT(([.A1:.A9]="x")*[.B1:.B9])`. Its parameter is ForceArray (§6.3.4), so it needed
  array evaluation inside one argument: operators applied place by place and a function
  given an array where it expects one value evaluated once per element (§3.3 2.2), each
  parameter's arity read off the catalog's own signature. Contained in `array.rs` and reduced
  to one value before anything else sees it, which is why `doc/not-doing.md`'s array row is
  narrowed rather than crossed. Where the oracle and §3.3 disagree — an operator over arrays
  of two lengths — the spec wins (`doc/ods-format.md` §4.1). Decided 2026-10-18.

- `LEGACY.NORMSINV` — §6.18.55
- `FIXED` — §6.20.10

  **Evidence:** a Monte Carlo retirement calculator exported from a spreadsheet service, whose
  simulation draws every percentile band through Excel's `NORMSINV` (imported as
  `LEGACY.NORMSINV`, as the oracle does) and spells its results with `FIXED`; without them a
  recalculation turned 413 cells into errors and every band of its charts into zero.
  `LEGACY.NORMSINV` is Φ⁻¹ by Halley's method over `erfc`; `FIXED` is `ROUND` followed by the
  number format a cell would use, in the document's locale. Measured against LibreOffice 26.8
  (`doc/ods-format.md` §4.1), the spec winning where the two disagree. Decided 2026-11-22.

- `OFFSET` — §6.14.11
- `INDIRECT` — §6.14.7

  **Evidence:** Enron's seventh by workbook (`OFFSET`, Jansen) and O'Beirne's fourteenth by
  formula; `INDIRECT` the most-blocking function in LibreOffice's own test documents
  (`doc/function-coverage.md`). Both return a reference, which `INDEX` already does: the engine
  evaluates the three itself (`Engine::index`, `offset`, `indirect`) and `array.rs` reads the
  cells of one inside an array. `INDIRECT` reads `.` and `!` before a sheet, quoted sheet names,
  named ranges and, with `A1` false, R1C1. What `graph.rs` sees of either is the cells its
  arguments name, not the ones it reaches at run time — so a cell read only through one is
  invisible to the precedent views and to lint. Measured against LibreOffice 26.8
  (`doc/ods-format.md` §4.1). Decided 2026-11-22.

- `ISEVEN` — §6.13.17
- `ISODD` — §6.13.23
- `ISREF` — §6.13.24
- `ISFORMULA` — §6.13.18
- `FORMULA` — §6.13.12
- `ERROR.TYPE` — §6.13.11
- `CHAR` — §6.20.3
- `CODE` — §6.20.5
- `UNICHAR` — §6.20.25
- `UNICODE` — §6.20.26
- `LOOKUP` — §6.14.8

  **Evidence:** the everyday helpers `doc/function-coverage.md` finds in every corpus — `LOOKUP`
  is Enron's fourteenth by workbook (Jansen), `CHAR(10)` is how a formula breaks a line,
  `ERROR.TYPE`, `ISREF`, `ISEVEN`, `ISODD`, `CHAR` and `FORMULA` each block several of
  LibreOffice's own test documents, and Excel's `FORMULATEXT` is imported as `FORMULA` (`doc/xlsx-format.md`
  §5.1). No new machinery: a reference operand for `ISREF`, the stored formula through the
  formula bar's own rendering for `ISFORMULA` and `FORMULA`, `MATCH`'s sorted search for
  `LOOKUP`. `CHAR` and `CODE` read Unicode code points, which §6.20.3 Note 1 allows and which
  keeps `CODE(CHAR(N))` = `N` for 1 to 255 on every system; the oracle answers from its own code
  page. Measured against LibreOffice 26.8 (`doc/ods-format.md` §4.1). Decided 2026-11-22.

- `TEXT` — §6.20.23

  **Evidence:** Enron's and EUSES's everyday function for putting a number or a date into a
  sentence, and seven of LibreOffice's test documents. Part 4 leaves the code's meaning to the
  implementation; this build reads it as Excel's files spell it — the parser the xlsx import
  already had, hoisted to `numfmt::code`, so the import and `TEXT` read a code one way — and spells
  the result in the document's locale, as `FIXED` does. A code with a part the model cannot spell
  (a fraction, an exponent, a display factor) is `#VALUE!`, where the oracle shows it. Measured
  against LibreOffice 26.8 (`doc/ods-format.md` §4.1). Decided 2026-11-22.

- `AVEDEV` — §6.18.2
- `COMBIN` — §6.16.16
- `CORREL` — §6.18.17
- `DEVSQ` — §6.18.20
- `FORECAST` — §6.18.28
- `GEOMEAN` — §6.18.34
- `HARMEAN` — §6.18.36
- `INTERCEPT` — §6.18.38
- `MODE` — §6.18.50
- `PEARSON` — §6.18.56
- `PERCENTILE` — §6.18.57
- `PERCENTRANK` — §6.18.58
- `PERMUT` — §6.18.59
- `QUARTILE` — §6.18.64
- `RSQ` — §6.18.66
- `SLOPE` — §6.18.69
- `SUMSQ` — §6.16.65

  **Evidence:** the statistics a workbook reaches for after `AVERAGE` and `STDEV` — percentiles,
  quartiles, the most frequent value, a percent rank, correlation and the regression line through
  two columns, sums of squares and the other means — and `COMBIN`/`PERMUT` beside `FACT`. Each is
  a formula over a number sequence or a pair of ForceArray arrays (`array.rs`), with no new
  machinery. Where the spec leaves a choice the oracle decides: `PERCENTRANK` rounds to its
  significance, an X outside the data and an empty sequence are `#VALUE!`; arrays of two shapes
  for the paired statistics are `#N/A`, as Excel answers, where the oracle says Err:502.
  Measured against LibreOffice 26.8. Decided 2026-11-22.

- `NORMDIST` — §6.18.52
- `NORMINV` — §6.18.53
- `LEGACY.NORMSDIST` — §6.18.54
- `STANDARDIZE` — §6.18.71

  **Evidence:** the normal distribution beside its inverse, `LEGACY.NORMSINV`, already here:
  the share of a class below a mark, the mark a share reaches, and the z-score between them.
  The import corpus calls them in marks books and quality charts, and Excel's `NORM.DIST` and
  `NORM.INV` below are these with new names. A standard deviation that is not positive is
  `#NUM!`, a zero one in `STANDARDIZE` `#DIV/0!`, as the oracle answers. Measured against
  LibreOffice 26.8. Decided 2026-11-27.

## Excel's functions, under LibreOffice's names

**Not Part 4 functions.** ODF has no namespace for a function the standard does not define,
and LibreOffice writes Excel's under a `COM.MICROSOFT.` prefix — measured both ways in
`doc/xlsx-format.md` §5.1: its own `.xlsx` import produces those names, and its own documents
carry them (109 of its test documents). An entry here is admitted for the same reason as
everything above it, one at a time on evidence, and its section is the **chapter** of Part 4
its ODF counterpart is in — where a help browser files it — never a definition: the
definition is Excel's, and the expectations are measured against the oracle's function of the
same name.

- `COM.MICROSOFT.CEILING` — §6.17
- `COM.MICROSOFT.FLOOR` — §6.17

  **Evidence:** every `.xlsx` with a `CEILING` or `FLOOR` in it. Excel's two-parameter
  functions round on the number line and §6.17's do not (`CEILING(-4.5,1)` is -4 in Excel and
  `#NUM!` in §6.17.1), so `grind-xlsx` imports them under these names, as the oracle does —
  and without them an imported workbook's `CEILING` would recalculate to `#NAME?`. Three of
  LibreOffice's test documents carry each. Decided 2026-10-18.

- `COM.MICROSOFT.MAXIFS` — §6.18
- `COM.MICROSOFT.MINIFS` — §6.18

  **Evidence:** `grind-xlsx` already imports Excel's `MAXIFS` and `MINIFS` under these names,
  as the oracle does, so an imported workbook that used either recalculated to `#NAME?`; and a
  script asking for the largest value under a condition had no way to say it. Neither is in
  ODF 1.4, and the oracle reads only these spellings: a bare `MAXIFS(…)` in an ODF file is
  `#NAME?` there too. No new machinery — `SUMIFS`'s matching with a largest or smallest in
  place of the sum. Measured against LibreOffice 26.8 (`doc/ods-format.md` §4.1): 0 when nothing
  matches, text in the range is skipped, and a matched error is the answer — which `SUMIFS` and
  its kin now give too. Decided 2026-11-09.

- `COM.MICROSOFT.GAMMA.DIST` — §6.18
- `COM.MICROSOFT.NORM.S.DIST` — §6.18

  **Evidence:** the same calculator as `LEGACY.NORMSINV` above. `grind-xlsx` imports Excel's
  `GAMMA.DIST` and `NORM.S.DIST` under these names, as the oracle does, and they are its
  ruin-probability columns. Both are §6.18's own functions in Excel's clothes — `GAMMADIST`,
  and `NORMDIST` with a mean of 0 and a deviation of 1 — with the last parameter required and,
  for `GAMMA.DIST`, a negative X refused rather than 0, as the oracle's functions of these names
  do (`doc/ods-format.md` §4.1). Decided 2026-11-22.

- `COM.MICROSOFT.IFS` — §6.15
- `COM.MICROSOFT.SWITCH` — §6.15
- `COM.MICROSOFT.CONCAT` — §6.20
- `COM.MICROSOFT.TEXTJOIN` — §6.20
- `COM.MICROSOFT.XLOOKUP` — §6.14
- `COM.MICROSOFT.XMATCH` — §6.14
- `COM.MICROSOFT.STDEV.S` — §6.18
- `COM.MICROSOFT.STDEV.P` — §6.18
- `COM.MICROSOFT.VAR.S` — §6.18
- `COM.MICROSOFT.VAR.P` — §6.18

  **Evidence:** `doc/function-coverage.md`'s Tier 2 — the functions Excel 2010–2019 added that a
  workbook of today is written with: nested `IF`s become `IFS` and `SWITCH`, `VLOOKUP` becomes
  `XLOOKUP`, `&` over a column becomes `TEXTJOIN`, and the dotted statistics are what Excel's
  function picker offers first. `grind-xlsx` already imports each under this name, as the oracle
  does, and each recalculated to `#NAME?`. No new machinery: `IF`'s conditions, `=`'s
  comparison, `CONCATENATE`'s text, `MATCH`'s search with Excel's match and search modes, and
  `STDEV`/`STDEVP`/`VAR`/`VARP` themselves. One value each: an `XLOOKUP` whose return array is
  wider gives the found row's first cell, where Excel would spill the row. Measured against
  LibreOffice 26.8 (`doc/ods-format.md` §4.1). Decided 2026-11-22.

- `COM.MICROSOFT.RANK.EQ` — §6.18
- `COM.MICROSOFT.RANK.AVG` — §6.18
- `COM.MICROSOFT.PERCENTILE.INC` — §6.18
- `COM.MICROSOFT.PERCENTILE.EXC` — §6.18
- `COM.MICROSOFT.QUARTILE.INC` — §6.18
- `COM.MICROSOFT.QUARTILE.EXC` — §6.18
- `COM.MICROSOFT.MODE.SNGL` — §6.18
- `COM.MICROSOFT.PERCENTRANK.INC` — §6.18
- `COM.MICROSOFT.PERCENTRANK.EXC` — §6.18
- `COM.MICROSOFT.FORECAST.LINEAR` — §6.18
- `COM.MICROSOFT.NORM.DIST` — §6.18
- `COM.MICROSOFT.NORM.INV` — §6.18
- `COM.MICROSOFT.NORM.S.INV` — §6.18
- `COM.MICROSOFT.CEILING.PRECISE` — §6.17
- `COM.MICROSOFT.ISO.CEILING` — §6.17
- `COM.MICROSOFT.FLOOR.PRECISE` — §6.17
- `COM.MICROSOFT.CEILING.MATH` — §6.17
- `COM.MICROSOFT.FLOOR.MATH` — §6.17

  **Evidence:** Excel 2010's names for functions this build already has, which its function
  picker offers first and so today's workbooks are written with; `grind-xlsx` imports each
  under this name, as the oracle does, and each recalculated to `#NAME?`. The import corpus
  and LibreOffice's own test documents carry the rounding five in three files each. Most are
  the function above under a new name; the new arithmetic is small: `RANK.AVG` gives ties the
  mean of their ranks, the `.EXC` forms interpolate at rank `X·(n+1)` and refuse a rank outside
  the data with `#NUM!` (the oracle's Err:504), and the `.PRECISE` and `.MATH` roundings step to
  a multiple of the significance's magnitude, `.MATH`'s Mode reversing the step for a negative
  number. Measured against LibreOffice 26.8 (`doc/ods-format.md` §4.1). Decided 2026-11-27.

---

## Attribution

The function list and section references on this page are extracted from
**ODF 1.4 Part 4 §2.3.2 E)**, Copyright © OASIS Open 2025, All Rights Reserved. The OASIS
terms permit derivative works that "assist in its implementation" provided the copyright
notice is carried along — see `LICENSES/LicenseRef-OASIS-IPR.txt`, and
`doc/OpenDocument-v1.4-os-part4-formula.html` for the document itself. The surrounding
commentary is CC-BY-4.0, as all of `doc/` is.
