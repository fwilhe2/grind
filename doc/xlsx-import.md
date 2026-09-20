<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# The xlsx import filter — phase 11, built through X1

This is the work plan for reading `.xlsx`, and the document that holds it to the rules once
building starts. It is normative for this phase the way `doc/sheet-shell.md` is for phase 9.

It was first written before phase 10 split the core in two, and **refreshed on 2026-09-14**
against the workspace as it actually stands: `grind-core` + `grind-sheet` rather than one
`core/`, the `grind <app> <verb>` CLI, `formula::shift` already built, M8's column widths
already landed, and a model that has since grown charts, an autofilter and table formats. The
same pass added "Transitional, and the real world", which is the section this plan was missing
and the one that decides whether it can open other people's files at all.

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
- **Its own crate, behind a cargo feature.** `xlsx/` (crate `grind-xlsx`), an optional
  dependency of each shell that wants it. A build without the feature contains none of it.
- **Transitional is the target, Strict is nearly free, and neither is a fork.** Real files are
  ECMA-376 Transitional; the whole difference that reaches a reader is a namespace table and a
  markup-compatibility rule. See "Transitional, and the real world" below — it is the section
  that decides whether this filter opens other people's documents or only our own test files.
- **The fidelity report is part of the output, not an afterthought.** What a conversion
  dropped is as important as what it carried, and a converter that lies about it is worse
  than one that refuses. `--strict` turns any loss into a non-zero exit, which is what a
  pipeline wants.
- **The CLI's read commands stay ODF-only.** `grind sheet import in.xlsx out.ods` is the
  filter; everything else operates on the result. A CLI can run two commands where a GUI
  cannot, so the GUI's Open dialog imports transparently (X6) and `grind sheet view book.xlsx`
  deliberately does not exist. One read path per format, chosen explicitly.
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
- **Rule 4 — whatever a GUI can do, the CLI can do.** The filter lands CLI-first: `grind sheet
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
core/     [GENERIC] container, namespaces, tolerant reading, styling primitives (untouched)
sheet/    the spreadsheet: model, ODF I/O, formula engine   (untouched, knows nothing of Excel)
cli/      the `grind` binary                                optional dep: grind-xlsx
ui_*/     the five shells                                   optional dep: grind-xlsx
xlsx/     the import filter — crate `grind-xlsx`            depends on grind-sheet, zip, quick-xml
```

```
xlsx/src/
  lib.rs        import_bytes / import_file / sniff, the Report, the public surface
  address.rs    Excel's spelling of a cell address (`B2`) — X1; X2's lexer is the second caller
  names.rs      the namespace and relationship-type tables — Transitional and Strict
  mce.rs        markup compatibility: Ignorable, AlternateContent/Choice/Fallback
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
the core already models in ODF's spelling, which is exactly why they live here. `names.rs` and
`mce.rs` are the two that did not exist in the first draft of this plan, and the section below
is why.

**The tolerant-reading shape is rebuilt here, not borrowed.** `core/src/odf/context.rs` has
exactly the property this filter needs — *a context that does not recognise a child ignores
its whole subtree* — but its dispatch key is `odf::names::Ns`, a closed enum of ODF namespace
URIs, and R8 keeps it that way: OOXML's namespaces are no more generic than ODF's. So `xlsx/`
grows its own, and it is the cheaper of the two because SpreadsheetML is shallow and regular
where ODF's style tree is deep. What matters is that the *property* is the same one, because
it is what makes Strict, markup compatibility, vendor extension namespaces and every future
version of Excel inert by construction rather than one match arm at a time.

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
/// Whether these bytes are an OOXML spreadsheet — the zip magic plus a workbook part reached
/// by relationship. Sniffed from content, never from the name, as `grind_core::kind` is.
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

**A password-protected workbook is not a zip at all**, and getting this wrong makes loop A′
report a failure for a file that is merely locked. Agile encryption (ECMA-376 Part 2 §3)
wraps the whole package in a CFB/OLE container, so the bytes begin `D0 CF 11 E0` rather than
`PK`. That signature is `Error::Encrypted` — the variant `grind_core` already has and loop A
already tolerates for the same reason — and everything else that is not a readable zip is
`Error::Package`. "This is locked" and "this is not a spreadsheet" are different sentences to
put in front of a person.

```rust
pub struct Report {
    /// Transitional or Strict — a fact about the file, stated rather than branched on.
    pub flavour: Flavour,
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
    /// The same losses by class: which kind of expression stopped each one (added at X2).
    pub refused: BTreeMap<formula::Refusal, usize>,
    /// Cells past the materialisation bound, read and not carried (added at X1).
    pub over_budget: usize,
    /// Namespaces the file said a consumer must understand and this one does not
    /// (`mc:MustUnderstand`). Not a refusal — see "Transitional, and the real world" §2.
    pub must_understand: BTreeSet<String>,
}

