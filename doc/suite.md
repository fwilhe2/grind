<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# The suite — a word processor beside the spreadsheet

This is the work plan for turning one ODF-native spreadsheet into an ODF-native **suite**, and
the document that holds it to the rules once building starts. It is normative for **phase 10**
the way `doc/gtk-shell.md` is for phase 9 and `doc/xlsx-import.md` is for what is now phase 11.

It answers five questions, in this order because each one constrains the next:

1. **What the things are called**, and why none of those names is anybody's trademark.
2. **How the core splits** so that a second document type is a peer rather than a guest.
3. **How the CLI restructures** into `grind <app> <verb>`, with the parity ratchet intact.
4. **How every shell reaches every document type** — CLI, terminal, GTK and browser alike,
   which is R10 and is not negotiable per shell.
5. **How it is packaged** — binaries, desktop entries, mime types, `.deb`/`.rpm`, containers.

And it names the one product decision this phase cannot make by itself: **whether the word
processor paginates.** That fork is in "The layout fork" below, and everything after it in the
milestone list is contingent on the answer.

---

## Decisions taken up front

Everything below assumes these. They are recorded here so that the rest reads as consequence
rather than as argument.

| Question | Answer |
|---|---|
| Suite command | **`grind`** — first level. `grind sheet …`, `grind text …` |
| Word processor | **`text`** — ODF's own word for it (`office:text`) |
| Repository, crates, binaries, env vars | **Renamed in full, now.** Nothing is released; this is the cheapest it will ever be |
| Relationship to the existing core | **Split, don't wrap.** A shared generic crate, one crate per document type |
| Two `App`s or one | **Two**, plus one shared lifecycle trait |
| Shells | **Every document type reaches every shell** — CLI, TUI, GTK, web. GTK splits per type because a `.desktop` file must; the TUI and the web shell do not |
| Layout | **See "The layout fork"** — the one decision deferred, with both branches costed |
| ODT reading before ODT writing before ODT editing | Same order phases 2–4 used, and for the same reason |
| Presentations | Out of scope here. The shape must not *preclude* a third type; `slides` is reserved |

---

## The names, and why

The project's README carries a promise: *"No trademark is used in the name of this project, its
binaries, its icon, or its packaging."* Renaming is where that promise is either kept or
quietly broken, so the reasoning is written down rather than assumed.

| Name | Role | Why |
|---|---|---|
| **`grind`** | The suite; the first-level binary; the crate prefix | A plain English word with no software mark of consequence. Reads as a sentence — `grind sheet new`, `grind text open`. Ironic about the work, which suits a project whose pitch is "the six things I actually do" |
| **`sheet`** | The spreadsheet app, unchanged | Already the name, already generic |
| **`text`** | The word processor | **ODF's own word.** The body element is `office:text`; the media type is `application/vnd.oasis.opendocument.text`. Exactly the argument that made `sheet` right |
| **`slides`** | Reserved, unbuilt | `office:presentation`'s plain-language name. Reserved so the registry in S2 has somewhere to put a third entry |

Rejected, and why — this list matters more than the accepted one:

| Rejected | Because |
|---|---|
| **`odf`** | **OpenDocument and ODF are OASIS trademarks.** A binary or crate named `odf` is a trademark in packaging, which is the exact thing the README promises not to do. Descriptive use in prose stays fine and stays everywhere |
| **`writer`** | LibreOffice Writer. The Document Foundation's mark, and the single most confusable choice available |
| **`word`, `office`** | Microsoft's. "Office" standalone has been held descriptive more than once, which is not the same as being safe, and a binary literally named `office` is the loudest possible target |
| **`docs`, `pages`** | Google's and Apple's respectively |
| **`document`** | Not a trademark problem — an *ambiguity* problem. In a suite, a spreadsheet is a document too, so `grind document open book.ods` reads as though it should work |

**Two checks to run before the rename is irreversible**, neither of which this document performed:

- A binary-name collision search in Debian and Fedora for `grind` (`apt-file search bin/grind`,
  `dnf provides '*/bin/grind'`). Locally there is none. There is a niche Go refactoring tool
  called `grind` on GitHub, unpackaged as far as anyone can tell — worth confirming, because a
  file conflict in `/usr/bin` is a packaging bug, not a naming preference.
- A trademark sanity check for GRIND in Nice classes 9 and 42.

If either comes back badly, `desk` and `suite` were the runners-up and nothing below depends on
which word won.

### The full rename

| Was | Becomes |
|---|---|
| repo `fwilhe2/sheet` | `fwilhe2/grind` |
| crate `sheet-core` | **split** — see "Crate layout" |
| crate `sheet-cli` | `grind-cli`, binary `grind` |
| crate `sheet-gtk` | `grind-sheet-gtk` |
| crate `sheet-tui`, `sheet-web` | `grind-tui`, `grind-web` |
| `SHEET_LO_CORPUS` | `GRIND_LO_CORPUS`, and it now names the **checkout root** rather than `sc/qa/unit/data`: Calc's corpus is at `sc/qa/unit/data`, Writer's at `sw/qa`, and one clone serves both |
| `SHEET_LOCALE`, `SHEET_REPO`, `SHEET_DEMO`, `SHEET_FUZZ_SEED`, `SHEET_LOOP_B_DUMP`, `SHEET_LOOP_E_DUMP`, `SHEET_LOOP_E_FORMULAS` | `GRIND_*`, same meanings |
| `$XDG_CONFIG_HOME/sheet/locale` | `$XDG_CONFIG_HOME/grind/locale`, with the old path **read as a fallback** and a one-line deprecation note on stderr. Costs four lines; the alternative is silently forgetting a user's locale |
| app ID `io.github.fwilhe2.Sheet` | unchanged — it is already per-app, and `io.github.fwilhe2.Text` slots in beside it |

**No `sheet` compatibility alias.** A multi-call binary preserving the old spelling was
considered and dropped: nothing has been released, so the only scripts that break are in this
repository (`examples/sample.sh`, `doc/cli-recipes.md`, `cli/tests/cli.rs`), and carrying a
second name forever to protect zero users is exactly the kind of accretion `doc/not-doing.md`
exists to refuse.

---

## What "suite level" actually costs

The honest framing, before any of the pleasant architecture below.

**Reading and writing ODT is the easy half, and probably easier than ODS was.** The text
content model is a tree of `text:p` / `text:h` / `text:span` with no `table:number-columns-repeated`
to get wrong, no per-cell value types, and no cached-result contract. The reader is contexts
over the existing `context.rs` driver, and the tolerance falls out by construction the same way
it did in phase 2.

