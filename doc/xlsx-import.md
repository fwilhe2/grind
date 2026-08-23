<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# The xlsx import filter — phase 10, planned

This is the work plan for reading `.xlsx`, and the document that holds it to the rules once
building starts. It is normative for this phase the way `doc/gtk-shell.md` is for phase 9.

**The decision it records.** `doc/not-doing.md` §2 has carried one row since phase 7:
*"Reading `.xlsx` — never scheduled, always allowed"*. It is now scheduled. The reason is
not that Excel matters as a format; it is that **Excel is where other people's documents come
from**, and a program that cannot read them is a program you cannot start using. Converting
them today means driving LibreOffice headless, which works and is unpleasant: a 400 MB
install, a startup profile, a user profile lock, and a process that occasionally decides to
open a dialog on a machine with no display.

That is the pitch, and it falls out of the architecture rather than being bolted on: the core
has no UI, the shells are separable crates, and a converter is the core plus one filter. The
whole conversion path is a library function over bytes.

**Writing `.xlsx` stays in §1 "Never", unchanged.** Reading is a one-way translation at the
edge that produces an ODF document. Writing means *owning* Excel's semantics — its error set,
its text→number rule, its 1900 leap-year bug — permanently and in both directions, which is
the trade `doc/plan.md` was written to avoid. The asymmetry is the whole point: one way in,
never out.

---

## Decisions taken up front

- **Import produces a `Document`, never a live translation layer.** The filter runs once,
  end to end, and everything downstream is ODF. No Excel vocabulary reaches `grind-sheet`.
- **Nothing is evaluated on import.** Excel's cached values are authoritative and are carried
  verbatim. A formula whose function this build does not implement still arrives with the
  number Excel computed, and `sheet recalc`'s existing `spoiled` counter is what warns before
  that value is ever replaced. Recalculating an imported document is the user's decision,
  exactly as it is for any other document.
- **Our own reader, not `calamine`.** See the trade below; it is close, and the trigger that
  would flip it is named.
- **Its own crate, behind a cargo feature.** `xlsx/` (crate `sheet-xlsx`), an optional
  dependency of each shell that wants it. A build without the feature contains none of it.
- **The fidelity report is part of the output, not an afterthought.** What a conversion
  dropped is as important as what it carried, and a converter that lies about it is worse
  than one that refuses. `--strict` turns any loss into a non-zero exit, which is what a
  pipeline wants.
- **The CLI's read commands stay ODF-only.** `sheet import in.xlsx out.ods` is the filter;
  everything else operates on the result. A CLI can run two commands where a GUI cannot, so
  the GUI's Open dialog imports transparently (X6) and `sheet view book.xlsx` deliberately
  does not exist. One read path per format, chosen explicitly.
- **ECMA-376 is the normative source.** Not LibreOffice's filter, not a blog post about it.

### Why not `calamine`

`doc/plan.md` itself suggested `calamine` as the cheap escape hatch, so declining it needs a
reason rather than a preference.

| | `calamine` | our own |
|---|---|---|
| Values, shared strings, dates | done, well | ~400 lines of quick-xml over parts we already unzip |
| Formulas | returns Excel A1 **text**; the translator is ours either way | same translator |
| Number formats, fonts, fills, borders, widths | not modelled — we would open `xl/styles.xml` ourselves anyway | one pass, one model |
| Fidelity report | not expressible — it does not know what it skipped | falls out of the reader |
| `.xls`, `.xlsb` (binary) | **yes**, and we would never write those ourselves | no |
| Dependencies | +1 crate, +1 XML stack, +1 data model to translate from | **zero new dependencies** — `zip` and `quick-xml` are already core's |

The half `calamine` does well is the half that is cheap for us, and the half that is
expensive for us it does not do. It also inverts the fidelity story: a reader that silently
skips what it does not model cannot report what it dropped.

**The trigger that flips this:** legacy `.xls` (BIFF8) or `.xlsb`. Those are binary formats
from 1997 and 2007 that we would be foolish to parse by hand, and if a real week of use
demands them, a dependency earns its place *for those formats only*, behind this same
feature flag and this same `Import` shape. One implementation is not a trait (`ponytail`);
the second one is.

