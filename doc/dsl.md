<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# The projection — documents as plain text, a generator that writes them, and a view that shows it

**Status: layer 0 is built for both document types, and layer 1 has begun. D0–D7, D9 and
D10's first row are done** —
`core/src/projection/`, `sheet/src/projection/` and `text/src/projection/` exist; **loop F is
green over 359/359 of loop A's spreadsheet corpus and 1755/1755 of its text corpus, with
nothing differing** (`sheet/tests/loop_f.rs`, `text/tests/loop_f.rs`); each grammar is held to
its own scope line by `doc/projection-sheet.md` and `doc/projection-text.md` with a test apiece;
and the projection is a `Form` — a third physical form beside the package and the flat file,
reached by `grind convert` and by every shell's save dialog rather than by an export verb of its
own; and **R6 holds for it** — `grind sheet set book.grind B1 4300` rewrites that value and
nothing else in the file, comments and hand alignment included, in both applications; and **every
shell shows the projection** (D9) — the grid or the page on one tab and its source on the other,
with the line the selection is on marked and moving in it selecting what that line projects.
**Layer 1 has its first half** (D7): `grind build model.rhai -o model.fods` runs a Rhai script
that *returns* a document and writes it — `grind-build`, the only crate in the workspace an
evaluator reaches, and the CLI is the only binary that links it (R11, checked). A script has no
filesystem, no network, no clock and no randomness, and is bounded; the same source produces the
same bytes, asserted by building `examples/budget.rhai` twice. `grind test` (D8) is not built.
§7's milestone table below records where each piece stands. `doc/plan.md`'s requirements and `doc/not-doing.md` outrank this document, and §2
is the argument that the feature does not contradict either. Phase 11 is spoken for
(`doc/xlsx-import.md`); the rest of this is a candidate for **phase 12**.

---

## 1. The idea, and the one decision that makes it tractable

JetBrains MPS stores a program as a tree and *projects* it — the same program shown as text,
as a table, as a diagram, edited through any of them. The ask is that shape for documents: a
plain-language form of a spreadsheet or a text document, writable in any editor, openable in
`grind` exactly as a `.fods` is, 1:1 with the subset this build supports and no wider.

Alongside it: loops, functions, unit tests and a linter, so that a thousand-row model is a
page of source rather than a thousand rows. And, once a document has a text form at all, that
form shown **live inside the shells** — the grid on one tab and its source on the other, the
way Delphi pairs a form with its `.dfm` (§6).

**Those are two features, and fusing them is the mistake to avoid.** A form that is 1:1 with
the model must be *bijective* — read it, edit it, write it back, and nothing has moved. A form
with a `for` loop in it cannot be written back at all: given a document whose row 400 was
edited in a GUI, there is no answer to the question *which iteration of the loop produced that
row*. Every "executable configuration" format in existence hits this wall, and the ones that
survive answer it the same way: **the computed form compiles to the declarative form, and only
the declarative form round-trips.** Typst source produces a PDF and is not recovered from one.
A Jsonnet file produces JSON and is not recovered from one.

So:

| | Name | Extension | Read | Written | Computes |
|---|---|---|---|---|---|
| **Layer 0** | **the projection** | `.grind` | yes | yes | no |
| **Layer 1** | **the generator** | `.rhai` | yes | **never** | yes |

The projection is a third *physical form* beside the package and the flat form
(`doc/flat-first.md`) — the same document, spelled differently. Every shell opens one, `grind
convert` moves between all three, and `grind_core::kind` sniffs which it is from the bytes.

The generator is a **source file that is compiled into a document**. `grind build model.rhai
-o model.fods`. It is not a document, does not open as one, and nothing ever writes one.

**The layer boundary is also what makes the language choice reversible.** The generator's
only output is a projection tree. Replacing Rhai with something else later changes one crate
and no file format, because the artefact was never the script — a property worth more than
picking the perfect language on the first try, and the reason §7's milestones ship layer 0
alone.

And layer 0 pays for itself twice. It is a *file format* on disk, and it is a **view** of an
open document — the same function, called on a document nobody saved. §6 is that half, and it
is the cheapest thing in this document because §3 already wrote it.

---

## 2. The macro line, and why this is on the right side of it

`doc/not-doing.md` §1 opens with:

> **Macros and Basic** — A scripting host is a second product with a second security model. A
> document that computes is the goal; a document that *executes* is not.

That row stays, unchanged, and this feature does not reopen it. The distinction is not
"is there a language" but **who runs it, when, and with whose authority**:

| | A macro | The generator |
|---|---|---|
| Lives | inside the document | in a source file beside it |
| Runs | when the document is **opened** | when the author types `grind build` |
| Triggered by | the reader, implicitly | a person, explicitly |
| Threat | *I open your file and your code runs* | *I ran a program I wrote* |
| Reaches | the document, the host, the network | a value tree, and JSON data in one directory a person named |

A `.fods`, `.odt` or `.grind` **never contains executable content**, and no code path that
opens a document may evaluate anything. That is a requirement, not an intention, and it needs
the same kind of mechanical guard R8 has:

> **R11 — no evaluator on any read path.** The generator crate is not a dependency of
> `grind-core`, `grind-sheet`, `grind-text` or any shell. `grind build` and `grind test` are
> the only binaries that link it. Checked the way R8 is checked (`core/tests/generic.rs`): a
> test reads the manifests and fails if any of them names it.

**Built with D7, in `build/tests/manifest.rs`**, and with one part the sentence above did not
ask for: the test also reads the *workspace's* member list and fails when a member is in
neither of its two lists. A shell added and not checked would leave the guard passing while
covering one crate fewer, which is how a ratchet quietly stops. `grind-cli` is the one crate
allowed to name it, and a third test asserts that it still does — a guard that passes because
the feature vanished is not a guard.

Two further restrictions on the generator itself, so that "a program I wrote" stays a small
claim:

- **No I/O, no clock, no randomness.** No filesystem, no network, no environment, no time.
  The same source produces the same bytes on every machine, which is a testability property
  before it is a security one.

  **Amended once, deliberately, and the amendment is narrower than the rule it replaces:** a
  script may read **data** — JSON, never code — from **one directory a person named**, which
  `grind build` roots at the script's own unless `--data` says otherwise. Separating what a
  document *is* from what is *in it* is most of why a generator is worth having, and the
  alternative is a `const` at the top of the script that only a programmer can edit. The four
  walls (JSON only; one directory, with `..`, absolute paths and symlinks out all refused;
  bounded in size and count; read-only and cached) are `build/src/data.rs`, and
  `doc/generator-spec.md` §3.5 is the reference. Determinism is restated rather than dropped:
  *the same source and the same data* produce the same bytes, which is a build system's
  contract. Nothing else about §2 moves — a script still cannot write, cannot reach the
  network, and still runs only when somebody types the command.
- **Bounded.** An operation limit, a call-depth limit and a string-size limit, all set by the
  host. A generator that does not terminate is a build error with a line number, not a hang.

---

## 3. Layer 0 — the projection

### 3.1 The container: KDL

The projection needs a syntax that is node-shaped (an ODF body is a tree), diffable
line-by-line, unambiguous to parse, and boring. Measured against the alternatives
(§5), **KDL** wins, and the deciding argument is not syntax at all:

