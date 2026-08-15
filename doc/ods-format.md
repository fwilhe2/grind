<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Clean-Room ODF Spreadsheet (ODS) Generation Spec

Reference for an AI agent implementing a clean-room Rust ODS reader *and* writer. No
LibreOffice (LO) source is reproduced here — only grammar facts derived from the OASIS
`OpenDocument-v1.4-schema.rng` (in repo root) and behavioral/architectural facts derived
from *observing and citing* (file + line, never copying) LO's own filter source
(`xmloff/`, `sc/source/filter/xml/`, `package/source/`) and test fixtures.

Goals, in priority order: (1) valid ODF 1.4 on write, (2) round-trips through LibreOffice
with full fidelity (styles/formats/formulas render exactly as intended) in both directions,
(3) reads the wide variety of ODS/FODS files LO itself and third-party tools actually
produce — which are frequently *not* minimal and often contain vendor cruft, unknown
extensions, or minor structural sloppiness — without failing, (4) on write, smallest byte
count that still satisfies (1)+(2). Scope: spreadsheets only. Sections marked **[GENERIC]**
apply verbatim to future odt/odp support; sections marked **[ODS]** are spreadsheet-specific.

Sections 1–7 cover the **write** path (wire format + minimal templates). Sections 8–9 cover
the **read** path: the parsing architecture LO itself uses and why it's structurally
tolerant of messy input, then a concrete playbook of specific tolerance behaviors to
replicate, each cited to the exact LO source mechanism.

---

## 1. Container formats **[GENERIC]**

An ODF document exists in exactly one of two physical forms. Both produce the *same*
logical XML content model (Part 2 schema) — only the packaging differs.

### 1.1 Package form (`.ods`, a ZIP)

Required entries, in this order:

1. `mimetype` — **first entry**, **stored (no compression)**, **zero extra-field length**,
   raw bytes = the ASCII media type string, byte-for-byte, **no trailing newline, no BOM**.
   For spreadsheets: `application/vnd.oasis.opendocument.spreadsheet` (confirmed against
   `package/source/zippackage/ZipPackage.cxx::WriteMimetypeMagicFile`, which writes it with
   `nMethod = STORED` before anything else). Readers (including LO) sniff this at a fixed
   offset to identify the format before parsing any XML — get this exact or the file may be
   misdetected.
2. `META-INF/manifest.xml` — lists every part with its media type (§1.3).
3. `content.xml` — required. Root: `office:document-content`.
4. `styles.xml` — optional but recommended (see §1.4 on when you can skip it).
5. `meta.xml` — optional. Root: `office:document-meta`.
6. `settings.xml` — optional. Root: `office:document-settings`.

All other entries may be `deflate`d normally. All XML parts are UTF-8, declared via
`<?xml version="1.0" encoding="UTF-8"?>`, no BOM.

### 1.2 Flat form (`.fods`, plain XML)

Single XML document, root element `office:document`, which inlines everything the four
package parts would have held, in this fixed order (all but body are optional — see
`office-document` grammar rule):
`office:meta?, office:settings?, office:scripts?, office:font-face-decls?, office:styles?, office:automatic-styles?, office:master-styles?, office:body`.

Root attributes:
```
office:mimetype="application/vnd.oasis.opendocument.spreadsheet"
office:version="1.4"
```
Confirmed ground truth: `sc/qa/unit/data/fods/*.fods` — every LO-authored `.fods` fixture
uses the **plain (non-flat-suffixed)** media type string as `office:mimetype`. The
`-flat-xml` suffixed string (`application/vnd.oasis.opendocument.spreadsheet-flat-xml`,
see `filter/source/config/fragments/types/calc_ODS_FlatXML.xcu`) is only LO's *internal*
UNO type-detection label — it is never written into the file. Do not put `-flat-xml` in
`office:mimetype`; that would make LO fail to detect the file as ODS.
Extension: `.fods` (also detected from plain `.xml` by content sniffing, but prefer `.fods`).

### 1.3 `manifest.xml` (package form only) **[GENERIC]**

Namespace: `urn:oasis:names:tc:opendocument:xmlns:manifest:1.0`, root `manifest:manifest`
with `manifest:version="1.4"`. One `manifest:file-entry` per part, plus one for the package
root itself (`manifest:full-path="/"`) declaring the overall document media type and
version. Minimal ODS manifest:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.4">
 <manifest:file-entry manifest:full-path="/" manifest:version="1.4" manifest:media-type="application/vnd.oasis.opendocument.spreadsheet"/>
 <manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/>
 <manifest:file-entry manifest:full-path="styles.xml" manifest:media-type="text/xml"/>
