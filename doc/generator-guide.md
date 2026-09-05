<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Generating a spreadsheet — the generator guide

**`doc/dsl.md` §7, D14.** This is the *guide*: from a five-line script to
`examples/timesheet.rhai`, in the order the problems arrive. It is not the specification —
`doc/generator-spec.md` is, exhaustively, and it is checked against the engine in both
directions. When this file and that one disagree about what is legal, that one is right.

Every script named as `examples/…` below is a file in this repository, and `cli/tests/editor.rs`
builds each one — so a guide that stops working fails the build rather than the reader. The
handful shown without that prefix are the deliberately broken ones in §6 and §10, whose whole
purpose is to fail; they are quoted from a scratch directory rather than shipped.

---

## 1. Why anybody would want this

A spreadsheet's characteristic bug is **row 23**.

Not a wrong formula — a formula that is right in twenty-two rows and wrong in one, because
somebody dragged a fill handle over a gap, or inserted a row above and one absolute reference
did not move, or pasted a corrected column over twenty-three cells of which twenty-two were
already correct. The bug is invisible: the sheet looks like a sheet, the total looks like a
total, and the only way to find it is to check every row.

A generator removes the category. There is **one line per kind of cell, not one per cell**:

```rhai
for e in source.time {
    let at = time.push([e.day, e.job, e.who, e.role, e.hours, (), (), e.task]);
    time.set(time.at(at, 5), formula("VLOOKUP(" + time.at(at, 3) + ";" + rate_table + ";2;FALSE())"));
    time.set(time.at(at, 6), formula(time.at(at, 4) + "*" + time.at(at, 5)));
}
```

Twenty-two rows, or two hundred, get the same two formulas — each written *for the row it
landed on*, which is also why a generated sheet has no `$` in it. A fill handle needs absolute
references because it copies one formula; a loop writes each one where it goes.

Three more things follow:

| | |
|---|---|
| **The data is not the document** | the numbers live in a JSON file somebody who has never read a line of Rhai can edit, and the script says only what the document *is*. §6 |
| **Rebuilding is the whole of "updating the spreadsheet"** | new export, `grind build`, done — with no chance of a stale row surviving because nobody scrolled that far |
| **The same source produces the same bytes** | on any machine, at any time, so a generated document can be committed and its diff read as *what the numbers did*. §11 |

**And what it is not.** A script is not a macro and cannot become one. It runs when *you* type
`grind build`, never when a reader opens a document; it has no filesystem beyond reading JSON
from one directory you named, no network, no clock, no randomness; and no code path in this
project that *opens* a document can evaluate anything, which is a requirement (R11) with a test
that reads every manifest in the workspace. `doc/dsl.md` §2 is the argument in full.

---

## 2. The first script