**`kdl-rs` is a formatting-preserving document model — "`toml_edit`, but for KDL".** R6 says a
write must change as little as possible. This project has built retain-and-splice machinery
twice already (`sheet/src/odf/source.rs`, `text/src/odf/source.rs`); for the projection,
`kdl-rs` is that machinery. Probed directly against `kdl 6.7.1`: parse a document, change one
entry, re-print — **one line differs, and the comments are still there.**

Which settles a question the projection would otherwise have to answer awkwardly. A `.grind`
file will have comments, blank lines and hand-chosen alignment in it, none of which exist in
the ODF model. They survive an edit for exactly the reason `office:settings` survives an edit
to a `.fods`: **the writer never regenerates what nobody touched.** R6, applied to a second
format, with the same honest limit — a *structural* change regenerates and loses them.

Nine transitive crates, Apache-2.0 — **four**, as it resolved when it was actually added
(`kdl`, `miette`, `winnow`, `unicode-width`).

**What D5 found when it built that.** The promise above holds and the *route* to it was not the
obvious one, which is worth writing down because the obvious one looks like it works:

* **`kdl`'s mutation API does not reprint what you change.** Setting an entry's value leaves the
  printed output byte-identical, because a parsed entry retains the text it was spelled as and
  that text wins. `clear_format()` forces a reprint and also drops the whitespace around the
  value, so `row "North"  4200` comes back as `row "North" 4300` — a one-line diff that has eaten
  the alignment the file was keeping.
* **So D5 splices bytes**, at the spans `kdl` reports. That is the machinery
  `sheet/src/odf/source.rs` and `text/src/odf/source.rs` already are, it needs no second document
  model in memory, and it keeps `kdl` as the authority on the two things it is actually best at:
  where the spans are, and how a value is spelled.
* **The map is nearly free, which is the part that was predicted correctly.** The projection
  *reader* already computes an address for every node it walks — that is what makes the format
  bijective — so recording a span beside each one is bookkeeping rather than a pass.
* **An untouched save is byte-identical**, which is the strongest form of "the writer never
  regenerates what nobody touched" and is asserted over both corpora rather than argued for.

### 3.2 Where it lives — the `odf/` seam again, not a new crate

**R8 decides this, and it rules out the obvious shape.** A single `grind-projection` crate
would have to spell `sheet`, `cell`, `row`, `p`, `h` and `li` — both applications' body
vocabularies in one place — and would then have to depend on both app crates. `grind-sheet-gtk`
depends on `grind-sheet` alone today; a shared projection crate would drag the word processor
into the spreadsheet's window to get it.

So the projection splits exactly where `odf/` already splits, and for the same reason:

| Where | Holds |
|---|---|
| `core/src/projection/` | **\[GENERIC\]** — the KDL container, the span map, the token map, and the kind header below. No node name of either application |
| `sheet/src/projection/` | `sheet`, `at`, `row`, `cell`, `style`, `format`, `col` — beside `sheet/src/odf/`, which is its ODF twin |
| `text/src/projection/` | `p`, `h`, `list`, `li`, `image` — beside `text/src/odf/` |

One consequence worth stating: **a projection is a third reader and writer per app, not a third
crate.** `grind_sheet::App` gains `project`/`open_projection` the way it has `save`/`open`, and
nothing that depends on one application learns about the other.

### 3.3 The kind header, and why the file states it

`core/src/kind.rs` decides which document type some bytes are **before** parsing, sniffed from
content and never from the name — because a tolerant reader handed the wrong type returns an
empty document rather than an error. For ODF it reads `office:mimetype`.

The projection needs the same, and inferring it from the first node name (`sheet` → a
spreadsheet) would put document vocabulary in `grind-core`, which is exactly what R8 forbids.
So a projection **declares its kind in its first node**, as ODF declares its media type:

```kdl
grind spreadsheet
```

`kind.rs` maps three words — `spreadsheet`, `text`, `presentation` — onto the `DocumentKind`
variants it already owns, beside the three media types it already holds. Generic by
construction, and no parse of the body required.

### 3.4 A spreadsheet

Verified to parse against `kdl 6.7.1`:

```kdl
grind spreadsheet

// Q3 forecast. Comments survive an edit; see R6.
sheet Sales {
    at A1 {
        row "Region" "Q1" "Q2"
        row "North"  4200 4800
        row "South"  3100 3300
    }
    cell B5 "=SUM([.B2:.B4])"

    style  B1:C1 bold=#true background="#0074d9"
    format B2:C5 currency="EUR" decimals=2
    col 1 width="2.258cm"
    row 3 height="0.45cm"
}
```

Notes on the shape:

- **Formulas are strings, in ODF syntax, verbatim.** `"=SUM([.B2:.B4])"` is what the file
  holds and what `grind sheet set` already takes on a command line. No second grammar: the
  projection reader hands the string to `formula::lex` unchanged.
- **`at`/`row` and `cell` are two spellings of the same state.** The reader takes both; the
  writer emits one. Tolerance on the way in, strictness on the way out — the project's rule,
  applied to its own format. It also means loop F (§8) compares *models*, never bytes.
- **A range is a bare KDL identifier** (`B1:C1`), so styles and formats are stated once over
  the span they cover rather than per cell, which is both how a person thinks and how the ODF
  writer pools them anyway.

### 3.5 A text document

```kdl
h 1 "Field Notes"
p "Written entirely from a shell, which is **rather** the point."
h 2 "Addresses"
p "{#intro}p12 is a position. §2.1 survives edits above it."
li 1 "by position"
li 2 "invalidated by an insert above"
p #"a literal ** stays literal, and    four spaces survive"#
p "name\tvalue\nsecond line"
```

**What D2 changed about this sketch, and why.** Two things, both because the model won an
argument the sketch had not had yet — `doc/projection-text.md` is the built grammar:

* **A list is flat.** The sketch nested `li` inside a `list`. `model.rs` does not: the body is
  a flat sequence (rng:16938) and a list item carries a *depth*, so nesting would be a shape the
  reader has to invent and the writer has to fold. The reader still takes `list { li … }`,
  because somebody hand-writing a list will type it — the second spelling of one state, exactly
  as `at`/`row` and `cell` are on the sheet's side — and the writer only ever emits `li 2 "…"`.
* **There is no `image` node**, and that is the milestone's one named gap. See §3.8 below, which
  D2 amends.

Three things fall out of KDL's string rules that would otherwise each be a decision:

| ODF | Spelling | Why it is free |
|---|---|---|
| `text:s` | interior spaces in the string | KDL does not collapse whitespace; XML does |
| `text:tab` | `\t` | KDL's escape, in an escaped string |
| `text:line-break` | `\n` | likewise — and a *new block* is a new node, so the two are visibly different |

A raw string (`#"…"#`) turns the inline notation off, which is how a paragraph *about* markdown
is written.

### 3.6 The inline notation, and the one thing it must gain

Inside a string, character formatting uses **the notation this project already has** —
`grind_text::markdown`: `**bold**`, `*italic*`, `__underline__`, `~~struck~~`, `` `code` ``.
Reusing it is the point. `doc/tui-shell.md` and `doc/text-core.md` both argue that four shells
recognising `**` four ways would be four editors; a fifth reading in a file format would be
worse.

