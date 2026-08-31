<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# The generator's language — the specification

**Normative for `build/` (`grind-build`).** `doc/dsl.md` §4 is the *argument* — why there is a
generator at all, why Rhai, and why it is on the right side of `doc/not-doing.md`'s macro line.
This is the *reference*: what a script may say, what each thing means, and what it may not do.
`doc/dsl.md` outranks it on any question of intent; on a question of behaviour, the check in §8
outranks both, because it reads the code.

A guide is a different document and is not written yet (`doc/dsl.md` §7, D14). This one is for
somebody arguing with an edge case, not for somebody learning. Everything here is true of the
build in this repository; `examples/budget.rhai` and `examples/report.rhai` are the worked
examples, and `build/tests/smoke.rs` executes most of what follows.

---

## 1. What a script is

A **source file that is compiled into a document**:

```sh
grind build model.rhai -o model.fods
```

It is not a document, does not open one, and nothing ever writes one — the arrow points one
way, as it does for Typst and for Jsonnet (`doc/dsl.md` §1). `grind build` is the only verb in
the suite that runs a script, and `grind-cli` is the only crate in the workspace that links the
evaluator (**R11**, `build/tests/manifest.rs`).

A script is a sequence of Rhai statements. **Its value is the value of its last expression**,
and that value must be a document:

| Returned | Meaning |
|---|---|
| `Spreadsheet` | a spreadsheet, from `spreadsheet()` |
| `Sheet` | a one-sheet spreadsheet — sugar, so a script that builds one sheet may end with it |
| `Text` | a text document, from `text()` |

Anything else is an error naming what came back. The **kind of document is decided by what the
script returned**, never by the output's name — the same rule `grind_core::kind` follows on the
way in. The output's *extension* picks the physical form (`.ods`/`.fods`, `.odt`/`.fodt`,
`.grind`), exactly as it does for `grind convert`.

---

## 2. The dialect

### 2.1 Which Rhai

