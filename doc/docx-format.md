<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# WordprocessingML, as measured

Clean-room notes on `.docx` for the Word import filter (`docx/`, crate `grind-docx`) — the
`doc/xlsx-format.md` of that filter, under the same rule: **every fact carries `MEASURED`,
`SPEC` or `UNVERIFIED`, and an `UNVERIFIED` fact may not be implemented.** A measurement names
the file and the command; a `SPEC` fact names the ECMA-376 section. LibreOffice is the oracle
(`soffice --headless --convert-to fodt`) and is never a source: nothing below was learned by
reading its code.

`doc/docx-import.md` is the plan these facts serve.

---

## 1. The package

### 1.1 Transitional — `MEASURED`

LibreOffice's own `.docx` (from `odtgen`'s `01-text-and-formatting.odt`, 2026-10-05) and every
Word-written file in `sw/qa/extras/ooxmlimport/data`:

| | URI |
|---|---|
| `w:` | `http://schemas.openxmlformats.org/wordprocessingml/2006/main` |
| `r:` | `http://schemas.openxmlformats.org/officeDocument/2006/relationships` |
| `wp:` | `http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing` |
| `a:` | `http://schemas.openxmlformats.org/drawingml/2006/main` |
| `pic:` | `http://schemas.openxmlformats.org/drawingml/2006/picture` |

The main part is found from `_rels/.rels` by the **same** relationship type a workbook's is
(`…/relationships/officeDocument`), so the type cannot tell the two apart: the main part's root
element does (`w:document` against `workbook`). `grind_xlsx::sniff` asks the same question
since this filter landed.

LibreOffice's minimal output declares fourteen namespaces on `w:document` and uses five.

### 1.2 Strict — `MEASURED`

From `sw/qa/extras/ooxmlexport/data/strict.docx` and the five other Strict files in that tree
(found by grepping `[Content_Types].xml` and `_rels/.rels` for `purl`), 2026-10-05:

| | URI |
|---|---|
| `w:` | `http://purl.oclc.org/ooxml/wordprocessingml/main` |
| `r:` | `http://purl.oclc.org/ooxml/officeDocument/relationships` |
| `wp:` | `http://purl.oclc.org/ooxml/drawingml/wordprocessingDrawing` |
| `a:` | `http://purl.oclc.org/ooxml/drawingml/main` |
| `pic:` | `http://purl.oclc.org/ooxml/drawingml/picture` |

Relationship types take the `…/ooxml/officeDocument/relationships/` prefix, as in a Strict
workbook; the package relationships and content types do not vary. `strict.docx`'s own
`_rels/.rels` mixes a Transitional core-properties type with Strict ones. The root carries
`w:conformance="strict"`. The DrawingML Strict URI is the first one this project measured —
no workbook it holds had a Strict theme — and `grind_ooxml::names` now recognises it for both
filters.

### 1.3 The parts — `MEASURED`

Relationship types from the main part in the first 400 Word documents of `sw/qa`, by count:
`styles` 395, `fontTable` 394, `settings` 393, `theme` 354, `webSettings` 323, `header` 211,
`footer` 199, `customXml` 162, `footnotes` 130, `numbering` 127, `endnotes` 127, `image` 82,
`hyperlink` 77, `chart` 21, `oleObject` 8, `comments` 6. Comments share their relationship
type with a workbook's.

### 1.4 Attributes are namespaced — `MEASURED`