It cannot express everything a `CharStyle` carries — colour, family, size, highlight — nor a
link or a bookmark. The gap is filled by **borrowing Djot's attribute syntax**, which is the
best-designed answer to this exact problem in any light markup language:

```
[the words]{color=#ff4136 size=14pt family="Liberation Serif"}
[the words](https://example.org)      // text:a
{#intro}                              // text:bookmark
```

Borrowing the *syntax shape*, not the parser: `jotdown` parses whole Djot documents, and only
the inline layer is wanted here — the block layer is KDL.

**The real cost, named up front: `markdown.rs` becomes bidirectional.** Today it is a notation
for *typing* and deliberately never for *showing* — one direction, no escaping problem. A file
format needs the other direction too, and with it an escaping rule (a literal `**` in a
document must come back as a literal `**`). Raw strings cover the common case; a document that
mixes literal asterisks *and* formatting in one paragraph needs a backslash escape. This is
where the projection actually touches existing code, and it is the piece to prototype first.

**What D2 found when it did.** Three things, and the first is a correction to the sentence
above:

* **`markdown.rs` did *not* become bidirectional.** The other direction lives in
  `text/src/projection/inline.rs`, beside it. Parsing a whole string and printing one back are
  different questions from *what did the character just typed complete?*, and that module's
  contract — a notation for typing, never for showing — is one three shells depend on. What must
  not exist twice is the **table**, and it does not: `Emphasis::markers()` and
  `Emphasis::style()` are read from `markdown.rs` and nothing restates them.
* **One emphasis per marker pair; two at once is the attribute form.** `***x***` needs
  CommonMark's delimiter-run algorithm to be unambiguous, and a file format whose meaning
  depends on a heuristic loses documents. Bold-and-italic is written
  `[x]{bold=#true italic=#true}`, marker content never nests, and the parser is therefore total.
* **The escaping rule is narrower than "escape everything".** `_` and `~` open nothing unless
  doubled and `{` opens nothing unless a `#` follows, so `snake_case` is written as itself. The
  writer escapes exactly what would open something *there*, which is the same lookahead the
  reader uses, from the other side.

### 3.7 The grammar cannot drift from the subset

The requirement was "1:1 with **the subset we support** (which is still evolving)". That last
clause is the whole engineering problem: a hand-written grammar is a second scope line, and
two scope lines diverge.

They are not written twice. `doc/text-core.md`'s element table is already parsed by
`text/tests/scope.rs` and checked against `grind_text::implemented()`; `doc/small-group.md` is
already parsed and checked against `funcs::implemented()`. **The projection's node vocabulary
is checked against the same two lists by the same kind of test.** An element that enters the
scope line without a projection node fails the build, and a projection node with no element
behind it fails it too.

That is the mechanical answer to "the subset is still evolving", and it is the reason this
feature is affordable at all.

**What building it found: the spreadsheet has no element scope line, and needs a different
one.** For the *text* projection the paragraph above is literal — `doc/text-core.md`'s element
table is the vocabulary and `text/tests/projection_scope.rs` checks it exactly that way, so
every element `grind_text::implemented()` returns has a node, a piece of inline notation, or a
`gap:` with a reason. For the spreadsheet it cannot be. A formula reaches `formula::lex` as one verbatim string, so
`doc/small-group.md`'s 110 functions sit behind a single `formula=` property and are not a
vocabulary at all; and there is no `doc/ods-core.md` listing the elements a spreadsheet models.

What a spreadsheet has instead is a **model**, and the model's fields are the thing that grows.
So the spreadsheet's version of this rule is checked against `sheet/src/model.rs` itself: every
field of `Document` and `Sheet` has a node or a named gap, read out of the source at compile
time. That is the check that would have caught charts, and it is the shape
`doc/cli-parity-sheet.md` already uses for `App`'s methods.

`doc/projection-sheet.md` is the grammar note, and it is executable rather than descriptive:
each node's row carries a one-line **example**, and `sheet/tests/projection_scope.rs` reads
every example, projects the model it produced, and requires the node to come back. That last
step is what separates *accepted* from *carried* — a node the reader parses and then throws
away passes a parse check and fails this one. It caught its first mistake while the table was
being written.

### 3.8 The parts that are genuinely awkward

- **Images.** `Run::Image` carries decoded bytes. Base64 in a file whose selling point is that
  a human reads it is absurd. The projection writes `image "figures/plot.png"` — a path
  relative to the file — and keeps the bytes in a sidecar directory next to it. Which makes a
  `.grind` document *two* filesystem objects for the first time in this project, and that is a
  real cost. The alternative, a `data:` URI, is honest and unreadable. Sidecar, and named as a
  cost.

  **D4 took that answer away, and D2 records it rather than working around it.** A sidecar needs
  a *path*, and D4 made the projection a `Form` — reached through `write_bytes` and `read_bytes`,
  bytes and no path. Rule 5 says every `*_file` has a `*_bytes` twin, and `grind-web` is that
  rule's honest test: a browser tab cannot write a sidecar and cannot read one. So a form that
  only works when there is a directory to put things beside is not a form this project can have.
  All three options are now real — `data:` (unreadable), sidecar (needs a path), bytes-only and
  drop images (what D2 ships) — and choosing between them is an open question, written up in
  `doc/projection-text.md` and failing loudly the day it changes.
- **Number formats.** `numfmt::Format` is an ordered sequence of `Part`s, not a format string
  (deliberately — `doc/ods-format.md` §5.2). The projection must spell the parts, not invent
  Excel's `#,##0.00`. `numfmt::preset` covers the common case in one word (`currency="EUR"`);
  a format outside the preset vocabulary spells its parts out, which is verbose and correct.
  `Format::is_preset` already draws exactly this line for the GTK format picker.
- **Charts.** `Chart` carries a plot area, axes and per-point colours. Expressible, verbose,
  and nobody hand-writes one. It goes in for bijectivity, not for authoring.

---

## 4. Layer 1 — the generator

### 4.1 What it is not

Not a template engine emitting projection text. A template that interpolates a value
containing a `"` produces a syntactically broken `.grind` file, which is the injection problem
every stringly-typed generator has. The generator builds a **value tree** and the host
serialises it, so the question cannot arise.

Not a document mutator either. The script does not open a document and change it — that is
the macro shape §2 rules out. It returns a tree; `grind build` writes it.

### 4.2 The recommendation: Rhai

**Built — D7.** `grind build model.rhai -o model.fods`, in `build/` (`grind-build`), reached
from the CLI and from nowhere else. The sketch below is what this section proposed; the
subsection after it is what changed when it was built, and why.

```rhai
// Quarterly model. No I/O, no clock, no randomness — same source, same bytes.
const REGIONS = ["North", "South", "East", "West"];
const QUARTERS = ["Q1", "Q2", "Q3", "Q4"];

fn header(cells) { row(cells).bold() }

let s = sheet("Sales");
s.push(header(["Region"] + QUARTERS));
for r in REGIONS {
    s.push(row([r] + QUARTERS.map(|_| 0)).format(format("currency").symbol("EUR")));
}
s.push(header(["Total"] + QUARTERS.map(|q, i| sum_above())));
s
```

Why Rhai, against the measured field in §5:

1. **26 transitive crates**, MIT OR Apache-2.0, pure Rust, and it builds for wasm and
   `no_std` — which matters because `grind-web` exists and rule 5 forbids filesystem
   assumptions.