pub enum Dropped {
    Chart, PivotTable, ConditionalFormat, DataValidation, Comment, Drawing, Macro,
    ArrayFormula, StructuredReference, ExternalLink, SheetLocalName, MergedCells,
    RichText, HiddenSheet, ThemeColor, FontFamily, Protection,
}

// X2's, and a different question: `Dropped` is what the *model* cannot express, this is what
// the *translator* would not carry. Three of these are also a `Dropped` kind; the other four
// are expressions ODF could hold and §2.3.2 leaves out.
pub enum Refusal {
    Array, InlineArray, StructuredReference, ExternalLink, Intersection, Union, Syntax,
}
```

`Dropped` is an enum rather than strings so the list is finite, greppable, and testable —
and so a new kind of loss is a compile-time decision rather than a new string somewhere.

**The list shrank when the model grew, and that is the point of keeping it honest.** Three
entries the first draft of this plan carried are gone, because phase 9 and the two phases
after it built homes for them:

- `ColumnWidth` / `RowHeight` — M8 landed. `Sheet::set_col_width` / `set_row_height` take the
  verbatim ODF length strings the model stores, so X4 converts instead of counting (§5 below).
- `AutoFilter` was never in the list and should have been: `sheet/src/filter.rs` exists now,
  and `<autoFilter ref="A1:D10"/>` → `Filter::new` is close to free. It is **carried**.
- `Chart` stays in the list but is now a *decision* rather than a necessity. The model has
  `sheet/src/chart.rs` and `doc/chart-format.md`, so there is somewhere to put one; what is
  still missing is a reader for `xl/charts/chart1.xml`, which is DrawingML — a second
  vocabulary the size of this whole filter. It is dropped and counted in this phase, and the
  reason is cost rather than the absence of a home. `doc/xlsx-format.md` records that.

**One more field since X1: `over_budget`**, the cells past the materialisation bound
(`sheet::MAX_CELLS`, four million — `odf/read.rs`'s number). It is not a `Dropped` kind, because
the model could hold those cells and the admission rule below is about what it *cannot* express,
but a bound somebody hit is a loss all the same and `lossless()` says so.

The rule the shrinking illustrates: an entry in `Dropped` must name a construct the **model**
cannot express, never one the filter simply has not got to yet. The second kind belongs in the
milestone table, where it is work rather than a permanent property of a converted document.

### Compiling with and without each feature

Two mechanisms, and they are not the same one:

- **The GUI is a separate crate.** It is already optional by construction: `cargo build -p
  grind-cli` never compiles a line of GTK, and neither does `cargo test -p grind-sheet`. The
  one gap is that bare `cargo build` / `cargo test` at the root walk every workspace member,
  which is why CI names crates explicitly. **Fix: `default-members` in the root manifest** —
  `["core", "sheet", "text", "build", "cli", "xlsx"]`, the six that need no system packages —
  so the default commands skip the shells and `--workspace` still builds everything on demand.
  One line, and it makes the common case need no flags. (The GTK shells are already out of
  `cargo build --workspace`'s practical path for want of `libgtk-4-dev`; this states it.)
- **The import filter is a cargo feature**, because it belongs *inside* the CLI binary rather
  than beside it.

```toml
# cli/Cargo.toml
[features]
default = ["xlsx"]
xlsx = ["dep:grind-xlsx"]

[dependencies]
grind-xlsx = { path = "../xlsx", optional = true }
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

## Transitional, and the real world

The first draft of this plan said "ECMA-376 is the normative source" and stopped there, which
skipped the question that decides whether the filter can open anybody else's files. ECMA-376
is not one format. ISO/IEC 29500 defines **Strict** and **Transitional**, and the second is
what every real file is: Excel has written Transitional by default since 2007 and still does,
Strict is an opt-in save format almost nothing produces, and every other generator in the wild
— LibreOffice, Apache POI, ClosedXML, EPPlus, SheetJS, Google Sheets' export — writes
Transitional too.

So **Transitional is the target and Strict is a namespace table away**. That is the whole
shape of it: the difference that reaches a reader built like this one is which URIs it
recognises, plus one rule about markup compatibility. Neither is a fork, and building it as a
fork would be the mistake.

### 1. Which flavour is a table, not a branch

Dispatch is on `(namespace-uri, local-name)` exactly as `core/src/odf/names.rs` does it, and
for exactly the same reason — a prefix is a local choice and can be redeclared on any element.
Two families of URI resolve to the same key:

| | Transitional | Strict |
|---|---|---|
| SpreadsheetML | `http://schemas.openxmlformats.org/spreadsheetml/2006/main` | `http://purl.oclc.org/ooxml/spreadsheetml/main` |
| `r:` attributes | `…/officeDocument/2006/relationships` | `http://purl.oclc.org/ooxml/officeDocument/relationships` |
| Relationship `Type=` | `…/officeDocument/2006/relationships/worksheet` | `…/ooxml/officeDocument/relationships/worksheet` |
| OPC package rels | `…/package/2006/relationships` | **the same** — OPC is Part 2 and does not vary |
| Content types | `…/package/2006/content-types` | **the same** |
| Markup compatibility | `…/markup-compatibility/2006` | **the same** — Part 3, likewise |