</manifest:manifest>
```
Only list parts you actually included (omit the `meta.xml`/`settings.xml` lines if skipped
— but if you include a part in the ZIP, it must be listed here or LO's package layer will
reject/ignore it).

### 1.4 Minimality guidance **[GENERIC]**

- `meta.xml` and `settings.xml` are always safe to omit — LO fills in defaults (generator
  string, view state) silently.
- `styles.xml` may be omitted entirely if you have no named/reusable styles beyond what
  fits in `content.xml`'s own `office:automatic-styles` (automatic-styles is legal in
  *both* `content.xml` and `styles.xml`; content.xml's copy is scoped to that content, and
  omitting `styles.xml` costs you nothing schema-wise). For non-trivial documents keep
  `styles.xml` anyway once you have named (user-facing) cell styles, since
  `office:styles` (named styles) only exists in `styles.xml` / flat root, not in
  `office:document-content`.
- Prefer the flat form for tiny/generated/test documents (fewer moving parts, no zip
  machinery); prefer the package form for anything a human will resave in LO, since that's
  LO's native save format and round-trips more predictably.
- Never declare a namespace prefix you don't use on that part. Each namespace used by that
  document part's XML must be declared on its own root element (parts don't share scope).

---

## 2. Namespaces actually needed for ODS **[ODS]**

Only declare what you use. The core OASIS list (all in the RNG's grammar prologue) plus the
two LO-only extension namespaces:

| Prefix | URI | Needed for |
|---|---|---|
| `office` | `urn:oasis:names:tc:opendocument:xmlns:office:1.0` | always |
| `table` | `urn:oasis:names:tc:opendocument:xmlns:table:1.0` | always (sheet/grid) |
| `style` | `urn:oasis:names:tc:opendocument:xmlns:style:1.0` | any styling |
| `text` | `urn:oasis:names:tc:opendocument:xmlns:text:1.0` | cell text runs (`text:p`) |
| `number` | `urn:oasis:names:tc:opendocument:xmlns:datastyle:1.0` | number/date/currency formats |
| `fo` | `urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0` | fo:* style props (font-size, borders, padding, background-color) |
| `svg` | `urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0` | width/height on some props, e.g. draw objects |
| `of` | `urn:oasis:names:tc:opendocument:xmlns:of:1.2` | only if you write `table:formula` (the value is a plain string like `of:=SUM(...)`; the namespace declaration is documentation convention, not structurally validated, but LO and real files always declare it — do the same) |
| `xlink` | `http://www.w3.org/1999/xlink` | hyperlinks in cells, external data links |
| `meta` | `urn:oasis:names:tc:opendocument:xmlns:meta:1.0` | only in `meta.xml` |
| `dc` | `http://purl.org/dc/elements/1.1/` | only in `meta.xml` (dc:title/creator/date) |
| `config` | `urn:oasis:names:tc:opendocument:xmlns:config:1.0` | only in `settings.xml` |
| `loext` | `urn:org:documentfoundation:names:experimental:office:xmlns:loext:1.0` | only if using an LO-only extension attribute/element (§6) |
| `calcext` | `urn:org:documentfoundation:names:experimental:calc:xmlns:calcext:1.0` | only for pivot tables / some conditional-format extensions (§6) |

