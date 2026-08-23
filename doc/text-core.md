<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Text Core — the scope line for `grind text`

`doc/small-group.md` is the spreadsheet's scope line, and it had an advantage this document
does not: **it was extracted, not invented.** OpenFormula §2.3.2 enumerates 110 functions
normatively, so the spreadsheet's feature list ends where a specification says it ends, and
`funcs::implemented()` is checked against that list by a test.

ODF Part 3 defines no such tier for text documents. The sixteen alternatives of `text-content`
(`doc/odt-format.md` §2) are a flat choice with nothing marking a core. So the line below is a
**product decision**, which makes this the most important and the most fragile document in
phase 10 — and makes it correspondingly more important that it is mechanical rather than
prose. `grind_text::implemented()` is checked against this file by a test, exactly as
`doc/small-group.md` is. Drift is a build failure, not a discussion.

**The rule for changing it is `doc/not-doing.md`'s rule, unchanged**: one item at a time, by
explicit decision, and it must survive a round trip through LibreOffice (loop C). Nothing
moves because it was easy.

---

## The principle

A spreadsheet's scope line was chosen by a standards committee. A word processor's has to be
chosen by asking what a document *is*, and the answer this project takes is:

> **A text document is a sequence of styled paragraphs, some of which are headings, some of
> which are lists, and some of which are tables. Everything else is either presentation or
> a second product.**

Two consequences that decide most of the table below:

**Structure over layout.** Anything that exists to control where content lands on a *page* —
sections, columns, frames, page sequences — is out or gated, because it is only meaningful
once there is a page model, and whether there is ever a page model is `doc/suite.md`'s open
fork. Anything that describes what content *is* — a heading, a list item, an emphasis — is in,
because it is true regardless of how it is drawn.

**Generated content is not content.** A table of contents, an index and a bibliography are
*derived* from the document. Authoring one means owning the derivation, the numbering, and the
update policy. Reading one means preserving a block this build did not compute. The second is
free under R6; the first is a feature with a gate.

---

## In — the elements this build models

`grind_text::implemented()` returns exactly these names, and the test fails if it does not.

### Block level

| Element | Schema | Carries | Note |
|---|---|---|---|
| `text:p` | rng:17950 | `text:style-name` | The unit of everything. Also the R6 splice unit |
| `text:h` | rng:17095 | `text:style-name`, `text:outline-level` | A paragraph plus a level. **Read at any level, authored at 1–6** |
| `text:list` | rng:17494 | `text:style-name` | Nests through its items, never through itself |
| `text:list-item` | rng:17538 | `text:start-value` | Holds blocks, so nesting is structural |

### Inline

| Element | Schema | Carries | Note |
|---|---|---|---|
| character data | rng:8406 | — | |
| `text:span` | rng:8422 | `text:style-name` | Nests; see the flattening decision below |
| `text:s` | rng:8408 | `text:c` | ODF's run-length spaces. **Expanded on read, re-encoded on write** — the `table:number-columns-repeated` trap in a new costume |
| `text:tab` | rng:8415 | — | |
| `text:line-break` | rng:8418 | — | A break *within* a paragraph, which is not a new paragraph |
| `text:a` | rng:16453 | `xlink:href` | Cannot nest inside itself, per the schema |
| `text:bookmark` | rng:16801 | `text:name` | **The named-range analogue** — what `loc.rs` resolves `#intro` against |

### Styles

| | `style:family` | Properties in scope |
|---|---|---|
| paragraph | `paragraph` | `fo:text-align`, `fo:margin-*`, `fo:text-indent`, `fo:line-height`, `fo:break-before`/`-after`, `fo:keep-together` |
| text | `text` | `fo:font-weight`, `fo:font-style`, `fo:font-size`, `fo:color`, `fo:background-color`, `style:text-underline-style`, `style:text-line-through-style` |

The text family is **the same `style:text-properties` element and the same `fo:` attributes** a
cell style uses, which is why `grind_core::style` holds the pieces and neither app holds them
twice.

### Two decisions inside the "in" list

**Nested spans are flattened on read.** `text:span` nests (`doc/odt-format.md` §3.3), and the
model carries a flat `Vec<Run>`. Reading composes the style stack down each branch, so
`<text:span text:style-name="bold"><text:span text:style-name="italic">x</text:span></text:span>`
arrives as one run whose direct formatting is both. This is **lossy for the style names** and
lossless for the rendering: the two named styles become one composed set, and writing back
produces one span rather than two. R6 is what makes that acceptable — an *unedited* paragraph
is never rewritten, so the loss only touches paragraphs a user actually changed.