**Laying text out is the schedule**, in the way the formula engine was the schedule for the
spreadsheet — but worse in one specific respect. Phase 4 had ODF Part 4: a *normative*
specification that made a from-scratch engine a transcription job, and a 509-file corpus with
the right answers already embedded, which made progress a number from the first week. Text
layout has neither. Line breaking has UAX #14, and everything above it — tab stops, indents,
list label placement, widow and orphan control, footnote placement, table splitting across
pages — is a set of behaviours you either match by observation or diverge from visibly.

That asymmetry is the entire reason for "The layout fork" below, and it should be read before
the milestone list rather than after it.

**Everything else scales linearly and cheaply**, because the architecture already assumed a
second document type: `doc/ods-format.md` §10 names the media types and root elements for text
and presentation, three of the reader's files already carry `**[GENERIC]**` in their module
docs, and `Context<S>` is already generic over its sink.

---

## The requirements, extended

`doc/plan.md`'s R1–R7 are normative and **apply per document type**, unchanged in wording. Two
consequences and two additions.

**R2 (schema validity) and R5 (tolerance) get much sharper for text.** An ODT in the wild
carries change tracking, fields, sections, frames, drawing shapes, forms and bibliography
markup — a far larger share of the file is content this build has no model for than in any
spreadsheet. Tolerance on the way in is the same architecture; strictness on the way out means
the *regenerating* writer must produce valid ODF for the subset it owns.

**R6 (minimal diffs) is worth more for text than it was for the spreadsheet, and is the single
strongest reason to do this at all.** A `.fodt` that changes one line of XML when one paragraph
is edited is a word-processor document that lives in git like a source file. Nothing else in
this category does that. The retain-and-splice machinery in `core/src/odf/source.rs` already
does exactly this per `table:table-cell`; per `text:p` is the same trick against a different
element, and it is what protects every unmodelled construct in an opened document for free.

Two new requirements, each with the thing that checks it:

| | Requirement | Checked by |
|---|---|---|
| **R8** | **No document type's vocabulary appears in the shared crate.** The generic crate must know about packaging, namespaces, the context stack, `fo:` primitives and `number:*-style`, and nothing about cells, sheets, paragraphs or headings. | Its `Cargo.toml` names neither app crate, plus a test grepping its sources for `table:table`, `text:p` and the other body vocabularies. A shared crate with no mechanical guard becomes a dumping ground within two milestones |
| **R9** | **Whatever any GUI can do, the CLI can do — per app.** `doc/plan.md`'s rule 4, generalised: one parity document per app, all of them checked by one test. | `cli/tests/parity.rs`, reading a registry of `(core crate, parity doc)` pairs instead of one hard-coded path |
| **R10** | **Every document type is reachable from every shell.** A document type that only has a window is a product decision nobody made. Per-shell *feature* gaps are allowed and must be named; a shell that cannot open and edit a type at all is a build failure. | `doc/shell-matrix.md` against the S2 registry's shell axis — the same shape, and the same mandatory-reason rule, as the parity documents |

R4 (`calcext:` opt-in, outranked by R2) gains a sibling in practice: LibreOffice Writer's
`loext:` extensions are read and ignored on the same terms, and earn a place on the way out only
with a *measured* behaviour recorded in `doc/odt-format.md` that cannot be had any other way.

---

## Crate layout

### Now

```
core/    sheet-core   model, ODS I/O, formula engine, numfmt, style, locale, a1
cli/     sheet-cli    the `sheet` binary
ui_gtk/  sheet-gtk
ui_tui/  sheet-tui
ui_web/  sheet-web
```

### After

```
core/          grind-core       [GENERIC] — nothing here knows what a cell or a paragraph is
sheet/         grind-sheet      the spreadsheet: model, ODS I/O, formula engine
text/          grind-text       the word processor: model, ODT I/O
cli/           grind-cli        the `grind` binary — one subtree per app
ui_common/     grind-ui         shared GTK plumbing (extracted at S9, not before)
ui_sheet_gtk/  grind-sheet-gtk
ui_text_gtk/   grind-text-gtk
ui_tui/        grind-tui        both document types, one binary, dispatching on kind
ui_web/        grind-web        both document types, one bundle, dispatching on kind
xlsx/          grind-xlsx       phase 11, unchanged plan
```

### What moves where, and why

**Into `grind-core` (the generic crate):**

| From | What | Why it is generic |
|---|---|---|
| `odf/package.rs` | zip / flat sniffing, `content.xml`, `styles.xml`, encryption detection | Already marked `**[GENERIC]**`. §1 of `doc/ods-format.md` is format-agnostic in full |
| `odf/names.rs` | `(uri, local-name)` resolution, the `Ns` enum | Already `**[GENERIC]**`. `Ns` gains `Draw`, `Loext`, and whatever `doc/odt-format.md` proves necessary |
| `odf/context.rs` | the element-context stack and its default-ignore driver | Already `**[GENERIC]**`, already generic over its sink `S`. Zero changes expected |
| `locale.rs` | decimal point, grouping separator | Two characters. Nothing spreadsheet-specific ever touched it |
| ~~`numfmt/`~~ | `number:*-style` parse and apply | **Did not move at S1 — see "What S1 actually did".** It depends on `formula::date`, `formula::value::format_number` and `model::CellValue`, so what is generic is the *format model*, not the code that applies one to a cell |
| half of `style.rs` | `Color`, `Border`, `length_mm`, `PALETTE`, the alignment enums, `EDGES` | These are `fo:` properties. A paragraph style and a cell style are made of the same vocabulary |
| **new:** `kind.rs` | `fn kind(&[u8]) -> Option<DocumentKind>` | Sniffing whether bytes are a spreadsheet or a text document, from the package `mimetype` entry or the flat root's `office:mimetype` / body element. Needed by `grind info`, by every GUI's Open dialog, and by the cross-app handoff |
| **new:** `observer.rs` | the `Observer` trait | The one part of the shell contract with nothing document-shaped in it: something changed, come and re-read it. The `Editor` trait it was meant to sit beside **was not built at S1** — see below |

**Into `grind-sheet`:** `model.rs`, `grid.rs`, `action.rs`, `a1.rs`, `filter.rs`, `formula/`,
`odf/read.rs`, `odf/write.rs`, `CellStyle`, and the spreadsheet `App`. Which is to say: almost
all of the code, and none of the surprises.