2. **Familiar syntax.** Braces, `let`, `fn`, `for … in`. The ask was familiar; this is
   JavaScript with Rust's keywords.
3. **Built to be restricted.** Keywords and operators can be individually disabled, looping
   can be turned off entirely, and the engine carries limits on operations, call depth,
   expression depth and string size. Filesystem and network are not in core Rhai at all —
   they are separate crates one simply does not take. This is the difference between
   *sandboxing a language* and *using a language designed to be reduced to a DSL*, and it is
   what §2's promises rest on.
4. **One language for generating and for testing.** `grind test` needs assertions over real
   values (`assert(cell("B12").value == 4200)`), which a template engine cannot express and
   which would otherwise be a second mechanism. Rhai gets a different function set per verb:
   `build` gets the builders, `test` gets the assertions and a read-only document.

#### What D7 found when it built that

**The dependency was 13 crates, not 26** — `rhai` with `default-features = false` and
`features = ["std", "no_time"]`, which is what `Cargo.lock` gained. Both flags are load-bearing
rather than thrift: `no_time` removes `timestamp()` from the *language*, so §2's "no clock" is a
fact about the build rather than a function this project remembered to unregister, and dropping
the default `ahash/runtime-rng` takes the operating system's seed out of the hasher, so a map
iterates in the same order on every machine. "The same source produces the same bytes" is then
checked by a test that builds `examples/budget.rhai` twice and compares them, rather than
argued.

**The host vocabulary is the projection's**, and that was not a decision this section made. A
generator that invented its own names for *sheet*, *row*, *style* and *format* would be a third
spelling of one model, and §3.7's argument about two scope lines applies word for word to two
vocabularies. So `doc/projection-sheet.md`'s nodes and attributes are the API:
`style().background("silver").align("center")` is that document's
`style … background=navy align=center`, and `format("currency").symbol("€").decimals(2)` is its
`format … currency symbol="EUR" decimals=2`. Which is why the sketch's `currency("EUR")` became
`format("currency").symbol("EUR")`: one constructor named after the node, and `currency`,
`date` and `number` left free for a *script* to use as its own names.

**`sum_above()` lost its argument.** The sketch passed the column (`sum_above(i + 1)`); the cell
already knows which column it landed in, and a helper that can be told a different one is a
helper that can disagree with where its answer sits. What it sums is the contiguous run of
numbers or formulas directly above — an AutoSum's rule, which is the only one that does not
reach up into a header and sum a heading.

**Styles layer here and replace in the core**, and both are right for who they serve.
`App::set_style` replaces because a toolbar's Bold button sets what a cell *is*, and a shell
wanting "bold as well" reads first. A script is not a toolbar: it says `row(…).bold()` and then
`style("A1:H1", style().background("silver"))` and means both, in the order a stylesheet means
both. Replacing there would silently drop the bold, which is exactly the kind of loss a
generated document is least likely to be checked for. The layering is `grind-build`'s, and what
reaches the core is one ordinary `set_style` per run of cells that agree.

**A script has types, so the host does no typing.** `App::enter`'s rule — `1800` is a number,
`TRUE` is a logical — exists because a person typing has only characters to say it with. In a
script `1800` is already an integer and `"2091"` is already a string, so re-deriving the type
from the spelling would be a second rule that can disagree with the first. The one guess kept is
the leading `=`, and even there both spellings are the suite's own: a `=` string is verbatim ODF
(what the file holds and the projection writes), and `formula("SUM(B2:B7)")` is the display form
put through `formula::display::from_display`, the converter already behind
`grind sheet fmt --from-display`. **A date *value* has no spelling and is a named gap**:
`App::enter` reads `2026-08-16` as a date only in a cell already known to hold one, which the
cells of a document being generated are not, so a generated date is `=DATE(2026;8;16)` under a
date format — what `examples/sample-sheet.sh` writes by hand.

**Every builder is a shared handle** (`Rc<RefCell<…>>`). Rhai copies values on assignment, so a
sheet pushed into a document and then written to again would have updated a *copy* — a script
that looks right and a document missing its last rows. This is the one implementation detail
worth stating out loud, because it is the difference between the API meaning what it reads as
and not.

**Runner-up: MiniJinja** (4 crates, Apache-2.0). If dependency weight turns out to dominate
everything else, Jinja2 syntax is more widely known than any other candidate's and the
projection is line-oriented enough to template well. It loses on the value model and on
`grind test`, and it would need KDL-aware autoescaping to be safe.

### 4.3 Linting

`grind lint` is worth building **whichever language wins, and it belongs to layer 0**, because
the interesting rules are about documents, not about scripts. No third-party linter knows what
a heading is.

**Built — D6.** The table below is the rule list, and it is *executable*: `cli/tests/lint.rs`
reads this section and checks every id against `grind_sheet::lint::RULES` and
`grind_text::lint::RULES`, in both directions. A rule with no row fails the build, and so does a
row naming a rule nothing implements — the mechanism `doc/small-group.md` uses against
`funcs::implemented()`, which is D6's exit criterion ("every rule named in a table and covered
by a test") made mechanical rather than promised.

| Rule | Id | Applies to |
|---|---|---|
| A heading level skipped (1 → 3) | `heading-skip` | text — after the first heading, which sets the baseline |
| A bookmark referenced and never declared | `unknown-bookmark` | text — an error: the link is dead |
| A style name used and never declared in `office:styles` | `undeclared-style` | text — this is `doc/text-core.md`'s known loss, made visible. `Document::styles` is the set of declared names, read and never written |
| A formula referencing an empty cell | `empty-reference` | sheet — single-cell references only; an empty cell inside a range is ordinary |
| A formula referencing a deleted sheet | `missing-sheet` | sheet — an error, and `doc/not-doing.md` §3's known bug seen from the other side |
| A cell whose cached value disagrees with its formula | `stale-value` | sheet — loop B's check, pointed at one document |
| A colour outside `style::PALETTE` | `off-palette` | both, as a hint, off by default |
| A construct the projection cannot spell | `unspellable` | both — the bijectivity guard, as a diagnostic. Charts for the sheet, images for text |

Two rows of the original table became two rules rather than one (`empty-reference` and
`missing-sheet`): they share a sentence and nothing else — one is an error and the other is a
warning, and a user who wants to silence "reads an empty cell" on a template full of them must
not thereby silence "reads a sheet that is gone".

The last row is the one that earns the feature: opening a LibreOffice document with change
tracking and three indices in it and being *told*, by name, what the projection will not carry.

**What a rule may not do is write.** Linting a document leaves its bytes exactly as they were —
the promise `doc/view-modes.md` makes for the overlays, for the same reason: a diagnostic stored
in a file goes stale, and a derived one cannot. `grind lint` exits non-zero on an *error* and
not on a warning or a hint, so CI can gate on a document contradicting itself without every
existing warning breaking a build.

### 4.4 Testing

`grind test model.rhai` builds the document, recalculates it, and runs the assertions:

```rhai
test "the total is the sum of the regions" {
    let d = build();
    assert_eq(d.cell("B6").value, 15_400.0);
    assert(d.lint().is_empty());
}
```

A spreadsheet whose totals are checked by a test that runs in CI is a genuinely new thing to
have, and it is the clearest single answer to *why bother with a generator at all*.

