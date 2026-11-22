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

- Four more spreadsheet functions: `FIXED`, `LEGACY.NORMSINV`, and Excel's `NORM.S.DIST` and
  `GAMMA.DIST`, so workbooks that simulate with the normal and gamma distributions recalculate
  instead of turning into errors.
- Importing an Excel workbook keeps a one-cell array formula as a formula when it computes the
  same as an ordinary one, instead of freezing it to its saved value.
- Area charts are shown: stacked bands, with lines drawn over them, as LibreOffice draws them.

### Fixes

## 0.1.1 — 2026-11-21

### Fixes

- The Linux packages now install on Debian 13 and Ubuntu 24.04, and on other distributions
  of the same age. 0.1.0's needed a newer system C library than they have.

## 0.1.0 — 2026-11-21

This is the first release with the initial feature set.
