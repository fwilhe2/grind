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

### 1.2 The Strict URIs — `UNVERIFIED`

ISO/IEC 29500 Strict is believed to replace the `schemas.openxmlformats.org` families with
`purl.oclc.org/ooxml` ones — `http://purl.oclc.org/ooxml/spreadsheetml/main` for
SpreadsheetML and `http://purl.oclc.org/ooxml/officeDocument/relationships` for the `r:`
attributes and the relationship `Type` values — while leaving the OPC package namespaces
(content types, package relationships) and the markup-compatibility namespace alone, since
those are Parts 2 and 3 rather than Part 1.

**Written from memory and not to be implemented from this paragraph.** Confirm each URI
against ECMA-376 and against one Strict file before `xlsx/src/names.rs` gets a Strict column.
`doc/xlsx-import.md` says why recognising Strict is worth ten lines anyway.

### 1.3 Markup compatibility — `SPEC`

ECMA-376 Part 3. `mc:AlternateContent` holds one or more `mc:Choice Requires="…"` and at most
one `mc:Fallback`; a consumer takes the first `mc:Choice` whose required namespaces it all
understands, and otherwise the `mc:Fallback`. We understand no extension namespace, so **every
`mc:Choice` is skipped and the `mc:Fallback` is read in its place**.

Not yet confirmed against a file that exercises it, which is what fixture 4 in
`doc/xlsx-import.md`'s verification section is for: a `Choice` and a `Fallback` holding
*different* cell values, so taking the wrong one is a visible failure rather than a subtle one.

---

## 2. Values and dates

### 2.1 The 1900 leap-year bug — `SPEC`, to be measured

The correction table is in `doc/xlsx-import.md` Part II §2, and serial 60 — the day Excel
believes existed and no calendar does — is deliberately left open there. **X1 measures it
against the oracle**: convert a workbook whose A1 is the literal 60 formatted as a date, read
what LibreOffice made of it, and record the answer here with the command. Three tests at 59,
60 and 61, because that boundary is where every implementation of this gets it wrong once.

### 2.2 Cell types — `SPEC`

ECMA-376 §18.18.11 (`ST_CellType`); the mapping onto `CellValue` is tabulated in
`doc/xlsx-import.md` Part II §2 and is spec-derived. `t` defaults to `n`.

---

## 3. Number formats

### 3.1 Built-in ids 0–49 — `SPEC` (§18.8.30), to be measured

The spec lists literal codes; Excel renders several of them in the user's locale, so the
mapping this filter wants is **by meaning onto `numfmt::preset`**, not by code. The table is
X3's work and belongs here with the oracle output that produced it — including which ids have
no `preset` equivalent and are therefore dropped and counted.

### 3.2 Format code grammar — `SPEC`

§18.8.31. The pieces the parser must handle are listed in `doc/xlsx-import.md` Part II §4.
What is *not* spec is how Excel resolves a four-section code against ODF's two-branch
`style:map`; that is a measurement and goes here.

---

## 4. Styles

### 4.1 Indexed colours — `SPEC` (§18.8.27), to be transcribed

The legacy 56-entry palette. A table, and transcribing a table from a specification is not
copying an implementation.

### 4.2 Theme colours and tint — `SPEC`, to be measured

`<color theme="4" tint="-0.25"/>` resolves through `xl/theme/theme1.xml`'s `<a:clrScheme>`
plus ECMA's tint formula. Theme slot order is the part most likely to be got wrong; measure
against the oracle before implementing, and record which slot index means which scheme colour.

### 4.3 Column width in characters — `SPEC` (§18.3.1.13), to be measured

`<col width="8.43"/>` is a count of the Normal font's maximum digit width plus padding, not a
length. The conversion constant is measured against the oracle rather than taken from the
prose, and the measurement goes here.

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

---

## 6. Where facts come from

- ECMA-376 / ISO-IEC 29500, cited by part and section. Not vendored here: unlike the OASIS
  ODF specs in `doc/`, it is not redistributed with this repository.
- The pinned oracle (`ci/libreoffice-image`, via `scripts/soffice-docker/soffice`), for
  anything the spec leaves room in. Every measurement records the command and the date.
- LibreOffice's `sc/source/filter/oox/`, read and cited by `file:line`, never copied — and
  only after the specification has been consulted first.