---

## 5. The field, measured

Dependency counts are `cargo tree --no-dedupe`, unique crates, resolved on 2026-08-29 against
current versions. **For scale: `grind-core` + `grind-sheet` + `grind-text` + `grind-cli` have
45 transitive dependencies between them today,** and the direct list is `quick-xml`, `zip`,
`serde`, `clap`, `unicode-linebreak`.

### Layer 0 — the container

| Candidate | Crates | Licence | Verdict |
|---|---|---|---|
| **KDL** (`kdl 6.7.1`) | **9** | Apache-2.0 | **Chosen.** Node-shaped, formatting-preserving (R6 for free), unambiguous, boring |
| Djot (`jotdown 0.10`) | **1** | MIT | Beautiful for prose, no vocabulary for a spreadsheet. Its *attribute syntax* is borrowed (§3.6); the crate is not taken |
| TOML (`toml 1.1`) | 7 | MIT/Apache-2.0 | Tables of tables for a document tree. Wrong shape |
| YAML | 9 | — | Significant whitespace, ten spellings of `true`, and a body of security history. No |
| Invent one | 0 | — | The serious alternative, and the project has form (`markdown.rs`, `numfmt::Part`, `loc.rs`). Rejected only because R6's splice is *already written* in `kdl-rs` and would otherwise be built a third time |

### Layer 1 — the generator

| Candidate | Crates | Licence | Notes |
|---|---|---|---|
| **Rhai 1.26** | **26**, and **13** as taken | MIT/Apache-2.0 | **Chosen**, and built — §4.2. The lock gained thirteen packages rather than twenty-six, because `default-features = false` drops the runtime-seeded hasher, which §4.2 wanted gone anyway |
| MiniJinja 2.24 | **4** | Apache-2.0 | Runner-up. Loops, macros, filters, a real sandbox, the most widely known syntax on the list. Stringly-typed output; no assertion story |
| Tera 2.3 | 3 | MIT | Same family, smaller community, same objections |
| Koto 0.16 | 37 | MIT | Well made, Rust-embedded, unfamiliar syntax, small community |
| jrsonnet 0.4 | 36 | MIT | Jsonnet is hermetic and pure — the right *semantics*. Its syntax is not familiar and large documents in it are painful to read |
| **Starlark 0.14** | **158** | Apache-2.0 | **The best semantics on the list, rejected on weight.** Frozen modules, hermetic by design, Python syntax, a linter in the box, Bazel and Buck2 as proof at scale. 158 crates is 3.5× this project's entire core. And `starlark-rust` deviates from the spec — recursion and top-level `for` are extensions, and `Dialect` has no flag to turn recursion back off, so the strongest reason to prefer Starlark (guaranteed termination) is not actually on offer |
| Rune 0.14 | 49 | MIT/Apache-2.0 | A general-purpose language with async. Wrong category |
| Nickel | **241** | — | Out on weight alone |
| Typst (`typst-syntax` 0.15) | 48 | Apache-2.0 | The closest thing to this feature that exists, and instructive: markup plus scripting, one direction only, source is never recovered from output — §1's argument, already proven in production. Taking `typst-syntax` gives a *parser* and no evaluator, so adopting Typst's language means writing Typst's interpreter. Out |
| KCL | — (C API) | Apache-2.0 | Schemas with `check` blocks, plus `kcl test`, `kcl lint`, `kcl fmt` — closest to the stated wish list. Embedding is via a C-ABI shared library rather than a Rust crate, which is a build-system dependency this workspace does not have |
| CUE, Dhall, Pkl | — | — | Go, no complete Rust implementation, and JVM respectively |

---

## 6. The code view — the projection, live, inside the shells

The projection is a *file format* in §3, and that is the smaller half of it. The larger half is
that a shell can show it **while the document is open**, next to the visual view, the way
Delphi puts a form beside its `.dfm` and Visual Studio puts a designer beside its XAML. That is
the MPS idea arriving where it started: one document, two projections, both live.

**Read-only to begin with**, and the reasons that is the right first cut are in §6.4.

### 6.1 What it costs, which is close to nothing

The projection *writer* is D1 and D2. A code view is that writer plus a widget that shows text,
and every shell already shows text:

| Shell | The view | Effort |
|---|---|---|
| `grind-cli` | `grind sheet project` / `grind text project` — the same bytes, printed | Exists the moment D1 lands. This is the R9 twin, and it lands *before* any shell |
| `grind-tui` | a pane, toggled by a key | Small — it renders text for a living |
| `grind-sheet-gtk`, `grind-text-gtk` | a `GtkTextView` in a `GtkStack`, with the stack switcher as the Delphi tab | Small |
| `grind-web` | a `<pre>` of `<span>`s, which `ui_web`'s line-cutting already builds for the text pane | Small |

**What it actually cost.** The table above was an estimate and it was close. What it missed is
that four shells would each have written the same *line cutting* — how many lines, what is on
one, which address a line belongs to — so those four questions moved into `Projection` before any
shell was written (`line_count`, `line_span`, `line_pieces`, `address_on_line`, plus `byte_at`).
Each shell then contributed one file: a pane in `grind-tui`, a `<div>` per line in `grind-web`, a
`gtk::TextView` with a tag per kind in each GTK window. `address_on_line` needed a rule that was
not obvious — see it for why "the narrowest anchor" answers a grid row with whichever cell had
the shortest value on it.

**Syntax highlighting comes from the writer, not from a highlighter.** The projection writer
knows what every byte it emits *is*; a highlighter would re-derive that with regexes and get it
wrong at the edges. So `project()` returns the text and a token map beside it, and no shell
takes `syntect` or `GtkSourceView`. Same argument `formula::display` already makes: the A1
formula bar is the existing lexer's output, not a second grammar.

### 6.2 The span map is the artefact worth building carefully

The toggle is the cheap half and it is not where the value is. Delphi and Visual Studio both
shipped the tabbed toggle first and both eventually shipped a **split** view, because what a
person actually wants is to see the *correspondence* — click a cell, see which line is it.

That needs one thing, in both directions:

```
address → source range     select B5 in the grid, highlight its line in the code
source range → address     put the caret on a line, select the cell in the grid
```

`kdl-rs` hands over the first half for free: it is a concrete syntax tree with spans on every
node, which is the same property that makes it formatting-preserving (§3.1). The map is
therefore mostly bookkeeping the projection writer does as it goes.

**And it is what every later IDE feature needs** — go to definition, find references,
diagnostics drawn under the line that caused them. Building it during the read-only milestone
is not throwaway work; it is the milestone's actual output, with the text view as the thing
that proves it correct.

### 6.3 It is an observer over a range, not a getter

Rule 1 says no getter hands out the whole document, and a code view looks exactly like one. The
tension is real and resolves the same way the grid's did:

- **`App::project(range)` takes a range**, and the code view's range is *its own scroll window*
  — not the grid's. A code view showing lines 400–460 asks for the blocks or cells behind them.
  The whole-document call is the degenerate case, and it is the CLI's.
- **The view is an `Observer`** (rule 3): the core pushes after dropping the write lock, and the
  view re-projects. It never polls, and it never holds a copy of anything.