A bare data-only sheet needs only `office` + `table` (see §5.1 template — this is exactly
what LO's own minimal test fixture uses, zero style/number/text namespaces).

---

## 3. Document structure **[ODS]**

### 3.1 `office:document-content` (or the equivalent slice of the flat root)

Fixed child order: `office:scripts?`, `office:font-face-decls?`, `office:automatic-styles?`,
`office:body` (required). `office:body` contains exactly one `office:spreadsheet`.

`office:spreadsheet` content, fixed order:
`table:tracked-changes?`, `text:variable-decls?/text:sequence-decls?/text:user-field-decls?`
(rarely needed), `table:calculation-settings?`, `table:content-validations?`,
`table:label-ranges?`, then `table:table*` (zero or more sheets), then
`table:named-expressions?`.

For a plain grid you only need: `office:body > office:spreadsheet > table:table+`.

### 3.2 `table:table` (one sheet)

```
table:table
  table:table-attlist        (table:name, table:style-name, table:print-ranges, ...)
  table-title? table-desc?
  table-columns-and-groups    -- REQUIRED, at least one table:table-column
  table-rows-and-groups       -- REQUIRED, at least one table:table-row
  table:named-expressions?
```
Both the column block and the row block are **mandatory** (`oneOrMore` in the grammar) —
even an all-empty sheet needs at least one `<table:table-column/>` and one
`<table:table-row>` containing at least one cell. `table:name` should be set (LO uses it as
the visible tab name); avoid `/`, `\`, `[`, `]`, `*`, `?`, `:`, `'` in names for maximum
compatibility with other consumers even though ODF itself doesn't restrict this.

### 3.3 Columns and rows

`table:table-column` is empty (no children), attributes only:
`table:number-columns-repeated`, `table:style-name` (a `table-column`-family style, for
width), `table:default-cell-style-name` (style applied to any cell in this column that
doesn't specify its own `table:style-name` — this is how LO gives a whole unformatted
column a format cheaply), `table:visibility` (`visible|collapse|filter`).

`table:table-row` wraps `oneOrMore` of `table:table-cell | table:covered-table-cell`.
Attributes: `table:number-rows-repeated`, `table:style-name` (a `table-row`-family style,
for height).

**Repeat compression is the main lever for minimal file size.** A sheet used up to row 1000
but only populated through row 20 does *not* need 980 empty `<table:table-row>` elements —
emit one `<table:table-row table:number-rows-repeated="980"><table:table-cell/></table:table-row>`
(or omit the tail rows entirely: ODF/LO treat "not mentioned" the same as "empty default"
up to the sheet's implicit max dimension — but writing a single trailing repeated
empty-row/empty-cell run is the conventional, safest way to bound the sheet size explicitly
without ambiguity). Do the same for repeated empty cells within a row via
`table:number-columns-repeated` on `table:table-cell`.

### 3.4 `table:table-cell`

Attributes (interleaved, all optional except where a value-type forces one):
- `table:number-columns-repeated` — run-length compression, identical cell repeated N times.
- `table:style-name` — a `table-cell`-family style (formatting + number format).
- `table:formula` — string, `of:` grammar, see §4.
- `table:content-validation-name` — links to a `table:content-validations` entry.
- one of the **value-type groups** (`common-value-and-type-attlist`, mutually exclusive):

| `office:value-type` | required companion attribute | notes |
|---|---|---|
| `float` | `office:value` (xsd:double) | plain number |
| `percentage` | `office:value` (xsd:double, e.g. `0.5` for 50%) | pair with a `number:percentage-style` |
| `currency` | `office:value` + optional `office:currency` (ISO 4217 code, e.g. `EUR`) | pair with a `number:currency-style` |
| `date` | `office:date-value` (ISO 8601 date or dateTime) | actual calendar date, no epoch ambiguity |
| `time` | `office:time-value` (ISO 8601 duration, e.g. `PT13H30M00S`) | |
| `boolean` | `office:boolean-value` (`true`/`false`) | |
| `string` | optional `office:string-value` | if omitted, the display text (`text:p` children) *is* the value |
| `error` | optional `office:string-value` | formula error cells |

**`office:value` is xsd:double, but LibreOffice writes it at 15 significant digits.** A
double needs up to 17 to round-trip, so a value LO has saved is not necessarily the value
that was in memory: `1/3` comes back as `0.333333333333333`. This is not a Calc decision but
a platform-wide one — every ODF double goes through `sax::Converter::convertDouble`
(`sax/source/tools/converter.cxx:779`), which asks `rtl::math::doubleToUStringBuffer` for
`rtl_math_StringFormat_Automatic`. LO's own reader carries a special case for the fallout:
`sal/rtl/math.cxx:364-366` recognises `1.79769313486232e+308` as the `DBL_MAX` it
"wrote everywhere", because at 15 digits the largest double no longer parses back.

Consequences for a writer, both worth having:
- Emitting full precision ourselves is correct and safe — LO reads all 17 digits fine; it
  simply will not write them back.
- Any round-trip check through LO must compare doubles at 15 significant digits. Insisting
  on exact equality is not a stricter test, it is a test of LibreOffice's serialiser.

- span/merge attributes (`table-table-cell-attlist-extra`): `table:number-columns-spanned`,
  `table:number-rows-spanned` (merged cell — only on the top-left cell; the covered cells
  underneath must each be written as `<table:covered-table-cell/>`, not omitted and not
  `table:table-cell`), and `table:number-matrix-columns-spanned` /
  `table:number-matrix-rows-spanned` for legacy array-formula ranges (modern dynamic-array
  spill uses `loext:spill`, §6, instead — the matrix-spanned attributes are the old
  ODF 1.2-style array formula, still valid and simpler if you don't need LO's
  spill-recalculation behavior).

Cell content: zero or more `text:p` (display paragraphs). **A bare `<table:table-cell/>`
with no attributes at all is a valid, correctly-typed empty cell** — this is the single most
common minimal-size element in a sheet.

An empty cell that only carries a style (formatted but no value) is
`<table:table-cell table:style-name="..."/>` — no value-type needed.

---

## 4. Formulas **[ODS]**

`table:formula` is a plain string attribute (schema type: unrestricted `string` — the `of:`
prefix is a convention, not a namespace-resolved requirement, but write it anyway since
every real consumer expects OpenFormula syntax when they see it):

```
table:formula="of:=SUM([.A1:.A10])"
```

Reference syntax (OpenFormula, ODF Part 4):
- `[.A1]` — relative reference on the current sheet.
- `[.$A$1]` — absolute column/row (Excel's `$A$1` equivalent).
- `[Sheet1.A1]` / `[Sheet1.A1:Sheet1.B2]` — cross-sheet reference/range.
- `[.A1:.A10]` — range on current sheet.
Function names are the OpenFormula canonical names (mostly identical to Excel's, e.g.
`SUM`, `IF`, `VLOOKUP`); vendor-specific functions LO recognizes but ODF doesn't standardize
are namespaced, e.g. `COM.MICROSOFT.UNIQUE(...)` (observed in
`sc/qa/unit/data/fods/DynamicArraySpill.fods`).

**Always write a cached result alongside the formula** (`office:value-type` +
`office:value`/`office:string-value`/etc., exactly as if it were a plain value cell, plus
`table:formula`). LO displays the cached value immediately and only recalculates lazily
(on edit, or if the document's calc settings force recalc-on-load); an omitted cached value
is schema-legal but shows blank/0 until the next recalculation.

Named ranges: `table:named-expressions` (document- or sheet-scoped) contains
`table:named-range table:name="Foo" table:cell-range-address="Sheet1.$A$1:.$A$10" table:base-cell-address="Sheet1.$A$1"`. Formulas reference it by bare name: `of:=SUM(Foo)`.

---

## 5. Styles and number formats **[ODS]**

### 5.1 Style elements

`style:style` (family-polymorphic — one element type, `style:family` attribute selects the
allowed property child):

| `style:family` | property child element | governs |
|---|---|---|
| `table` | `style:table-properties` | sheet-wide: `style:width`, `table:display` (sheet visible/hidden), `table:border-model` |
| `table-column` | `style:table-column-properties` | `style:column-width`, `style:use-optimal-column-width` |
| `table-row` | `style:table-row-properties` | `style:row-height`, `style:use-optimal-row-height` |
| `table-cell` | `style:table-cell-properties` (+ optionally `style:paragraph-properties`, `style:text-properties`) | background/border/padding/alignment + font |
| `paragraph` | `style:paragraph-properties`, `style:text-properties` | text-block-level formatting (rarely needed standalone in a cell) |
| `text` | `style:text-properties` | character-run formatting inside `text:p` |

`style:style` attributes: `style:name` (required, referenced via `table:style-name` /
`style:parent-style-name`), `style:parent-style-name` (inheritance — use this instead of
repeating attributes, it's both smaller and matches how LO's own style pool dedups),
`style:data-style-name` (on a `table-cell` style: points to a `number:*-style` for the
display format — this is the link between "how it looks" and "how the number renders"),
`style:master-page-name` (on a `table` style: page-setup/printing; omit for on-screen-only
documents).

Common `style:table-cell-properties` attributes actually present in the schema:
`style:vertical-align` (`top|middle|bottom|automatic`), `fo:wrap-option` (`wrap|no-wrap`),
`fo:background-color`, `fo:border` / `fo:border-{left,right,top,bottom}` (shorthand:
`"0.05pt solid #000000"`), `fo:padding` / `fo:padding-{left,right,top,bottom}`,
`style:shrink-to-fit`, `style:rotation-angle`, `style:cell-protect`.

Common `style:text-properties` attributes (observed in real LO fixtures, e.g.
`sc/qa/unit/data/fods/lookup_source.fods`): `fo:font-family`, `fo:font-size`,
`fo:font-weight` (`normal|bold`), `fo:font-style` (`normal|italic`), `fo:color` (`#rrggbb`),
`style:font-name` (references `style:font-face` in `office:font-face-decls` — optional,
`fo:font-family` alone is sufficient and simpler).

Conditional two-branch formatting (e.g. red negative currency) is done at the *number
style* level via `style:map`, not on the cell:
```xml
<number:currency-style style:name="N114">
 <number:text>-</number:text>
 <number:number number:decimal-places="2" number:min-decimal-places="2" number:min-integer-digits="1" number:grouping="true"/>
 <number:text> </number:text>
 <number:currency-symbol number:language="de" number:country="DE">€</number:currency-symbol>
 <style:map style:condition="value()&gt;=0" style:apply-style-name="N114P0"/>
</number:currency-style>
```
(`N114P0` is a second, simpler `number:currency-style` used for the non-negative case —
verified pattern from `sc/qa/unit/data/fods/lookup_source.fods`.)

Full formatting/conditional-formatting extensions beyond this (icon sets, data bars,
color scales) are LO/ODF-1.2-`calcext` territory — see §6 pointers if you need them; treat
as a later increment, not part of the minimal baseline.

### 5.2 Number formats

Root elements (all children of `office:styles` or `office:automatic-styles`):
`number:number-style`, `number:currency-style`, `number:percentage-style`,
`number:date-style`, `number:time-style`, `number:boolean-style`, `number:text-style`. Each
has `style:name` and is built from an ordered sequence of literal/format pieces:

- `number:number` — `number:decimal-places`, `number:min-decimal-places`,
  `number:min-integer-digits`, `number:grouping` (thousands separator, boolean).
- `number:text` — literal text/separator between pieces (also used as a leading `-` for
  negative-value sub-styles).
- `number:currency-symbol` — element content is the symbol, `number:language` /
  `number:country` pick the locale form.
- `number:day`, `number:month`, `number:year`, `number:day-of-week`, `number:era` —
  each takes `number:style="short|long"`; assemble with `number:text` literals between
  them, e.g. ISO date: `year(long) + "-" + month(long) + "-" + day(long)`.
- `number:hours`, `number:minutes`, `number:seconds` — analogous, for `number:time-style`.
- `number:boolean` — for `number:boolean-style` (renders as locale True/False text).

A `table-cell` style references its number format via `style:data-style-name="N2"`
(pointing at a `number:*-style style:name="N2"`) — the cell's `office:value-type` +
`office:value` is the *actual* numeric value; the number style only controls *display*.
Keep these consistent (a `percentage` value-typed cell should point at a
`number:percentage-style`, etc.) even though the schema doesn't cross-check it — LO will
still render inconsistent pairings, just confusingly.

### 5.3 Style pooling (minimality + correctness)

Define each distinct formatting/number-format combination exactly once as a named
automatic style, then reference it by name from every cell/row/column that needs it.
Do not inline per-cell property sets (there is no such construct — all formatting is via
`table:style-name` indirection) and do not emit duplicate `style:style` elements for
identical property sets. This mirrors LO's own internal item-pool/flyweight design (see
architecture doc) and is the single biggest lever for keeping large sheets small.

---

## 6. LibreOffice-only extensions **[ODS]**

These are outside the OASIS schema (not in `OpenDocument-v1.4-schema.rng`); only declare
their namespaces if you actually emit one of their attributes/elements. Omitting them never
breaks ODF validity — LO simply falls back to standard behavior/recomputation.

- `loext:spill="true"` on a `table:table-cell` carrying a dynamic-array formula — marks
  that the formula is expected to spill into following (currently-empty) cells; LO
  recomputes spill extent itself, so this is a hint, not load-bearing (observed in
  `sc/qa/unit/data/fods/DynamicArraySpill.fods`).
- `loext:*` attributes also appear on `style:text-properties` (e.g. `loext:opacity`) and in
  the package `META-INF/manifest.xml` for encryption metadata (`loext:keyinfo`) — irrelevant
  unless you're doing encrypted documents or the specific extended text property.
- `calcext:` namespace — pivot table (`office:spreadsheet` DataPilot) definitions and some
  conditional-format extensions beyond plain `style:map`. Out of scope for the minimal
  baseline; if/when needed, the authoritative grammar lives in LO's own
  `sc/source/filter/xml/xmlstyle.cxx` / `xmlexprt.cxx` (export side) and
  `sc/source/filter/xml/xmlimprt.cxx` (import side) — read those for the exact
  element/attribute names before implementing, don't guess.
- Sparklines (`sc/source/ui/sparklines`, in-cell mini-charts) also serialize through a
  LO-specific extension block; same guidance — inspect the exporter before implementing,
  treat as a post-baseline feature.

- `calcext:value-type="error"` on a `table:table-cell` — how LO marks a cell whose formula
  evaluated to an error. Part 4 §4.6 says an error result "shall be stored as if it was a
  string", in `office:string-value`; **LO stores the empty string there and puts the error
  name only in the display `text:p`**:

  ```xml
  <table:table-cell table:formula="of:=NA()" office:value-type="string"
                    office:string-value="" calcext:value-type="error">
   <text:p>#N/A</text:p>
  </table:table-cell>
  ```

  (observed in `sc/qa/unit/data/functions/fods/reference_operators.fods`, and in every
  fixture with a cached error). A reader that trusts `office:string-value` therefore loses
  *which* error it was, and cannot tell a failed formula from one that returned `""`. The
  rule that recovers it: when `calcext:value-type` is `error`, the display text is the
  value. Loop B depends on this — an error fixture is unverifiable without it.

Practical rule: **use the OASIS-standard element/attribute for anything ODF already covers
(values, formulas, borders, number formats, merges) — reach for `loext`/`calcext` only for
the handful of features ODF genuinely has no vocabulary for.**

---

## 7. Minimal templates

### 7.1 Absolute minimum flat ODS (one empty sheet, one empty cell)

Byte-for-byte pattern confirmed against `sc/qa/unit/data/fods/tdf144758-dbdata-no-orientation.fods`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" office:version="1.4" office:mimetype="application/vnd.oasis.opendocument.spreadsheet">
 <office:body>
  <office:spreadsheet>
   <table:table table:name="Sheet1">
    <table:table-column/>
    <table:table-row>
     <table:table-cell/>
    </table:table-row>
   </table:table>
  </office:spreadsheet>
 </office:body>
</office:document>
```

### 7.2 Flat ODS with data, one style, one number format, one formula

```xml
<?xml version="1.0" encoding="UTF-8"?>
<office:document
    xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
    xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0"
    xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0"
    xmlns:number="urn:oasis:names:tc:opendocument:xmlns:datastyle:1.0"
    xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0"
    xmlns:of="urn:oasis:names:tc:opendocument:xmlns:of:1.2"
    xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"
    office:version="1.4" office:mimetype="application/vnd.oasis.opendocument.spreadsheet">
 <office:automatic-styles>
  <number:number-style style:name="N2">
   <number:number number:decimal-places="2" number:min-decimal-places="2" number:min-integer-digits="1"/>
  </number:number-style>
  <style:style style:name="ce1" style:family="table-cell" style:data-style-name="N2">
   <style:table-cell-properties fo:background-color="#ffff00"/>
   <style:text-properties fo:font-weight="bold"/>
  </style:style>
 </office:automatic-styles>
 <office:body>
  <office:spreadsheet>
   <table:table table:name="Sheet1">
    <table:table-column table:number-columns-repeated="2"/>
    <table:table-row>
     <table:table-cell office:value-type="float" office:value="10"/>
     <table:table-cell office:value-type="float" office:value="20"/>
    </table:table-row>
    <table:table-row>
     <table:table-cell table:style-name="ce1" table:formula="of:=SUM([.A1:.B1])" office:value-type="float" office:value="30">
      <text:p>30.00</text:p>
     </table:table-cell>
    </table:table-row>
   </table:table>
  </office:spreadsheet>
 </office:body>
</office:document>
```

### 7.3 Package (.ods) layout for the same content

```
mimetype                    (STORED, first entry, raw bytes, no newline)
META-INF/manifest.xml
content.xml                 (root office:document-content; automatic-styles + body as above)
styles.xml                  (root office:document-styles; empty/minimal is fine — see §1.4)
```
`content.xml` differs from the flat form only in root element and omitting the `office:mimetype`
attribute (that lives only in the manifest for package form):
```xml
<office:document-content xmlns:office="..." xmlns:table="..." ... office:version="1.4">
 <office:automatic-styles> ... </office:automatic-styles>
 <office:body><office:spreadsheet> ... </office:spreadsheet></office:body>
</office:document-content>
```

---

## 8. Reading / import architecture **[GENERIC]**

LO does not parse ODF with a DOM + XPath-style approach. It uses a **streaming SAX "fast
parser"** driving a **stack of per-element context objects**, dispatched by an **integer
token** resolved from `(namespace-URI, local-name)` rather than by string/prefix matching.
This is the architecture to replicate — it is what makes LO's reader naturally tolerant of
noisy documents, almost as a side effect of the design rather than through bolted-on
special-casing. Translating it to Rust:

```rust
trait ElementContext {
    // Return Some(child) to handle a recognized child element; return None to make
    // the framework install a default no-op context that swallows that child's
    // whole subtree (text, attributes, further descendants) silently.
    fn start_child(&mut self, name: ResolvedName, attrs: &Attributes)
        -> Option<Box<dyn ElementContext>> { None }
    fn characters(&mut self, text: &str) {}
    fn end(&mut self, name: ResolvedName) {}
}
```
This mirrors `SvXMLImportContext` exactly: `createFastChildContext` /
`createUnknownChildContext` both default to returning `nullptr`
(`xmloff/source/core/xmlictxt.cxx:54-64`), and when a context's callback returns null,
`SvXMLImport::startFastElement`/`startUnknownElement` (`xmloff/source/core/xmlimp.cxx:1008-1023`,
`:1046-1067`) install a bare `SvXMLImportContext` — whose `characters`, `startFastElement`,
`startUnknownElement` and `endFastElement` are all empty no-ops
(`xmloff/source/core/xmlictxt.cxx:36-52`) — as the handler for that element and everything
under it. **Nothing has to specifically detect "junk"; anything not explicitly recognized
is structurally inert.** Build your reader the same way: one context type per element you
care about, a fallback "ignore" context for everything else, pushed/popped on a stack that
mirrors the XML nesting.

### 8.1 Name resolution: URI first, prefix is irrelevant

Namespace prefixes in the wild vary (`office:`, `ns0:`, no prefix + default xmlns, etc).
LO never compares prefix strings. Every namespace URI is registered once into a stable
integer key (`SvXMLNamespaceMap::Add`, `xmloff/source/core/namespacemap.cxx`); an
unrecognized URI still gets a key (with an `XML_NAMESPACE_UNKNOWN` flag set,
`namespacemap.cxx:93-96`) rather than failing. Element/attribute dispatch then matches on
`(namespace-key, local-name)` pairs against a static token table
(`xmloff/source/core/xmltoken.cxx`, `xmltkmap.cxx`), which is effectively a perfect hash —
O(1), and total: an unmatched pair simply yields "no token", which is the trigger for the
default-ignore behavior in §8 above. **Design your Rust reader's dispatch table the same
way: key on `(uri, local_name)`, never on the prefix string written in the document,** and
make "unrecognized URI" and "unrecognized local name under a recognized URI" both route to
the same harmless ignore-path rather than being distinguished error cases.

Also note namespace declarations are **not global** — they can be introduced or shadowed on
any element, not just the document root (`processNSAttributes` in `xmlimp.cxx:1000`, with
an explicit "rewind map" pushed/popped per element so a child's redeclaration doesn't leak
to its siblings, `PutRewindMap`/`xmlimp.cxx:1026-1027`). Keep a **stack** of namespace
scopes, not one flat map, exactly mirroring the element stack.

### 8.2 Two-tier strictness: normal parse, then a separate repair pass

LO's philosophy is *not* "make every check lenient." A handful of checks are deliberately
strict (see §9's last item). Instead, when something structural fails outright, LO retries
under an explicit, separate relaxed mode rather than quietly downgrading its normal checks
(`RepairPackage` storage property gating `IsODFVersionConsistent`,
`xmloff/source/core/xmlimp.cxx:1899-1908`; `ZipFile`'s constructor takes a `bForceRecovery`
flag that switches `readCEN()` for the byte-scanning `recover()`,
`package/source/zipapi/ZipFile.cxx:93-97`). Model your reader the same way: an initial
strict/fast parse, and a fallback recovery mode that only engages on outright structural
failure (bad zip, unparseable XML), not a single pass with leniency sprinkled everywhere.

---

## 9. Tolerant reading playbook (handling messy real-world documents) **[ODS]**

Concrete behaviors to copy, each independently verified against LO source. None of these
require detecting "is this document messy" up front — they're just how the normal reader
always behaves.

**Unknown elements, attributes, and whole namespaces (foreign vendor extensions, LO
features from a newer version than you support, editor-added metadata, etc.): ignore and
skip, don't fail the document.** This falls out of §8's default-context behavior for free.
Explicitly do *not* attempt to validate against the full RNG schema at load time — treat
the schema as a spec for what you *emit*, not a filter for what you *accept*.

**Never trust a repeat/span count enough to allocate off it directly.** `table:number-rows-repeated`
and `table:number-columns-repeated` are clamped to the sheet's own hard row/column limit
before use:
```
nRepeatedRows = std::max(it.toInt32(), 1);
nRepeatedRows = std::min(nRepeatedRows, pDoc->GetSheetLimits().GetMaxRowCount());
```
(`sc/source/filter/xml/xmlrowi.cxx:76-77`; the column equivalent is
`sc/source/filter/xml/xmlcoli.cxx:59`). A document claiming
`table:number-rows-repeated="4000000000"` must clamp to your own max sheet size (e.g.
1,048,576) before doing anything with the count — otherwise a hostile or simply
buggy/generated file is a trivial memory-exhaustion vector.

**Missing or unparseable value attributes degrade to a safe default, never abort the cell
or the document.** `office:value`/`office:date-value`/`office:time-value` parse into a
double that's pre-seeded with `NaN` (`sc/source/filter/xml/xmlcelli.cxx:122`); if nothing
parses into it, later code checks `std::isfinite(fValue)` and substitutes `0.0`
(`xmlcelli.cxx:1180-1181`) rather than propagating an error. `office:string-value` is
genuinely optional even where present in the grammar — when absent, the imported string
value falls back to the cell's own paragraph text (`GetFirstParagraph()`,
`xmlcelli.cxx:1580`), matching the ODF rule that display text *is* the value when no
explicit value is given. Implement value parsing as "try to extract a typed value; on any
failure, fall back to empty/0/derived-from-text" — never as "malformed attribute ⇒ reject
document."

**Recognize the same semantic attribute under more than one namespace when a real ecosystem
splits on it.** Cell value-type is accepted as both `office:value-type` *and*
`calcext:value-type` (`xmlcelli.cxx:196,200`, same handling code for both tokens) — LO
tolerates an alternate/legacy namespace for a concept without treating it as unknown. When
you encounter a document from a different, non-LO ODF producer that namespaces something
slightly differently than the OASIS-standard attribute, prefer resolving by known meaning
across a small set of accepted `(namespace, local-name)` aliases over rejecting it.

**Dangling/unresolvable references (a `table:style-name` or number-format name that doesn't
exist) are dropped silently, not treated as corruption.** Style application in
`ScXMLImport::SetStyleToRanges` looks the style up and only acts `if (pStyle)` is non-null
(`sc/source/filter/xml/xmlimprt.cxx:984-1005`) — a broken reference just means "no style
applied," full stop. Apply the same rule to every name-based cross-reference you support
(named ranges, data-style links, validation names): resolve-if-present, silently skip if
not, never hard-fail the containing element.

**Malformed or unrecognized formula text is preserved verbatim as an error token, not
rejected.** The formula compiler's tokenizer, on hitting text it cannot parse as a known
function/operator, emits an `ocBad` opcode carrying the original substring untouched
(`sc/source/core/tool/compiler.cxx`, e.g. `:4757`, `:4802`, `:4955`, `:5064`, comment at
`:5010`: *"ocBad to preserve input instead of #REF!"*), and unresolvable names set
`FormulaError::NoName` on that token (`compiler.cxx:4065,5127`) rather than aborting
compilation. The cell ends up showing a formula error (e.g. `#NAME?`), but the sheet
finishes loading and the *original formula text survives* for a later re-save. Implement
formula parsing so a single unparseable token degrades to "store raw text + mark this cell
as formula-error," scoped to that cell only.

**Corrupt or non-standard ZIP structure gets a second, brute-force pass instead of an
immediate failure.** The reader first trusts the End-Of-Central-Directory record
(`ZipFile::readCEN()`); if that fails to parse consistently, it falls back to
`ZipFile::recover()`, which linearly scans the raw file byte-by-byte for local-file-header
(`'P','K',3,4`) and data-descriptor (`'P','K',7,8`) signatures and reconstructs the entry
table from those (`package/source/zipapi/ZipFile.cxx:1827` on, loop at `:1844-1889`),
ignoring the (possibly wrong/truncated) central directory entirely. This handles files with
a missing/corrupt central directory, appended junk, or naive/streaming zip writers that
never finalized one properly. Implement the same two-step read: parse EOCD → central
directory normally; on any inconsistency, fall back to a linear local-header scan of the
whole byte stream rather than surfacing a hard error.

**Whitespace-only text between structural elements (pretty-printing indentation many
tools/humans add) must never be mistaken for cell content.** Only contexts that explicitly
override the text-callback (e.g. inside `text:p`) accumulate characters; every other
context's default is a no-op (`xmlictxt.cxx:66-68`) — so indentation whitespace sitting
between e.g. `</table:table-row>` and the next `<table:table-row>` is automatically
discarded rather than needing a special "skip whitespace" pass.

**The one place LO is deliberately strict, not lenient — don't accidentally loosen it.**
For ODF ≥ 1.2, the document version recorded in `META-INF/manifest.xml`'s root
`manifest:file-entry` and the `office:version` attribute on `content.xml`'s root element
must match exactly; a mismatch throws a hard `SAXException` wrapping a `ZipIOException`
during parsing (`xmloff/source/core/xmlimp.cxx:986-993`, gated by
`IsODFVersionConsistent`, `:1880-1917`), *unless* the storage is explicitly opened in
repair mode. If your reader needs to tolerate this particular mismatch too, do it the way
LO does — as an explicit, opt-in "repair" retry (§8.2) — not by silently ignoring version
metadata everywhere, since version does matter for correctly interpreting some
grammar/behavior differences across ODF revisions.

---

## 10. Forward-looking: other document types **[GENERIC]**

Section 1 (packaging: mimetype/manifest/flat-root rules) is format-agnostic — reuse it
unchanged. Only §§2–7 (content model, cell/paragraph model, styles) are ODS-specific.
When text/presentation support is added, the same skeleton applies with:

| Type | mimetype | body element | root element names |
|---|---|---|---|
| Spreadsheet (this doc) | `application/vnd.oasis.opendocument.spreadsheet` | `office:spreadsheet` | `office:document{,-content,-styles,-meta,-settings}` |
| Text | `application/vnd.oasis.opendocument.text` | `office:text` | same pattern |
| Presentation | `application/vnd.oasis.opendocument.presentation` | `office:presentation` | same pattern |

Confirm each new type's exact mimetype string the same way this doc did (grep
`filter/source/config/fragments/types/*.xcu` for `MediaType`) rather than assuming — don't
copy these three from memory into code without re-verifying against that source of truth
at implementation time, in case of future spec revisions.