**`odf/source.rs` stays in `grind-sheet` at S1 and is generalised at S5**, when the text writer
needs it and the second implementation reveals the seam. Its `Cell { range, cols, keep }` and
its `rows: HashMap<(usize, u32), Vec<Cell>>` are spreadsheet-shaped today; the generic form is a
registry of byte spans keyed by an opaque node id, with each document type owning the mapping
from id to its own addressing. Generalising it *before* there is a second caller is guessing,
and `doc/plan.md` already made that call once — *"empty crates now are scaffolding for later;
later can scaffold for itself."*

**S1 is a move, not a redesign, and it must be provable as one.** Its exit criterion is that
all four loops stay green with byte-identical results and `kb.rs`'s R3/R6 percentages do not
move by a digit. If any number changes, something was rewritten that should have been moved.

### What S1 actually did

Met: R6 retention came back **64 / 42 / 53 / 50 / 14 / 48 %**, digit for digit, and no test was
lost — 189 spreadsheet unit tests became 183 plus the 6 that moved, with 7 new ones in
`grind-core`. Two things in the plan above did not survive contact, both in the same direction,
and both are recorded here rather than quietly taken.

**`numfmt/` did not move.** The claim above was that ODF uses `number:*-style` outside
spreadsheets and it should therefore be generic. That is still true of the *format model* and is
not true of the module: it depends on `formula::date`, `formula::value::format_number` and
`model::CellValue`, because most of it is not "what is a `number:currency-style`" but "render
*this cell* with one". Splitting a 827-line module along a seam no second caller has pushed on
is the guessing S1 exists not to do. It moves when `grind-text` asks for a data style — a table
cell with an `office:value`, or a date field — and the question is then answered by a caller
instead of by a paragraph.

**The `Editor` trait was not built.** Writing it turned up a question the plan had not asked:
*what error type does it speak?* Every failure `open_bytes` and `save_bytes` can actually
produce is generic, so `grind_core::Error` is honest — but `App`'s inherent methods are typed in
the spreadsheet's `Result`, so either the impl narrows them or the signatures change, and an
associated `type Error` keeps both honest at the cost of the uniform `dyn Editor` that was half
the point. **None of those can be chosen well against one implementation.** `grind-text`'s `App`
is what decides it. Until then `grind_core::Observer` carries the part that was never in doubt,
`core/src/observer.rs` records the open question where the next person will meet it, and the
suite CLI dispatches on `kind` — a `match` in exactly one file, which is a smaller thing to
undo than a wrong trait.

The pattern both share is the one the plan already applied to `odf/source.rs`, and it is worth
stating as a rule rather than as two anecdotes: **a seam is extracted when a second caller
pulls on it, never when a document predicts it will.**

---

## The two `App`s, and the one shared trait
<!-- Status: the two `App`s are the plan of record. The trait is deferred — see
     "What S1 actually did" above for the question that has to be answered first. -->


`App` today is spreadsheet-shaped to its bones: `get_viewport(sheet, rows, cols)`,
`set_col_width`, `enter`, `recalc`, `calculations`. Three ways forward were considered:

| Option | Verdict |
|---|---|
| One generic `App<D: Document>` | **No.** A grid's viewport and a text flow's viewport share nothing but the word. Every reader in every shell would carry a type parameter to buy an abstraction with one method in it |
| One `App` enum dispatching on kind | **No.** Every method becomes a match with an unreachable arm, and the parity ratchet loses the ability to say which app is missing what |
| Two `App` types + one shared trait | **Yes** |

So: `grind_sheet::App` (unchanged) and `grind_text::App` (new), each with its own `RwLock<State>`,
its own `Action` enum and inverse, its own undo stack, and its own parity document.

What they genuinely share goes in `grind_core::Editor`:

```
open_bytes / open_file / save_bytes / save_file
undo / redo / can_undo / can_redo
set_observer
session / restore_session
```

That trait is what lets `grind`'s suite-level verbs (`info`, `convert`, `validate`, `undo`,
`redo`) be written once against either app, and what lets a future shell host both without
knowing which it has. It is deliberately small: everything about *reading* a document stays on
the concrete type, because rule 1 ("reads go through a windowed API") means the window is
different per type and pretending otherwise is how the abstraction rots.

**`Session` gains a `kind` field and a version.** Today a session file is two `Action` stacks.
With two `Action` enums, `grind text undo --session s.json` against a spreadsheet session would
either fail confusingly or, worse, deserialise into something plausible. The kind is checked on
load and a mismatch is a plain error.

---

## Addressing text — `loc.rs`, the `a1.rs` analogue

The spreadsheet got a gift: A1 notation is normative, universally understood, and already
specified. `core/src/a1.rs` is the workspace's only 0↔1 conversion and every shell leans on it.

Text has no such gift, so the address vocabulary has to be **invented, written down, and kept in
one module** — `grind_text::loc`, held to exactly the rules `a1.rs` is held to: the only 0↔1
conversion, parsed in one place, never re-derived by a shell.

Proposed spellings, all 1-based on the outside and 0-based within the core:

| Spelling | Means | Notes |
|---|---|---|
| `p12` | the 12th block element | Paragraphs, headings, lists and tables all count. The primary address |
| `p12:p20` | a range of blocks | The analogue of `A1:C9` |
| `p12+40` | character offset 40 within block 12 | For insertion points and `find` results |
| `p12+40:p13+7` | a span across blocks | What a selection is |
| `#intro` | a `text:bookmark` named `intro` | **The named-range analogue**, and it round-trips through LibreOffice because ODF already has the construct |
| `§2.1.3` | outline path — chapter 2, section 1, subsection 3 | Derived from `text:h` levels. Stable under edits elsewhere in the document, which `p12` is not. Resolves to a `p` address |

The last two are the interesting ones, and both should be built: `p12` is what a machine uses
and is invalidated by every insertion above it, while `#intro` and `§2.1.3` are what a *person*
or a *script* uses and survive editing. A script that says `grind text set report.fodt §3.2
"…"` still works next week. That is a genuine CLI capability the big one does not have.

---

## The text model

`grind_text::Document` is an ordered sequence of blocks plus the document-level tables the
spreadsheet's `Document` already has analogues for (styles, names, source bytes for R6).

```
Document { blocks: Vec<Block>, styles, bookmarks, source: Option<Source>, edits: Edits }

Block   = Paragraph { style: Option<Name>, direct: Option<ParaStyle>, runs: Vec<Run> }
        | Heading   { level: 1..=6, .. as Paragraph }
        | List      { style, items: Vec<Vec<Block>> }        // nested
        | Table     { .. }                                    // S6 or later; see below
Run     = Text { text: String, style: Option<Name>, direct: Option<TextStyle> }
        | Break | Tab | Space(n)
        | Link { href, runs }
        | Bookmark { name } | BookmarkRange { name, .. }
        | Note { kind, id, body: Vec<Block> }                 // footnote / endnote
        | Field { .. }                                        // a small named set
```