*ponytail:* carrying the tree is the alternative and costs a recursive run model plus a writer
that re-nests. Worth doing if a real document turns out to depend on the named nesting, which
no corpus file has yet been checked for.

**Headings are read at any level and authored at 1–6.** The schema's `positiveInteger` has no
ceiling (`doc/odt-format.md` §3.2), so tolerance means level 9 loads. Authoring stops at 6
because that is where every outline UI stops and an unbounded level is a numbering feature
rather than a structural one.

---

## Not yet — inside the line, unbuilt, with the gate that owns it

| Not yet | Gate |
|---|---|
| **`table:table` in a text document** | S6 or later. The *element vocabulary* is shared with the spreadsheet; the model is not — an ODT table cell holds **blocks**, not a value. And `doc/odt-format.md` §5 has an open UNVERIFIED question about whether `table:formula` in a Writer table is even OpenFormula |
| **`text:note` — footnotes and endnotes** | When there is somewhere to put one. A footnote's *content* is ordinary blocks and models fine; a footnote's *placement* is the page model, which is the open fork |
| **Fields** (`text:page-number`, `text:date`, `text:title`, `text:file-name`) | A small named set, when something can display one. `text:page-number` needs pages, which is the fork again |
| **`text:table-of-content`** | Read and preserved from the start (R6). *Authoring* one needs heading numbering and an update policy |
| **`office:annotation` — comments** | Genuinely used, and arguably in. Decided with evidence rather than now: it needs an author, a date and a threading model, and guessing which of those matter is how a feature arrives half-built |
| **Markdown import** | A one-way filter at the edge, exactly like `.xlsx` (`doc/xlsx-import.md`), allowed by the same argument — it produces an ODF document and no foreign vocabulary reaches the core |
| **`style:parent-style-name`** | Followed properly, which the spreadsheet reader does not do either (`doc/not-doing.md` §3). It matters **more** here: text documents are built on a named-style hierarchy, so not following it loses most of a document's formatting rather than one number format. **Decide before S4 ships, not after** |

---

## Never

Additions to `doc/not-doing.md` §1, and each one is a boundary rather than a backlog entry.

| Not doing | Because |
|---|---|
| **`text:section`, multi-column layout** | A section exists to give a run of content its own page geometry. It is layout, and it is the fork |
| **Text frames as layout boxes** | Positioned boxes with flow-around are a page-layout program |
| **A drawing canvas** (`shape` beyond an anchored image) | Shapes, connectors and their layout model are a different application — the same line `doc/not-doing.md` already draws for spreadsheets |
| **Generated indices** — illustration, table, object, user, alphabetical, bibliography | Six of `text-content`'s sixteen alternatives. Each is a derivation with its own numbering and update rules, for content nobody edits by hand. Read and preserved; never authored |
| **`change-marks` — change tracking** | Already `doc/not-doing.md` §1 for spreadsheets, and the reason is stronger here. **But it must survive a splice**: R6 means an opened document keeps it, and *that* distinction is the point — refusing to author change tracking is not the same as destroying it |
| **Mail merge, forms and controls** | Each is a second product with its own data model |
| **Master documents** | A document made of other documents is a build system |
| **`.docx` writing** | The `doc/xlsx-import.md` asymmetry, for the same reason: one way in, never out |
| **`text:numbered-paragraph`** | A second numbering mechanism beside `text:list`, for the same result. One is enough |

---

## What "preserved" means, and why it is most of the value

Every "never" above is followed by *read and preserved*, and that is not a consolation prize.

R6's retain-and-splice means an opened document is **edited in place**: the reader keeps the
source bytes and the writer replaces only the elements that changed. So a document with change
tracking, three indices, a frame and a vendor's extension loads, gets one paragraph edited, and
saves with all of it intact — because the writer never regenerated it and therefore never had
to understand it.

That is the same property `doc/plan.md` R6 already delivers for spreadsheets, and it is worth
far more here: a text document carries a much larger share of content this build has no model
for. **A reader that models ten of sixteen block types and a writer that touches only what
changed is a better custodian of the other six than a reader that models all sixteen badly.**

The boundary is honest and stated: what is preserved is what is *not edited*. Regenerating —
a new paragraph in a document that has none, a conversion between forms, a style change — drops
what the model does not carry, exactly as it does for spreadsheets, and `doc/not-doing.md` §2
already carries that row.

---

## The check

`grind_text::implemented()` returns the element names in the **In** section, and
`text/tests/scope.rs` asserts it matches this file — parsed from the tables above, the way
`doc/small-group.md` is parsed. Adding an element to the reader without adding it here fails
the build; so does listing one here that nothing implements.

That is the anti-bloat rule made mechanical, and for a scope line that was invented rather
than extracted, it is the only thing standing between this document and a wish list.
