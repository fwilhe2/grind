<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Writing a spreadsheet by hand — the projection guide

**`doc/dsl.md` §7, D12.** This is the *guide*: how to write a `.grind` file, in the order the
problems arrive. It is not the specification — `doc/dsl.md` is the design record and
`doc/projection-sheet.md` is the grammar, node by node, with an example the build executes for
every one. When this file and that one disagree about what is legal, that one is right.

Everything below is a real command. The worked example is `examples/quote.grind`, which is in
this repository and is asserted cell by cell in `cli/tests/cli.rs`; the outputs shown are the
outputs those commands produce.

Nothing here needs a spreadsheet application to be installed.

---

## 1. Why anybody would want this

A spreadsheet is the last file in a project that nobody can review.

Everything else in a repository is text: a reviewer opens a pull request, reads six lines of
diff, and knows what changed. A `.xlsx` or a `.ods` is a zip full of XML with a rendering of
itself inside, so the honest review is *"@alice changed budget.ods"* and the honest answer is
to open it and hunt. Which is why the number that was wrong for two quarters was wrong for two
quarters.

`doc/flat-first.md` makes this project's version of that argument, and the flat form (`.fods`)
is most of the way there — it is XML, it diffs, git can merge it. The projection is the rest of
the way. This:

```kdl
sheet Sales {
    at A1 {
        row Region Q1   Q2
        row North  4200 4800
        row South  3100 3300
    }
    cell B4 "=SUM([.B2:.B3])"
}
```

is the same document as forty lines of `<table:table-cell office:value-type="float"
office:value="4200"><text:p>4200</text:p></table:table-cell>`, and it is the one a person
reviews.

Four things follow from a spreadsheet being text, and they are the reason to bother:

| | |
|---|---|
| **A change is reviewable** | one rate edited is one line of diff, with the comment above it still there. §7 shows the diff |
| **A document can be checked in CI** | `grind lint` finds a cached total that disagrees with its formula, a formula naming a deleted sheet, a reference into an empty cell — and exits non-zero on an error, so a build can gate on it. §8 |
| **A document can be written without being calculated** | a formula does not need its answer, so a model can be typed out and `grind sheet recalc` fills the arithmetic in. §4 |
| **Merge conflicts are conflicts** | two people editing different rows of a `.grind` merge; two people editing a zip do not |

**And the honest limit, up front.** The projection carries what this build models, and nothing
else. Converting a LibreOffice document *to* one drops whatever has no node — charts today, and
anything ODF has that this project does not implement. That is not a silent loss: `grind lint`
names it before it happens, and §10 is how.

---

## 2. The smallest file that opens