The Transitional column is measured (see §4 below); **the Strict column is from memory and
must be confirmed against ECMA-376 and recorded in `doc/xlsx-format.md` before it reaches
code**, like every other fact here. A `Flavour` is then a fact the `Report` states rather than
a branch the reader takes, and a file mixing the two — they exist — resolves per element and
is reported as `Flavour::Mixed` rather than refused.

Recognising Strict costs about ten lines. It is worth them not because anybody sends us Strict
files, but because writing the table forces the reader to be namespace-driven, which is the
property that makes the *next* namespace inert for free.

### 2. Markup compatibility is the tax real files charge

ECMA-376 Part 3 (MCE) is how a producer writes content a consumer may not understand, and
every file written after about 2010 uses it. Three constructs matter:

- **`mc:Ignorable="x14ac xr xr2"`** on a root element — a list of prefixes whose *attributes*
  a consumer may skip. We skip unknown attributes already: an attribute in a namespace no
  table knows simply misses every lookup. Nothing to build; it is listed here so nobody
  builds something.
- **`mc:AlternateContent` / `mc:Choice Requires="x14"` / `mc:Fallback`** — the one that needs
  real handling. The rule: we `Requires` nothing, so **every `mc:Choice` is skipped and the
  `mc:Fallback`, if there is one, is read in place of the whole `mc:AlternateContent`**. A
  reader that does not know this either misses the fallback content entirely or — worse —
  reads both the choice and the fallback and doubles whatever was inside.
- **`mc:MustUnderstand`** — a producer asserting a consumer cannot proceed without a
  namespace. It lands in `Report::must_understand` by name and **is not a refusal**: in a
  spreadsheet it guards a feature rather than the cell values, and cell values are what an
  import is for. Refusing a whole workbook because one slicer needs `x14` would be the same
  mistake as refusing it over one chart.

This lands in `mce.rs` as a rule in the element dispatcher, **once**, not as a match arm in
every context that might contain one. `mc:AlternateContent` can appear around a sparkline
group, a slicer, a data validation, a drawing, a `sheetPr` — anywhere — so per-site handling
is per-site bugs.

### 3. What real producers do that the spec permits and nobody expects

Each of these is a one-line rule in the reader and a bug report each if it is not written down
now. They are what "works with real-world documents" actually means:

- **`<row>` and `<c>` with no `r` attribute.** Position is implicit — the next row, the next
  column. The spec allows it; Excel always writes `r`; POI and SheetJS do not always. A reader
  that unwraps `r` panics on a file half the ecosystem produces.
- **`count` and `uniqueCount` on `sharedStrings`, and `dimension` on a sheet, are claims.**
  Never preallocate from them and never trust `dimension` as a bound — `A1:XFD1048576` is a
  claim a lot of generators make. Bound materialisation the way `odf/read.rs` does.
- **`xml:space="preserve"`** on `<t>` — leading and trailing spaces in a shared string are
  meaningful, and a reader that trims them corrupts text silently.
- **Part names.** Resolve by relationship, never by path convention; then tolerate the
  targets real producers write: a leading `/`, a `..` segment, a `\` separator, and percent
  encoding. Normalise, and refuse anything that escapes the package.
- **A BOM, or an XML declaration with `standalone="yes"`, on any part.** Both are common and
  neither is a problem unless the reader assumes the first byte is `<`.
- **`<f>` with no cached `<v>`.** A non-Excel writer that does not evaluate leaves the formula
  with no value. Given the "nothing is evaluated on import" decision this is the only case
  where an imported cell has a formula and no number — **carry the formula, leave the value
  empty, count it**. Recalculating is still the user's call, and `grind sheet recalc` is
  exactly the command that fills them in.
- **`.xlsm` is the same XML.** The macro is a separate part, counted as `Dropped::Macro` and
  never executed. A macro-enabled workbook imports its data like any other.

### 4. The evidence

Measured on 2026-09-14 by converting `sheet/tests/data/kb/formula.fods` with the pinned
oracle (`soffice --headless --convert-to xlsx`) and reading the parts back. Even
**LibreOffice's own** xlsx output — not Excel's, and about as plain a file as this project can
produce — declares on its root elements:

```
xmlns    = http://schemas.openxmlformats.org/spreadsheetml/2006/main     (Transitional)
xmlns:r  = http://schemas.openxmlformats.org/officeDocument/2006/relationships
xmlns:mc = http://schemas.openxmlformats.org/markup-compatibility/2006
xmlns:x14, x15, xr, xr2, xr6, xr10 = Microsoft extension namespaces
```

If the minimal case declares seven namespaces beyond the one it uses, the median real
document is not going to be kinder. That is the argument for the tolerant shape rather than a
schema-shaped reader, and it is why `names.rs` and `mce.rs` are in the file list at all.

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
| `Sheet1:Sheet3!A1:A2` | `[Sheet1.A1:Sheet3.A2]` | the same rule where the body is a range, rather than a cell |
| `H2:INDIRECT(…)` | `[.H2]:INDIRECT(…)` | a range whose far end is computed: `:` as §5.8's operator, since it cannot be one `Reference` |
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
semantics, not Excel's** — §5.8 is where relative references are defined — so it belongs in
the core, and **it is already there**: `sheet/src/formula/shift.rs`, built for the fill and
drag that wanted the same function, and it turns a reference that would leave the sheet into
`#REF!` the way a delete does. This phase adds no core function for it; it calls one. The
*grouping* stays in the importer, where Excel's spelling belongs.

