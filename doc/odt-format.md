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
   is the whole value of this document — it separates what is known from what is assumed. §5
   is the assumptions; §5a and §5b are what loops A and C have since **measured**, and two of
   §5's questions are struck through because §5b answers them.

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

The trap is **wider than spaces**, and §5b is where that was measured rather than guessed:
`text:tab` and `text:line-break` are the same mechanism for the other two whitespace
characters, and a tab or a newline written as itself in character data comes back from
LibreOffice as one space. So the rule is not "re-encode space runs" but *every* piece of
significant whitespace is an element — three elements, one reader expansion, one writer
re-encoding.

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

**Nothing in this section has been checked** except where a row is struck through, which means
§5b answers it. It is the list of questions a text reader will run into, written down so the
answers land here with citations rather than in code as folklore. `doc/ods-format.md` §§5.4, 6
and 9 are what this section should look like once it has been done, and until then **no
unanswered item below may be implemented**.

| Question | Why it matters | How to settle it |
|---|---|---|
| Do ODT table cells use OpenFormula in `table:formula`, or a vendor dialect? | Decides whether `grind-text` can reuse the whole formula engine or must treat a table formula as opaque text | Write a Writer table with a sum, save `.fodt`, read the attribute. Cite the filter that produced it |
| ~~Which `text:` elements does Writer write for a plain document?~~ | The gap between "the schema permits" and "real files contain" is where §5.4's spreadsheet equivalent lives | **Partly answered — §5b** |
| Does Writer re-quantise paragraph measurements the way it re-quantises border widths? | Loop C compares round-tripped values; a re-quantised `fo:text-indent` needs the same numeric comparison borders got | Round-trip a document with known indents through `soffice --convert-to` |
| How is `fo:font-family` handled? | `doc/ods-format.md` §5.4 measured that LO rewrites it into an `office:font-face-decls` reference. Fonts matter *far* more in a text document than in a spreadsheet | Same method as the spreadsheet measurement, which is already cited there |
| ~~What does Writer do with a `text:s` run of one?~~ | Decides whether the writer's re-encoding round-trips byte-identically or merely semantically | **Answered — §5b** |
| Is `text:soft-page-break` preserved, dropped, or recomputed on save? | It is a layout artifact in a content file; if LO recomputes it, loop C has to ignore it | Round-trip and diff |

---

## 5a. What loop A measured — VERIFIED

The first section of this document that is not a claim. Loop A (`text/tests/corpus_read.rs`)
was pointed at LibreOffice's Writer corpus, `sw/qa`, and read everything in it.

```
1763 documents, 1755 read, 4 password-protected, 4 not documents at all, 0 failed
```

**Tolerance held by construction, with no file special-cased.** That is §8's claim and this is
the first evidence for it in a second document type: 1,755 real Writer documents — two decades
of regression fixtures in `sw/qa/extras/` — loaded through a reader that models ten elements.

The four that are not documents are named in the test with the reason for each, and **each was
confirmed by a parser that is not ours** before being written down, because "our reader rejects
it" is not evidence that a file is bad:

| File | Independent verdict |
|---|---|
| `forcepoint-dtor-1.odt` | `zipfile`: bad CRC-32 for `content.xml`. LibreOffice files it under `sw/qa/core/data/odt/**fail**/` |
| `CVE-2012-4233-1.odt` | Not a zip and not XML — 9021 bytes of binary noise, a fuzzed crash reproducer |
| `forcepoint108.fodt` | `xml.etree`: mismatched tag at line 66 |
| `threadedException.fodt` | `xml.etree`: unbound namespace prefix at line 403 |

Refusing these is correct and is not a tolerance failure. §8's tolerance is about
*unrecognised* content; `Error::Xml` is the separate **structural** case (§8.2), and a
container that will not decompress is `Error::Package`.

**A finding worth keeping.** The first of those four came back as `Error::Io` — "the filesystem
failed" — because `zip` reports a bad CRC-32 as an `io::Error` and `content_xml` let it convert.
It is now `Error::Package`, which is what a caller needs in order to tell "I could not read the
file" from "the file is damaged". Loop A found that within a minute of first running, over a
document type it was not even testing: `content_xml` is shared, so the spreadsheet had the same
wrong answer.

**`sw/qa/core/data/odt/` has `pass/`, `fail/` and `indeterminate/` subdirectories** — a
convention worth knowing before reading any result from that corpus, since a file under `fail/`
is one LibreOffice's own harness expects *not* to import.

---

## 5b. What loop C measured — VERIFIED

The second measured section, and the one that checks the **writer**. Loop C
(`text/tests/roundtrip.rs`) writes a document, has LibreOffice convert it, reads the result
back, and asserts it is the same document — then does it the other way round, starting from a
Writer-authored file out of `sw/qa`.

```
loop C (text, out):  14 documents (7 cases x 2 physical forms), 0 differences
loop C (text, back): 20 documents, 5095 blocks,                 0 differences
```

Measured against LibreOffice 26.2.5.2. First measured against a full install of that version
rather than the pinned image, which had no Writer in it; re-measured against the pin once it was
rebuilt, with the same figures. See "the oracle" at the end of this section.

### The bug it found in the first run

**A tab or a newline inside a run's text was written as itself, and came back as a space.**
The model has had `Run::Tab` and `Run::Break` since S4, so *structured* tabs were fine — but a
paragraph whose text merely *contained* `\t`, which is what `grind text set` produces, wrote the
character literally. XML character data is whitespace, and an ODF consumer collapses a run of it
to one space, so the user's tab was silently gone.