[Rhai](https://rhai.rs) **1.26**, taken as `default-features = false` with `features = ["std",
"no_time"]`. **The language is Rhai's own, as its book documents it**; this section is
exhaustively the difference between that language and this one. Where Rhai's documentation and
this file disagree about a construct, the difference is a bug in one of them and §2.3's table is
the list of intended ones.

The version is pinned in `build/Cargo.toml` and, exactly, in `Cargo.lock`. A script is not
promised to survive a Rhai upgrade that changes the language; the *documents* it produced are,
because they are ODF and have nothing to do with Rhai.

### 2.2 What a script may use

Everything in core Rhai: `let`, `const`, `if`/`else`, `while`, `loop`, `for … in`, `switch`,
`fn`, closures, arrays, object maps, strings and string interpolation, integers, floats,
booleans, `()`, and the operators over them.

### 2.3 What is not there, and how it is not there

**How** matters as much as what: three of these are absent from the *build*, which no line of
code can undo by accident, and three are turned off in one function that is deliberately short
enough to read in one sitting (`build/src/engine.rs`).

| Not available | How | Why (`doc/dsl.md` §2) |
|---|---|---|
| The network, the environment | never in core Rhai; the crates that add them are not taken | a generator reaches a value tree and nothing else |
| The filesystem — **except reading data** | the same, plus one function this project registers: `json(…)`, §3.5, which reads JSON from one directory a person named and nothing else | separating what a document *is* from what is *in it* is most of why a generator is worth having; the walls are what keep it an exception |
| Randomness | the same | the same source must produce the same bytes |
| `timestamp()`, and every clock | the `no_time` **feature** removes it from the language | as above, and a fact about the build rather than about this project remembering to unregister a function |
| Map iteration seeded by the OS | `ahash/runtime-rng` **off**, by taking Rhai's default features off | determinism at the level of the language |
| `eval` | `Engine::disable_symbol("eval")` | a script that rewrites itself cannot be read to know what it does |
| `import`, and modules | `Engine::set_max_modules(0)` | `doc/dsl.md` §9 — an `import` that reaches a URL is a supply chain, one that reaches a path is I/O |

There is **no way to read the document being built**. A script says what a document is; it does
not ask. (`sum_above()` is not an exception — see §6.3: the script does not receive the answer,
the host resolves it.)

### 2.4 Limits

Every limit is the host's, set in one place, and named so an error can be recognised. A script
that exceeds one is a **build error with a position**, never a hang and never an out-of-memory
kill.

| Constant | Value | Trips when | Rhai reports |
|---|---|---|---|
| `MAX_OPERATIONS` | `10_000_000` | a script runs too long — a loop that never ends, or a real script far larger than any document needs | *Too many operations* |
| `MAX_CALL_DEPTH` | `64` | a function recurses without bottoming out | *Stack overflow* |
| `MAX_STRING` | `1_000_000` | a string grows past a megabyte; `s += s` doubles, so this is what the operation limit alone would not catch | *String too large* |
| `MAX_ARRAY` | `1_000_000` | an array or an object map grows past a million entries | *Array too large* / *Map too large* |

Two more bounds belong to the *document* rather than to the language and are §6.5's, and two
belong to the data a script reads and are §3.5's. All eight are read out of the source by
`build/tests/spec.rs`, so a number here is the number the build uses.

`examples/budget.rhai` uses on the order of four thousand operations, which is the scale these
numbers are set against: far enough above a real script to be invisible, far enough below
forever to stop in about a second.

### 2.5 Determinism

**The same source produces the same bytes.** Not merely the same document: the same file, byte
for byte, on any machine, at any time, in any order of environment. It rests on exactly three
things — no clock, no randomness, and no OS-seeded hashing (§2.3) — and on the writer being
deterministic, which it already is because R6 and loop C both depend on it.

Checked rather than promised: `cli/tests/cli.rs` builds `examples/budget.rhai` twice and
compares the two files.

What is **not** promised: that two *different* builds of `grind` produce the same bytes for the
same script. A change to the writer, to a number format's spelling or to the ODF the project
emits will change the output, and that is what loops C and F are for.

### 2.6 Errors

An error is reported as

```
script:line:column: message
```

The position is Rhai's, and it is the position of the call that failed — so a host function
handed a bad argument reports the line of the script, not of `build/src/`. Nothing is written
when a script fails: `grind build` writes the document only after the script has returned one.

**A failure while turning the returned tree into a document has no position**, and the message
says what and where in the *document* instead (`sum_above() at B8: …`). By then the script has
finished and the fault is in what it asked for rather than in a line that asked. §6 is the list
of what can fail there.

### 2.7 Output

`print` and `debug` go to **stderr**. A command's own report goes to stdout and may be JSON, so
a script printing into the middle of it would corrupt it. There is no other output.

---

## 3. Values

### 3.1 What a cell holds

A script has types, so the host does not guess. `App::enter`'s typing rule — a leading `=` is a
formula, `1800` is a number, `TRUE` is a logical — exists because a person typing has only
characters; re-deriving a type from a spelling here would be a second rule that can disagree
with the first.

| The script writes | The cell holds |
|---|---|
| an integer or a float | a number |
| `true` / `false` | a logical |
| a string | **text, exactly as written** — `"2091"` is the text `2091`, not the number |
| a string starting with `=` | a formula, verbatim, in ODF syntax |
| a string starting with `'` | text, with the quote removed — the core's own escape, and the only way to write a text cell that begins with `=` |
| `()` | nothing; the cell is left empty |
| `formula(…)`, `sum_above()` | as §3.2 and §6.3 say |

Anything else — an array, a map, a `Style` — is an error naming the type.

### 3.2 The two ways to write a formula

Both are the suite's own spellings, and neither is this crate's invention:

