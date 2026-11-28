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

### Fixes

- A Word document no longer gains an empty line at every section break that starts a new page,
  which could push what follows onto a page of its own.

## 0.3.0 — 2026-11-28

### Changes

- The spreadsheet, the word processor and every Linux package now come for 64-bit Arm too
  (`arm64` `.deb`, `aarch64` `.rpm`).
- An Ansible role installs the latest released desktop apps and the `grind` command on
  Debian, Ubuntu, Fedora, RHEL and openSUSE, and upgrades them when a new release comes out
  (`ansible-galaxy collection install 'git+https://github.com/fwilhe2/grind.git#/packaging/ansible,main'`).
- The browser version works on a phone: the bars scroll sideways, a finger scrolls the
  spreadsheet and two fingers zoom it, a second tap on a cell edits it, and the text page fits
  the screen and takes the phone's keyboard.
- `grind calc --open` reads a tape saved with `--save` back into the scratch calculator,
  even after LibreOffice opened and saved it, and adds the lines given after it; `:open` does
  the same at the interactive tape. A spreadsheet that is not a saved tape is refused with
  what makes it not one. Every window's scratch calculator opens one too (*Open Tape…*, or
  `:open` in the terminal version), and Undo brings back the lines it replaced.
- A Windows installer, `grind-X.Y.Z-x86_64.msi`: Grind in the Start menu and in *Open with*,
  `grind` and `grind-tui` on your `PATH`, for you alone and without an administrator. The next
  release's installer updates it in place, and *Installed apps* removes all of it.
- Homebrew installs grind on macOS and Linux and `brew upgrade` keeps it current:
  `brew install fwilhe2/grind/grind` for the `grind` command and `grind-tui`,
  `brew install --cask fwilhe2/grind/grind-app` for the Mac app.
- A GitHub Action puts the `grind` command on the path of a workflow, on Linux, macOS and
  Windows, x64 and Arm, the latest release or a pinned one: `uses: fwilhe2/grind@main`.
- More of Excel's functions recalculate instead of showing `#NAME?`: `RANK.EQ`, `RANK.AVG`,
  `PERCENTILE.INC`, `PERCENTILE.EXC`, `QUARTILE.INC`, `QUARTILE.EXC`, `MODE.SNGL`,
  `PERCENTRANK.INC`, `PERCENTRANK.EXC`, `FORECAST.LINEAR`, `NORM.DIST`, `NORM.INV`, `NORM.S.INV`,
  `CEILING.PRECISE`, `ISO.CEILING`, `FLOOR.PRECISE`, `CEILING.MATH` and `FLOOR.MATH`; and the
  normal distribution in ODF's own names, `NORMDIST`, `NORMINV`, `LEGACY.NORMSDIST` and
  `STANDARDIZE`.
- The scratch calculator takes currencies without their symbol: `1200 eur`, `EUR 1200` or `$5`
  show as `1,200.00 €` and `$5.00`, and the result of a calculation keeps the currency
  (`net * 19%` is euros, `net / net` a plain number). Adding euros to dollars is refused. A
  percentage shows as `19%`, and a tape saved as a spreadsheet formats every cell the same way.
- `grind-sheet-gtk --tape-from lines.txt` opens the scratch calculator with a file's lines typed
  in.
- Dates: `DAYS`, `DAYS360`, `DATEDIF` and `YEARFRAC`, and Excel's `WORKDAY.INTL` and
  `NETWORKDAYS.INTL` for a working week that is not Monday to Friday.
- `ROMAN`, `BASE`, `DECIMAL` and the binary, octal and hexadecimal conversions (`DEC2HEX`,
  `HEX2DEC`, `BIN2DEC` and the rest); `NUMBERVALUE`, `CLEAN`, `AVERAGEA`, `MAXA`, `MINA`,
  `SHEET`, `SHEETS`, `DB`, `XNPV`, `XIRR`, `RAND` and `RANDBETWEEN`.

### Fixes

- A span of tape lines, `SUM(#1:#3)`, is shown as typed rather than as `[.B1]:[.B3]`, and saved
  as one range even when its first line has a name.
- Zooming the spreadsheet in the browser (Ctrl+wheel, Ctrl+plus and minus) changes the size the
  cells are drawn at again, not only how many fit.
- `PERCENTRANK` rounds to significant digits, as LibreOffice does: a small rank such as 0.00333
  no longer comes out as 0.003.

## 0.2.0 — 2026-11-26

### Changes

- `grind calc`, a scratch calculator: type formulas line by line, refer to an
  earlier line as `#3` or name one as `vat = 0.19`, and save the lines as a spreadsheet with
  `--save`. Numbers follow `--locale`. In the terminal version it is `grind-tui --tape`, or
  `:tape`; in the spreadsheet window, the calculator button in the header bar or *Scratch
  Calculator* in the menu, with *Open as Spreadsheet* to carry on in a sheet; in the browser,
  the calculator button over a spreadsheet, or *Scratch calculator* in Ctrl+K; on Windows,
  *Data ▸ Scratch Calculator…*; on a Mac, *Edit ▸ Scratch Calculator…*.
- The scratch calculator in the browser, on Windows and on a Mac is a notebook: type a line,
  press Enter, and the next one is ready; Enter on an earlier line opens a new one under it, the
  arrow keys move between lines, and Backspace in an empty line removes it.
- The scratch calculator completes as you type: Tab takes the names your lines define, offered
  first with their values, and the functions. A name the tape knows is coloured apart from a
  misspelt one. In every window and in the terminal.
- `grind sheet extract`, and *Extract a Number into a Named Cell…* in the spreadsheet window,
  move a number out of a formula into a named input cell, and have the formula use the name
  instead.
- Excel's newer functions recalculate instead of showing `#NAME?`: `IFS`, `SWITCH`, `CONCAT`,
  `TEXTJOIN`, `XLOOKUP`, `XMATCH`, `STDEV.S`, `STDEV.P`, `VAR.S` and `VAR.P`.
- Statistics: `PERCENTILE`, `QUARTILE`, `MODE`, `PERCENTRANK`, `CORREL`, `PEARSON`, `RSQ`, `SLOPE`,
  `INTERCEPT`, `FORECAST`, `SUMSQ`, `DEVSQ`, `AVEDEV`, `GEOMEAN`, `HARMEAN`, `COMBIN` and `PERMUT`.
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

- Formulas in a spreadsheet grind saved open as formulas in current LibreOffice, rather than as
  `Err:510` in every formula cell.
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