This is §3.3's trap, and the point is that the reasoning behind `text:s` had been done and then
*not carried across the other two characters it applies to* — which is exactly the kind of gap
that survives a self-consistent round trip, because our own reader was collapsing nothing.
`odf::write::characters` re-encodes all three now.

`\r` and `\r\n` are both written as one `text:line-break` and therefore read back as `\n`. That
is not a choice: XML line-ending normalisation (XML 1.0 §2.11) hands a parser's caller `\n` for
either, so writing anything else would only be a lie about what a reader will see.

### A Writer document cannot be empty

The degenerate document — `grind text new` and nothing else — comes back holding **one empty
paragraph**. Writer's model has no body without a paragraph in it.

Loop C allows for exactly that and nothing more, and
`a_document_with_no_blocks_comes_back_holding_one` pins the fact, so the allowance goes red if
LibreOffice ever stops doing it. An allowance that nothing checks is indistinguishable from a
bug.

### What happens to a `text:style-name`

Six cases, measured together because only the contrast makes the rule legible:

| What the document says | Comes back as | |
|---|---|---|
| A name LibreOffice itself defines (`Quotations`) | `Quotations` | kept |
| `office:styles`, with a property | `NamedWith` | kept |
| `office:styles`, with no properties at all | `NamedBare` | kept |
| `office:automatic-styles`, with a property | **`P1`** | formatting kept, *name* renumbered |
| `office:automatic-styles`, with no properties | `Standard` | dropped |
| Declared nowhere | `Standard` | dropped |

So the rule is not "LibreOffice mangles style names". It is ODF's own distinction applied
exactly: a **named** style is an identity and keeps its name; an **automatic** style is
anonymous direct formatting by definition, so its name is not identity and LibreOffice
renumbers it into its own sequence; a name that resolves to nothing is not formatting at all and
goes.

**The last row is this build's gap**, and it is loop C's one documented loosening for text —
the comparison checks structure and text but not style names. The writer is minimal by intent
(R3) and emits no `office:styles`, and the model carries a style's *name* but never its
properties (`doc/text-core.md`), so a **regenerated** document refers to styles that are not
there: `grind text style p1 --style Mine` on a document this build authored means nothing to
LibreOffice. R6 keeps that off the common path — a document *read from a file* splices, so its
own `office:styles` is still in the bytes and its names still resolve — but a document authored
from nothing carries no formatting out.

### Two of §5's questions, answered

**A `text:s` run of one round-trips byte-identically.** `<text:s/>` comes back `<text:s/>` and
`<text:s text:c="2"/>` comes back `<text:s text:c="2"/>`, so the convention this writer follows
— keep the first space of a run literal, encode the rest — is LibreOffice's own and the
re-encoding is not merely semantic.

**What Writer writes for a plain document**, beyond what it was given: a `text:sequence-decls`
block of five `text:sequence-decl`s, a `text:style-name` on *every* paragraph and heading
(`Standard`, `Heading_20_1`, `P1`, …), a `text:style-name` on every `text:list`, an
`office:automatic-styles` holding a `style:page-layout`, and an `office:master-styles`. On a
`text:a` it also adds `xlink:type="simple"`, `text:style-name="Internet_20_link"` and
`text:visited-style-name`. All of it is inert to our reader — §8's default-ignore — which is why
the round trip is clean despite the file coming back several times the size it went in.

### What loop C does not cover, and why

The comparison is **structure and text**: block count, each block's kind (paragraph, heading
with its level, list item with its depth), each block's plain text, and the set of bookmark
names. It is not a formatting comparison, because there is no formatting in the model to
compare yet — see the style rule above. When the writer learns to declare styles, the
comparison gains a column and the last row of that table goes red.

### The oracle: the pin had no Writer in it, and now does

**`ci/libreoffice-image` could not serve `grind-text`.** The image these figures were first
measured against held `calc.xcd` and no `writer.xcd` in its `share/registry/`, so that build
imported a `.fodt` *as a spreadsheet* and had no `fodt` export filter to convert one back with.
The figures therefore came from a full LibreOffice 26.2.5.2 — the same version as the pin, but
not the same install.

Rather than hard-code a skip or drop the pin, `oracle_ready` in `text/tests/roundtrip.rs`
**probes the capability by doing it**: convert a one-paragraph document, and see whether output
appears. Against a Calc-only `soffice` the four soffice-backed tests skipped with a notice;
against a full one they ran.

The image has since been rebuilt with Writer in it
(`sha256:adb88646…`), and loop C for text **began gating CI's `roundtrip` and `corpus` jobs
without a line of any file changing** — which is the whole argument for detecting a capability
over hard-coding a skip. Every figure in this section now comes from the pin itself, and
`FLOOR` in `sheet/tests/loop_e.rs` was re-read against the new image as the upgrade procedure
demands: 913, unchanged, so the rebuild added Writer and moved nothing else. The probe stays,
because a developer with `libreoffice-calc` and no `libreoffice-writer` is a normal thing to be.

A second, unrelated finding on the way there: the shim that runs the pinned image
(`scripts/soffice-docker/soffice`) could not write to its own bind mount on an SELinux-enforcing
host, so `soffice` could not create its `UserInstallation` profile and **every** loop C and
loop E test failed on Fedora and RHEL for a reason unconnected to the code. Fixed with
`--security-opt label=disable`, which is scoped to the container; the alternative, `:z`,
relabels the host's whole temp directory as a side effect of running a test.

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
   observations will land, each cited `file:line`; §5a and §5b are the ones the loops have
   already measured, cited by the test that measures them.