| Written | Meaning |
|---|---|
| `"=SUM([.B2:.B7])"` | **ODF syntax**, stored verbatim. What the file holds, what `grind sheet set` takes, and what `doc/projection-sheet.md` writes |
| `formula("SUM(B2:B7)")` | **display syntax**, the way a formula bar shows one, converted by `formula::display::from_display` — the converter behind `grind sheet fmt --from-display`. The leading `=` is optional |

`formula(…)` therefore *validates*: a formula that does not parse is an error at the line that
wrote it. A `=` string is not checked, exactly as it is not when typed into a cell — a document
this build cannot parse is a document it must still be able to write.

### 3.3 Addresses and indices

Two different things, deliberately spelled differently:

* **An address is a string** — `"B7"`, `"A1:E9"`, `"A:A"`, `"1:1"`. Whatever `grind sheet` takes,
  in the same syntax, including a whole column or row. An unqualified address means **the sheet
  it was written on** (`a1::resolve_in`), not the first sheet.
* **An index is an integer, counted from zero** — a row is where it sits in the sheet, a column
  is where a cell sits in a `row([…])` array, like every other index in the language.

`s.at(row, col)` is the one conversion between them, and it is `a1::format` underneath — the
workspace's only `+ 1` (`sheet/src/a1.rs`). A script never does address arithmetic on its own;
building `"B" + n` works and is not wrong, but it is a second conversion and this is the first.

`s.push(…)` answers with the index of the row it wrote, so `s.at(s.push(…), 2)` is an address in
the row just written.

### 3.4 The small vocabularies

Each is the core's, so a script, a command line and a GUI cannot disagree.

| Where | Values | Held by |
|---|---|---|
| `format(kind)` | `general`, `number`, `percent`, `currency`, `date`, `datetime`, `time`, `boolean`, `text` | `numfmt::Kind` plus the two that are not kinds — `general` is the absence of a format, `datetime` is §4.3.4's Date carrying a time |
| a colour | a `style::PALETTE` name (`navy`, `red`, `silver`, …), `#rrggbb`, or `transparent` | `grind_core::style::color` |
| a border | `"<width> <line> <colour>"`, e.g. `"0.5pt solid navy"` | `grind_core::style::border` |
| a length | an ODF length — `4cm`, `8mm`, `12pt`, `0.5in` | kept verbatim, as the document's own would be |
| `align` | `left`, `center`, `right`, `justify` | translated to §16.5's writing-direction-relative values |
| `valign` | `top`, `middle`, `bottom` | ODF's own |
| a locale | a language tag — `de-DE` | `grind_core::locale::Locale::parse` |

An unknown word is an error listing what was allowed.

### 3.5 Data

**A script may read data, and only data.** Separating what a document *is* from what is *in it*
is the reason to have a generator at all: the shape belongs in a script and the numbers belong
in a file somebody who does not read Rhai can edit.

| Call | Returns | Meaning |
|---|---|---|
| `json(name)` | the file's value | one JSON file from the data directory, as Rhai values |

This is the only exception to §2.3's "no filesystem", and it is bounded so that it stays an
exception rather than becoming a door:

| Wall | What it means |
|---|---|
| **JSON only** | parsed by a real JSON parser (`serde_json`). JSON has no references, no includes, no functions and no side effects — reading one cannot *do* anything. The function is named for the parser that ran, so a `csv(…)` later is a peer rather than a surprise |
| **One directory, named by a person** | `grind build` roots it at the **script's own directory**; `--data <DIR>` names another. `..` is refused before the filesystem is touched, an absolute path is refused, and the resolved path is canonicalised and checked to be inside the root — which is what stops a symlink pointing out |
| **`MAX_BYTES`** | `8 * 1024 * 1024` — the largest a data file may be |
| **`MAX_FILES`** | `64` — how many *distinct* files one script may read |
| **Read only, and cached** | there is no writing, and reading the same name twice returns the same value without touching the filesystem again, so `json(…)` inside a loop is harmless |

The mapping is one to one with what a script already has: an object is a map, an array is an
array, a string is a string, `null` is `()`, and a number is an integer when it is one and a
float otherwise. **Object keys come back sorted** — JSON says an object is unordered, and a
script that needs the author's order wants an array.

