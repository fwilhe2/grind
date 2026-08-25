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
once there is a page model, and **there is none** — pagination is gated in `doc/not-doing.md`
§2 under every option in `doc/text-layout.md`. Anything that describes what content *is* — a
heading, a list item, an emphasis — is in, because it is true regardless of how it is drawn.
That split began as a hedge while the layout fork was open and holds either way: it is the
difference between the document and its presentation, which is a distinction ODF makes itself.

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
| `draw:frame` | rng:5089 | `svg:width`, `svg:height` | Holds a `draw:image` (rng:5380). One `Run`, occupying one caret position like a tab — `text:anchor-type="char"` and `"paragraph"` are both this variant, since either way the frame is one child of a `text:p` (rng:8405 lists it in `paragraph-content` directly). LibreOffice nests a second frame inside a `draw:text-box` purely for resizing; both are read as one image, the outermost frame's size preferred over the inner one's. The image itself is `common-draw-data-attlist`'s `xlink:href` (rng:1621, a package's own `Pictures/foo.jpg`) or `office-binary-data` (rng:5383, inline base64) — the schema's own choice, and both are read, the first resolved against the package this document came from. The text-box's own caption paragraph — plain text and a `text:sequence` field's computed value (rng:8655's `<rng:text/>`) — becomes a second run, right after the image |

### Styles

| | `style:family` | Properties in scope | Built |
|---|---|---|---|
| paragraph | `paragraph` | `fo:text-align`, `fo:margin-*`, `fo:text-indent`, `fo:line-height`, `fo:break-before`/`-after`, `fo:keep-together` | **no** — the block carries the style's *name* and nothing else |
| text | `text` | `fo:font-family`, `fo:font-size`, `fo:font-weight`, `fo:font-style`, `fo:color`, `fo:background-color`, `style:text-underline-style`, `style:text-line-through-style` | **yes** — `grind_text::style::CharStyle`, read, written and reachable from `grind text format` |

The text family is **the same `style:text-properties` element and the same `fo:` attributes** a
cell style uses, which is why `grind_core::style` holds the pieces and neither app holds them
twice.

**Only the text family is built, and only as *direct* formatting.** The distinction that makes
that coherent is ODF's own, and it is the whole of `text/src/style.rs`'s module note:

- A style declared in **`office:automatic-styles`** is generated. `T3` means nothing outside the
  file that generated it, so reading resolves it into properties on the run and forgets the
  name, and writing invents a fresh `T1`, `T2`, … from the properties. That is what makes
  "select this and press Ctrl+B" expressible.
- A style declared in **`office:styles`** is the document's own vocabulary. `Emphasis` keeps its
  name and is *not* resolved, because the name is the meaning; turning it into
  `fo:font-style="italic"` would throw the document's structure away in order to draw it.

`fo:font-family` is the one value not kept verbatim: XSL-FO quotes a family whose name contains
a space, so `'Liberation Serif'` is decoded on read and re-encoded on write — the `text:s`
trade, for the same reason. A comma-separated *list* is left exactly as written, because picking
one of them would be this build choosing a font.

Two consequences worth stating plainly, both measured by loop C:

- A paragraph's own `style:text-properties` is **not** read, so when LibreOffice hoists a
  character style that covers a whole paragraph onto that paragraph (it does, always), this
  build sees the run formatting disappear. `text/tests/roundtrip.rs`'s
  `a_character_style_over_a_whole_paragraph_is_hoisted_into_it` pins it.
- `style:font-name` pointing into `office:font-face-decls` — LibreOffice's spelling — **is**
  resolved on read, so the model holds `Georgia` rather than `F1`. The indirection is a
  container detail; the family is the fact. (The spreadsheet carries no font at all for exactly
  the reverse reason: nothing there draws text.)

### Two decisions inside the "in" list

**Nested spans are flattened on read.** `text:span` nests (`doc/odt-format.md` §3.3), and the
model carries a flat `Vec<Run>`. Reading composes the stack down each branch with the inner
style winning — CSS's rule and ODF's — so two nested *automatic* styles arrive as one run whose
direct formatting is both, and writing back produces one span rather than two. Lossless for the
rendering, and lossy only for how many elements it took to say it.