---

## The rules, applied to an import filter

The seven rules (`doc/plan.md`, `CONTRIBUTING.md`) and the seven requirements (R1–R7) bind
this phase as they bind every other. Where they say something specific:

- **R1 — independence, ODF-native semantics.** The filter is a *producer of `Document`s*, and
  a `Document` is ODF's model. Nothing Excel-shaped may appear in `grind-sheet`: no format
  code strings, no 1900 serials, no `!` sheet separator, no `,` argument separator. If the
  importer needs a core capability, the core gains it in ODF's own terms or not at all.
- **R2 — everything written validates.** An imported document is written by *our* writer, so
  `jing -i` applies to it unchanged. This is a test, not a hope: X1's exit criterion runs the
  validator over the corpus's output.
- **R6 — the diffable writer.** An imported document has no source bytes to splice, so it
  regenerates. That is already the documented behaviour for a document this program authored
  (`odf/source.rs`), and an import *is* authoring one.
- **Rule 4 — whatever a GUI can do, the CLI can do.** The filter lands CLI-first: `sheet
  import` exists before any Open dialog learns the extension.
- **Rule 5 — no filesystem assumptions.** `import_bytes` is the real function and
  `import_file` is a thin twin, because the browser has no filesystem and a cloud converter
  usually has a byte stream rather than a path.
- **Clean room.** LibreOffice's `sc/source/filter/oox/` may be *read and cited by
  `file:line`*, never copied — and every fact learned from reading it goes into
  `doc/xlsx-format.md` before it reaches code, exactly as `doc/ods-format.md` works for ODF.
  ECMA-376 is the specification; LibreOffice remains the conformance oracle.
- **Nothing user-facing names LibreOffice.** Naming *Excel* is unavoidable and fine — it is
  the format's name. The file filter reads "Excel Workbook".

---

## Part I — the seam

### Crate layout

```
core/     document model, ODF I/O, formula engine       (unchanged, knows nothing of Excel)
cli/      the `sheet` binary                            optional dep: sheet-xlsx
ui_gtk/   the GNOME shell                                optional dep: sheet-xlsx
xlsx/     the import filter — crate `sheet-xlsx`         depends on grind-sheet, zip, quick-xml
```

```
xlsx/src/
  lib.rs        import_bytes / import_file / sniff, the Report, the public surface
  package.rs    OPC: the zip, [Content_Types].xml, relationships, part lookup
  workbook.rs   xl/workbook.xml — sheets, order, visibility, date system, defined names
  strings.rs    xl/sharedStrings.xml — the string table, rich-text runs flattened
  sheet.rs      xl/worksheets/*.xml — rows, cells, values, shared/array formula groups
  formula.rs    Excel A1 expression → grind_sheet::formula::Expr  (lexer + Pratt parser)
  numfmt.rs     Excel format codes and built-in ids → grind_sheet::numfmt::Format
  styles.rs     xl/styles.xml + theme1.xml → grind_sheet::style::CellStyle
  dates.rs      the 1900 and 1904 systems, and the leap-year rule
  report.rs     what was carried, and what was not
```

Every file maps to one part of the format. `formula.rs` and `numfmt.rs` are the two that
would otherwise be tempting to put in the core — and both are *Excel's* spelling of something
the core already models in ODF's spelling, which is exactly why they live here.

### The data flow, and what it may touch

```
bytes ──► package ──► workbook ──► styles ──► sheets ──► Document + Report
                                                │
                                                └─ formula.rs → grind_sheet::formula::Expr
                                                                → canonical text via Display
```

The importer builds a `Document` through the model's **existing public API** — `Sheet::new`,
`set`, `set_formula`, `set_kind`, `set_format`, `set_style`, `Document { sheets, names,
null_date, .. }`. No core change is needed to construct one, and none may be added that hands
out mutable internals: if the importer cannot express something, either the model gains it in
ODF's terms (with a test, and reachable from the CLI) or the report records that it was
dropped.

**Styles are read before sheets**, for the same reason `odf/read.rs` parses `styles.xml`
before `content.xml`: a cell's number format decides whether its number is a date, and the
date correction cannot be applied without knowing that.