Unlike SpreadsheetML, whose attributes are unprefixed (`<c r="A1" t="s">`), almost every
WordprocessingML attribute is in the `w:` namespace: `<w:sz w:val="24"/>`, `<w:pStyle
w:val="Heading1"/>`. DrawingML's are not (`<wp:extent cx="…" cy="…"/>`, `<a:latin
typeface="…"/>`). `docx/src/xml.rs` is the reader for each.

### 1.5 A part that stops being XML — `MEASURED`

`sw/qa/extras/ooxmlimport/data/math-malformed_xml.docx` closes `<m:t>` with `</m:sPre>`.
The oracle opens it. This filter keeps every block read before the damage and counts the rest
(`Dropped::Damaged`).

### 1.6 Files that are not Word documents — `MEASURED`

Four `.docx` in `sw/qa` are ODF text packages wearing the extension (`file` says
"OpenDocument Text": `odfimport/data/tdf76322_columnBreakInHeader.docx`,
`ooxmlexport/data/tdf171025_pageAfter.docx`, `tdf171038_pageAfter.docx`,
`uiwriter/data/tdf132596.docx`). Four are password-protected (CFB). Loop A″ counts both.

---

## 2. Text

### 2.1 `ST_OnOff` — `SPEC`

§17.17.4: `<w:b/>` is on; `w:val` of `0`, `false` or `off` is off; `1`, `true`, `on` are on.

### 2.2 Whitespace in `w:t` — `MEASURED`

`ooxmlimport/data/xml_space.docx` and `tdf108806.docx`, against the oracle, 2026-10-05:

- With `xml:space="preserve"`, tabs and spaces are kept as written, and a line ending in the
  character data (`tdf108806.docx` has a literal CRLF) is a **space**, not a line break — a
  break is `w:br`.
- Without it, every run of whitespace, tabs included, is one space. A `w:t` holding a single
  space between two words (`tdf124670.docx` writes every character in its own run) is kept,
  so the collapsing does not trim; the oracle drops a space left at the start of a line, which
  this filter does not imitate.

### 2.3 Fields — `MEASURED`

- A complex field (`w:fldChar begin` … `separate` … `end`) shows the runs between `separate`
  and `end`; the runs before `separate` are its instruction (`w:instrText`).
- **A paragraph mark inside an instruction is part of the instruction.**
  `ooxmlimport/data/tdf125038b.docx` spreads one `IF` instruction over two paragraphs; the
  oracle shows its result as one paragraph. This filter joins what such a paragraph shows to
  the next.
- `ASK` sets a bookmark and shows nothing: `n751017.docx`'s `ASK foo` has cached result `bar`
  inside the bookmark `foo`, and the oracle shows only the `REF` to it. `SET` is the same kind
  of field (§17.16.5.56, `SPEC`).
- `PAGE`, `NUMPAGES` and `SECTIONPAGES` become ODF's `text:page-number` / `text:page-count`.
  `HYPERLINK "url"` and `HYPERLINK \l "anchor"` become a link around the result. Every other
  field keeps its cached result as text, counted (`Dropped::Field`).

### 2.4 Page breaks — `SPEC`

`w:br w:type="page"` (§17.3.3.1) ends the page wherever it stands in a paragraph. ODF has no
break element inside a paragraph — a page starts at a paragraph, `fo:break-before="page"` — so
this filter splits the paragraph at the break.

### 2.5 Footnote marks — `MEASURED`

`ooxmlimport/data/tdf129912.docx`: a reference with `w:customMarkFollows="1"` is followed by
the mark itself — a `w:sym` there — and takes **no number**: the next ordinary footnote is 1.
The custom mark becomes the citation (`text:note-citation text:label`).

### 2.6 A document ends in a paragraph — `MEASURED`

The oracle, reading this filter's ODF of `ooxmlimport/data/table_width.docx` (whose body ends in
a table) and of `math-malformed_xml.docx` (whose body, after the damage, holds nothing), adds an
empty paragraph at the end of each; Word, for its part, never ends a document in a table. This
filter writes that paragraph itself, so the document the oracle reads back is the one written.

### 2.7 Comments — `MEASURED`

`ooxmlexport/data/CommentDone.docx`: a comment is `w:commentRangeStart`/`w:commentRangeEnd` around
the text and a `w:commentReference` after it, with the comment itself — author, date, paragraphs
— in `word/comments.xml`. This filter writes an `office:annotation` named after the comment at
the start and an `office:annotation-end` at the end; the oracle, saving our ODF of the file back
to Word, writes the same comments with the same paragraphs. An annotation holds only paragraphs
and lists (rng:7787), so a table in a comment gives up its grid and keeps its text.

---

## 3. Paragraphs and styles

### 3.1 `start` and `end` — `MEASURED`

LibreOffice writes the second edition's `w:ind w:start="…" w:end="…"` and `w:jc w:val="end"`
(seen in its own `.docx`, §1.1's file), where Word writes `w:left`/`w:right`/`right`. Both are
read; `start`/`end` are taken as left/right since bidirectional layout is out of scope
(`doc/text-layout.md`).

### 3.2 Headings — `MEASURED`

A paragraph is a heading when `w:outlineLvl` (0–8; 9 is body text, §17.3.1.20 `SPEC`) is stated
on it or anywhere up its style's `w:basedOn` chain. A style *named* `heading 1` that states no
level is **not** one: `ooxmlimport/data/tdf104167.docx`'s style `12` is called `heading 1`, has
no `w:outlineLvl`, and the oracle reads its paragraph as a paragraph. Word's own saves write the
level into every heading style (seen in every Word document in the corpus with headings that
this filter reads as headings), so the name is never needed.

### 3.3 The defaults when there are none — `MEASURED`

A document with no styles part at all, converted by the oracle (a hand-made one-table
document, 2026-10-06): the default paragraph style is 11-point Calibri. This filter states the
same, since ODF leaves an unstated default to the reader (and LibreOffice's ODF default is 12).
Word's own behaviour with no styles part was not measured.

### 3.4 Tab stops — `SPEC`

§17.3.1.37: a tab stop's `w:pos` is measured from the page's text margin. ODF's
`style:tab-stop/@style:position` is measured from the paragraph's left indent (ODF 1.4 Part 3
§17.8). This filter subtracts the paragraph's resolved left indent, and drops a stop that would
fall before it.

---

## 4. Numbering

### 4.1 The indirection — `SPEC`

§17.9: a paragraph names a `w:num` by `w:numId`; the `w:num` names a `w:abstractNum`, may
replace any of its levels (`w:lvlOverride/w:lvl`) and restart any count
(`w:lvlOverride/w:startOverride`). `w:numId="0"` is "no numbering", stated (§17.9.18).

### 4.2 Two `numId`s, one abstract — `UNVERIFIED`

Whether Word counts on across two `w:num`s that share an abstract definition, absent an
override. Not implemented: each `numId` is its own list, continued across interruptions with
`text:continue-list`.

### 4.3 Bullets in symbol fonts — `MEASURED`

`numbering.xml` in LibreOffice's own `.docx` and in Word's: a bullet level's `w:lvlText` is very
often a private-use code point in *Symbol* (`U+F0B7`, the round bullet) or *Wingdings*
(`U+F0A7`, the square), drawn in that font. In any other font it is nothing. This filter maps
the handful seen (`U+F0B7`, `U+F06C` → •; `o` → ◦; `U+F0A7`, `U+F06E`, `U+F0A8` → ▪; `U+F0D8` →
➢; `U+F0FC` → ✓; `U+F076` → ❖; `U+F0A1` → ○) and any other private-use bullet to •.

### 4.4 A style's numbering resolves in halves — `MEASURED`

`sw/qa/extras/uiwriter/data/tdf76817.docx`: the `Heading2` style states `w:numPr` with only
`w:ilvl="1"`; its `w:numId` comes from `Heading1`, the style it is based on. The oracle numbers
its paragraphs `1.1` and `2.1`. So `w:numId` and `w:ilvl` are each taken from the nearest style
that states *it*, not from the nearest that states either. A level stated nowhere is the one
whose `w:lvl/w:pStyle` names the paragraph's style (§17.9.24, `SPEC`), else the first.

### 4.5 Numbered headings — `MEASURED`

The same file, converted by the oracle to text: `1 Should be 1`, `1.1 Should be 1.1`. This
filter writes the list that numbers the headings — the one most of them are numbered by, every
one of them at the list level of its outline level — as ODF's `text:outline-style`, and the
oracle's text of *our* ODF reads the same. A heading at a numbered level that Word left
unnumbered is written `text:is-list-header="true"`.

---

## 5. Sections and the page

### 5.1 The page when none is stated — `MEASURED`

The same styleless document as §3.3: the oracle sets it on US Letter with one-inch margins.

### 5.2 Header distance — `MEASURED`

`ooxmlimport/data/text-copy.docx` (`w:top="2574" w:header="1985"`), against the oracle: the ODF
page's `fo:margin-top` is `w:header` (1.3783 in), and the header's `fo:min-height` is `w:top −
w:header` (0.4091 in). The oracle adds spacing of its own below the header with
`style:dynamic-spacing`; this filter writes none.

### 5.3 The paragraph that closes a section — `MEASURED`

`ooxmlimport/data/unbalanced-columns-compat.docx`: an empty paragraph holding nothing but a
`w:sectPr`, followed by a `continuous` section, is not a paragraph in the oracle's document. Two
counter-examples bound the rule: `text-copy.docx` and `tdf103931.docx`, where such a paragraph is
the *whole* of its section, keep it. This filter drops it only between a section with other
blocks and one continuing on the same page.

### 5.4 Pages per section, and their marginals — `MEASURED`

`sw/qa/extras/uiwriter/data/tdf168157.docx` has an A4 section and a Letter one. The oracle's PDF
of the document and its PDF of *this filter's* ODF of it have the same page sizes, page by page:
each distinct section page is a master page, started by its section's first paragraph.

A hand-made document with a default and an even-page footer and `w:evenAndOddHeaders` (2026-10-06):
the oracle prints `ODD`, `EVEN`, `ODD` on three pages, from the `.docx` and from our ODF alike —
the even variant is ODF's `style:footer-left`. Without the setting Word shows no even variant
(§17.10.1, `SPEC`), and neither does a first-page variant without `w:titlePg`, so neither is a
loss. A section with no reference of a kind shows the previous section's (§17.10.5, `SPEC`).

The schema allows a left or first-page variant only after the default one (rng:12140); a
document with only a first-page header gets a default `style:header style:display="false"`.

---

## 6. Tables

### 6.1 Merges — `SPEC`

`w:gridSpan` (§17.4.17) spans grid columns; `w:vMerge w:val="restart"` (§17.4.85) starts a
vertical merge and a `w:vMerge` with no value continues the one above. A row may skip grid
columns at either end (`w:gridBefore`, `w:gridAfter`).

### 6.2 Cell margins when none are stated — `MEASURED`

§3.3's document: the oracle pads a cell 0.075 in (108 twips) left and right, and nothing above
and below.

### 6.3 Floating tables — `MEASURED`

A table with `w:tblpPr` is converted by the oracle into a table inside a frame; thirteen of
`ooxmlimport/data`'s documents have one. This filter keeps it as a table in the flow and counts
the position (`Dropped::Frame`).