Two nested **named** styles are the lossy case: they compose into one space-joined string, which
is a name no document declares. R6 is what makes that acceptable — an *unedited* paragraph is
never rewritten, so the loss only touches paragraphs a user actually changed.

Writing puts them back in that order: the named span outside, the generated one inside. Not an
automatic style whose `style:parent-style-name` is the name — that would apply the parent twice
and compose the name into itself on the next read, which is a feedback loop rather than a
round trip.

*ponytail:* carrying the tree is the alternative and costs a recursive run model plus a writer
that re-nests. Worth doing if a real document turns out to depend on the named nesting, which
no corpus file has yet been checked for.

**Headings are read at any level and authored at 1–6.** The schema's `positiveInteger` has no
ceiling (`doc/odt-format.md` §3.2), so tolerance means level 9 loads. Authoring stops at 6
because that is where every outline UI stops and an unbounded level is a numbering feature
rather than a structural one.

### What editing means here, and what it does not

This list is elements, and elements are only half of a scope line for an *editor*. The other
half is the set of operations, and S7 fixed it at six: replace a block (`set_text`), and the
four a cursor performs — insert text at a caret, erase a span of characters, split a block,
join two. `move`, `kind`, `style` and `name` are the block-level verbs on top.

**A seventh joined them**, and it is the one that took a change to the model rather than a
change to the App: `set_char_style(from, to, style)` — replace the direct formatting of a span
of characters. It belongs in the same set for the same reason, and it answers its own question
once: setting **replaces** rather than adds, so a bold button is a read, one field and a write
(`grind_sheet::App::set_style`'s contract, unchanged). What a caret with no span reports is what
the *next keystroke* will look like, through the same primitive `insert_text` uses, because two
spellings of "the run at the caret" would be a toolbar lying about what typing does.

Six was a deliberate number. Every one of them is a question with exactly one defensible answer
that has to be given *once*, in the core, because three shells asking it separately would
disagree: what formatting does typed text take (the run at the caret, preferring the left);
what happens to a bookmark inside an erased span (it survives, collapsed to the caret — an
anchor is a position, not content); what does Return at the end of a heading produce (a body
paragraph, and only at the *end*). `doc/plan.md` rule 4 then puts each on the CLI, which is
what stops any of them becoming a UI's private behaviour.

**What is not in that set, and is not a gap:** anything requiring a *selection model*. A
caret is two numbers and belongs in the core; a selection is a shell's idea of what a user is
pointing at, and `erase(from, to)` takes the two carets rather than an object called Selection
for that reason. Also not in it, **today**: layout of any kind. There is no `wrap_width`, no
measurement and no font anywhere in this crate. Whether that is permanent is the open decision
in `doc/text-layout.md` — if line layout moves into the core, this section grows the caret
operations defined in terms of a *line* (down, home, end, hit-test, selection extent), which is
the argument that reopened it.

> That decision closed on Path C and those operations exist (`App::layout_block`, `caret_line`,
> `caret_line_bounds`, `caret_x`). The paragraph above is left as the argument that produced
> them. One consequence lands back here: a run measures with **its own** formatting now, so a
> bold word is measured bold and the caret lands where the ink is — `lay_out` projects each
> run's `CharStyle` into the four metric properties and hands the rest to the provider.

---

## Not yet — inside the line, unbuilt, with the gate that owns it

| Not yet | Gate |
|---|---|
| **`table:table` in a text document** | S6 or later. The *element vocabulary* is shared with the spreadsheet; the model is not — an ODT table cell holds **blocks**, not a value. And `doc/odt-format.md` §5 has an open UNVERIFIED question about whether `table:formula` in a Writer table is even OpenFormula |
| **`text:note` — footnotes and endnotes** | When there is somewhere to put one. A footnote's *content* is ordinary blocks and models fine; a footnote's *placement* is the page model, which is gated. A continuous view can still show one — inline, or at the end of the flow — so this waits for a shell rather than for pages |
| **Fields** (`text:page-number`, `text:date`, `text:title`, `text:file-name`) | A small named set, when something can display one. `text:page-number` is the one that needs a page model specifically, and it stays out with the rest of pagination |
| **`text:table-of-content`** | Read and preserved from the start (R6). *Authoring* one needs heading numbering and an update policy |
| **`office:annotation` — comments** | Genuinely used, and arguably in. Decided with evidence rather than now: it needs an author, a date and a threading model, and guessing which of those matter is how a feature arrives half-built |
| **Markdown import** | A one-way filter at the edge, exactly like `.xlsx` (`doc/xlsx-import.md`), allowed by the same argument — it produces an ODF document and no foreign vocabulary reaches the core |
| **`style:parent-style-name`** | Followed properly, which the spreadsheet reader does not do either (`doc/not-doing.md` §3). It matters **more** here: text documents are built on a named-style hierarchy, so not following it loses most of a document's formatting rather than one number format. Read and kept — an automatic style's parent becomes the run's style *name* — but never resolved |
| **Style *definitions* — reading `office:styles` and writing it back** | **Half built.** The `text` family is: an *automatic* one is resolved into direct formatting on the run and written back as a generated `style:style`, which is what makes a formatting UI possible and what loop C now compares character by character in the "out" direction. What is still gated is a **named** style's properties (its name is carried and its meaning is not) and the whole `paragraph` family. Both stay gated together, because they are the same question — what a style *is* in this model — and answering one without the other gets a half-formatted document |
| **Paragraph-level `style:text-properties`** | The consequence of the row above, and the one that shows: LibreOffice hoists a character style covering a whole paragraph onto that paragraph, so this build reads the run formatting as gone. Measured, not assumed — `a_character_style_over_a_whole_paragraph_is_hoisted_into_it` — and it is why loop C's "back" direction compares structure and text but not formatting |

---

## Never

Additions to `doc/not-doing.md` §1, and each one is a boundary rather than a backlog entry.

| Not doing | Because |
|---|---|
| **`text:section`, multi-column layout** | A section exists to give a run of content its own page geometry. It is layout, and there is none — see pagination's named gate in `doc/not-doing.md` §2 |
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

**Loop C measured what that costs for styles, and it is sharper than the general rule** — but
it now cuts a narrower line than it used to. The distinction is between a style's *name* and its
*properties*, and only the first is still lost.

A **name** the model carries but cannot define is gone from a regenerated document. The writer
emits no `office:styles` at all (R3), and LibreOffice **drops a `text:style-name` that resolves
to nothing** — see `doc/odt-format.md` §5b for the six-case measurement — so `grind text style
p1 --style Mine` on a document authored from nothing means nothing to LibreOffice.

**Direct character formatting is not in that boat any more.** It has no name to lose: the writer
declares the automatic styles it refers to, in the same file, so `grind text format p1+0:p1+4
--bold` survives a regenerate, a conversion between forms and a LibreOffice round trip — checked
character by character in loop C's "out" direction. Which is the practical shape of the split:
**this build can say how text looks and cannot yet say what it is called.**

R6 keeps even the name loss off the common path: a document read from a file splices, so its own
`office:styles` is still in the bytes and its names still resolve. A formatting edit splices too
whenever the file already declares a style with those exact properties, and regenerates when it
would need a declaration the file has no room for — the same line `grind_sheet` draws for a cell
style the file has no entry for.

The remaining gap is real for authored documents, it is loop C's one documented loosening for
text, and closing it means carrying style *definitions* — a feature, gated above beside
`style:parent-style-name`, not a bug in the writer.

---

## The check

`grind_text::implemented()` returns the element names in the **In** section, and
`text/tests/scope.rs` asserts it matches this file — parsed from the tables above, the way
`doc/small-group.md` is parsed. Adding an element to the reader without adding it here fails
the build; so does listing one here that nothing implements.

That is the anti-bloat rule made mechanical, and for a scope line that was invented rather
than extracted, it is the only thing standing between this document and a wish list.