*ponytail:* D9 may project the whole document once and cache it, invalidating on every
mutation — correct, obvious, and fine up to documents far larger than anyone hand-edits. The
ceiling is a document big enough that a keystroke re-serialises visibly; the upgrade is
projecting by range, keyed off the code view's own viewport, which the span map already makes
possible.

### 6.4 Read-only first, and what editable actually costs

An editable code view is not "the same view with input enabled". It is a text editor whose
buffer is a document, and there are exactly two ways to reconcile a free-text buffer with a
structured model:

1. **A modal *Apply*.** The two views disagree while you type, and the result is worse than
   either half on its own. No.
2. **Reparse and diff.** Parse the buffer, diff the model it describes against the live model,
   and apply the difference as ordinary `Action`s. Undo stays in the core (rule 2), the code
   view never mutates anything directly, and one edit is one `Action::Batch` — one Ctrl+Z,
   whatever it touched.

(2) is the right answer and it is a **feature, not a flag**. Three things it needs that nothing
here has:

- **An error-tolerant parser.** A half-typed line must not blank the visual view. `kdl-rs` is
  formatting-preserving but not error-tolerant in the way an editor needs, and *that specific
  gap is the first thing to check* before this is scheduled.
- **A model diff.** `Document` has no `diff`, and the naive one — regenerate everything —
  destroys R6 and every stable `BlockId` with it.
- **An answer for the other view's caret** while you type in this one.

> **Gate:** an editable code view waits for a shell where the read-only one has been used
> enough to say what editing it would be *for*. `doc/not-doing.md`'s rule, unchanged: one item
> at a time, by explicit decision, on evidence.

### 6.5 Refactoring — what an IDE feature means for a document

This is the part that looked like a lot of effort, and most of it turns out to be already
built. The reframing that makes it affordable: **a refactoring is a core operation returning an
`Action`, not a shell feature.** Rule 4 then puts every one of them on the CLI, and rule 2
makes a rename touching three hundred formulas one undo step — the argument
`set_char_style` already made for character formatting.

| IDE feature | For a document | Where it stands |
|---|---|---|
| **Go to definition** | a bookmark, a named expression, a sheet, a style name | The addressing exists — `loc.rs`'s `#intro` and `§2.1.3`, `Document::names`. `grind-web`'s Ctrl+K palette is already this, without the name |
| **Find references** | which formulas name this cell or sheet; which blocks point at this bookmark | The spreadsheet half is the dependency graph `eval.rs` already walks to recalculate |
| **Rename** | rename a sheet and rewrite every formula naming it; rename a bookmark, a style, a named expression | **Done for a sheet** — `formula::rename` is the AST rewrite, `App::rename_sheet` the one `Action::Batch` that carries formulas, named expressions *and* chart ranges, and `doc/not-doing.md`'s row now says *deleting* rather than "renaming or deleting". Never a textual substitution: `Sales` occurs inside `SalesTax` and inside `"Sales"`, and a name needing quotes is spelled differently before and after. A formula this build cannot parse is left alone and `grind lint`'s `missing-sheet` finds it. A bookmark, a style and a named expression are the same shape and not built |
| **Extract** | a repeated formula into a named expression | Named expressions exist; finding the repetition is the new part |
| **Inline** | a named expression back into the formulas using it | The inverse, same machinery |
| **Fold** | a section by outline level; a sheet | `Document::section()` and `outline()` already compute the extents |
| **Diagnostics, inline** | `grind lint` (§4.3) drawn under the line that caused it | D6 plus the span map, and nothing else |
| **Format document** | the writer's canonical spelling of what is already there | D1, free |
| **Symbol tree** | the outline | Exists; `grind-text-gtk` has the dialog and `grind-web` the palette |

**So the answer to "this seems like a lot of effort" is that it is not one project.** It is one
table row at a time, by `doc/not-doing.md`'s rule, and the first row is Rename because it fixes
something that is currently wrong. Nothing below it is scheduled, and nothing below it blocks
the read-only view.

### 6.6 What is in the core, and what each shell still writes

Everything in this document is a core capability except the generator, and the exception is a
*requirement* rather than an omission.

| Capability | Logic lives in | CLI | TUI | GTK ×2 | web |
|---|---|---|---|---|---|
| Reading and writing a `.grind` | `*/src/projection/` (§3.2) | ● | ● | ● | ● |
| The projection **as a view** — `App::project(range)` | same | ● | ● | ● | ● |
| Syntax highlighting — the writer's token map | same | — | ● | ● | ● |
| The span map, and selection sync both ways | same | — | ● | ● | ● |
| `grind lint` | core, per app | ● | ● | ● | ● |
| Refactorings — each one an `Action` | core, per app | ● | ● | ● | ● |
| **`grind build`** — the generator | **its own crate** (`grind-build`) | ● | ✗ | ✗ | ✗ |
| **`grind test`** | same | ○ (D8, not built) | ✗ | ✗ | ✗ |

Three qualifications, because "in the core" is not the same as "free in every shell". (● is
built, ✗ is by decision.)

`grind lint` landed in the CLI first and in the other three a day later, which is the row's own
evidence for the paragraph below: each shell owed a list and a way to jump to an address, and
nothing else. `ui_sheet_gtk/src/lint.rs` and `ui_text_gtk/src/lint.rs` are a dialog each — the
second a copy of the first with `loc` where it has `a1`, `code.rs`'s `ponytail` covering both —
and `ui_tui/src/problems.rs` and `ui_web/src/problems.rs` are one pane each, *shared by both
document types*, which `code.rs` is too but for a weaker reason: a projection is plain text
either way, and a diagnostic is document-type-neutral by construction. Four widgets, one
vocabulary, no rule anywhere but the core.

**Shared logic still leaves each shell a widget to write.** This is Path C's shape exactly:
`grind_core::layout` holds the line breaker, and three shells still implement `Metrics` and
still draw. Here the text, the tokens and the span map are the core's; the pane, the tab and
the highlight colours are the shell's. Small work, four times — not no work.

**The generator is CLI-only by R11, and that is the point.** No shell links an evaluator, so no
GUI can run a script. If a window ever wants a *Rebuild from source* button, it **runs the
`grind` binary** rather than linking the evaluator — one process, and R11 stays exact.
`grind-web` cannot do even that, being wasm in a browser tab, and that is an honest and
permanent consequence: a generator produces a document, and the browser opens the document.

**R10 is about document types, not features.** A shell that has no code view yet is a *named
gap*, which R10 permits and `doc/web-shell.md` and `doc/text-shell.md` already keep lists of.
So D9 can land in one shell first and finish in the others without failing a build — unlike
D4, where a shell that could not open a `.grind` at all *would*.

---

## 7. Milestones

Layer 0 ships alone and is useful alone. The generator is a late milestone on purpose:
its language choice is reversible (§1) and layer 0's bijection is not.

