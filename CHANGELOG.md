# Release notes

<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
SPDX-License-Identifier: CC-BY-4.0
-->

What changed for the people who use grind, release by release. `## Unreleased` is the draft of
the next release's notes: every change someone using grind would notice adds a line there as it
lands, and the release workflow publishes the section as it stands.

## Unreleased

### Breaking changes

### Changes

- Excel's newer functions recalculate instead of showing `#NAME?`: `IFS`, `SWITCH`, `CONCAT`,
  `TEXTJOIN`, `XLOOKUP`, `XMATCH`, `STDEV.S`, `STDEV.P`, `VAR.S` and `VAR.P`.
- `TEXT`: a number or a date spelled in a format code such as `#,##0.00` or `dd.mm.yyyy`.
- `LOOKUP`, `ISEVEN`, `ISODD`, `ISREF`, `ISFORMULA`, `FORMULA` (Excel's `FORMULATEXT`),
  `ERROR.TYPE`, `CHAR`, `CODE`, `UNICHAR` and `UNICODE`.
- `OFFSET` and `INDIRECT`: a reference moved and resized, and a reference spelled as text — with
  `.` or Excel's `!` before a sheet, a defined name, or in R1C1 style.
- Four more spreadsheet functions: `FIXED`, `LEGACY.NORMSINV`, and Excel's `NORM.S.DIST` and
  `GAMMA.DIST`, so workbooks that simulate with the normal and gamma distributions recalculate
  instead of turning into errors.
- Importing an Excel workbook keeps a one-cell array formula as a formula when it computes the
  same as an ordinary one, instead of freezing it to its saved value.
- Area charts: offered beside bar, line and pie wherever a chart is inserted or changed, and
  drawn as LibreOffice draws them — stacked bands, with lines over them.
- Charts LibreOffice anchors to a cell are shown; they used to be invisible in grind.
- Stacked bar and line charts are drawn stacked; they used to be drawn side by side.
- A formula whose saved result was rounded to ten digits no longer counts as out of date: editing
  such a workbook no longer warns about thousands of formulas, and recalculating leaves them be.
- Importing an Excel workbook brings its charts along — bar, line, pie and area, stacked or not,
  with a line over bars or areas — placed where they were, drawn from the workbook's cells, and
  with their series' names even where the workbook gives a name as text rather than a cell, and
  their colours, theme colours included. A 3D chart comes in as its flat kind.

### Fixes

- A date format that shows the day of the week names the right day; every day but Sunday was
  shown as the day before.
- `MATCH` searches an array as well as a range, so `MATCH(TRUE(); INDEX(B1:B9>15; 0); 0)` finds
  the first row over 15 instead of failing; `INDEX` picks a row or column of an array inside one.
- `INDEX` over a range gives back a cell, row or column you can use as a range, so
  `MAX(A2:INDEX(A2:A99; B1))` and `SUM(INDEX(A1:C9; 0; 2))` work instead of showing `#VALUE!`.

## 0.1.1 — 2026-11-21

### Fixes

- The Linux packages now install on Debian 13 and Ubuntu 24.04, and on other distributions
  of the same age. 0.1.0's needed a newer system C library than they have.

## 0.1.0 — 2026-11-21

This is the first release with the initial feature set.
