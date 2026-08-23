<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Clean-Room ODF Text (ODT) Notes

The `doc/ods-format.md` of the word processor. Same job, same rules, and deliberately much
shorter — because §§1, 8, 8.1 and 10 of that document are marked `[GENERIC]`, are already
extracted into `grind-core`, and apply here **unchanged**. Nothing about packaging, namespace
resolution or the element-context stack is restated below; read it there.

What is here is the part that differs: the `office:text` content model, the style families a
text document uses, and the observations about real files that the schema does not carry.

**The rules this document is written under** (`CONTRIBUTING.md`):

1. Facts come from the OASIS specifications first. Every claim below that is *structural*
   cites `doc/OpenDocument-v1.4-schema.rng` by line number, so it can be re-checked against
   the file in this repository rather than believed.
2. LibreOffice may be **read** and **cited by `file:line`**, never copied. A fact learned that
   way goes here before it reaches code.
3. **A claim with no citation is marked `UNVERIFIED` and may not be implemented.** That marker
   is the whole value of this document — it separates what is known from what is assumed, and
   §5 below is currently all assumption.

Line numbers are for the vendored `OpenDocument-v1.4-schema.rng` at the revision in this
repository.

---

## 1. Container — unchanged, and the one string that differs

`doc/ods-format.md` §1 in full: the package form (`mimetype` first, stored, byte-exact), the
flat form, `META-INF/manifest.xml`, minimality. `grind_core::odf::package` already implements
it and does not care what is inside.

The only difference is the media type:

| | Spreadsheet | Text |
|---|---|---|
| media type | `application/vnd.oasis.opendocument.spreadsheet` | `application/vnd.oasis.opendocument.text` |
| body element | `office:spreadsheet` (rng:7711) | `office:text` (rng:7693) |
| package extension | `.ods` | `.odt` |
| flat extension | `.fods` | `.fodt` |

`grind_core::kind` reads exactly this, from the `mimetype` entry or the flat root's
`office:mimetype`, and both strings are already in `core/src/kind.rs`.

**The `-flat-xml` suffixed media types are not written into files.** `doc/ods-format.md` §1.2
established this for spreadsheets — the suffixed spelling is LibreOffice's internal
type-detection label. The same is assumed for text and is **UNVERIFIED**; confirm against
`filter/source/config/fragments/types/writer_ODT_FlatXML.xcu` before relying on it for
anything but a `.desktop` file's `MimeType=` line, where the suffixed forms *are* correct.

---

## 2. The body content model

`office:text` is `office-text-content-prelude`, `-main`, `-epilogue` (rng:7693-7698). The
prelude and epilogue carry declarations (variables, sequences, user fields) and are outside
the scope line; **`office-text-content-main` is the document** (rng:8352).

It is `zeroOrMore text-content`, and `text-content` is a flat choice of sixteen (rng:16938):

```
text:h · text:p · text:list · text:numbered-paragraph · table:table · text:section
text:soft-page-break · text:table-of-content · text:illustration-index · text:table-index
text:object-index · text:user-index · text:alphabetical-index · text:bibliography
shape · change-marks
```

**That list is the whole decision this phase has to make**, and it is why `doc/text-core.md`
exists rather than being folded in here: this document records what ODF *says*, that one
records what this build *does*. Six of the sixteen are index machinery, one is change
tracking, one is a drawing canvas.

Two structural facts worth stating because they shape the model:

**A text document's body is a flat sequence, not a tree.** Headings do not contain the
paragraphs beneath them — outline structure is implied by `text:outline-level` on each
`text:h` and nothing else. So "move section 3.2" is an operation over a *range* of the flat
sequence, computed from outline levels, rather than a subtree move. That is the single
biggest structural difference from what a person expects a word processor's model to be, and
it is what makes `grind_text::loc`'s `§2.1.3` addressing a derived view rather than a
container path.

**Lists nest through their items, not through themselves.** `text:list` holds
`text:list-item`s (rng:17494); a `text:list-item` holds `text-list-item-content`, which is
where a nested `text:list` appears. So depth is structural and the model has to carry it.

---

## 3. Paragraphs, headings, spans

### 3.1 `text:p` (rng:17950)

```
<rng:element name="text:p">
  <rng:ref name="paragraph-attrs"/>
  <rng:zeroOrMore><rng:ref name="paragraph-content-or-hyperlink"/></rng:zeroOrMore>
</rng:element>
```