A projection is [KDL](https://kdl.dev) — a document language that reads like a list of
commands. Two rules cover most of it: a *node* is a name followed by arguments and
`property=value` pairs, and a `{ … }` block after one holds more nodes.

The first node says what kind of document this is:

```kdl
grind spreadsheet

sheet Sales {
    at A1 {
        row Region Q1   Q2
        row North  4200 4800
        row South  3100 3300
    }
}
```

Save that as `small.grind` and ask what it is:

```console
$ grind info small.grind
spreadsheet
Sales	3 rows	3 cols	0 formulas
small.grind  (no change)  (not written)
```

```console
$ grind sheet view small.grind A1:C3
Region	Q1	Q2
North	4200	4800
South	3100	3300
```

**The header line is not decoration.** `grind_core::kind` decides which document type some
bytes are *before* anything parses them, because a tolerant reader handed the wrong kind
returns an empty document rather than an error. ODF states its media type; a projection states
`grind spreadsheet` (or `grind text`). A file without it is not one.

**`at A1` says where the grid starts**, and each `row` under it is one line of the sheet. You
can have as many `at` blocks as you like, at any address — one per table is the usual shape.

---

## 3. How a value is spelled

There are five spellings and they mean five different things. This is the part worth reading
twice, because a spreadsheet's oldest bug is a number that is secretly text.

| Written | The cell holds |
|---|---|
| `4200`, `-12.5`, `0.19` | a number |
| `North`, `LJ-1041` | text — a bare word is a string in KDL |
| `"Ash & Iron"`, `"2042"` | text, quoted because it has a space or would otherwise read as a number. **`"2042"` is text**, `2042` is a number |
| `#true`, `#false` | a logical |
| `#null` | **nothing** — the cell is empty, which is not the same as an empty string |

`#null` is how a row skips a column:

```kdl
row Housing 1800 #null #null #true
```

Three more things fall out of KDL's string rules, and each of them would otherwise have been a
decision somebody had to invent:

| You write | The document gets |
|---|---|
| `"two  spaces"` | two spaces. KDL does not collapse whitespace and XML does, which is why this survives at all (in a text document the same rule is what produces a `text:s`) |
| `"name\tvalue"` | a tab inside one cell |
| `"one\ntwo"` | a line break inside one cell — and a *new row* is a new node, so the two are visibly different |
| `#"a literal \n stays literal"#` | a raw string: no escapes are processed at all |

---

## 4. Formulas, and why they have no answers

```kdl
grind spreadsheet

sheet Sales {
    at A1 {
        row Region Q1   Q2
        row North  4200 4800
        row South  3100 3300
    }
    cell B4 "=SUM([.B2:.B3])"
    cell C4 "=SUM([.C2:.C3])"
}
```

A `cell` node is one cell that carries more than a plain value. A bare `=…` in the value
position **is** the formula, and there is no cached answer beside it — which is the single most
useful property of this format for anybody writing a model by hand. You type the shape of the
calculation; you do not do the calculation.

`grind sheet recalc` does it:

```console
$ grind sheet recalc small.grind
Sales	4 rows	3 cols	2 formulas
small.grind  undo
```

and the file afterwards:

```kdl
grind spreadsheet

sheet Sales {
    at A1 {
        row Region Q1   Q2
        row North  4200 4800
        row South  3100 3300
    }
    cell B4 7300 formula="of:=SUM([.B2:.B3])"
    cell C4 8100 formula="of:=SUM([.C2:.C3])"
}
```

Three things to notice, because each is a rule rather than an accident:

* **Two lines changed and nothing else did.** The alignment inside `row North  4200 4800` is
  still two spaces wide. That is R6 — the writer splices at the byte ranges it recorded when it
  read the file, and never regenerates what nobody touched. §7 is the whole of it.
* **The formula grew an `of:` prefix.** That is ODF's namespace on a formula, and the canonical
  spelling. The reader takes it with or without; the writer emits it. Tolerance on the way in,
  strictness on the way out, applied to this project's own format.
* **The answer went in front of the formula**, in the value position, because that is where a
  value goes: `cell B4 7300 formula="…"` is *this cell holds 7300, and here is why*.

**A formula with no answer is legal ODF and renders blank in LibreOffice.** So
`grind convert model.grind model.fods` on an uncalculated model gives a file that opens empty
until something recalculates it. `grind` tells you every time it writes one — the staleness
warning on the CLI is exactly this — and `doc/projection-sheet.md`'s "A formula does not need
its answer" is the long version, including the bug this cost to get right.

**Formulas are ODF syntax, verbatim.** `[.B2]` is a reference to B2 on this sheet, `[.B2:.B3]`
a range, `[$Sales.$B$2]` an absolute reference on a named sheet, and arguments are separated
with `;` rather than `,`. It is what the file stores and what `grind sheet set` takes on a
command line — the projection hands the string to the same lexer, so there is no second
grammar to learn. If you have a formula in the display spelling a formula bar shows, convert
it:

```console
$ grind sheet fmt --from-display 'SUM(B2:B3)'
=SUM([.B2:.B3])
```

---

## 5. Names, so a formula says what it means

```kdl
name kind      "[$Quote.$C$5:.$C$13]"
name amount    "[$Quote.$G$5:.$G$13]"
name benchRate "[$Assumptions.$B$2]"
```

Named ranges are **document-level** in ODF, so they sit outside every `sheet` block — putting
one inside is an error that says so. A name may also be an expression (`name total
"=SUM(amount)"`).

What they buy is the difference between

```kdl
cell G14 "=SUMIF([.C5:.C13];\"Labour\";[.G5:.G13])"
cell F5  "=[$Assumptions.$B$2]"
```

and

```kdl
cell G14 "=SUMIF(kind;\"Labour\";amount)"
cell F5  "=benchRate"
```

Both are the same document. Only one of them can be read by the person who has to approve it.

---

## 6. Styles, formats, widths — the parts that are not the numbers

Each is stated **once over the range it covers**, never per cell, which is both how a person
thinks about it and how the ODF writer pools them anyway.

```kdl
sheet Quote {
    col 2 width="9cm"
    row 1 height="9mm"

    style A4:G4  bold=#true background=silver align=center border="0.5pt solid navy"
    style A5:A13 italic=#true color=navy

    format G14:G19 currency decimals=2 symbol="£"
    format G22     percentage decimals=1
    format G23     date
}
```

| | |
|---|---|
| `style` | `bold` `italic` `wrap` `size` `color` `background` `align` `valign` `border`. A colour is a palette name (`navy`, `silver`, `red`, …), `#rrggbb`, or `transparent`; a border is `"<width> <line> <colour>"` |
| `format` | `number` `percentage` `currency` `date` `time` `datetime` `boolean` `text`, with `decimals=` `grouping=` `symbol=` `locale=`. Display only — **a format never touches the value**. There is no `general`: the absence of a format is the absence of a `format` node |
| `col` / `row` | a width or a height as an ODF length (`4cm`, `8mm`, `12pt`, `0.5in`), kept verbatim, plus `hidden=#true` |

A format outside that vocabulary is spelled as the **ordered sequence of parts ODF actually
stores**, rather than as an Excel format string, because that is what the model is
(`doc/ods-format.md` §5.2). `examples/quote.grind` has one — hours shown as `88.0 h`:

```kdl
format G21 number { number decimals=1 min-decimals=1; text " h" }
```

`doc/projection-sheet.md`'s second table is every part, with an example each.

> **One spelling to watch.** The projection says `percentage`, matching ODF's own element name;
> the generator (§12) says `percent`, matching `numfmt::preset`'s vocabulary. They are the same
> format. If a `format … percent` line is rejected, that is why.

---

## 7. Editing one, and what a diff looks like

This is the section the format exists for.

`examples/quote.grind` is a quote for a joinery job: nine line items, a rate card on a second
sheet, and a summary block that reads both. Suppose the workshop puts its bench rate up from
£62 to £65. That is one number, on the `Assumptions` sheet:

```console
$ grind sheet set quote.grind Assumptions.B2 65
grind: 23 formula cell(s) now disagree with their cached value — run `grind sheet recalc`
```

```diff
@@ -140,5 +140,5 @@ sheet Assumptions {
     at A1 {
         row Assumption      Value  Note
-        row "Bench rate"    62     "£ per hour, 2026 rate card"
+        row "Bench rate"    65     "£ per hour, 2026 rate card"
         row "Overhead"      0.18   "shop, insurance, consumables"
         row "VAT"           0.2    "standard rate"
```

**One line.** Not "the file changed". The comment above it, the column alignment, the blank
lines — none of them moved, because none of them were touched.

Then recalculate, and the diff becomes the *consequences* of that edit:

```console
$ grind sheet recalc quote.grind
$ git diff --stat
 quote.grind | 34 +++++++++++++++++-----------------
 1 file changed, 17 insertions(+), 17 deletions(-)
```

```diff
     cell F14 Labour
-    cell G14 5063.23 formula="of:=SUMIF(kind;\"Labour\";amount)"
+    cell G14 5308.225 formula="of:=SUMIF(kind;\"Labour\";amount)"
     cell F15 Materials
     cell G15 3650.71 formula="of:=SUMIF(kind;\"Materials\";amount)"
     cell F16 "Workshop overhead"
-    cell G16 911.3813999999999 formula="of:=[.G14]*overhead"
+    cell G16 955.4805 formula="of:=[.G14]*overhead"
     cell F17 Subtotal
-    cell G17 9625.321399999999 formula="of:=SUM([.G14:.G16])"
+    cell G17 9914.415500000001 formula="of:=SUM([.G14:.G16])"
```

Seventeen cells moved and the diff names all seventeen, with the formula that produced each one
beside it. A reviewer can see that the materials line did *not* move, which is the thing they
would actually want to check.

Two properties hold underneath all of this, and both are asserted over the whole corpus rather
than argued for:

* **An untouched save returns the bytes that were read.** `grind convert quote.grind copy.grind`
  produces a byte-identical file.
* **A *structural* change regenerates.** Adding a sheet, or an edit the splicer cannot place,
  rewrites the file in canonical form and the comments and alignment go with it. That is the
  same honest limit `.fods` has, and `doc/dsl.md` §3.1 records the measurement behind it.

---

## 8. Checking one before anybody opens it

```console
$ grind lint broken.grind
Sales.D3: error: names the sheet "Archive", which this document does not have [missing-sheet]
Sales.D2: warning: reads Sales.E2, which is empty [empty-reference]
Sales.B4: warning: holds 9999 and its formula computes 7300 [stale-value]
broken.grind: 1 error(s), 2 warning(s), 0 hint(s)
```

Eight rules, listed in `doc/dsl.md` §4.3. The three above are the ones a hand-written model
actually hits. **It exits non-zero on an error and not on a warning**, so a CI job can gate on
a document contradicting itself without every existing warning breaking a build:

```yaml
- run: grind lint quote.grind
```

`--format json` gives a machine-readable form for an annotation bot:

```console
$ grind --format json lint broken.grind
{"path":"broken.grind","diagnostics":[{"rule":"missing-sheet","severity":"error","at":"Sales.D3", …
```

**Nothing is written.** Linting leaves the file's bytes exactly as they were — a diagnostic
stored in a document goes stale and a derived one cannot, which is the same promise
`doc/view-modes.md` makes for the overlays.

`grind lint` reads the *kind* out of the file, so it works on a `.grind`, a `.fods`, an `.ods`
and a text document alike. `--off <rule>` silences one by id; `--hints` turns on the ones that
are off by default.

---

## 9. The worked example, walked

`examples/quote.grind` is about a hundred and sixty lines and uses every node a hand-written
model needs. In order:

| Lines | What |
|---|---|
| `grind spreadsheet` | the kind header (§2) |
| a comment block | what the file is, and the four commands that do something with it. Comments are free and they survive edits — use them |
| five `name` nodes | the ranges the formulas read, so they read like sentences (§5) |
| `col` / `row` | widths and one row height (§6) |
| one `at A1` block | thirteen rows: a title block, a blank row, a header row, nine line items |
| a run of `cell F…` | the labour rates, each a formula reading the rate card on the other sheet |
| a run of `cell G…` | nine line totals, `=[.D5]*[.F5]` — nine nearly identical lines, written out because a person can see all nine at once |
| a summary block | `SUMIF` over the two names, then overhead, subtotal, VAT and total |
| a facts block | hours quoted, materials share, and a `=DATE(2026;10;9)` under a date format |
| `style` / `format` | six styles and five formats, each over a range |
| a second `sheet` | `Assumptions` — the prices, on their own sheet, so re-pricing is a one-line diff |

**The nine repeated lines are the honest edge of this format.** Nine is fine. Ninety is not,
and at ninety the answer is the other half of `doc/dsl.md` — a generator, which writes them
with one `for`. §12.

A date is worth a note of its own. A date *value* is a number with a date kind on it, and
nothing in this format spells one directly; what you write is the formula that makes one, under
a date format:

```kdl
cell G23 "=DATE(2026;10;9)"
format G23 date
```

---

## 10. Converting between the three forms, and what it costs

A `.grind` is a **physical form**, like `.ods` (a zip) and `.fods` (one XML file) — not an
export. So `convert` reaches all three, both ways, and the form comes from the output's
extension:

```console
$ grind convert quote.grind quote.fods     # to a file LibreOffice opens
$ grind convert quote.fods  quote.grind    # and back
$ grind convert book.ods    book.grind     # from a package, too
```

Every shell opens one, because the form is sniffed from the bytes rather than from the name.

**Two things are lost, in two different directions, and both are worth knowing before you rely
on either.**

*Converting `.fods` → `.grind` drops what the projection has no node for.* Charts are the one
named gap for spreadsheets (images for text documents), along with anything ODF carries that
this build does not model. This is not silent:

```console
$ grind lint c.fods
Sheet1: warning: chart 1 (chart:bar) at 1cm,1cm has no projection node, so a .grind of this
document would not carry it [unspellable]
```

That rule exists for exactly this moment. Run it before you convert something you did not
write.

*Converting `.grind` → `.fods` → `.grind` drops your formatting of the file.* The round trip
preserves the **document** — that is loop F, asserted over 359 spreadsheets and 1755 text
documents with nothing differing — but not your comments, blank lines and alignment, because
those are not in the document. They live in the file, and they survive an *edit* rather than a
*conversion*:

```kdl
// what you wrote
row L-BENCH   "Reception desk, bench joinery"   Labour   38   hours

// what comes back from a round trip through .fods
row L-BENCH "Reception desk, bench joinery" Labour 38 hours
```

The practical rule: **the `.grind` is the file you keep**, and the `.fods`/`.ods` is what you
send to somebody who needs to open it in a spreadsheet application.

---

## 11. Opening one in the shells

Every shell in the suite opens a `.grind` directly, and every shell can also show *any* open
document as its projection — the grid on one page, its source on the other, with the line the
selection is on marked and moving in it selecting what that line projects.

| | Open a `.grind` | Show the source |
|---|---|---|
| CLI | any verb | `grind sheet project book.fods`, plus `--tokens` and `--anchors` |
| `grind-tui` | `cargo run -p grind-tui -- quote.grind` | `:source` |
| `grind-sheet-gtk` | `cargo run -p grind-sheet-gtk -- quote.grind` | Ctrl+Shift+U |
| `grind-web` | the file picker | Ctrl+K → *Show the source* |

The code view is read-only, by decision rather than by omission: `doc/dsl.md` §6.4 is the three
things an editable one would need and why it waits for evidence that anybody wants it. Editing
the file in a text editor is, of course, the whole point.

---

## 12. When to stop writing these by hand

The projection is **declarative**: it says what a document *is*, it round-trips, and it has no
loops, no variables and no functions — deliberately, because a form with a `for` in it cannot
be written back (given a row edited in a GUI, which iteration produced it?). `doc/dsl.md` §1 is
that argument.

So when the repetition gets tiring, you do not want a bigger projection. You want the other
layer:

```console
$ grind build examples/timesheet.rhai -o month.fods
```

A script *returns* a document and `grind build` writes it. Adding a job to the data file adds
its rows, its formulas and its summary line without a line of the script changing.
**`doc/generator-guide.md` is the guide to that**, and it is a one-way arrow: a script produces
a document and is never recovered from one.

The two fit together the obvious way — a generator can write a `.grind` as easily as a `.fods`,
because a projection is a form:

```console
$ grind build examples/timesheet.rhai -o month.grind
```

which gives a generated document that is *also* reviewable in a pull request.

---

## 13. Setting up an editor

`.grind` is KDL, so any editor with KDL support highlights it once it knows the extension, and
this repository ships the VS Code configuration for that plus a snippet for every node in the
grammar. **`doc/editor-setup.md`** is the setup, including what was measured about each
extension and what it actually does.

---

## 14. What this guide is not

It is not the grammar — `doc/projection-sheet.md` is, with an executable example per node and a
row for every field of the model. It is not the design — `doc/dsl.md` is, and it outranks this
file on every question of why. And it is not a specification anybody else should implement
against: this is one build's third serialisation of its own model, not a proposal to anyone
(`doc/dsl.md` §9).

`cli/tests/editor.rs` checks that every `examples/…` file named above exists. What it does not
check, and what a reader should therefore treat as prose, is that every output shown is the
output produced today — the commands are ordinary verbs with their own tests, and the two
examples both guides are built on are asserted cell by cell in `cli/tests/cli.rs`.