| | What | Done when | Status |
|---|---|---|---|
| **D0** | This document, plus the grammar note derived from the two scope lines (§3.7) | The names are settled and the vocabulary check is designed | **done** — `doc/projection-sheet.md` and `doc/projection-text.md`, with `sheet/tests/projection_scope.rs` and `text/tests/projection_scope.rs`. The two are checked *differently*, and the note under §3.7 is why |
| **D1** | `core/src/projection/` (generic) + `sheet/src/projection/`: KDL ⇄ `grind_sheet::Document` (§3.2) | Loop F green over `sheet/tests/data/kb/` and `data/samples/`; `core/tests/generic.rs` still passes | **done** — and `generic.rs` gained a third guard, `no_projection_node_name_is_spelled_in_the_shared_crate` |
| **D2** | `text/src/projection/`, including the bidirectional inline notation (§3.6) | Loop F green over `text/tests/data/` | **done** — and green over `sw/qa` too: 1755/1755, nothing differing. The notation went beside `markdown.rs` rather than into it, and §3.6 records why. Images are the one named gap, and §3.8 records what took its answer away |
| **D3** | Corpus scale | Loop F over loop A's whole corpus — 359 sheets, 1755 texts — with a `FLOOR` that ratchets | **done** — 359/359 and 1755/1755, nothing differing, `FLOOR = 359` and `FLOOR = 1755` |
| **D4** | `grind_core::kind` sniffs it; `App::open_bytes` accepts it; `grind convert` reaches all three forms | Every shell opens a `.grind` with no shell change (rule 5, R10) | **done** — `Form::Projection` is the third arm of the form enum, so `grind convert book.fods book.grind` and back both work, for either application. No shell needed a change to *open* one; the two that pick a file gained a pattern so the user can reach it |
| **D5** | R6 for the projection: splice at the spans `kdl-rs` reports | One cell edited changes one line; comments survive | **done**, both document types — `grind_core::projection::Source` plus a splice per app. An untouched save returns the bytes that were read, over both corpora. Not through `kdl-rs`'s *mutation* API, and §3.1 records why |
| **D6** | `grind lint`, suite level, rules per app (§4.3) | Every rule named in a table and covered by a test | **done**, both applications — eight rules, five per app with two shared, each in §4.3's table and each with a test. `grind sheet lint` / `grind text lint`, plus `grind lint` at the suite level, which reads the kind out of the file. What a diagnostic *is* is `grind_core::lint` (R8); every rule is asked through machinery that already answers its question, so the linter cannot disagree with the document's own behaviour |
| **D7** | `grind build` — Rhai, the host API, the R11 manifest check | `examples/sample-sheet.sh`'s document, generated | **done** — `build/` (`grind-build`), `grind build model.rhai -o model.fods`, **both document types**. `examples/budget.rhai` is the exit criterion: the budget's table, its formulas, formats, styles, widths, names and second sheet, with the categories as data and a loop for the rows, asserted in `cli/tests/cli.rs` and run by `examples/sample-sheet.sh`. The host vocabulary is `doc/projection-sheet.md`'s, `sum_above()` lost the argument the sketch gave it, styles layer where the core's replace, and the dependency was 13 crates rather than 26 — §4.2's own subsection is the record |
| **D8** | `grind test` (§4.4) | A generated document's totals asserted in CI | not started |
| **D9** | The **read-only code view** and the span map (§6) | Every shell shows it, selection syncs both ways, and `grind <app> project` is its CLI twin | **done, all four shells.** `grind <app> project` was the CLI half; `:source` in `grind-tui`, *Show the source* in `grind-web`, and Ctrl+Shift+U on the other page of a `gtk::Stack` in both GTK windows. Selection syncs both ways in each. Editing it is still gated (§6.4) |
| **D10** | Refactorings, **one at a time**, starting with rename-a-sheet (§6.5) | Each one is an `Action`, reachable from the CLI, undone by one Ctrl+Z | **first row done** — rename-a-sheet, which closed a documented bug rather than adding a feature. One `Action::Batch` reached from `grind sheet rename` and from all four shells, undone by one Ctrl+Z, and it counts what it rewrote so a document-wide edit is visible. The rest of §6.5's table is open and moves one row at a time, on evidence |
| **D11** | The projection's **specification** — the language, not the vocabulary (below) | Every production has an example a test runs; somebody with only that file can hand-write a `.grind` that opens | not started |
| **D12** | The projection's **guide** — writing one by hand, keeping it in git, what converting to it drops | Every snippet is a real command with its real output, and the page runs top to bottom | not started |
| **D13** | The generator's **specification** — the Rhai dialect, its limits, and the whole host API | The API reference is checked against what `engine()` registers, in `doc/small-group.md`'s shape, both directions | **done** — `doc/generator-spec.md`, with `build/tests/spec.rs` holding it to the code in three ways: every registered function has a row, every documented function is registered, and every limit in §2.4 carries the constant's own value. §7 of it is the list of what a generator cannot say and what to do instead |
| **D14** | The generator's **guide** — from a first script to `examples/budget.rhai` | Every script in it is a file under `examples/` that `cli/tests/cli.rs` builds | not started |

D1–D5 are the feature. D6–D8 are the reason to want it. **D9 is the cheapest milestone on the
list and possibly the most visible** — it needs no new dependency, no new format and no core
change beyond one range-taking method, because D1 already wrote the hard part.

D10 is not a milestone in the sense the others are: it is an open table (§6.5) whose rows move
by `doc/not-doing.md`'s rule, on evidence, one at a time.

### D11–D14 — the two languages owe a specification and a guide each

**This project has invented two languages and documented neither of them as a language.** What
exists is a design record: this file argues *why* they are shaped as they are,
`doc/projection-sheet.md` and `doc/projection-text.md` hold each grammar's vocabulary to the
model, and the doc comments in `*/src/projection/` and `build/src/` say what each function
does. None of that is what somebody writing a `.grind` or a `.rhai` needs. A person who wants
to hand-write a projection has to read a Rust module to find out whether `#true` is spelled
that way, and a person writing a generator has to read `build/src/sheet.rs` to find out that
`format("currency")` exists at all.

**Two documents per language, and they are different documents on purpose.** A specification
answers *what is legal and what does it mean*, exhaustively, for somebody implementing against
it or arguing with an edge case. A guide answers *how do I do the thing I came here to do*, by
example, in the order a person meets the problems. Merging them produces a document that is
too long to read and too vague to settle an argument, which is the failure mode
`doc/cli-recipes-sheet.md` and `doc/cli-parity-sheet.md` already avoid by being two files.

**The house rule applies to all four: a document that nothing checks drifts.** So each one
carries the mechanical check its genre allows, in the shape §3.7 and §4.3 already use — an
example that is executed rather than illustrated, a vocabulary read out of the source, or a
snippet whose output is the real one.