`paragraph-attrs` (rng:8373) is `text:style-name` (optional), `text:class-names`,
`text:cond-style-name`, `xml:id` / `text:id`, and in-content metadata. **Only
`text:style-name` is in scope**; the rest are read and preserved by R6's splice, which is the
whole point of R6.

### 3.2 `text:h` (rng:17095)

```
<rng:element name="text:h">
  <rng:ref name="heading-attrs"/>
  <rng:ref name="paragraph-attrs"/>
  <rng:optional><rng:ref name="text-number"/></rng:optional>
  <rng:zeroOrMore><rng:ref name="paragraph-content-or-hyperlink"/></rng:zeroOrMore>
</rng:element>
```

A heading is **a paragraph plus `heading-attrs`** — same content model, same style attribute.
`heading-attrs` (rng:6867) is:

| Attribute | Required | Type |
|---|---|---|
| `text:outline-level` | **yes** | `positiveInteger` |
| `text:restart-numbering` | no | `boolean` |
| `text:start-value` | no | `nonNegativeInteger` |
| `text:is-list-header` | no | `boolean` |

**`text:outline-level` is required and is a `positiveInteger` with no upper bound in the
schema.** The familiar 1–6 limit is a convention, not a constraint. Tolerance therefore means
accepting level 9 and level 400 on the way in; the scope line caps only what this build can
*author* (`doc/text-core.md`).

`text-number` is a rendered copy of the heading's number, for readers that do not compute
numbering. Read and preserved, never authored — authoring one means owning list numbering.

### 3.3 `paragraph-content` (rng:8405)

The inline model, which is a flat choice including bare `<rng:text/>`:

| | Element | Notes |
|---|---|---|
| text | `<rng:text/>` | Character data |
| spaces | `text:s` | Optional `text:c` (`nonNegativeInteger`) — a **run count**, absent meaning one |
| tab | `text:tab` | `text-tab-attr` |
| break | `text:line-break` | `rng:empty` |
| page break | `text:soft-page-break` | A *rendered* artifact of a previous layout |
| span | `text:span` | `text:style-name`, `text:class-names`, then nested `paragraph-content-or-hyperlink` |
| metadata | `text:meta` | And a long tail of fields, bookmarks, notes, references |

Three consequences:

**`text:s` is ODF's run-length encoding of spaces, and it is a correctness trap in the same
family as `table:number-columns-repeated`.** A single leading space is `<text:s/>`;
`<text:s text:c="4"/>` is four. The reader must expand it and the writer must re-encode it,
because XML collapses whitespace and a document that round-trips through naive text loses its
indentation. `Attrs::count` in `grind_core::odf::context` already exists for exactly this
shape of attribute and applies unchanged.

**`text:span` nests.** Its content is `paragraph-content-or-hyperlink`, so a span inside a
span inside a hyperlink is legal and appears in real files. A model of flat runs must
therefore *flatten on read* — composing the style stack down each branch — or carry the tree.
Flattening is lossy for the style names but not for the rendering, and which one this build
does is a `doc/text-core.md` decision, not a schema one.

**`text:a` (rng:16453) is inline and contains `paragraph-content`** — note: *not*
`paragraph-content-or-hyperlink`, so a hyperlink cannot nest inside a hyperlink. Its
attributes are `text-a-attlist` (rng:16463), where `xlink:href` carries the target.

### 3.4 `text:bookmark` (rng:16801)

```
<rng:element name="text:bookmark">
  <rng:ref name="text-bookmark-attlist"/>
  <rng:empty/>
</rng:element>
```

`text:name` is required (`string`), `xml:id` optional. Empty element, positioned inline.
`text:bookmark-start` / `-end` are the range form.

**This is the named-range analogue**, and the reason `grind_text::loc` can offer `#intro` as
an address that survives editing: the anchor moves with the text because it *is* in the text.
Nothing has to be kept in step, which is the same argument `core/src/filter.rs` makes for not
storing which rows a filter hides.

---

## 4. Styles

The families differ; the properties do not. `grind_core::style` already holds the pieces —
`fo:` values kept verbatim, ODF lengths, three-part borders — because they are the same
XSL-FO vocabulary a cell style is made of. That was the S1 bet and this section is where it
pays.

