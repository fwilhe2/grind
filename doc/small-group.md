<!--
SPDX-FileCopyrightText: 2025 OASIS Open
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: LicenseRef-OASIS-IPR AND AGPL-3.0-or-later
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

---

## Attribution

The function list and section references on this page are extracted from
**ODF 1.4 Part 4 §2.3.2 E)**, Copyright © OASIS Open 2025, All Rights Reserved. The OASIS
terms permit derivative works that "assist in its implementation" provided the copyright
notice is carried along — see `LICENSES/LicenseRef-OASIS-IPR.txt`, and
`doc/OpenDocument-v1.4-os-part4-formula.html` for the document itself. The surrounding
commentary is AGPL-3.0-or-later.