### The public surface

```rust
/// Whether these bytes are an OOXML spreadsheet — the zip magic plus `xl/workbook.xml`.
pub fn sniff(bytes: &[u8]) -> bool;

/// Read an Excel workbook. Never evaluates, never fails on a construct it cannot carry:
/// what it cannot carry is counted in the report.
pub fn import_bytes(bytes: &[u8]) -> Result<(Document, Report), Error>;
pub fn import_file(path: &Path) -> Result<(Document, Report), Error>;
```

`Error` is for a *file* that cannot be read at all — not a zip, no workbook part, encrypted.
Everything else is a `Report` entry, because a conversion that refuses a whole document over
one unsupported chart is a conversion nobody can use. This is the same split
`odf/read.rs` already makes between `Error::Xml` and silent tolerance.

```rust
pub struct Report {
    pub sheets: usize,
    pub cells: usize,
    pub formulas: usize,
    /// Constructs the model has no home for, by kind and count.
    pub dropped: BTreeMap<Dropped, usize>,
    /// Functions a carried formula names that this build does not implement. The cached
    /// value is intact; recalculating would replace it with #NAME?.
    pub unknown_functions: BTreeSet<String>,
    /// Cells whose formula could not be translated at all — the value was kept.
    pub untranslated: Vec<(usize, Pos)>,
}

pub enum Dropped {
    Chart, PivotTable, ConditionalFormat, DataValidation, Comment, Drawing, Macro,
    ArrayFormula, StructuredReference, ExternalLink, SheetLocalName, MergedCells,
    RichText, HiddenSheet, ColumnWidth, RowHeight, ThemeColor, FontFamily, Protection,
}
```

`Dropped` is an enum rather than strings so the list is finite, greppable, and testable —
and so a new kind of loss is a compile-time decision rather than a new string somewhere.

### Compiling with and without each feature

Two mechanisms, and they are not the same one:

- **The GUI is a separate crate.** It is already optional by construction: `cargo build -p
  grind-cli` never compiles a line of GTK, and neither does `cargo test -p grind-sheet`. The
  one gap is that bare `cargo build` / `cargo test` at the root walk every workspace member,
  which is why CI names crates explicitly. **Fix: `default-members = ["core", "cli"]` in the
  root manifest**, so the default commands skip the shells and `--workspace` still builds
  everything on demand. One line, and it makes the common case need no flags.
- **The import filter is a cargo feature**, because it belongs *inside* the CLI binary rather
  than beside it.

```toml
# cli/Cargo.toml
[features]
default = ["xlsx"]
xlsx = ["dep:sheet-xlsx"]

[dependencies]
sheet-xlsx = { path = "../xlsx", optional = true }
```

| Command | core | cli | xlsx | gtk |
|---|---|---|---|---|
| `cargo build -p grind-cli --no-default-features` | ✓ | ✓ | — | — |
| `cargo build -p grind-cli` | ✓ | ✓ | ✓ | — |
| `cargo build -p grind-sheet-gtk` | ✓ | — | — | ✓ |
| `cargo build` (with `default-members`) | ✓ | ✓ | ✓ | — |
| `cargo build --workspace` | ✓ | ✓ | ✓ | ✓ |

The subcommand is compiled out with the filter — `#[cfg(feature = "xlsx")]` on the `Import`
variant and its match arm — so a build without it does not advertise a command that would
only apologise. `cli/tests/parity.rs` keeps working because it tracks `App` methods, and the
importer is not one; the *parity document* gains a "Beyond `App`" row saying so.

**CI** gains one job that builds the matrix above and runs `cargo test -p grind-cli
--no-default-features`, because "it still compiles without the feature" is exactly the kind
of claim that rots silently.

---

## Part II — the translation

Six areas. Each names what is carried, what is dropped, and where the rule comes from.
Everything measured rather than specified — Excel's actual behaviour where ECMA-376 leaves
room — goes into **`doc/xlsx-format.md`**, cited by section, before it reaches code.

### 1. The package (X0)

OPC: a zip whose `[Content_Types].xml` types the parts and whose `_rels/.rels` points at the
workbook. Parts are found by **relationship**, not by path convention: `xl/workbook.xml` is
where every producer puts it and nowhere in the spec promises that.