A group is resolved **once the whole sheet has been read**, not as each follower arrives:
document order says nothing about where a master sits, and `formulas/shared-groups.xlsx` has one
at F2 whose only follower is F1, above it. That file is also where the `#REF!` case comes from —
the follower shifts a reference to A1 up by a row, off the sheet.

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

**Column widths and row heights** were the one place this plan waited on another, and the wait
is over: phase 9's **M8** landed them, so `Sheet::set_col_width` / `set_row_height` exist and
X4 converts rather than counting. `<col width="8.43"/>` needs ECMA-376 §18.3.1.13's
character-width conversion (through the Normal font's maximum digit width) and `<row ht="15"/>`
is points; both become the verbatim ODF length strings the model stores, and the conversion
constant is measured against the oracle and recorded in `doc/xlsx-format.md` rather than
taken from the spec's prose alone. `<col hidden="1"/>` and `<row hidden="1"/>` map onto
`set_col_hidden` / `set_row_hidden`, which the model also has now.

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
- **Autofilters.** `<autoFilter ref="A1:D10"/>` → `Filter::new`, which the model gained after
  this plan was first written. Carried, not counted. The filter's *criteria*
  (`<filterColumn>`) are a second question and follow `sheet/src/filter.rs`'s own vocabulary
  where they fit, counted where they do not.
- **Everything else in the part list** — charts, pivot tables, conditional formatting, data
  validation, comments, drawings, protection — is counted by kind and dropped. Recognising
  them costs a match on the content type and buys an honest report.

---

## Part III — milestones

Every milestone lands green: `cargo test`, clippy clean, `reuse lint`, the loops, and the
feature matrix.

| # | Milestone | Contents | Exit criterion |
|---|---|---|---|
| X0 | **The seam** — **DONE (2026-09-14)** | `xlsx/` crate, feature flags, `default-members`, CI matrix steps, `names.rs` + `mce.rs` + `xml.rs`, `package.rs` + `workbook.xml` sheet list, `grind sheet import` writing an empty document with the right sheets | the matrix builds; `cargo test -p grind-cli --no-default-features` passes; the output validates with `jing -i`; **loop A′ green** — every corpus file reaches a sheet list without an `Err` or a panic |
| X1 | **Values** — **DONE (2026-09-19)** | shared strings, cell types, the two date systems and the leap-year rule, bounded materialisation, implicit `r`, `Report` v1, `doc/xlsx-format.md` opened | **loop D** green on the value-only corpus: every cell equals what the oracle's conversion produced, at 15 significant digits — **68 workbooks, 501,335 cells, 0 disagreements**, against the pinned oracle, with eight named divergences where the oracle is the one that differs (below) |
| X2 | **Formulas** — **DONE (2026-09-20)** | the Excel expression translator (`formula.rs`), shared-formula groups over the core's existing `formula::shift`, `_xlfn.`, 3-D refs, the exclusion classes as a `Refusal` enum, `<f>` with no `<v>` | every formula in the corpus either round-trips through our canonical serialiser or falls in a named class — **12681 of 13039 translated (97.3%) over 362 workbooks, and every one of the other 358 counted by class**; the generated corpus asserts formula *text* per cell, 122 of 125 claims, 3 named |
| X3 | **Number formats** | built-ins by meaning, the code parser, sections → `style:map` | loop D compares **displayed text** per cell, which is loop C's rule for the same reason |
| X4 | **Styles and geometry** | fonts, fills, borders, alignment, theme and indexed colours; column widths, row heights and hidden tracks, all of which the model now has | loop D compares styles the way loop C does — borders numerically, everything else exactly |
| X5 | **The document level** | defined names, sheet order and visibility, merges, autofilters, the report as JSON, `--strict` | `grind sheet import --format json` counts every dropped construct; `--strict` exits non-zero when anything was dropped |
| X6 | **The shells** | the GTK Open dialog learns `.xlsx` (import → a new unsaved document, retitled `.ods`), file filters, the wasm shell's note | open an `.xlsx` in the GUI, edit it, save it as `.ods` |

### What X0 found

Four things the plan did not predict, recorded here rather than absorbed silently:

1. **`xml.rs` is a fourth file.** The plan's file list had `names.rs` and `mce.rs` but no
   walker, as though the tolerant shape came from somewhere. It does not: `context.rs`
   dispatches on a closed enum of *ODF* namespace URIs and R8 keeps it that way, so the
   ignore-the-subtree property is rebuilt here. It is cheaper than the original — SpreadsheetML
   is shallow, so a visitor per element beats a context object per element — and markup
   compatibility lives inside it, in one place, which is the whole reason MCE is tractable.
2. **`--format json` landed in X0, not X5.** A command has to return a `Report`, and the CLI
   has no other way to print one; building the variant was unavoidable, so the JSON came with
   it. `--strict` is still X5, because a flag nobody has tested is worse than no flag.
3. **The corpus keeps three files in a wrapper.** `sc/qa/unit/data/README` says files with
   `CVE` in the name are RC4-encrypted so that virus scanners leave the repository alone, and
   gives the key. They are not malformed and excluding them would have excused the wrong
   thing; loop A′ undoes the wrapper in fifteen lines and imports them like anything else,
   which turns three skips into three genuinely hostile inputs. **360/360.**
4. **Strict was measurable after all.** Three corpus workbooks use it, so §1.2 of
   `doc/xlsx-format.md` went from `UNVERIFIED` to `MEASURED` before a line of `names.rs` was
   written — and measuring found two things guessing had missed: a `conformance="strict"`
   attribute, and a file carrying both families at once.

### What X1 found

Seven things, the first three of them outside this crate:

1. **`grind-sheet`'s own writer could not write a sheet with a cell in each far corner.**
   `scale/wide-and-sparse.xlsx` has six cells, at A1 and at `XFD1048576`; its used rectangle is
   the whole grid, and the ODF writer asked every one of its seventeen billion cells whether it
   was blank. It never finished. Nothing about that is Excel's — a projection written by hand
   says the same thing in two lines — and the fix is in ODF's terms: `Sheet::rows_carrying`
   answers "which rows hold anything" from the column store's runs and the side tables, once,
   and the writer asks it (23 ms for that file now). A test holds it to the per-cell rule it
   replaces. **Risk 6's "add nothing to the core" held for the import** — the filter needed no
   capability it did not have — and was broken by a bug the import happened to be first to reach.
2. **`xml.rs` read past a self-closed element.** `children()` and `text()` on `<t/>` or
   `<c r="A11"/>` went looking for an end tag that does not exist and consumed the parent's. X0
   survived it only because its one self-closed container, `<sheets/>`, was last in its part.
3. **The oracle recalculates.** This section used to say a formula's cached value "is what
   makes X1's oracle comparison meaningful"; loop D found the oracle replaces cached values with
   its own evaluation on load (`formulas/shared-groups.xlsx` caches C2 = 4 and the oracle writes
   6). So a formula cell's value is compared with the oracle only from X2 on, and meanwhile
   `ooxmlgen.rs` holds every cached value to the manifest instead. `doc/xlsx-format.md` §2.3 has
   this and the oracle's five other divergences — an empty string dropped, a sheet dropped for
   its name, an external link turned into a sheet, a `\` target not followed, a pivot table
   regenerated — plus §2.1's phantom day and §2.2's `#REF!`.
4. **A bare `m` is a month.** `numfmt::classify` first read `m` with no year or day beside it as
   minutes; it is minutes only after an hour or before a second (`doc/xlsx-format.md` §3.2).
   `numfmt/datetime-codes.xlsx` caught it, one cell per spelling.
5. **X1 needed a sliver of X3.** A serial is not a date until its format says so, so styles are
   read before sheets and `numfmt.rs` answers *date, time or number* for a format — nothing
   else. The translation of a format onto `numfmt::Format` is still X3's.
6. **`mc:MustUnderstand` is reported by URI**, X0's open question 3 below. The prefix is
   resolved while its declaration is in scope, which is the only moment it can be.
7. **The generated corpus got expensive.** Its 500,000-cell sheet is imported, written and
   read back by `every_imported_document_survives_a_write_and_a_read` in a debug build: about two
   minutes, against a second and a half in release — the one test in the suite whose cost is a
   corpus file's size. quick-xml, zip, zlib-rs and memchr are now optimised in the dev profile,
   which took the import alone from 14 s to 5; the rest is the debug ODF reader, left as it is.

### What X2 found

Six things, and the first is the one that changed the shape of the milestone:

1. **A refusal needed a class, and the class needed to reach the report.** The plan said an
   untranslatable formula is counted; it did not say *by what*. `Report::untranslated` gives
   the addresses and `Dropped` covers exactly two of the classes — a structured reference and
   an external link are constructs the model has no home for, where an inline array or an
   intersection is an expression ODF could hold perfectly well and this build chooses not to
   evaluate. So `formula::Refusal` is the vocabulary and `Report::refused` counts by it: where
   and why are different questions, and "four formulas lost" is a number where "four structured
   references" is something a person can act on. It is also what makes X2's exit criterion a
   scoreboard rather than a percentage.
2. **`Refusal::Array` is a class the parser never returns.** `t="array"` is a fact about the
   *cell*, decided before the expression is read, and giving it a variant anyway is what keeps
   `refused` summing to `untranslated` — a report whose two halves disagree is worse than one
   half.
3. **The range operator is not only a reference.** `SUM(H2:INDIRECT(ADDRESS(ROW()-1,COLUMN())))`
   appears once in the corpus, and a scanner that only knows `A1:B2` refuses it. ODF has the
   operator (§5.8), the core's own parser reads `[.H2]:INDIRECT(1)` and prints it back
   unchanged, so refusing it would have made this filter narrower than the format it writes.
   The scan-a-whole-reference path stays, because `A1:B2` must be *one* `Reference` rather than
   two operands and an operator; the operator is what is left when that path cannot.
4. **15 formulas in 362 workbooks land in `Syntax`, and 12 of them are one file.**
   `tdf165886.xlsx` writes `OR(D1=0,D1<>““)` with typographic quotes where string delimiters
   belong — the file behind a LibreOffice bug about exactly that. The remainder is noise of the
   same kind. The open-ended class is doing its job: it is 0.1% of the corpus and every other
   loss has a name.
5. **The manifest and its fixtures disagree three times, not once.** X1 found one
   (`xml-space-preserve.xlsx`'s indentation); X2 found `semantics-differ.xlsx` expecting
   ROUNDDOWN and ROUNDUP to be reported as unknown when the workbook calls neither, and
   `tables.xlsx` expecting four structured references where its worksheet holds seven — four is
   the number of *distinct forms*, and every other kind in the report is counted once per cell
   that lost something. Both are now `DECIDED_OTHERWISE` entries citing the bytes.
6. **Two spellings are ours rather than the oracle's, and both are the core's.** A reference
   error prints as `#REF!` where LibreOffice writes `[#REF!]` — §5.8's bracketed form and
   §5.12's name are one value in `Expr`, and its serialiser writes the name — and a cross-sheet
   reference prints as `[Data.A1]` where the oracle writes `[$Data.A1]`. Neither is this
   filter's decision to take, which is why loop D compares values and the *manifest* is the
   oracle for formula text.

**Order.** Values before formulas before formats is not arbitrary: a date is only a date once
its format is known, so X3 closes a gap X1 opened rather than adding a new one — X1 took the one
question it could not do without (finding 5) and nothing more.

---

## Verification

The project checks correctness against LibreOffice rather than against its own opinion, and
this phase adds the loop that does it for import.

| Loop | Asserts | Corpus |
|---|---|---|
| **A′** — read tolerance | every `.xlsx` in the corpus imports without an `Err` and without a panic | `sc/qa/unit/data/xlsx/`, plus `xlsm/` — the count this plan first claimed (352) is **unverified**; X0 measures it and writes the real `FLOOR`, which then only goes up |
| **D** — import fidelity | our conversion and the oracle's conversion of the same file agree, semantically | **built at X1** over the vendored generated corpus (`xlsx/tests/loop_d.rs`), so it needs `soffice` and no checkout; LibreOffice's own 352 are still to come |
| **R2** | every imported document validates against the ODF schema (`jing -i`) | the same |
| **the generated corpus** | every claim a manifest makes about a fixture is satisfied, or named in one of two tables | `xlsx/tests/data/corpus/`, vendored — **never skips**. See below |

Loop D, concretely:

```
ours   = grind_xlsx::import_bytes(bytes)              → Document
theirs = soffice --headless --convert-to ods <file>   → read with our own reader → Document
compare(ours, theirs)
```

The comparison is `sheet/tests/roundtrip.rs`'s existing semantic comparator's rule, restated: values at
15 significant digits (because that is all LibreOffice writes — `doc/ods-format.md` §3.4),
formulas as canonical text, formats as **the text the cell displays**. The oracle's output is
cached by content hash so a full run is one conversion per file, ever.

**As built at X1** it compares every cell's value and kind, sheets matched by name. Where the
oracle is the one that differs the disagreement is a named **divergence** — a construct, never
a file, each written only after the oracle's own output was read for that cell — asserted to
still occur, so that one LibreOffice stops having fails the loop and has to be deleted. The
cache is keyed by the manifest's SHA-256 and the oracle's version string, and lives in the temp
directory because that is what the pinned oracle's shim mounts. It runs in CI's `oracle` job
beside loops C and E, and in `scripts/soffice-tests.sh`.

**Loop A′ runs in CI and costs nothing to get there** — it needs no oracle and no display,
only the corpus, and `ci.yml`'s `corpus` job already sparse-checks out `sc/qa/unit/data`,
which is the directory `xlsx/` sits in. The loop is a test file and no workflow change. Loop D
needs `soffice` and skips with a notice without it, exactly as loop C does. Neither may be
special-cased per file: an exclusion is a *construct* with a name, never a file name
(`CLAUDE.md`).

Five more checks, each cheap and each catching a different class of mistake:

1. **Every imported document survives our own writer and reader unchanged.** Import → write →
   read → compare. It is the same identity check phase 3 already owns, and it proves the
   importer produced something ODF can actually express rather than something that only lives
   in memory.
2. **The report is asserted, not printed.** A test fixture with a chart, a pivot table, a
   merged range and an array formula must report exactly those four kinds. A report that
   quietly stops counting is worse than no report.
3. **Hand-built fixtures for the boundaries** that no corpus reliably contains: serial 59/60/61
   in both date systems, a shared-formula group crossing a sheet edge, a four-section format
   code, a sheet name Excel allows and ODF does not.
4. **Fixtures for the real-world section**, which is the half a LibreOffice corpus is least
   likely to cover, since its files are minimal bug reproductions rather than things Excel
   wrote: an `mc:AlternateContent` whose `mc:Choice` and `mc:Fallback` hold *different* cell
   values (a reader that takes the wrong one, or both, fails visibly rather than subtly),
   rows and cells with no `r` attribute, a shared string with `xml:space="preserve"`, a
   `<f>` with no `<v>`, a `dimension` claiming `A1:XFD1048576` over four cells, a part target
   with a leading `/`, and a CFB-wrapped encrypted workbook that must come back
   `Error::Encrypted` rather than `Error::Package`. Each is small enough to write by hand and
   each is a class of file the wild contains.
5. **A Strict fixture**, one file, asserting only that it imports and reports
   `Flavour::Strict`. Not a supported configuration — a proof that the namespace table is a
   table.

### The generated corpus — `xlsx/tests/ooxmlgen.rs`

Checks 3, 4 and 5 above were a plan to write fixtures by hand, and hand-assembly stops where
hand-assembly stops: nobody writes a four-section number format, eighteen fill patterns and
500,000 cells into a test file. So they are **generated** instead, by
[ooxmlgen](https://github.com/fwilhe2/ooxmlgen), from the Open XML SDK — which means the bytes
are a real producer's rather than our idea of one, and which answers risk 3 without waiting
for a dozen real documents to be collected. `xlsx/tests/fixtures.rs` stays exactly as it is:
its packages are readable *in the source*, which is a different and still necessary property.

**76 fixtures, 5.4 MB, vendored** at `xlsx/tests/data/corpus/` from run
[34999727235](https://github.com/fwilhe2/ooxmlgen/actions/runs/34999727235) (ooxmlgen 0.2.0,
2026-09-15) — R7's rule, so the check cannot skip, and no workflow change: `grind-xlsx` is a
default member and the whole corpus imports in 1.2 s. Eight families, one per thing that can
go wrong: `values/`, `formulas/`, `numfmt/`, `styles/`, `geometry/`, `document/`, `scale/`,
`realworld/` and `hostile/`.

What earns it its size is `manifest.json`, which is not an index but **the expectation**,
machine-readable and written in this crate's own vocabulary: the `Dropped` variants by name,
`Flavour` by name, the `Error` variant a hostile file must produce, the sheet list in workbook
order, every fixture's SHA-256, and for each of 1341 cells both what the file literally
contains and what a conversion of it should produce. That is loop D's oracle, travelling with
the corpus instead of waiting for `soffice` — and where a conversion is genuinely ambiguous a
cell carries an `oracle` field, so the two can be told apart.

Since this build is X0, most of that oracle is about a later milestone. Two tables carry the
difference and both are checked in **both directions**:

- **`PENDING`** — a claim this build does not satisfy yet, with the milestone that will and
  the reason it cannot from here. A claim that starts passing **fails the test**, so the entry
  has to be deleted when its milestone lands. It is loop F's "a test that fails the day it is
  projected" applied to a roadmap, and it is what stops a milestone table from quietly
  becoming a list of things that already work. 21 entries at X0; **18 since X1**, which
  satisfied three and had to delete them.
- **`DECIDED_OTHERWISE`** — a claim this build answers differently *on purpose*. 2 entries at
  X0, **3 since X1**: the third is `realworld/xml-space-preserve.xlsx`'s A5, whose manifest
  wants twenty spaces of indentation the fixture's own `sharedStrings.xml` does not contain.

Everything else is asserted. At X0 that was 138 of 161 claims, with the cell-level half held by
one assertion that `report.cells == 0`, written to fail the day X1 began. It did, and was
replaced by the comparison it promised: **1481 of 1502 claims now, and 1340 of 1341 cells** —
every cell's value and kind, one claim per cell (`cell:Sheet!A1`), so a table entry can name
exactly the cell it excuses.

### What the generated corpus found

1. **Two corpus expectations are wrong about this filter, not the other way round**, and both
   are in `hostile/`. `no-workbook-part.xlsx` expects `Error::Package`; a package that opened
   perfectly and contains no workbook is `Error::NotSpreadsheet`, which is the sentence that
   variant exists to say. `zip-slip.xlsx` expects a refusal; this filter imports the one
   legitimate workbook in it and both escape attempts fail *structurally* — a zip entry name
   is a key in a package that is never extracted, and the two relationship targets that climb
   out resolve to no part. Refusing the file would throw away a readable workbook over an
   attack that already missed. The property that actually matters is now asserted directly:
   nothing outside the package is touched, checked against the `/tmp` marker path the fixture
   carries for exactly that purpose.
2. **`Flavour::Mixed` needs X1 to be detectable in the ordinary case** — and X1 detects it. `mixed-flavour.xlsx`
   is a Transitional workbook with a Strict *worksheet*, and X0 opens no worksheet, so the
   Strict namespace is never seen. The three Mixed files in LibreOffice's corpus all declare
   both families in `_rels/.rels`, which is why X0 could measure `Mixed` at all (§1.2) —
   a second spelling of the same fact was not visible until now.
3. **`mc:MustUnderstand` has a spelling question waiting at X1** — answered there: the URI. The manifest expects the
   namespace **URI**; `mce::must_understand` deliberately yields the **prefix**, and says why.
   The URI is the stronger spelling — a prefix is a local alias and a report that prints one
   tells a bug report nothing — but resolving it needs the declaring element's namespace
   scope, which `xml.rs` does not keep. Decide it when X1 opens the part the attribute is on.
4. **"A sheet name Excel allows and ODF does not" has no milestone.** It is check 3 in the
   list above and appears in no row of Part III's table. `sheets.xlsx` has ten names, eight of
   which — spaces, an apostrophe, CJK, 31 characters — already come through exactly;
   `Has[Brackets]` and `Has/Slash` are carried verbatim where the manifest wants
   `Has_Brackets_` and `Has_Slash`. They survive our own writer and reader, so what is lost is
   *addressability* rather than the name: `[.Has/Slash.A1]` is not a reference. Parked in
   `PENDING` against X5 until the table says otherwise.

Nothing else disagreed: 69 fixtures import, 6 refuse with exactly the named error, 75 of 76
sheet lists match name for name and in order, and the whole `hostile/` family — entity
expansion, fifty thousand levels of nesting, a DTD naming a remote URL, a quarter-gigabyte
part in a 263 KB archive, ten thousand parts — returns in 0.5 s with no panic and nothing
fetched, which is risk 5 answered by a test rather than by a paragraph.

---

## What this will not do

Named here so nobody has to ask, and mirrored into `doc/not-doing.md` when the phase lands.

- **Writing `.xlsx`.** Unchanged, §1, never. One way in.
- **`.xls` and `.xlsb`.** Binary formats; see the `calamine` trigger above. Not scheduled.
- **SpreadsheetML 2003** — Excel's *other* XML format, a bare `.xml` file in
  `urn:schemas-microsoft-com:office:spreadsheet`. It is XML and it is Excel's, so "we read
  the XML one" does not exclude it and this line has to. A different vocabulary with a
  different data model that Excel stopped writing by default in 2007; not scheduled.
- **Strict as a supported configuration.** The namespace table recognises it and one fixture
  proves it, which is a different claim from being tested against Strict files in anger.
  `Flavour` is in the report so a bug report can say which it was.
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
   rule: vendored, so the requirement cannot skip. **Answered, in the shape that scales**: 76
   workbooks from the Open XML SDK are vendored at `xlsx/tests/data/corpus/` with a manifest
   saying what each one should convert to (`xlsx/tests/ooxmlgen.rs`, above). Generated by a
   real producer beats a dozen collected by hand, and it keeps growing without a licensing
   question attached to somebody's actual spreadsheet. The dozen real documents are still
   worth having for the things a generator does not think of — they are no longer what the
   phase is waiting on.
4. **Large sheets.** Excel files with a million rows exist. The reader streams (quick-xml
   already does) and bounds materialisation the way `odf/read.rs` does; the check is a
   generated 500k-cell file in the timing test, not an assumption.
5. **Untrusted input.** A headless converter is a program that eats files from strangers. The
   hardening list in Part II §1 is the answer, and it is a test with a hostile fixture rather
   than a paragraph.
6. ~~**`formula::shift` in the core.**~~ Retired: it was built for fill and drag before this
   phase started, justified in ODF's own terms (§5.8) by a caller that is not an import
   filter. **This phase now plans to add nothing to the core at all**, which is the stronger
   position and the one to defend — a "the importer needs it" core change is the first sign
   that Excel's semantics are leaking inward, and R1 is what it would be leaking past.