Reading is a *host capability*, not a language one: `grind_build::Data` is a trait,
`Directory` is the implementation with the walls above, and a caller that supplies `NoData` —
which is the default, and what `build()` uses — gets an error naming `--data` instead. A host
with no filesystem can therefore still run every script that does not ask for a file.

**Determinism, restated honestly.** §2.5 promised that the same source produces the same bytes.
With data it is *the same source and the same data*: a build system's contract rather than a
weakening of one, with the inputs named in the script and living beside it.

---

## 4. The host API — the spreadsheet

The vocabulary is `doc/projection-sheet.md`'s: `sheet`, `row`, `style`, `format` mean here what
they mean there, and the adjectives are that document's attributes. **Every method returns its
receiver** so a script may chain, except `push`, which returns where the row landed. Builders
are shared handles: a sheet pushed into a document and then written to again is the same sheet,
not a copy.

### 4.1 Constructors

| Call | Returns | Notes |
|---|---|---|
| `spreadsheet()` | `Spreadsheet` | an empty document |
| `sheet(name)` | `Sheet` | a sheet, not yet in any document. Returning one *is* a one-sheet document (§1) |
| `row(cells)` | `Row` | an array of values (§3.1). Errors on a value a cell cannot hold |
| `style()` | `Style` | nothing set |
| `format(kind)` | `Format` | §3.4's vocabulary. Errors on an unknown kind |
| `formula(source)` | `Cell` | display syntax, validated (§3.2) |
| `sum_above()` | `Cell` | resolved where it lands (§6.3) |

### 4.2 `Spreadsheet`

| Call | Returns | Meaning |
|---|---|---|
| `d.push(sheet)` | `Spreadsheet` | append a sheet. Sheets appear in the document in the order pushed |

### 4.3 `Sheet`

| Call | Returns | Meaning |
|---|---|---|
| `s.push(row)` | the row's index | append a `Row` under everything written so far |
| `s.push(cells)` | the row's index | the same, from a bare array — `s.push(["a", 1])` |
| `s.set(at, value)` | `Sheet` | one cell at one address. A range is an error. A `set` below the rows pushed so far moves the next `push` under it, so mixing the two cannot silently overwrite |
| `s.format(range, format)` | `Sheet` | a number format over a range |
| `s.format(range, kind)` | `Sheet` | the same, from §3.4's word — `s.format("B17", "date")` |
| `s.style(range, style)` | `Sheet` | styling over a range, **layered** onto whatever is already there (§6.4) |
| `s.width(cols, length)` | `Sheet` | column widths. `"A"` and `"A:A"` mean the same run of one |
| `s.height(rows, length)` | `Sheet` | row heights, the same way |
| `s.name(name, target)` | `Sheet` | a named range (`"B2:B7"`) or a named expression (`"=MAX(budgeted)"`). Document-level in ODF, said on the sheet whose cells it names, and written out qualified with that sheet (§6.6) |
| `s.rows()`, `s.rows` | an integer | how many rows have been written — where the next `push` will land |
| `s.at(row, col)` | an address | §3.3. Errors on a negative index |

### 4.4 `Row`

| Call | Returns | Meaning |
|---|---|---|
| `r.bold()` | `Row` | the whole row bold |
| `r.italic()` | `Row` | the whole row italic |
| `r.style(style)` | `Row` | any styling, over the row's own cells |
| `r.format(format)` | `Row` | a number format over the row's own cells |
| `r.format(kind)` | `Row` | the same, from §3.4's word |

A row's styling and format cover exactly the cells the row holds — `row(["a", 1])` styles two
cells, not the whole line.

### 4.5 `Style`

Each method sets one attribute and returns the style, so they chain. §3.4 holds the
vocabularies, and an unknown value is an error at the line that wrote it.