Three decisions inside that, each of which is the kind of thing that is cheap now and expensive
later:

**Storage is `Vec<Block>` with a `String` per run, not a rope.** The spreadsheet built a
bespoke run-length column store because a million rows demanded one. Text does not have the
same shape: a paragraph is the natural edit unit and is rarely more than a few kilobytes, and
documents are many paragraphs rather than one enormous one. `Vec<Block>` also makes R6 splicing
per-paragraph natural, because a block *is* an element. The upgrade path — a rope or piece
table inside a paragraph, for the case where someone pastes a novel into one — gets a
`ponytail:` comment naming it, and nothing more, until a profile asks.

**Blocks carry stable ids alongside their index.** An index is what a user types (`p12`); an id
is what an undo entry, a splice registry and an observer notification carry, because an
insertion two blocks up must not silently re-target them. This is the one place where the text
model needs machinery the spreadsheet did not, and it is cheaper to have from block one than to
retrofit — the same argument that put `get_viewport` in phase 1.

**Tables inside text documents are a milestone of their own, deliberately late.** `table:table`
appears in both body types, and the temptation to share the reader is strong and wrong: an ODT
table's cells hold *blocks*, not values, and its column model is different. What is shared is
the element vocabulary, not the model. And there is a specific unknown to settle in
`doc/odt-format.md` before any of it: **do ODT table cells use OpenFormula?** LibreOffice
Writer appears to write its own dialect into `table:formula` under a vendor prefix rather than
`of:`. That is a claim to *verify by observation and cite*, per the clean-room rule, not to
carry into code from this sentence.

---

## The CLI

### Three levels

```
grind <app> <verb> [args]      grind sheet set book.ods A1 1
                               grind text  outline report.fodt
grind <verb> [args]            grind info anything.odt
```

Global flags stay global and stay where they are: `--format json`, `--session`, `--dry-run`.

**Suite-level verbs** operate on any ODF document, dispatching on `grind_core::kind`:

| Verb | Does |
|---|---|
| `grind info FILE` | What it is, what is in it, what this build can and cannot model in it. Today's `sheet info`, generalised, with the kind on the first line |
| `grind convert IN OUT` | Between the package and flat forms of the same type. Never between types |
| `grind validate FILE` | `jing -i` against the bundled RELAX NG, which today only tests can do. R2 made available to the user, which is exactly the kind of thing this project should expose |
| `grind undo/redo FILE` | Reads the kind, routes to the right `App` |
| `grind functions` | Stays — it is about the formula catalogue, and there is only one |