| Family | `style:family` | Property element | Used by |
|---|---|---|---|
| paragraph | `paragraph` | `style:paragraph-properties` | `text:p`, `text:h` |
| text | `text` | `style:text-properties` | `text:span` |
| list | — | `text:list-style` | `text:list` |
| page layout | — | `style:page-layout-properties` | `style:master-page` |

`style:text-properties` is **the same element a cell style uses** for weight, style, size,
colour and background — so `grind_sheet::style::CellStyle`'s fields and a text span's are the
same `fo:` attributes on the same element, reached from a different family. This is the one
place the suite genuinely shares code rather than merely shape.

`style:paragraph-properties` adds what a cell has no use for: `fo:margin-left/right/top/bottom`,
`fo:text-indent`, `fo:line-height`, `fo:break-before` / `-after`, `fo:keep-together`.

**`style:parent-style-name` matters far more here than it does for spreadsheets.** A cell
style not following its parent loses a number format (`doc/not-doing.md` §3 records that
limit). A *paragraph* style not following its parent loses most of its formatting, because
text documents are built on a named-style hierarchy — `Text_20_body` inheriting from
`Standard` is the ordinary case, not an edge one. Whatever the reader does about it has to be
decided before S4 rather than discovered at S8.

---

## 5. What LibreOffice actually does — UNVERIFIED

**Nothing in this section has been checked.** It is the list of questions a text reader will
run into, written down so the answers land here with citations rather than in code as
folklore. `doc/ods-format.md` §§5.4, 6 and 9 are what this section should look like once it
has been done, and until then **no item below may be implemented**.

| Question | Why it matters | How to settle it |
|---|---|---|
| Do ODT table cells use OpenFormula in `table:formula`, or a vendor dialect? | Decides whether `grind-text` can reuse the whole formula engine or must treat a table formula as opaque text | Write a Writer table with a sum, save `.fodt`, read the attribute. Cite the filter that produced it |
| Which `text:` elements does Writer write for a plain document? | The gap between "the schema permits" and "real files contain" is where §5.4's spreadsheet equivalent lives | Author a document with each construct in `doc/text-core.md`, save, diff |
| Does Writer re-quantise paragraph measurements the way it re-quantises border widths? | Loop C compares round-tripped values; a re-quantised `fo:text-indent` needs the same numeric comparison borders got | Round-trip a document with known indents through `soffice --convert-to` |
| How is `fo:font-family` handled? | `doc/ods-format.md` §5.4 measured that LO rewrites it into an `office:font-face-decls` reference. Fonts matter *far* more in a text document than in a spreadsheet | Same method as the spreadsheet measurement, which is already cited there |
| What does Writer do with a `text:s` run of one? | Decides whether the writer's re-encoding round-trips byte-identically or merely semantically | Round-trip a document with leading spaces |
| Is `text:soft-page-break` preserved, dropped, or recomputed on save? | It is a layout artifact in a content file; if LO recomputes it, loop C has to ignore it | Round-trip and diff |

---

## 6. What the reader gets for free

Worth stating, because it is the return on the architecture and it means S4 is smaller than it
looks:

- **Tolerance.** `context::Ignore` swallows any unrecognised subtree, so the ten of sixteen
  `text-content` alternatives outside the scope line cost *no code at all* — not a match arm,
  not a skip list. That is §8's whole design, and it is why loop A can be pointed at Writer's
  corpus on the day the reader exists.
- **Whitespace.** `Context::text` defaults to a no-op, so pretty-printer indentation between
  elements is discarded without a pass. Only the paragraph contexts override it — which is
  exactly the set of elements that *should* collect character data.
- **Prefixes.** Dispatch is on `(uri, local-name)`, so a document using `ns0:` for the text
  namespace reads identically. Already true, already tested.
- **The diff.** R6's retain-and-splice is per-element and the element registry is the only
  spreadsheet-shaped part; a text document splices per `text:p`, which is what makes a `.fodt`
  live in git.

---

## 7. Sources

1. **ODF 1.4 Part 3** — `doc/OpenDocument-v1.4-schema.rng`, cited by line throughout.
2. **`doc/ods-format.md`** — §§1, 8, 8.1, 10 apply here unchanged and are not restated.
3. **LibreOffice** — as an oracle and a corpus, never as a source. §5 is where its
   observations will land, each cited `file:line`.