| Call | Sets |
|---|---|
| `st.bold()` | `fo:font-weight` |
| `st.italic()` | `fo:font-style` |
| `st.wrap()` | `fo:wrap-option` |
| `st.size(length)` | `fo:font-size` |
| `st.color(colour)` | `fo:color` |
| `st.background(colour)` | `fo:background-color` |
| `st.align(align)` | `fo:text-align` |
| `st.valign(align)` | `style:vertical-align` |
| `st.border(border)` | all four edges |

### 4.6 `Format`

A *request* for a format, built by `numfmt::preset` when it is used — so a script cannot produce
a format a shell's format picker could not. A format outside that vocabulary has no spelling
here; the projection spells one part by part and this deliberately does not (`doc/dsl.md` §3.8).

| Call | Meaning |
|---|---|
| `f.decimals(n)` | fraction digits, `0`–`255`. Exactly `n`, never "up to" |
| `f.grouping()` | thousands separators |
| `f.symbol(text)` | the currency symbol — `"€"`, `"EUR"` |
| `f.locale(tag)` | the locale whose separators it uses |

`format("general")` is the *absence* of a format. Using it where a format is required is an
error rather than a silent no-op: a script asking for no format should leave the cells alone.

---

## 5. The host API — the word processor

A text document is a **flat sequence of blocks** (`text/src/model.rs`), so this is smaller by
exactly that much. Every method returns the document.

| Call | Returns | Meaning |
|---|---|---|
| `text()` | `Text` | an empty document |
| `t.para(text)` | `Text` | a paragraph |
| `t.heading(level, text)` | `Text` | a heading, level `1`–`6`. The outline is implied by the levels and by nothing else |
| `t.item(depth, text)` | `Text` | a list item nested `1`–`9` deep |
| `t.item(text)` | `Text` | the same, at depth 1 |
| `t.bookmark(name)` | `Text` | anchor a bookmark at the start of the block last said. `#name` then addresses it, and survives editing where `p12` does not |
| `t.blocks()`, `t.blocks` | an integer | how many blocks have been said |

**The inline notation is `grind_text::markdown`'s**, read through `App::type_markdown` — the
same function a shell's typing goes through. `**bold**`, `_italic_`, `` `code` `` mean here
exactly what they mean while typing in any shell of this suite, because there is one reader.
Markers that make a *block* into something else (`# `, a code fence) apply too; a script that
wants a heading says `heading(1, …)` and does not need them.

---

## 6. From the returned tree to a document

The script returns a tree of *requests*; the host turns it into a document. Everything in this
section happens after the script has finished, which is why §2.6's errors here have no line.

### 6.1 The order

**Every cell first, then everything that decorates them.** Not the order the script said them
in, and the order it meant: a range is resolved against the sheet's used extent, so
`style("A:A", …)` means the column the script actually filled — and would mean an empty column
if it were applied when the script said it. Within each pass the script's order is kept, so a
later `format` over the same cells wins.

### 6.2 Sheets

The first sheet takes over the empty one a new document arrives with, rather than leaving a
`Sheet1` in front of everything; the rest are added in order. Two sheets with one name is an
error, from `App::add_sheet` rather than from here.

### 6.3 `sum_above()`

Resolved where the cell landed, as `=SUM([.X<top>:.X<row-1>])` over **the contiguous run of
cells directly above it that hold a number or a formula** — an AutoSum's rule. Not "everything
above", which reaches into a header and sums a heading; not the whole column, which cannot be
written before the sheet exists. Nothing above it is an error naming the address.

`doc/dsl.md` §4.2's sketch passed the column (`sum_above(i + 1)`); the built one takes no
argument, because the cell knows which column it landed in and a helper that can be told
otherwise can disagree with where its own answer sits.

### 6.4 Styles layer

`App::set_style` **replaces**, because a toolbar's Bold button sets what a cell *is* and a shell
wanting "bold as well" reads first. A script is not a toolbar: `row(…).bold()` followed by
`style("A1:H1", style().background("silver"))` means both, the way a stylesheet means both, and
replacing would silently drop the bold. So styling **layers** here — a later style's set fields
win, an unset field keeps what is underneath, edge by edge for borders — and what reaches the
core is one ordinary `set_style` per run of adjacent cells that agree.