**`grind sheet`** is today's command set, verbatim, one level down: `new get view set paste fill
eval clear format style width height hide name filter add rename remove recalc calculations fmt`.
No verb changes meaning. `doc/cli-recipes.md` gains one word per line and nothing else.

**`grind text`** — the proposed verb set, with each one's spreadsheet analogue named, because a
verb with no analogue is a verb that needs justifying:

| Verb | Does | Analogue |
|---|---|---|
| `new FILE` | An empty document | `sheet new` |
| `view FILE [LOC]` | Print the text. `--plain`, `--marks` (show style names inline) | `sheet view` |
| `get FILE LOC` | One block. `--input` gives back what an editor would show | `sheet get` |
| `set FILE LOC TEXT` | Replace a block's content | `sheet set` |
| `insert FILE LOC TEXT` | Insert a block before `LOC`; `--after`, `--heading N`, `--list` | — |
| `delete FILE RANGE` | Remove blocks | `sheet clear` |
| `move FILE RANGE LOC` | Move blocks — **including a whole outline subtree** | — |
| `style FILE RANGE …` | Named style (`--style Heading1`) or direct formatting (`--bold`, `--size`) | `sheet style` |
| `outline FILE` | Every heading, its level and its address; `--filter` | **`sheet calculations`** |
| `formatting FILE` | Every block carrying direct formatting that overrides its named style | **the differentiator; see below** |
| `find FILE NEEDLE` | Addresses and context, one per line | — |
| `replace FILE NEEDLE REPL` | `--dry-run` shows what would change | — |
| `name FILE BOOKMARK LOC` | Define a `text:bookmark`; `--delete` | **`sheet name`** |
| `words FILE` | Words, characters, blocks, headings by level | `sheet info` |
| `export FILE --to md\|txt` | One way out | — |

`grind text` has **no `recalc`**, and that absence is worth stating: a text document has no
cached-value contract, so the whole staleness apparatus (`stale`, `spoiled`, the spoilage
banner) has no analogue and must not grow one by imitation.

A word processor that is *completely* drivable from a shell is genuinely unusual, and it falls
straight out of rule 4 rather than being a feature anybody had to want. It is also the half of
this project the README's "point a script — or an agent — at it" pitch cares about most.

### The parity ratchet, generalised

`cli/tests/parity.rs` today `include_str!`s exactly two paths and splits on `impl App {`. The
generalisation is small and is a **hard gate on this phase**:

- A registry — a const array of `(app name, core source path, parity doc path)` — replaces the
  two constants. Three lines to add a third app later.
- `doc/cli-parity.md` becomes `doc/cli-parity-sheet.md` and `doc/cli-parity-text.md`, same
  format, same "not exposed:" rule with a mandatory reason.
- The `methods.len() >= 12` vacuity guard becomes per-app, for the same reason it exists: a
  scanner that matches nothing passes and quietly retires the ratchet.

**The text app is not done until its parity document is complete and green.** That is R9, and it
is the mechanism that stops the GTK text shell from growing a feature the CLI cannot reach —
which is precisely how a suite turns back into LibreOffice.

### What breaks

`examples/sample.sh` (`$SHEET` → `$GRIND`, `sheet X` → `grind sheet X`), `cli/tests/cli.rs`,
`cli/tests/parity.rs`, `doc/cli-recipes.md`, `README.md`, `CLAUDE.md`, all five workflow files,
`Containerfile.distroless-cli`, and both `[package.metadata.*]` asset blocks. Every one of them
is a mechanical substitution, and every one of them is caught by CI if missed.

A second sample script arrives with S6: **`examples/sample-text.sh`**, which builds a document
out of every text feature the build supports, through the CLI and nothing else, run by
`cli/tests/cli.rs` and validated against the schema. The rule that made `sample.sh` valuable —
*a feature without a line there is invisible* — applies unchanged.

---

## The shells

### Every document type reaches every shell

This is **R10**, and it is the rule the whole Shared Core / Native Shell architecture exists to
make cheap. A suite whose word processor only has a window is a suite that has quietly decided
the window is the real product and the rest is a demo. The matrix is full:

| | `grind` (CLI) | `grind-tui` | GTK | `grind-web` |
|---|---|---|---|---|
| **sheet** | done | done | done | done, with named gaps |
| **text** | S6 | S8 | S9 | S10 |
| *slides* | *reserved* | *reserved* | *reserved* | *reserved* |

**Full does not mean identical.** `ui_web` already ships deliberate spreadsheet gaps — no point
mode, no styling controls, one column width — and that is fine and stays fine. What R10 forbids
is a shell that cannot **open and edit a document type at all**. A per-shell feature gap is
allowed and must be *named* in that shell's gap list; a missing cell in the matrix above is a
build failure.

Checked the way R9 is checked: the S2 registry gains a shell axis, and a test asserts every
`(document type, shell)` cell either resolves to a crate that depends on that type's core, or
carries a stated reason in `doc/shell-matrix.md`. Same mechanism as `doc/cli-parity-*.md`, same
"an unexplained exemption is how a ratchet quietly stops ratcheting" rule.

### One binary per document type — for GTK only

`grind-sheet-gtk` and `grind-text-gtk` are separate binaries with separate app IDs, separate
`.desktop` files, separate AppStream components and separate icons. The alternative — one
`grind-gtk` that switches on the open document — was considered and declined:

- A `.desktop` file's `MimeType=` is how a desktop associates an app with a file type, and one
  entry claiming both spreadsheets and text documents is what makes "Open With" useless.
- GNOME's HIG and Software both assume an application is one thing. Two entries in Software is
  the *correct* outcome, not a compromise.
- The existing packaging is already per-app shaped: `io.github.fwilhe2.Sheet.desktop`,
  `.metainfo.xml` and a scalable icon under that ID. `io.github.fwilhe2.Text` slots in beside
  it with no restructuring.
- LibreOffice's single-process model exists to amortise a startup cost this project does not
  have. Copying the mitigation without the problem is how suites get heavy.

**The TUI and the web shell do *not* split, and the reason is the desktop file rather than
taste.** Every argument above is about how a desktop associates a file type with an
application; none of it survives outside one. `grind-tui book.ods` and `grind-tui notes.fodt`
are the same invocation with different bytes, so the binary sniffs `grind_core::kind` and opens
the right mode — one binary, two modes. The web shell is the same: a document arrives from a
file picker as bytes with no path and no mime association, so one bundle dispatches on the
kind. Splitting either would be consistency for its own sake, and would make `grind-tui` a
worse tool for exactly the person who wants a suite in a terminal.

### `grind-ui`, extracted on evidence

The shared GTK crate is extracted **at S9, when the second GTK shell exists and shows the seam** —
not at S1 on speculation. Expected contents, all of which `ui_gtk/` already has in a form the
second shell will want verbatim:

theme tokens and the "every colour comes from the theme" rule · the observer bridge
(`async-channel` into the main loop) · `gtk::FileDialog` wrapping and `RecentManager` ·
the `ShortcutsWindow` built from the accelerator table · the about dialog · the
`Accessible::announce` helper that is M9's a11y floor · the `--render-to` PNG harness · the
spoilage/status banner shape · the style strip widgets that map onto `fo:` properties.

What stays per-shell: everything in `geom.rs` and `grid.rs`. A grid's layout arithmetic and a
text flow's have nothing to say to each other.

### Cross-app handoff

Opening a `.ods` in `grind-text-gtk` must not fail obscurely. `grind_core::kind` gives the
answer before any parsing, and the shell shows an `adw::Banner`: *"This is a spreadsheet."* with
an **Open in Sheet** button that launches the sibling binary, or says plainly that it is not
installed. Same in reverse. The CLI's equivalent is a one-line error naming the right
subcommand.

### The terminal shell, and why it comes first

**`grind-tui` gets the text document at S8, *before* the GTK shell — not after it, and not as a
port of it.** Three reasons, and the first is the one that matters:

**It is the cheapest complete editor for text that exists.** A terminal is a fixed-width
continuous flow: line breaking is counting characters, there are no font metrics, no glyph
shaping, no zoom, no device pixels and no page. Everything that makes the GTK text shell
expensive is absent, and what is left is exactly the part that has to be right — the editing
model, the block addressing from `loc.rs`, the undo granularity, the observer round trip. The
CLI proved the spreadsheet core before any window existed; the TUI proves the *editing* model
before the expensive window exists, which is the same trick one layer up.

**It is the honest test of the layout fork**, and it is a sharper one than the GTK shell.
Under Path A layout lives in the shell, so the TUI does its own trivial monospace wrapping and
nothing in the core has to know a terminal exists. Under Path B layout moves into the core —
and a core layout engine that can only express itself in font metrics cannot render into
character cells at all. So the TUI is what forces Path B's engine to be parameterised by a
metric provider rather than hard-wired to a font, which is a design constraint far better
discovered at S8 than at S10. If Path B is taken and the TUI becomes awkward, that is
information about the engine, not about the TUI.

**And it is what the suite is for.** A vi-modal spreadsheet earned its place because a grid in
a terminal is genuinely good. The same is true of prose, and more obviously so — a structured
document editor over a document format that is plain XML, driven from a terminal, is the thing
this project's whole thesis points at. The earlier draft of this document deferred it as "not
the demonstrated need". That was wrong: the demonstrated need is the architecture, and a shell
matrix with a hole in it is an architecture that has not been demonstrated.

Shape, following `ui_tui/`'s existing vi-modal design so the two document types feel like one
program:

| Mode | Sheet today | Text at S8 |
|---|---|---|
| **Normal** | `hjkl`/arrows, `g`/`G`, `Ctrl-f`/`Ctrl-b` | the same motions over blocks and words; `{`/`}` by paragraph, `[[`/`]]` by heading |
| **Insert** | `i`/`a`/`c` | the same keys, typing into a block |
| **Command** | `:w`, `:q`, `:recalc`, or a bare address to jump | `:w`, `:q`, `:outline`, `:style Heading1`, or a bare `loc` — `p12`, `#intro`, `§2.1.3` — to jump |

`:outline` is the navigation primitive and the TUI's version of the differentiator: the
outline pane is a split, headings are addresses, and `:move §3.2 §1` relocates a subtree. None
of it needs a page model, and none of it needs a font.

### The web shell

**`grind-web` gets the text document at S10**, after both native shells, for the reason phase 9
gave: the wasm shell is rule 5's honest test, and it is most valuable once there is something
proven to port. One bundle, dispatching on `grind_core::kind`. Its existing gaps list in
`doc/gtk-shell.md` grows a text section — gaps are allowed, absence is not (R10).

---

## Packaging

The suite shape, which is the shape every suite has for a reason:

| Package | Contains |
|---|---|
| `grind-cli` | `/usr/bin/grind`. No runtime deps beyond libc |
| `grind-tui` | `/usr/bin/grind-tui`, both document types. No runtime deps beyond libc either, which is the point of it |
| `grind-sheet-gtk` | binary, `.desktop`, metainfo, icon. `depends = "$auto"` |
| `grind-text-gtk` | the same, under `io.github.fwilhe2.Text` |
| `grind` | **meta-package**, depends on the four above and contains nothing |

`.deb` via `cargo deb` and `.rpm` via `cargo generate-rpm`, as today. The meta-package has no
Cargo equivalent — `cargo deb` builds from a crate — so it is either a hand-written `control`
stanza in `packaging.yml` or a stub crate with an empty asset list and a `depends` line. The
stub crate is uglier and keeps everything in one mechanism; recommend it, and note that
`generate-rpm` needs its `requires` block spelled out because it has no `$auto`.

**Mime types**, which is the detail that decides whether double-clicking a file works:

```
sheet: application/vnd.oasis.opendocument.spreadsheet
       application/vnd.oasis.opendocument.spreadsheet-flat-xml
text:  application/vnd.oasis.opendocument.text
       application/vnd.oasis.opendocument.text-flat-xml
```

The flat-XML types are the ones distributions most often get wrong, and `.fods`/`.fodt` are the
forms this project cares about most. `shared-mime-info` already knows all four; the `.desktop`
`MimeType=` lines just have to name them, and `StartupWMClass` has to match what GTK sets.

**Verify the media type strings against the source of truth at implementation time**, exactly as
`doc/ods-format.md` §10 instructs — grep LibreOffice's
`filter/source/config/fragments/types/*.xcu` for `MediaType` rather than copying the four lines
above into code from memory.

**Container.** `Containerfile.distroless-cli` carries `grind` instead of `sheet`, unchanged
otherwise. The GUI apps are not containerised.

**Flatpak** stays skipped, as it was in M9. If it is revisited: Flathub prefers one app ID per
manifest, so it is two manifests, not one suite manifest with two desktop files — which is the
same conclusion the binary split reached, for the same reason.

**CI.** `ci.yml`'s jobs (`build`, `roundtrip`, `loop_e`, `corpus`) become matrixed over apps
where the loop applies. `gtk.yml` builds two shells. `packaging.yml` builds four packages. The
`corpus` job's blobless sparse clone widens from `sc/qa/unit/data` to include Writer's test data
— which is much larger and mostly *not* ODF, so the sparse pattern and the loop's own file
filter both need to select `.odt`/`.fodt` rather than walking everything.

---

## Verification: the loops, extended

| Loop | Spreadsheet | Text |
|---|---|---|
| **A** — read tolerance | 359 documents, 0 failures | **Done.** 1763 documents in `sw/qa`, 1755 read, 4 password-protected, 4 independently confirmed not to be documents, 0 failures — and nothing special-cased, which is what §8 promised |
| **B** — formula conformance | 13327/52213, ratcheted | **No analogue exists.** There is no normative per-function corpus for text because there are no functions. This absence is why the scope line has to be invented rather than derived |
| **C** — round-trip differential | green both directions | **Done, green both directions.** 14 documents out (7 cases x 2 forms), 20 corpus documents and 5095 blocks back, 0 differences. One documented loosening — style names, because this writer declares no styles (`doc/odt-format.md` §5b). **Not the same pinned container**: that image is Calc-only, so the tests probe the oracle's capability and skip against it until it is rebuilt with Writer |
| **E** — generated differential | 913/1000 | No analogue. Generating random documents to compare *layout* is loop D's problem, not this one |
| **D** — layout differential | — | **New, and gated on the fork below.** See there |
| `kb.rs` — R7 | 14 vendored documents | Gains a text corpus: hand-written sparse `.fodt` (the thin side) and LibreOffice-authored ones normalised the same way (the thick side), with the same two-corpora reasoning R7 already records |

The text R7 corpus needs the same deliberate split the spreadsheet's has. The thin side is
hand-written against the spec: a paragraph with no style, a heading with no outline level, an
empty `text:p`, a `text:span` nested three deep. The thick side is LibreOffice's own output:
change tracking, fields, sections, a frame, a table, several hundred elements this build has no
model for — which is also R6's evidence, because regenerating them and reporting the byte
retention per document is what gives the phase a number to beat.

---

## The layout fork

**This is the product decision this document cannot make, and everything from S7 onward depends
on it.** Both branches are real; they lead to different programs.

### Path A — continuous flow, no page model *(recommended)*

The editor shows one reflowed column. Page size, margins, `fo:break-before`, headers, footers
and footnote placement are **read, preserved, round-tripped and never rendered as pages**.
Printing and PDF go through the platform, exactly as the spreadsheet already plans.

- **Layout lives in the shell, not the core.** GTK gets Pango for free; the web shell gets the
  browser; the CLI needs none. The core carries no font metrics and no new dependencies, and
  rule 6 ("no filesystem assumptions", and its spirit: no platform assumptions) survives intact.
- **Cost:** weeks, not years. **Delivers:** a usable structured document editor.
- **The honest label:** people will say it is a rich-text editor rather than a word processor,
  and they will be partly right. `README.md` should say so first, in the same voice
  `doc/not-doing.md` uses. A program that says what it is not is the whole thesis here.
- Every differentiator below works without a page model. None of them needs one.

### Path B — paginated

A real page model in the core: line breaking, font metrics, tab stops, widow and orphan control,
footnote placement, table splitting.

- **The terminal is the constraint that keeps this branch honest.** A core layout engine
  hard-wired to font metrics cannot render into character cells, so R10's terminal shell forces
  the engine to be parameterised by a metric provider from the start. Discover that at S8, not
  at S10.
- **Layout must move into the core**, because the CLI, the wasm shell and any future shell all
  need the same answer — which means taking on a shaping and metrics stack (`rustybuzz` +
  `fontdb` + `unicode-linebreak`, or `cosmic-text` over all three; all MIT/Apache and therefore
  AGPL-compatible). The project's existing ladder applies cleanly: **shaping and UAX #14 line
  break opportunities are neutral plumbing and can be reused; how `fo:` properties become layout
  constraints is semantics and gets written.**
- **Exit criterion is loop D**, and loop D is what makes this branch tractable at all: render a
  corpus document with our engine, render it with the pinned LibreOffice container to PDF,
  compare line-break positions and page boundaries within a tolerance, and ratchet the
  percentage exactly as loop B does. **The oracle being pinned to a container image by digest
  pays off twice here** — the digest pins the *fonts* as well as the renderer, and without
  pinned fonts a layout differential is noise.
- **Cost:** the largest single item in the project's history, larger than phase 4. **Risk:**
  this is the "thirty years of accreted edge cases" the project was founded to avoid, met head
  on.

### The recommendation

Build Path A, ship it, and put pagination in `doc/not-doing.md` §2 as *not yet, with a named
gate*: it moves when a real week of use proves the continuous view insufficient, and its exit
criterion is loop D at a stated floor. That is the same gate every other deferred capability
here has, and it keeps a feature list that ends.

---

## The scope line for text

`doc/small-group.md` is the spreadsheet's scope line and it was *extracted* from a normative
document. Text has no §2.3.2 to extract, so **`doc/text-core.md` must be written**, element by
element with a Part 3 schema reference each, and then made mechanical the same way: a
`text::implemented()` function checked against the document by a test. That test is the
anti-bloat rule for this half of the suite, and without it there is nothing stopping the word
processor from becoming Writer one reasonable-sounding element at a time.

**Proposed contents — the starting draft, and the single most important thing to get right
before S4:**

*Blocks:* `text:p` · `text:h` (outline levels 1–6) · `text:list` / `text:list-item`, nested ·
`table:table` (S6+) · `draw:frame` + `draw:image`, anchored as-char or to-paragraph only.

*Inline:* `text:span` · `text:a` · `text:line-break` · `text:tab` · `text:s` ·
`text:bookmark` / `-start` / `-end` · `text:note` (footnotes and endnotes) ·
`text:soft-page-break` (read, preserved, never authored) · a **named, small** field set —
`text:page-number`, `text:page-count`, `text:title`, `text:file-name`, `text:date`.

*Styles:* `style:style` families `paragraph` and `text` · `style:paragraph-properties`
(`fo:margin-*`, `fo:text-align`, `fo:text-indent`, `fo:line-height`, `fo:break-before/after`,
`fo:keep-together`) · `style:text-properties` (weight, style, size, colour, background,
underline, strikethrough) · `style:list-style` · `style:page-layout` + `style:master-page`,
**one master page**, read and preserved.

*Never* — the additions to `doc/not-doing.md` §1: sections and multi-column layout · text frames
as layout boxes · a drawing canvas · mail merge · forms and controls · bibliography and
citations · master documents · generated indices · `.docx` **writing** (the same asymmetry the
xlsx plan records, for the same reason) · WYSIWYG page view, if Path A is taken.

*Not yet, with gates:* tables in text (S6) · table of contents generation · comments
(`office:annotation` — arguably in, and genuinely used; decide with evidence) · Markdown
**import** (a one-way filter at the edge, exactly like xlsx, so it is allowed by the same
argument that allows that one) · change tracking, which stays in §1 "Never" for editing but
**must be preserved through a splice**, and that distinction is worth spelling out where a
reader will hit it.

---

## The differentiators

The spreadsheet's pitch has two things the big one does not do: formulas in plain English, and
*find everything that is calculated*. The word processor needs its own, and they should be
decided now rather than discovered, because they shape what gets built first.

**1. A word processor whose files live in git.** R6 applied to `text:p`: editing one paragraph
in a 500-page `.fodt` changes one line of XML, and opening a document to read it is not a
commit. Nothing else in this category does this. It is already built — it just needs pointing at
a different element.

**2. Find everything that is hand-formatted.** `grind text formatting` lists every block whose
direct formatting overrides its named style, with the address and the property. This is the
exact analogue of `sheet calculations`, it answers the question every shared document raises
("why is this paragraph different?"), and no mainstream word processor answers it well. In the
GTK shell it is `Ctrl+Shift+F`, the same key, the same searchable list, clicking through to the
block; in the TUI it is `:formatting`, the same list in a split. R10 means the differentiator
is not a GUI feature that the other shells hear about second-hand.

**3. The outline is the document.** `grind text outline` in the CLI; a persistent outline pane
in the shell; `grind text move §3.2 §1` moving a whole subtree by address. The address
vocabulary in `loc.rs` was designed for this, and `§2.1.3` surviving edits elsewhere is what
makes it scriptable.

---

## Milestones

Phase 10, in `doc/gtk-shell.md`'s shape: numbered milestones, each with an exit criterion, none
started before the previous is met. Sizes are relative, in `doc/plan.md`'s idiom.

| | Milestone | Exit criterion | Size |
|---|---|---|---|
| **S0** | **Names and the trademark note.** Run the two collision checks. Update `README.md`'s Trademarks section and this document's rejected list with whatever they return | **Repo renamed to `fwilhe2/grind`** (GitHub redirects the old URL), description updated, every in-tree reference moved. **The two checks below have still not been run** — no `/usr/bin/grind` exists locally, and that is the whole of the evidence so far | small |
| **S1** | **Extract `grind-core`.** The mechanical split above, plus `kind.rs` and `Observer`. No behaviour change | **Done.** R6 percentages unmoved to the digit; no test lost; R8's guard (`core/tests/generic.rs`) passing. `numfmt/` and the `Editor` trait deferred with reasons — see "What S1 actually did" | small |
| **S2** | **The `grind` CLI.** Three levels, suite-level verbs, the registry-driven parity ratchet, the full rename including env vars and the config-path fallback | Every existing CLI test passes under the new spelling; `parity.rs` reads a registry; `sample.sh` and `cli-recipes.md` updated and green | small |
| **S3** | **`doc/odt-format.md` and `doc/text-core.md`.** The clean-room spec and the scope line, before any text code exists | **Done.** Every structural claim cites the vendored RELAX NG by line; §5 (what LibreOffice does) is entirely `UNVERIFIED` and may not be implemented until it carries a citation | medium |
| **S4** | **The text model and reader.** `loc.rs`, `Block`/`Run`, block ids, contexts over the existing driver | **Done.** `implemented()` checked against `doc/text-core.md` by `text/tests/scope.rs`, and **loop A green over Writer's corpus**: 1763 documents, 1755 read, 0 failed, nothing special-cased. `GRIND_LO_CORPUS` now names the checkout **root**, so one clone serves both applications | large |
| **S5** | **The text writer, R6 splicing, loop C.** Both forms | **Done.** Writer in both forms; `package`/`manifest`/`esc`/`VERSION` extracted to `grind-core` now that a second caller pulls on them, with the sheet's R6 figures unmoved as proof it was a move; **R6 splicing works** — an unedited save returns the bytes exactly and editing one paragraph changes one line (`text/tests/diffable.rs`). Splicing is content edits only; a structural change regenerates, by one stated rule. **Loop C green both directions**, 0 differences, and it found a real writer bug on its first run (a literal tab or newline came back as a space) plus the fact that the pinned oracle has no Writer in it — `doc/odt-format.md` §5b | large |
| **S6** | **`grind_text::App` and CLI parity.** Every verb in the table above; `examples/sample-text.sh` | **Done.** `doc/cli-parity-text.md` green through the registry (R9); the sample script runs in `cli/tests/cli.rs` and its output is schema-checked. `undo`/`redo` are documented as not reachable — no session for text yet, and the reason is named | medium |
| **S7** | **The layout decision, executed.** Path A as recommended; Path B behind its gate | Path A: text renders as a continuous flow in the shell with no core dependency added. Path B: loop D exists and stands at a stated floor | A: medium · B: very large |
| **S8** | **The terminal text shell.** `grind-tui` opens both types off `grind_core::kind`; vi-modal over blocks; `:outline`, `:style`, bare-`loc` jumps. **Before the GTK shell, on purpose** — it is the cheapest complete editor and the sharpest test of S7's choice | Both types open in one binary; the editing model, block addressing and undo granularity are exercised end to end with no font metrics anywhere; `cargo test -p grind-tui` covers the keymap and the wrap arithmetic | medium |
| **S9** | **The GTK text shell.** Its own `doc/text-shell.md`, milestone by milestone, mirroring `doc/gtk-shell.md`. Extract `grind-ui` here | The shell opens, edits and saves; `--render-to` produces an assertable frame; `cargo test -p grind-text-gtk` covers the display-free half; the a11y floor matches M9's | large |
| **S10** | **The web text shell.** One bundle, dispatching on kind. Rule 5's honest test, now for the second type | `grind-web` opens and saves a `.fodt` with no path anywhere; `ui_web/smoke.sh` covers it; its gap list names what it does not do | medium |
| **S11** | **Package the suite.** Meta-package, two desktop entries, four mime types, AppStream, icons, the container, cross-app handoff, README rewritten as a suite pitch | `packaging.yml` produces all five packages; `reuse lint` green; `doc/shell-matrix.md` full and green (R10); double-clicking a `.fodt` opens the text app on a clean machine | medium |

Then **phase 11 — xlsx import**, `doc/xlsx-import.md` unchanged but for its number and the
`grind sheet import` spelling. It is renumbered rather than reprioritised: it is a separate
crate behind a feature flag and does not interact with any of the above, but the CLI rename it
would otherwise have to be redone under is cheaper to do first.

---

## Risks, honestly

- **Layout is the schedule, and it has no Part 4.** The single largest risk, addressed by the
  fork rather than by optimism. If Path B is taken and stalls, that is the honest signal to
  fall back to Path A rather than to keep going.
- **The scope line is invented, not extracted.** `doc/small-group.md` could be *derived* from a
  normative document, which is what made phase 4 bounded. `doc/text-core.md` is a judgement
  call, which makes it the most important and most fragile document in this phase. Mitigation:
  make it mechanical (`text::implemented()` against it) on day one, so drift is a build failure
  rather than a discussion.
- **The shared crate becomes a dumping ground.** Every shared-core project's failure mode.
  Mitigation: R8, with a test, from S1.
- **Writer's corpus is larger and messier than Calc's**, and mostly not ODF. Loop A should carry
  it by construction — that is what §8's default-ignore architecture is for — and if it does
  not, the architecture is wrong and gets fixed rather than the file special-cased. Budget for
  the sparse-clone and CI-time cost, which is real.
- **Two apps double the CI surface** — jobs, matrix entries, corpus clones, packaging targets.
  Worth watching before it becomes the reason a loop gets disabled "temporarily".
- **R10's matrix grows multiplicatively.** Two types across four shells is eight cells; a third
  type makes it twelve, and every cell is real work and real CI. This is the honest price of
  the rule, and it is worth paying — but it is also the reason `grind-ui` and the shared keymap
  crates matter more now than they did with one document type, and the reason a *named* gap has
  to stay a legitimate answer. R10 requires a shell to open a type, not to be finished.
- **The rename touches everything at once.** Mitigated by doing it before anything is released
  and by CI catching every miss, but it is one large mechanical commit and should be exactly
  that: mechanical, alone, and reviewable as a rename.
- **Feature pressure from the second app is different in kind.** Nobody asks a spreadsheet for
  mail merge. They will ask a word processor. `doc/not-doing.md` §1 needs its text section
  written *before* S8, not after, because the first person to see a cursor is the first person
  to ask.

---

## What does not change

Worth stating, because a restructuring document reads like a rewrite otherwise:

The clean-room rule and its spec-document-first discipline. R1–R7. The four loops and their
single documented loosening each. `ponytail:` as a tracked ledger. REUSE compliance and
AGPL-3.0-or-later. The Shared Core / Native Shell architecture and all six of its rules — the
windowed read, undo in the core, the core pushing and shells never polling with the lock
dropped first, CLI parity, no filesystem assumptions, and every feature surviving a LibreOffice
round trip. `doc/not-doing.md` as a product document rather than a backlog.

None of those is a spreadsheet rule. Every one of them is why a second document type is a
milestone list rather than a second project.

---

## Before any code: things to verify

Collected here so they are not buried in the prose above. Each is a claim this document makes
that should be *confirmed*, per the rule that a fact learned from LibreOffice goes into a cited
spec document before it reaches code.

1. The four media type strings, from `filter/source/config/fragments/types/*.xcu` (`doc/ods-format.md` §10 already says so).
2. That `number:*-style` is reachable from an ODT — table cells with `office:value`, and date/time fields — against the Part 3 schema, before `numfmt/` moves into `grind-core`.
3. Whether ODT table cells use OpenFormula or a vendor dialect in `table:formula`, by observation, cited.
4. Which `text:` elements LibreOffice actually writes for the constructs in `doc/text-core.md`'s draft, versus what the schema permits — the two differ, and §5.4's "what LibreOffice does not give back as written" has a text equivalent waiting to be found.
5. `grind` as a binary name in Debian and Fedora; GRIND in Nice classes 9 and 42.
6. Whether Writer's test corpus lives under one path or several, and how much of it is ODF, before widening the sparse clone.