| | What | Done when |
|---|---|---|
| **D11** | **The projection's specification** — `doc/projection-spec.md`. The *language*, which the two grammar notes deliberately are not: the KDL subset in use and what is excluded from it, the `grind <kind>` header line, how a node, an argument and a property map onto the model, the value spellings (`#true`, `#null`, a number, a quoted string, a bare word) and which is required where, addresses and ranges, the inline notation's escaping rule from both sides (§3.6), what a reader must reject rather than tolerate, and the bijectivity contract with its two named gaps. It **references** `doc/projection-sheet.md` and `-text.md` for the vocabulary rather than restating it — one scope line, still | Every production has an example the test runs, in `doc/projection-sheet.md`'s shape; the escaping rule's table is executed against `inline.rs` in both directions; a reader who has only this file can write a `.grind` that opens |
| **D12** | **The projection's guide** — `doc/projection-guide.md`. Task-shaped and short: write one by hand, keep it in git and read the diff, edit one cell and see one line change, convert to and from the other two forms, open it in every shell, and the honest list of what converting *to* it drops. The audience is somebody who liked `doc/flat-first.md`'s argument and wants to act on it | Every snippet is a real command with its real output, as `doc/cli-recipes-sheet.md` is; the whole thing is runnable top to bottom |
| **D13** ✅ | **The generator's specification** — `doc/generator-spec.md`, written. Two halves. The *dialect*: which Rhai (version, the features taken and left), which constructs a script may use, every limit with its value, and exactly what determinism is promised and on what it rests. The *host API*: every registered function with its arity, its argument types, its return, and its errors — plus the rules that are not visible in a signature, which are the value-mapping table, the two-pass materialisation order, the layering of styles, and `sum_above()`'s run | The API reference is checked against what `engine()` actually registers, the way `doc/small-group.md` is checked against `funcs::implemented()`: a function documented and not registered fails the build, and so does one registered and not documented. Every limit's value is read from `build/src/engine.rs` rather than retyped |
| **D14** | **The generator's guide** — `doc/generator-guide.md`. From a first three-line script to `examples/budget.rhai`, in the order the problems arrive: data at the top, a loop for the rows, a function for a repeated shape, formatting, several sheets, a text document, and reading an error message with a line number in it. It ends where §2 does — what a script may not do, and why that is the feature rather than the limitation | Every script in it is a file under `examples/` that `cli/tests/cli.rs` builds, so a guide that stops working fails the build rather than the reader |

**D13's check was the load-bearing one**, and it is why these were started in that order: the
host API is the surface most likely to grow a function nobody writes down, because adding one is
three lines in `register()`. The projection had §3.7's guard already; the generator had nothing
equivalent, and D13 is where it got one. Written and passing — and it bites, which was checked
the only way worth checking: by breaking it three ways and watching each one fail.

The other three are not scheduled. One at a time, by `doc/not-doing.md`'s rule.

---

## 8. Verification

The loops are how anything is believed here, and this feature brings a natural one.

**Loop F — the projection differential.** For every document loop A already reads: project it,
read the projection back, and assert the two models are identical. It runs over 2114 documents
on day one, at zero corpus cost, and it ratchets exactly as loop B does. A document that
cannot round-trip is either a scope-line gap (name it) or a bug (fix it), and the difference
is what the counter measures.

Built, in `sheet/tests/loop_f.rs`, and at **359/359 of the spreadsheet corpus with nothing
differing** — plus R7's fourteen vendored documents, which never skip. It compares in both
directions: `document → projection → document` catches a writer that drops something, and
re-projecting the result catches a *reader* that drops it instead. One named exclusion, charts
(§3.8), with a test that fails the day they are projected so the exclusion cannot outlive the
gap.

That is the bijection proof, and the whole of layer 0 stands on it.

Everything else is an existing check pointed at a new format:

| Check | Applied |
|---|---|
| **R2** | projection → ODF → `jing -i`. The projection writes through the existing writer, so this is nearly free |
| **R6** | one cell edited in a `.grind` changes one line — `kdl-rs` measured, see §3.1 |
| **R9** | `doc/cli-parity-sheet.md` and `-text.md` gain rows for the new verbs |
| **R10** | every shell opens a `.grind`, because `kind` decides and nothing else does |
| **R11** | a manifest test, in the shape of `core/tests/generic.rs` |
| Scope | the projection's vocabulary against `implemented()`, both crates (§3.7) |
| Loop C | unchanged and unaffected — the projection is never LibreOffice's problem |

**The code view needs no loop of its own, which is the point of building it on the projection.**
It shows what `project()` returns, and loop F already asserts that what `project()` returns is
the document. A code view that disagreed with the grid would be a loop F failure before it was
ever a rendering bug. The span map gets an ordinary test — every address in a projected
document maps to a range whose text contains it, and back — in the shape of
`ui_sheet_gtk/src/geom.rs`'s: arithmetic with no toolkit in it, so no display is needed.

---

## 9. What this will not do

| Not doing | Because |
|---|---|
| **Put a script inside a document** | `doc/not-doing.md` §1, unchanged and unweakened. R11 is the guard |
| **Recover a generator from a document** | Decompilation. §1 — the arrow points one way, as it does for Typst and Jsonnet |
| **Make the projection a general-purpose data format** | It spells this build's subset and nothing else. Widening it is widening the subset, one item at a time, by the `doc/not-doing.md` rule |
| **Standardise it** | It is this project's third serialisation, not a proposal to anyone |
| **A diagram projection** | MPS's other axis, and the honest answer is that a spreadsheet grid *is* the diagram projection and it already exists |
| **Import a script from anywhere but the project directory** | An `import` that reaches a URL is the supply chain this project does not have. Modules are off entirely (`set_max_modules(0)`), and the one thing a script *may* read is **data, not code**, from one directory: `doc/generator-spec.md` §3.5 |
| **Preserve unmodelled ODF through the projection** | It cannot: a construct with no projection node is not in the file. R6 preserves it in a `.fods`; converting *to* a `.grind` drops it, exactly as regenerating does today. `grind lint` says which, by name, before it happens |
| **A general-purpose text editor inside a shell** | The code view shows the projection of *this* document and nothing else. It does not open other files, it is not a place to keep notes, and it never grows a second buffer. That is somebody else's program and it is already installed |
| **Tooling for the generator** — an LSP, a formatter, a debugger for `.rhai` | Rhai has its own language server and its own ecosystem. Writing a second one is how a document project becomes a language project |

---

## 10. Risks, honestly

**The bijection is a much stronger claim than anything the readers make.** Tolerance means an
unrecognised element is inert; a *projection* means every modelled construct has a spelling and
the spelling means one thing. §3.7's generated vocabulary check is load-bearing, not
decorative, and if it turns out not to be mechanisable, this feature is a maintenance burden
rather than a differentiator.

**The inline notation is the piece most likely to hurt.** Making `markdown.rs` bidirectional
adds an escaping rule to a module whose current simplicity comes from having none. Prototype
it in D2 before committing to D3.

**Two formats to keep in sync as the subset evolves.** Mitigated by §3.7, not eliminated —
every new element is now two pieces of work, and the second one is easy to forget until CI
says so.

**The generator is a new attack surface behind one verb.** Small, bounded, and real. The
mitigation is that it is one verb, one crate and no read path (R11), and that `grind build` is
something a person types.

**The sidecar directory for images** breaks "a document is a file", which every other form in
this project honours.

**The code view invites the editable code view, and that is where effort goes to die.** A
read-only pane is a weekend; a two-way one is an editor with a reparse-and-diff pipeline under
it (§6.4). The pressure to "just enable typing" will arrive the day the pane ships, and the
mitigation is that §6.4 already wrote down the three things it would need — so the answer is a
list rather than an argument.

**Four shells showing the projection is four places for it to look different.** The mitigation
is the one this project already uses twice: the *writer* emits the token map, so a shell
chooses colours and nothing else. A shell that highlights on its own is the `**` problem again,
one layer up.

**And the one worth stating plainly: this may be the best thing here.** `doc/flat-first.md`
argues that office documents living in git is the project's genuine differentiator. A
spreadsheet that is a page of readable text, generated from a script, linted in CI and with
its totals asserted by a test, is that argument finished. It is also a large feature, and the
milestone order above exists so that the useful half ships before the expensive half is
started.