Hardening, because a converter is a program that eats files from strangers:

- a cap on total decompressed bytes and on the number of parts (zip bomb),
- a cap on any single part (a 4 GB `sharedStrings.xml` is not a document),
- no DTD, no entity expansion — quick-xml does not resolve entities, and the reader must not
  add anything that does (billion laughs),
- external workbook links are recorded as dropped and **never fetched**,
- macros are never executed; `.xlsm` imports its data and reports `Dropped::Macro`.

### 2. Values and dates (X1)

`<c r="B2" t="…" s="12"><v>…</v></c>`, with `t` defaulting to `n`.

| `t` | Excel | `CellValue` |
|---|---|---|
| `n` | number | `Number`, date-corrected iff its format is a date or time |
| `s` | shared string index | `Text` (runs flattened; `Dropped::RichText` if there was more than one) |
| `str` | formula string result | `Text` |
| `inlineStr` | `<is><t>` | `Text` |
| `b` | `0`/`1` | `Bool` |
| `e` | `#DIV/0!` … | `Text`, the error's name — which is how `formula::eval::to_cell` stores one already |
| `d` | ISO 8601 date (ECMA-376 2nd ed.) | `Number` + `NumberKind::Date` |

**The date systems, and the bug.** `workbookPr/@date1904` selects the epoch.

- *1904*: `null_date = 1904-01-01`, serials pass through unchanged. ODF carries the epoch
  per document (`table:null-date`), so this is a one-line translation and nothing else.
- *1900*: Excel's serial 1 is 1900-01-01 **and** Excel believes 1900 was a leap year, giving
  a phantom serial 60 = "1900-02-29". ODF's default epoch is 1899-12-30, so:

  | Excel serial | Meaning | ODF serial |
  |---|---|---|
  | 1 … 59 | 1900-01-01 … 1900-02-28 | serial **+ 1** |
  | 60 | a day that does not exist | measured against the oracle in X1, and recorded |
  | ≥ 61 | 1900-03-01 onwards | **unchanged** — the two agree exactly |

  The correction applies **only to cells whose format is a date or a time**, because Excel
  stores a date as a plain number and only the format says otherwise. Three tests, at 59, 60
  and 61, and the boundary is where every implementation of this gets it wrong once.

Sheets are bounded like the ODF reader bounds them: a `dimension` of `A1:XFD1048576` is a
claim, not a promise, and materialising it is refused by the same
`MAX_MATERIALISED_CELLS`-shaped rule rather than by hope.

### 3. Formulas (X2)

`<f>` holds an Excel A1 expression. It is **parsed**, not rewritten with string surgery:
`xlsx/src/formula.rs` lexes Excel's syntax and builds `grind_sheet::formula::Expr` — the same
AST the ODF parser builds — and the existing `Display` prints the canonical form. So a
translated formula is one our own parser could have produced, or it is not translated at all.

| Excel | OpenFormula | note |
|---|---|---|
| `A1`, `$A$1` | `[.A1]`, `[.$A$1]` | brackets and the leading dot |
| `Sheet1!A1` | `[Sheet1.A1]` | `!` → `.` |
| `'My Sheet'!A1` | `['My Sheet'.A1]` | both double an inner `'` |
| `Sheet1:Sheet3!A1` | `[Sheet1.A1:Sheet3.A1]` | 3-D reference → §4.8's cuboid |
| `A:A`, `1:1` | `[.A:.A]`, `[.1:.1]` | whole column/row |
| `,` between arguments | `;` | §5.6 |
| `TRUE`, `FALSE` | `TRUE()`, `FALSE()` | §6.15; `display.rs` already does this |
| `#REF!`, `#DIV/0!`, `#N/A` … | same names | §5.12's set is Excel's set |
| `_xlfn.XLOOKUP` | `XLOOKUP` | prefix stripped; then reported as unknown |
| `@A1`, `_xlfn.SINGLE(A1)` | `[.A1]` | the implicit-intersection marker, dropped |
| `-2^2` | `-2^2` | both bind prefix `-` above `^`; §5.5's surprise is Excel's too |
| `{1,2;3,4}` | — | inline array (§5.13, excluded by §2.3.2) → `Dropped::ArrayFormula` |
| `Table1[Column]` | — | structured reference → `Dropped::StructuredReference` |
| `[1]Sheet1!A1` | — | external workbook → `Dropped::ExternalLink` |
| ` ` (space) intersection, `,` union | `!`, `~` | out of the Small Group → dropped |

