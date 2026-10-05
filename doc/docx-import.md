<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# The docx import filter — done (DX0–DX6)

Reading `.docx` — the word processor's half of what `doc/xlsx-import.md` did for the
spreadsheet, for the same reason and under the same rules. Normative for `docx/` (crate
`grind-docx`) and `ooxml/` (crate `grind-ooxml`); `doc/docx-format.md` holds the facts.

**The decision it records.** People keep sending Word documents. A word processor that cannot
open one is a word processor you cannot start using, and converting them today means driving
LibreOffice headless. So reading `.docx` is scheduled, the way reading `.xlsx` was.

**Writing `.docx` stays in `doc/not-doing.md` §1, unchanged.** Reading is a one-way
translation at the edge that produces an ODF document. One way in, never out.

---

## Decisions

Every decision `doc/xlsx-import.md` took up front holds here unchanged, and is not repeated:
the filter runs once and produces ODF; our own reader rather than a library; its own crate
behind a cargo feature (`docx`, on by default); Transitional is the target and Strict a
namespace table away; the fidelity report is part of the output and `--strict` turns any loss
into a non-zero exit; the CLI's read commands stay ODF-only (`grind text import`, never `grind
text view letter.docx`); ECMA-376 is the source and LibreOffice the oracle; nothing user-facing
names LibreOffice. Three decisions are this filter's own.

### 1. A shared container layer: `grind-ooxml`

The spreadsheet filter had built OPC (the zip, relationships, part names, the zip-bomb caps),
markup compatibility and the tolerant walker, and none of it was about spreadsheets. They were
**hoisted** into `ooxml/` rather than copied — `package.rs`, `mce.rs`, `xml.rs` and `names.rs`,
history and tests with them — and `grind-xlsx` re-exports them under the paths it always used.
What stays in each filter is its own vocabulary: `name.is("c")` in one (`grind_xlsx::xml`),
`name.w("p")` in the other (`grind_docx::xml`). The namespace table holds both main
namespaces; that is the OOXML family's own table, not R8 bending, since `grind-ooxml` is not
the core.

One error type serves both (`grind_ooxml::Error`), with a `NotSpreadsheet` and a
`NotWordDocument` — because a workbook's main part and a document's are found by the **same**
relationship type, and "this is a spreadsheet, not a document" is the one question both filters
have to ask the other's files (`doc/docx-format.md` §1.1).

### 2. Why ODF and not the model

`grind-xlsx` builds a `grind_sheet::Document` through the model's API and lets the sheet's
writer produce ODF, because nearly everything a workbook says has a field in that model. A
word-processing document is the other way round. Its look lives in **named styles, automatic
paragraph styles, list styles, a page layout and a master page with a header** — and
`grind_text` *reads* every one of those (for showing and printing: `paragraph.rs`,
`marginal.rs`, `table_look.rs`, `Document::page`) and *writes* none of them, because a save
carries them out of the file the document came from untouched (R6, the envelope). A document
built through the model would have nowhere to put them, and its first save would write a
document with no styles at all.

So the filter writes **the file a save carries them out of**: one flat ODF text document
(`docx/src/emit.rs`), which `grind_text::read_bytes` then reads like any other. From that
point an imported document is indistinguishable from one LibreOffice wrote — the same reader,
the same saves, the same never-worse guards — and every property the text model has no field
for (a superscript, a letter spacing, a paragraph border, a list's numbering) still arrives in
the file a save writes and in what LibreOffice shows of it. The text model gains nothing for
this filter and owes it nothing (R1).

`grind_docx::open` therefore returns ODF bytes for `App::open_bytes`, exactly the shape of
`grind_xlsx::open`, and `import_bytes` returns that document read back.

### 3. An intermediate form, because some facts arrive late

Between the XML and the ODF is a small intermediate form (`docx/src/body.rs`), for two reasons
that are both about Word: some facts arrive after the content they change (a section's page is
known when it closes; a hidden bookmark is kept only if a hyperlink later points at it; a page
break splits the paragraph around it), and Word's fields are a state machine that crosses runs
and paragraphs. Each late fact is a pass over the intermediate form rather than a second pass
over XML.

---

## The translation

| Word | ODF | |
|---|---|---|
| `w:p` | `text:p`, or `text:h` with `text:outline-level` when an outline level is stated on it or its style chain (`doc/docx-format.md` §3.2) | |
| `w:pStyle`, `w:rStyle` | a named `style:style` of family `paragraph` / `text`, `w:basedOn` as its parent, `w:name` capitalised as its display name, encoded `Heading 1` → `Heading_20_1` as LibreOffice spells one | `styles.rs` |
| `w:docDefaults` | `style:default-style` | §3.3 |
| `w:pPr`, `w:rPr` (direct) | an automatic style under the named one, pooled | `props.rs` |
| `w:numPr` (direct or from the style) | a `text:list` of the right depth, with a `text:list-style` per `numId` — number format, prefix and suffix, levels shown, start value, indent, the bullet mapped out of Symbol/Wingdings (§4.3) — and `text:continue-list` across an interruption | `numbering.rs` |
| `w:tab`, `w:br`, `w:cr`, spaces | `text:tab`, `text:line-break`, `text:s` | |
| `w:br w:type="page"` | the paragraph split, the rest `fo:break-before="page"` (§2.4) | |
| `w:hyperlink` (`r:id`, `w:anchor`), `HYPERLINK` | `text:a` | never fetched |
| `w:bookmarkStart` | `text:bookmark` — Word's hidden `_` names only when a link points at one | |
| `PAGE`, `NUMPAGES` | `text:page-number`, `text:page-count` | |
| any other field | its cached result, as text | counted |
| `w:footnoteReference`, `w:endnoteReference` | `text:note`, numbered as Word numbers them, a custom mark as its label (§2.5) | |
| `w:tbl` | `table:table` with its grid's widths, `gridSpan`/`vMerge` as spans and covered cells, header rows, row heights, cell fill, borders (cell over table over table style), vertical alignment and margins | `emit.rs` |
| `w:drawing` / `w:pict` holding a picture | `draw:frame`/`draw:image` with the bytes inline, at the extent's size, `as-char` when inline | |
| the first section's `w:sectPr` | the page layout and the `Standard` master page, with the default header and footer (and the first-page pair under `w:titlePg`) (§5.2) | |
| a later section | `fo:break-before="page"` on its first block, when it starts a page | |
| `w:ins`, `w:moveTo`, `w:sdt`, `w:smartTag`, `w:customXml` | read through | |
| `w:del`, `w:moveFrom` | skipped | counted |

### What it costs

Every loss is a `Dropped` kind (`docx/src/report.rs`), counted, summed in `Report::summary` and
fatal under `--strict`: comments, tracked changes (accepted), text boxes, drawings that are not
pictures, equations, embedded objects, linked pictures, a floating picture's position, fields
and form fields and content controls (kept as their text), sections whose page differs,
multi-column sections, first-page or even-page header variants, heading numbers, table-style
banding, positioned paragraphs and tables (kept in the flow), macros (never executed), and the
remainder of a part whose XML is damaged.

Two costs are the **text model's**, not this filter's, and are named rather than worked around:

- **An edit inside a paragraph holding a footnote or a page field is refused a save**
  (`Error::WouldLose`). The model has no run for either, so a regenerated paragraph would drop
  it; this is true of every ODF document, not just an imported one.
- **A table inside a table cell** is read as the outer cell's paragraphs, so an edit that
  regenerates the outer table is refused rather than allowed to flatten the inner one.

Building this filter found and fixed two bugs in the text model's saves that every ODF
document had: a structural edit anywhere wrote every table without its styles (and was then
refused), and Backspace at a table cell's edge corrupted the table. `text/tests/table.rs` holds
both.

---

## How it is checked

| Check | Asserts | Where |
|---|---|---|
| **Fixtures** | packages assembled from XML a reviewer can read: each construct above, what it becomes in the document `grind_text` reads back | `docx/tests/fixtures.rs` |
| **Loop A″** — import tolerance | every `.docx`/`.docm`/`.dotx` in `sw/qa` imports without an error or a panic, reads back, and saves in both forms (`FLOOR`) | `docx/tests/corpus_read.rs` |
| **Loop D″** — import fidelity | our import against the oracle's conversion, block by block (kind, cell, text), over the vendored documents (all must agree) and Word's own (`sw/qa/extras/ooxmlimport/data`, `FLOOR`), with named divergences | `docx/tests/loop_d.rs` |
| **Editing** | an import opened as a shell opens it takes typing, Enter, Backspace and bold, and saves in both forms | `docx/tests/editing.rs` |

Scoreboard on 2026-10-06, LibreOffice 26.8: loop A″ 2159 of 2167 documents import (4
password-protected, 4 ODF documents named `.docx`); loop D″ 117 of 161 Word-written documents
agree block for block, 31 more differ only by a named divergence (a field's last result, a
floating table or paragraph, a picture's anchor) and 13 disagree — a table written inside a
paragraph, page breaks inside a table cell, Hebrew list numbers computed by fields, a
bibliography — each a construct rather than a crash.

---

## Milestones

| | | |
|---|---|---|
| DX0 | The seam: `grind-ooxml` hoisted, the crate, `sniff`, `Error`, `Report`, `grind text import` | done |
| DX1 | Paragraphs, headings, runs, whitespace, breaks, bookmarks, hyperlinks, fields | done |
| DX2 | Character and paragraph formatting, named styles and their inheritance, defaults | done |
| DX3 | Lists from `numbering.xml` | done |
| DX4 | Tables: grid, spans, header rows, borders, fills, margins, table styles | done |
| DX5 | The page: sections, header and footer, footnotes and endnotes, pictures | done |
| DX6 | Every shell opens a `.docx` as a new, unsaved document under its ODF name, with the report's one sentence: `grind-tui`, `grind-text-gtk` (and its `.desktop` file), `grind-web`, `grind-win32` (and its associations) and `grind-mac` (and its Info.plist) | done |