A script is a sequence of [Rhai](https://rhai.rs) statements, and **its value is the value of
its last expression**. That value has to be a document.

```rhai
let s = sheet("Sales");
s.push(row(["Region", "Q1", "Q2"]).bold());
s.push(["North", 4200, 4800]);
s.push(["South", 3100, 3300]);
s.push(row(["Total", sum_above(), sum_above()]).bold());
s
```

That is `examples/first.rhai`, in full:

```console
$ grind build examples/first.rhai -o first.fods
Sales	4 rows	3 cols	2 formulas
first.fods  undo

$ grind sheet view first.fods A1:C4
Region	Q1	Q2
North	4200	4800
South	3100	3300
Total	7300	8100
```

Five things are already doing work there:

* **`sheet("Sales")` alone is a document.** Returning a `Sheet` means a one-sheet spreadsheet;
  `spreadsheet()` with sheets pushed into it is the general case (§8).
* **`push` takes a bare array or a `row(…)`.** The second is what you want when the row needs
  an adjective: `row([…]).bold()`, `.italic()`, `.style(…)`, `.format(…)`.
* **`sum_above()` was not told anything.** It sums the contiguous run of numbers or formulas
  directly above wherever it lands — an AutoSum's rule, which is the only one that does not
  reach up into a header and sum a heading:

  ```console
  $ grind sheet get first.fods B4 --formula
  =SUM([.B2:.B3])
  ```
* **The kind of document comes from what the script returned**, never from the output's name.
  The *extension* picks the physical form — `.ods`, `.fods`, `.odt`, `.fodt`, `.grind` — exactly
  as it does for `grind convert`.
* **`grind build` recalculates at the end**, so every formula in the file has its answer
  (`--no-recalc` if you want the projection's uncalculated shape).

---

## 3. What a cell can hold

A script has types, so the host does not guess. `grind sheet set`'s rule — a leading `=` is a
formula, `1800` is a number — exists because a person typing has only characters; re-deriving a
type from a spelling here would be a second rule that can disagree with the first.

| You write | The cell holds |
|---|---|
| `4200`, `-12.5` | a number |
| `true` / `false` | a logical |
| `"North"`, `"2042"` | **text, exactly as written** — `"2042"` is the text, not the number |
| `"=SUM([.B2:.B7])"` | a formula in **ODF syntax**, verbatim, stored unchecked |
| `formula("SUM(B2:B7)")` | a formula in **display syntax** — what a formula bar shows — converted, and therefore *validated* |
| `"'=not a formula"` | text beginning with `=`; the leading quote is the core's own escape and is removed |
| `()` | nothing — the cell is left empty |
| `sum_above()` | resolved where it lands (§2) |

Anything else — an array, a `Style` — is an error naming the type.

**Prefer `formula(…)`.** It goes through the same converter as `grind sheet fmt --from-display`,
so a formula that does not parse is an error at the line that wrote it rather than a cell nobody
notices:

```console
$ grind build bad.rhai -o bad.fods
grind: bad.rhai:3:19: Runtime error: SUM(B2:: expected a value at byte 8
```

**A date value has no spelling**, and that is a named gap rather than an oversight: a generated
cell is not already known to hold a date, so there is nothing for `2026-08-16` to mean. Write
the formula that makes one, under a date format — which is what a hand-written projection does
too:

```rhai
s.set("B17", "=DATE(2026;8;16)");
s.format("B17", "date");
```

---

## 4. Addresses, indices, and the one conversion

Two different things, deliberately spelled differently:

* **An address is a string** — `"B7"`, `"A1:E9"`, `"A:A"`, `"1:1"` — in exactly the syntax
  `grind sheet` takes. An unqualified one means *the sheet it was written on*, not the first
  sheet.
* **An index is an integer counted from zero**, like every other index in the language.

`s.at(row, col)` is the one conversion between them, and it is the workspace's only `+ 1`.
`s.push(…)` answers with the index of the row it landed on, so:

```rhai
let at = s.push(["Housing", 1800, 1825.50]);
s.set(s.at(at, 3), formula(s.at(at, 1) + "-" + s.at(at, 2)));   // D2 = B2-C2
```

Building `"B" + n` works and is not wrong, but it is a second conversion where there is already
a first. `s.rows` is how many rows have been written — which is also the 1-based number of the
last one, and where the next `push` will land.

That last sentence is the one thing in this API worth saying twice, because it is where the
off-by-one lives:

```rhai
let last_job = summary.rows;              // the 1-based row number of the last job
let total = summary.push(row([…]));       // the *index* the totals landed on
let last = total + 1;                     // the row number that index wears
summary.format("C2:C" + last, money);
```

---

## 5. Data at the top — `examples/budget.rhai`

The first real script shape: the thing that changes goes in a `const` at the top, and the table
is what a loop does with it.

```rhai
const CATEGORIES = [
    ["Housing",       1800, 1825.50],
    ["Groceries",      500,  612.34],
    ["Transport",      220,  179.85],
];

let s = sheet("Budget");
s.push(row(["Category", "Budgeted", "Actual", "Difference"]).bold());

for c in CATEGORIES {
    let at = s.push([c[0], c[1], c[2]]);
    s.set(s.at(at, 3), formula(s.at(at, 1) + "-" + s.at(at, 2)));
}

s.push(row(["Total", sum_above(), sum_above(), sum_above()]).bold());
s
```

```console
$ grind build examples/budget.rhai -o budget.fods
```

Adding a row to the budget is adding a row to `CATEGORIES`. Every formula, format and total
follows without being touched, and nothing below the loop knows how many categories there are.

`examples/budget.rhai` is the full version — the same table with its formats, styles, widths,
named ranges and a second sheet that reaches across to the first. It is D7's exit criterion:
`examples/sample-sheet.sh`'s household budget, said once.

---

## 6. Data in a file — `examples/prices.rhai`

A `const` at the top works until somebody who does not read Rhai needs to change it. Then the
data wants to be a file:

```rhai
let source = json("prices.json");

let money = format("currency").symbol(source.currency).grouping().decimals(2);

let s = sheet("Prices");
s.push(row(["SKU", "Item", "Unit", "Price", "Qty", "Value"]).bold());

for item in source.items {
    let at = s.push([item.sku, item.name, item.unit, item.price, item.quantity]);
    s.set(s.at(at, 5), formula(s.at(at, 3) + "*" + s.at(at, 4)));
}
```

**This is the only exception to "a script has no filesystem", and it is bounded so that it stays
an exception rather than becoming a door:**

| Wall | |
|---|---|
| **JSON only** | parsed by a real JSON parser. JSON has no references, no includes, no functions — reading one cannot *do* anything |
| **One directory, named by a person** | the script's own by default, or `--data <DIR>`. `..` is refused before the filesystem is touched, an absolute path is refused, and the resolved path is canonicalised and checked to be inside the root — which is what stops a symlink pointing out |
| **Bounded** | 8 MB per file, 64 distinct files per script |
| **Read only, and cached** | reading the same name twice returns the same value without touching the disk, so `json(…)` inside a loop is harmless |

```console
$ grind build escape.rhai -o out.fods
grind: escape.rhai:1:1: Runtime error: ../secrets.json: a data file is inside the data
directory, and `..` leaves it
```

The mapping is one to one with what a script already has: an object is a map, an array is an
array, `null` is `()`, and a number is an integer when it is one. **Object keys come back
sorted** — JSON says an object is unordered, and a script that needs the author's order wants an
array.

Determinism, restated honestly: with data it is *the same source and the same data* produce the
same bytes. That is a build system's contract rather than a weakening of one, with the inputs
named in the script and living beside it.

---

## 7. A function for a repeated shape

Two shapes recur often enough to name. A Rhai `fn` **cannot see the script's variables** — it
gets only its arguments — which is why the sheet is one of them:

```rhai
/// A header row: bold, and nothing else.
fn header(cells) {
    row(cells).bold()
}

/// A labelled figure under a table — a name in column A, a formula in column B — answering
/// with the row it landed on.
fn fact(s, label, source) {
    let at = s.push([label]);
    s.set(s.at(at, 1), formula(source));
    at
}
```

```rhai
fact(summary, "Best margin", "MAX(" + margins + ")");
fact(summary, "Worst margin", "MIN(" + margins + ")");
```

That works because **every builder is a shared handle**. A sheet passed into a function, or
pushed into a document and then written to again, is the same sheet rather than a copy. It is
the one implementation detail worth knowing, because it is the difference between the API
meaning what it reads as and not.

---

## 8. Several sheets that agree — `examples/timesheet.rhai`

This is the shape a generator is actually for, and the one worth reading in full. A joinery
workshop's month, from its time-tracking export:

| Sheet | |
|---|---|
| `Rates` | the hourly rate per role — the table the Time sheet looks a rate up in |
| `Time` | one row per timesheet line, its rate looked up and its value calculated |
| `Materials` | one row per delivery note |
| `Summary` | one row per job, its hours, labour and materials summed back out of the other two |

```console
$ grind build examples/timesheet.rhai -o month.fods
Summary	13 rows	9 cols	36 formulas
Time	23 rows	8 cols	44 formulas
Materials	10 rows	4 cols	0 formulas
Rates	5 rows	2 cols	0 formulas
```

```console
$ grind sheet view month.fods A1:I6
Job	Client	Quoted	Hours	Labour	Materials	Cost	Margin	Margin %
LJ-1041	Ash & Iron Bakery	8,400.00 £	36.0	1,952.00 £	1,399.20 £	3,954.42 £	4,445.58 £	52.9%
LJ-1042	Bramble Court Hotel	14,250.00 £	59.0	3,261.50 £	3,508.75 £	7,988.90 £	6,261.11 £	43.9%
LJ-1043	Wren Street Library	6,100.00 £	33.0	1,742.00 £	1,133.50 £	3,393.09 £	2,706.91 £	44.4%
LJ-1044	Copperfield Dental	2,600.00 £	21.0	1,256.00 £	582.75 £	2,169.73 £	430.28 £	16.5%
Total		31,350.00 £	149.0	8,211.50 £	6,624.20 £	17,506.13 £	13,843.87 £	44.2%
```

Four techniques in it are worth taking away.

### Build in dependency order, push in reading order

`Rates` is built first because the Time sheet's `VLOOKUP` has to name its range, and a range is
a string that needs the row count of a table that exists. `Summary` is built last because it
reads the other three. But `Summary` is the sheet anybody opens, so it goes into the document
first:

```rhai
let d = spreadsheet();
d.push(summary);
d.push(time);
d.push(materials);
d.push(rates);
d
```

Sheets appear in the order pushed, which is not the order they were built.

### Measure a range, never count one

```rhai
let time_last = time.rows;
let time_jobs  = "Time.$B$2:$B$" + time_last;
let time_hours = "Time.$E$2:$E$" + time_last;
```

Absolute and sheet-qualified, because every job's row asks the same two sheets the same
question about a different code. Nothing in the script says twenty-two; the number comes from
the loop that wrote the rows:

```console
$ grind sheet get month.fods D2 --formula
=SUMIF([Time.$B$2:.$B$23];[.A2];[Time.$E$2:.$E$23])
```

Add a job to `examples/timesheet.json` and it gets a summary row whose three `SUMIF`s cover the
new lines. That is the property the whole design is for.

### Let the document do the lookup

```rhai
time.set(
    time.at(at, 5),
    formula("VLOOKUP(" + time.at(at, 3) + ";" + rate_table + ";2;FALSE())"),
);
```

The script *could* have substituted the number — it has the rates in hand. Both put the same
value in the cell today; only this one still explains itself in six months, and only this one
changes when somebody edits the Rates sheet in a spreadsheet application.

### Name what a formula means

```rhai
summary.name("margins", "I2:I" + last_job);
summary.name("overhead", "=" + (1 + source.overhead));
```

```console
$ grind sheet get month.fods G2 --formula
=([.E2]+[.F2])*overhead
```

A name is document-level in ODF and is written out qualified with the sheet that said it, so
`margins` means `[$Summary.$I$2:.$I$5]` read from anywhere. `"=…"` is a named *expression*;
anything else is an address.

---

## 9. Formatting

Three builders, and each method returns its receiver so they chain.

```rhai
let money = format("currency").symbol("£").grouping().decimals(2);

s.format("C2:C6", money);                      // a number format over a range
s.format("B17", "date");                       // the same, from the one-word vocabulary
s.style("A1:I1", style().background("silver").align("center").border("0.5pt solid navy"));
s.width("A:A", "3cm");
s.height("1:1", "8mm");
```

| | |
|---|---|
| `format(kind)` | `general` `number` `percent` `currency` `date` `datetime` `time` `boolean` `text`, then `.decimals(n)` `.grouping()` `.symbol(t)` `.locale(tag)` |
| `style()` | `.bold()` `.italic()` `.wrap()` `.size(l)` `.color(c)` `.background(c)` `.align(a)` `.valign(a)` `.border(b)` |

Two rules that are not visible in a signature:

* **Everything decorating cells happens after every cell exists.** A range is resolved against
  the sheet's used extent, so `style("A:A", …)` means the column the script actually filled —
  and would mean an empty column if it were applied when the script said it.
* **Styles layer; formats do not.** `row(…).bold()` followed by
  `style("A1:H1", style().background("silver"))` means *both*, the way a stylesheet means both.
  (`App::set_style` in the core *replaces*, because a toolbar's Bold button sets what a cell is.
  A script is not a toolbar, and replacing here would silently drop the bold.) A number format
  is one indivisible value, so the last one wins.

> **One spelling to watch.** The generator says `percent`, matching `numfmt::preset`'s
> vocabulary; the projection says `percentage`, matching ODF's element name. Same format.

---

## 10. Reading an error

```
script:line:column: message
```

The position is the position of the *call that failed*, so a host function handed a bad
argument reports the line of your script rather than a line of `build/src/`:

```console
$ grind build bad.rhai -o bad.fods
grind: bad.rhai:3:19: Runtime error: SUM(B2:: expected a value at byte 8
```

A script that will not terminate is a build error rather than a hang, because everything is
bounded:

| Limit | Value | Trips when |
|---|---|---|
| operations | 10,000,000 | a loop that never ends, or a script far larger than any document needs |
| call depth | 64 | a function recurses without bottoming out |
| string size | 1 MB | `s += s` doubles, which the operation limit alone would not catch |
| array / map | 1,000,000 entries | |
| the grid | `XFD1048576` | a cell past the end of a sheet |

```console
$ grind build loopy.rhai -o loopy.fods
grind: loopy.rhai:2:16: Runtime error: row 1048577 column 1 is past the end of the sheet
```

A failure while turning the returned tree into a document has **no line**, and says where in
the *document* instead (`sum_above() at B8: …`): by then the script has finished, and the fault
is in what it asked for rather than in a line that asked.

**Nothing is written when a script fails.** And if the script returns something that is not a
document:

```console
$ grind build nodoc.rhai -o nodoc.fods
grind: nodoc.rhai: a script has to end with the document it built — a `sheet(…)`, a
`spreadsheet()` or a `text()`. This one ended with i64
```

`print` and `debug` go to **stderr**, because a command's own report goes to stdout and may be
JSON.

---

## 11. Determinism, and what it is for

**The same source produces the same bytes.** Not merely the same document: the same file, byte
for byte, on any machine, at any time. It rests on three things and is checked rather than
promised — `cli/tests/cli.rs` builds `examples/budget.rhai` twice and compares the two files.

| | How |
|---|---|
| no clock | Rhai's `no_time` **feature**, so `timestamp()` is not in the language |
| no randomness | not in core Rhai; the crates that add it are not taken |
| no OS-seeded hashing | Rhai's default features off, so a map iterates the same way everywhere |

What that buys, concretely: a generated document can be committed. Rebuild it, and the diff is
*what the numbers did* — not noise from a timestamp in the file. Build it to a `.grind` and the
diff is line-by-line readable:

```console
$ grind build examples/timesheet.rhai -o month.grind
```

which is the point at which the two halves of `doc/dsl.md` meet: a script writes the document,
and the document is reviewable text. `doc/projection-guide.md` is that half.

What is **not** promised: that two different builds of `grind` produce the same bytes for the
same script. A change to the writer will change the output, and that is what loops C and F are
for.

---

## 12. A text document — `examples/report.rhai`

The same arrow, the other application. A text document is a flat sequence of blocks, so the
vocabulary is smaller by exactly that much:

```rhai
const REGIONS = [
    ["North", "led on volume", 12],
    ["South", "held flat", 0],
];

let d = text();
d.heading(1, "Quarterly report");
d.para("Revenue rose **12%** on the year, with growth concentrated in two regions.");
d.bookmark("summary");

d.heading(2, "By region");
for r in REGIONS {
    d.item(1, "**" + r[0] + "** " + r[1] + " (" + r[2] + "%)");
}
d
```

```console
$ grind build examples/report.rhai -o report.fodt
9 blocks	3 headings	78 words	444 characters
#summary
```

`para`, `heading(level, …)`, `item(depth, …)`, `bookmark(name)`, `blocks`. The inline notation
is `grind_text::markdown`'s — `**bold**`, `_italic_`, `` `code` `` mean here exactly what they
mean while typing in any shell of this suite, because there is one reader for them. A bookmark
is an address that survives editing above it, which `p12` does not.

---

## 13. What a script cannot say

These are **gaps, not refusals**: every one is reachable from the CLI, so a script writes the
document and then `grind` runs on it.

| Not sayable | Instead |
|---|---|
| a date or time *value* | `=DATE(2026;8;16)` under `format("date")` (§3) |
| charts | `grind sheet chart-add` |
| filters, hidden rows and columns | `grind sheet filter` / `hide` |
| the null date, the null year | — |
| character formatting beyond the notation — a font, a size, a colour on a run | `grind text format` — it reaches the model through a *caret*, and a script says blocks |
| images | `grind text image` |
| named paragraph styles | — a generator cannot *declare* one, so every block using it would be a `grind lint` `undeclared-style` finding |

And the two that are not gaps but the design: **a script cannot read the document being built**,
and **a document cannot be turned back into a script**. The arrow points one way, as it does for
Typst and for Jsonnet.

---

## 14. Setting up an editor

Everything a script may say is available to an editor, because Rhai has a format for it and this
build generates it from the engine itself:

```console
$ grind definitions > grind.d.rhai              # the Rhai language server's own format
$ grind definitions --snippets > .vscode/grind.code-snippets
```

Both are the *engine's* answer, so neither can describe a function that is not there — and
registering a function takes its documentation as an argument, so an undocumented one cannot be
added by forgetting. **`doc/editor-setup.md`** is the setup, including which of the two files
an editor of yours can actually use, and what was measured to find out.

---

## 15. Where to go next

* **`doc/generator-spec.md`** — the reference. Every function with its arguments, every limit
  with its value, the two-pass materialisation order, and what determinism rests on. For
  arguing with an edge case rather than for learning.
* **`doc/projection-guide.md`** — the other layer: a document as plain text, hand-written and
  reviewable. A generator can write one.
* **`examples/grind.d.rhai`** — the whole vocabulary in one file, generated and kept current by
  a test.
* **`doc/dsl.md` §4** — why there is a generator at all, why Rhai, and why this is on the right
  side of the macro line.