**Shared formulas.** `<f t="shared" ref="B2:B10" si="0">A2*2</f>` defines a group; the other
cells carry `<f t="shared" si="0"/>` and mean *the same formula with its relative references
shifted*. Shifting relative axes by (Δrow, Δcol) is a transform over `Expr` and is **ODF
semantics, not Excel's** — §5.8 is where relative references are defined — so it lands in the
core as `formula::shift`, with a test, and the GUI's fill and copy-with-formulas want exactly
the same function next. The *grouping* stays in the importer, where Excel's spelling belongs.

**Array formulas** (`t="array"`) are out of scope by §2.3.2: the cell keeps its cached value,
loses its formula, and is counted.

**What is deliberately not translated: semantics.** `CEILING`, `FLOOR`, `MOD` with negative
operands and `ROUND`'s tie rule differ between Excel and OpenFormula *under the same name*.
The importer does not rename or rewrite them — it carries the name and Excel's cached value.
Recalculating in this build then applies ODF's rule, which is the correct behaviour for an
ODF document and a change the user must choose: `sheet recalc` already reports how many
values it replaced, and `App::stale` already reports how many disagree. No new machinery, and
`doc/xlsx-format.md` gets the list of names whose semantics differ so the warning can name
them later.

### 4. Number formats (X3)

Excel spells a format as a **code string** — `#,##0.00;[Red]-#,##0.00;"—";@` — and
`CLAUDE.md` says, correctly, that no such string may exist in the core. So the parser for it
lives here, and its output is a `numfmt::Format`: an ordered sequence of `Part`s, which is
ODF's model and the only one the core has.

- **Sections.** Up to four (`positive;negative;zero;text`) → the base format plus
  `style:map` branches, which is exactly how §5.1 spells a two-branch format and what
  `numfmt`'s `maps` already carry. More than two branches is followed one level, as the
  renderer already does, and the rest is dropped and counted.
- **Built-in ids 0–49** are mapped **by meaning, not by their literal code**: ECMA-376
  §18.8.30 lists id 14 as `mm-dd-yy`, and Excel renders it in the user's locale. Mapping them
  onto `numfmt::preset`'s vocabulary keeps a converted document looking like what the author
  saw rather than like a US date in Germany. The mapping table is measured against the oracle
  and recorded.
- **Custom ids ≥ 164** carry a code, parsed here: digits (`0`, `#`, `?`), the decimal point,
  grouping (`,`), literals (quoted, escaped with `\`, and the `_`/`*` width tricks — the
  first consumes its argument, the second is dropped), `%`, `@`, currency (`[$€-407]`),
  date/time pieces (`yyyy`, `mmm`, `hh`, `[h]` elapsed, `AM/PM`), scientific (`0.00E+00`) and
  fractions (`# ?/?`).
- **Not carried, counted:** conditions beyond the section rule (`[>=100]`), colours other
  than through the section a `style:map` already models, elapsed-time formats (`[h]`) if they
  turn out to have no ODF spelling we already emit, and fractions — `numfmt` has no `Part`
  for either, and inventing one is a phase 5 decision rather than an import decision.

### 5. Styles and geometry (X4)

`xl/styles.xml`: `cellXfs[s]` indexes a font, a fill, a border, an alignment and a number
format. Each maps onto `style::CellStyle`, whose values are ODF's own strings.

| Excel | `CellStyle` |
|---|---|
| `<b/>`, `<i/>`, `<sz val="11"/>` | `font_weight: "bold"`, `font_style: "italic"`, `font_size: "11pt"` |
| `<color rgb="FF0000FF"/>` | `color: "#0000ff"` — alpha dropped |
| `<color indexed="12"/>` | the legacy palette (ECMA-376 §18.8.27), a table |
| `<color theme="4" tint="-0.25"/>` | `xl/theme/theme1.xml`'s `<a:clrScheme>` plus ECMA's tint formula |
| `<patternFill patternType="solid"><fgColor …/>` | `background` |
| any other pattern or a gradient | dropped, counted |
| `<border><left style="thin">…` | `"0.5pt solid #000000"` — the style→width table is measured |
| `horizontal`, `vertical`, `wrapText` | `align`, `vertical_align`, `wrap` |
| `<name val="Calibri"/>` | **dropped**, counted — `style.rs` deliberately does not carry a font family (§5.4) |

**Column widths and row heights** are the one place this plan waits on another: the model
gains them in phase 9's **M8**, and until that lands they are counted as
`Dropped::ColumnWidth` / `Dropped::RowHeight`. When it lands, `<col width="8.43"/>` needs
ECMA-376 §18.3.1.13's character-width conversion (through the Normal font's maximum digit
width) and `<row ht="15"/>` is points. Both convert into the verbatim length strings the
model stores.