Number formats do not layer: a format is one indivisible value, so the last one wins.

### 6.5 The document's own bounds

| Bound | Value | Meaning |
|---|---|---|
| the grid | `MAX_ROWS` × `MAX_COLS` (`grind_sheet`) | a cell past `XFD1048576` is an error naming the row and column, not a document |
| `MAX_CELLS` | `1_000_000` | the largest rectangle one `style`, `format`, `width` or `height` may cover — `App::set_format`'s own bound, restated because the layering walks the cells itself |

### 6.6 Names

`"=…"` is a named expression, stored as written. Anything else is an address, resolved on the
sheet that said it and written out **sheet-qualified and absolute** — `a1::as_definition`'s
rule, so `budgeted` means the same range read from anywhere.

### 6.7 What the CLI does afterwards

`grind build` **recalculates** the finished document unless `--no-recalc` is passed. Each
formula is already answered where it landed (`App::set_formula`), so this matters only for a
formula that reads a cell the script filled later; without it, such a cell keeps the value it
had at the time and is reported as stale exactly as an edit would be. A cell whose function this
build does not implement is left as the script wrote it, with a warning on stderr.

Then the document is written in the form the output's extension names — including `.grind`,
because the projection is a form and not an export.

---

## 7. What has no spelling here

A generator says the shape of a document. These are the parts of the model it cannot say, each
with what to do instead. **They are gaps, not refusals**: every one is reachable from the CLI,
and a script that needs one writes the document and then runs `grind` on it.

| Not sayable | Instead | Why |
|---|---|---|
| A **date or time value** (a number carrying `NumberKind`) | `=DATE(2026;8;16)` under `format("date")` | `App::enter` reads `2026-08-16` as a date only in a cell already known to hold one, which a cell being generated is not. `examples/sample-sheet.sh` writes one the same way |
| **Charts** | `grind sheet chart-add` | expressible, verbose, and nobody hand-writes one — the projection carries them for bijectivity, which a generator does not need |
| **Filters**, hidden rows and columns | `grind sheet filter` / `hide` | a view of a document rather than its content |
| The **null date**, the null year | — | a document-level setting no generated document has yet wanted |
| **Character formatting** beyond the notation — a font, a size, a colour on a run | `grind text format` | it reaches the model through a *caret*, and a script says blocks |
| **Images** | `grind text image` | `doc/dsl.md` §3.8's open question, and they have no projection either |
| **Named paragraph styles** | — | a generator cannot *declare* one, so every block using it would be a `grind lint` `undeclared-style` finding |

---

## 8. How this document is checked

A specification nothing checks drifts, and this one has the same guard
`doc/small-group.md` puts on `funcs::implemented()`, in `build/tests/spec.rs`.

**Two conventions make it possible, and editing this file means keeping them.** The API is §4
and §5 and nowhere else, because the tables elsewhere name things that are deliberately *not*
functions — `eval` in §2.3 is a row about its absence. And inside those two sections, a table
row's first cell is a code span holding the call as a script writes it: `` `sheet(name)` ``,
`` `s.push(row)` ``, `` `s.rows` ``. The name is what is left after dropping the receiver and
stopping at the parenthesis.

Then:

1. **Every function this document names is registered**, and
2. **every function `register()` registers is named here** — read out of `build/src/sheet.rs`
   and `build/src/text.rs` at compile time. Adding three lines to `register()` and not writing
   them down fails the build, which is the whole reason this document exists: the host API is
   the surface most likely to grow a function nobody documents.
3. **Every limit's value in §2.4 is the constant's value**, read from `build/src/engine.rs`
   rather than retyped.

What the check does not cover, and a reader should therefore treat as prose: argument types,
return values, and every "meaning" column. Those are held by `build/tests/smoke.rs`, which
executes most of them, and by the examples.