### 6. The document level (X5)

- **Defined names.** `<definedName name="X">Sheet1!$A$1:$B$2</definedName>` → `Document.names`,
  the expression translated by `formula.rs` like any other. A name with a `localSheetId` is
  sheet-local, which our model has no home for → dropped and counted. `_xlnm.Print_Area` and
  friends are print settings, and print is not a feature here.
- **Sheets.** Order and names carry; `state="hidden"` imports the data and loses the
  visibility, counted — a hidden sheet's data is the last thing to throw away silently.
  A name Excel allows and ODF does not is renamed deterministically and reported.
- **Merged cells.** `<mergeCell ref="B2:D4"/>` → counted, because the model carries no spans
  (`doc/not-doing.md` §3). The values are kept where they are; nothing is moved.
- **Everything else in the part list** — charts, pivot tables, conditional formatting, data
  validation, comments, drawings, protection — is counted by kind and dropped. Recognising
  them costs a match on the content type and buys an honest report.

---

## Part III — milestones

Every milestone lands green: `cargo test`, clippy clean, `reuse lint`, the loops, and the
feature matrix.

| # | Milestone | Contents | Exit criterion |
|---|---|---|---|
| X0 | **The seam** | `xlsx/` crate, feature flags, `default-members`, CI matrix job, `package.rs` + `workbook.xml` sheet list, `sheet import` writing an empty document with the right sheets | the matrix builds; `cargo test -p grind-cli --no-default-features` passes; the output validates with `jing -i` |
| X1 | **Values** | shared strings, cell types, the two date systems and the leap-year rule, bounded materialisation, `Report` v1, `doc/xlsx-format.md` opened | **loop D** green on the value-only corpus: every cell equals what the oracle's conversion produced, at 15 significant digits |
| X2 | **Formulas** | the Excel expression translator, shared-formula groups, `formula::shift` in the core, `_xlfn.`, 3-D refs, the exclusion classes | every formula in the corpus either round-trips through our canonical serialiser or falls in a named class; the scoreboard prints like loop B's |
| X3 | **Number formats** | built-ins by meaning, the code parser, sections → `style:map` | loop D compares **displayed text** per cell, which is loop C's rule for the same reason |
| X4 | **Styles and geometry** | fonts, fills, borders, alignment, theme and indexed colours; widths and heights **iff M8 has landed** | loop D compares styles the way loop C does — borders numerically, everything else exactly |
| X5 | **The document level** | defined names, sheet order and visibility, merges, the report as JSON, `--strict` | `sheet import --format json` counts every dropped construct; `--strict` exits non-zero when anything was dropped |
| X6 | **The shells** | the GTK Open dialog learns `.xlsx` (import → a new unsaved document, retitled `.ods`), file filters, the wasm shell's note | open an `.xlsx` in the GUI, edit it, save it as `.ods` |

**Order.** Values before formulas before formats is not arbitrary: a formula's *cached value*
is what makes X1's oracle comparison meaningful, and a date is only a date once its format is
known, so X3 closes a gap X1 opened rather than adding a new one.

---

## Verification

The project checks correctness against LibreOffice rather than against its own opinion, and
this phase adds the loop that does it for import.

| Loop | Asserts | Corpus |
|---|---|---|
| **A′** — read tolerance | every `.xlsx` in the corpus imports without an `Err` and without a panic | **352 files** in `sc/qa/unit/data/xlsx/`, plus `xlsm/` |
| **D** — import fidelity | our conversion and the oracle's conversion of the same file agree, semantically | the same 352, minus a named exclusion list |
| **R2** | every imported document validates against the ODF schema (`jing -i`) | the same |

Loop D, concretely:

```
ours   = sheet_xlsx::import_bytes(bytes)              → Document
theirs = soffice --headless --convert-to ods <file>   → read with our own reader → Document
compare(ours, theirs)
```

The comparison is `sheet/tests/roundtrip.rs`'s existing semantic comparator, reused: values at
15 significant digits (because that is all LibreOffice writes — `doc/ods-format.md` §3.4),
formulas as canonical text, formats as **the text the cell displays**. The oracle's output is
cached by content hash so a full run is one conversion per file, ever.

**Loop A′ runs in CI** — it needs no oracle and no display, only the corpus. Loop D needs
`soffice` and skips with a notice without it, exactly as loop C does. Neither may be
special-cased per file: an exclusion is a *construct* with a name, never a file name
(`CLAUDE.md`).

Three more checks, each cheap and each catching a different class of mistake:

1. **Every imported document survives our own writer and reader unchanged.** Import → write →
   read → compare. It is the same identity check phase 3 already owns, and it proves the
   importer produced something ODF can actually express rather than something that only lives
   in memory.
2. **The report is asserted, not printed.** A test fixture with a chart, a pivot table, a
   merged range and an array formula must report exactly those four kinds. A report that
   quietly stops counting is worse than no report.
3. **Hand-built fixtures for the boundaries** that no corpus reliably contains: serial 59/60/61
   in both date systems, a shared-formula group crossing a sheet edge, a four-section format
   code, a grind sheet name Excel allows and ODF does not.

---

## What this will not do

Named here so nobody has to ask, and mirrored into `doc/not-doing.md` when the phase lands.

- **Writing `.xlsx`.** Unchanged, §1, never. One way in.
- **`.xls` and `.xlsb`.** Binary formats; see the `calamine` trigger above. Not scheduled.
- **Executing anything.** Macros are data to be counted, never to be run. An `.xlsm` imports
  its cells and reports its macros.
- **Fetching anything.** External workbook links and web queries are dropped, never followed.
  A converter that makes network requests is a different threat model.
- **Round-trip fidelity.** An imported document is an *ODF* document. It is not a copy of the
  original with a different extension, and the report is how it says so.
- **Recalculating on import.** See the decisions above; it stays the user's call.

---

## Risks, honestly

1. **Scope creep into "we support Excel".** The report and the never-write rule are the
   defences, and both are mechanical. The line to hold: this is an *import filter*, not a
   compatibility layer, and a bug report of the form "Excel shows X" is answered by ECMA-376
   and the report, not by growing the core.
2. **Theme colours and built-in formats are version- and locale-dependent.** Measured against
   the oracle and recorded in `doc/xlsx-format.md`; where measurement is ambiguous, the
   construct is dropped and counted rather than guessed.
3. **The corpus is a regression suite, not a sample of the world.** LibreOffice's xlsx files
   are minimal reproductions of bugs, so they over-represent the strange. A dozen real
   documents — the ones that motivated this phase — belong in `xlsx/tests/data/` under R7's
   rule: vendored, so the requirement cannot skip.
4. **Large sheets.** Excel files with a million rows exist. The reader streams (quick-xml
   already does) and bounds materialisation the way `odf/read.rs` does; the check is a
   generated 500k-cell file in the timing test, not an assumption.
5. **Untrusted input.** A headless converter is a program that eats files from strangers. The
   hardening list in Part II §1 is the answer, and it is a test with a hostile fixture rather
   than a paragraph.
6. **`formula::shift` in the core.** The one core addition, and it must be justified in ODF's
   own terms (§5.8 relative references) rather than as "the importer needs it". If it cannot
   be, it stays in the importer.
